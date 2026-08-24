use embassy_futures::select::{select, select3, Either, Either3};
use embassy_nrf::gpio::{Flex, Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::Peri;
use embassy_time::{Duration, Instant, Timer};
use rmk::core_traits::Runnable;
use rmk::driver::bitbang_spi::BitBangSpiBus;
use rmk::event::{publish_event, Axis, AxisEvent, AxisValType, ConnectionType, EventSubscriber, PointingEvent};
use rmk::input_device::pmw3610::{Pmw3610, Pmw3610Config};
use rmk::input_device::pointing::PointingDriver;
use rmk::processor::Processor;

use crate::module_settings;

const FAST_PROBE_INTERVAL: Duration = Duration::from_millis(250);
const SLOW_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const FAST_PROBE_WINDOW: Duration = Duration::from_secs(10);
// MOTION wakes the task immediately. A connected sensor only needs a sparse
// identity check; reading its registers every second prevents deep rest.
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(60);
// A local central can report at 250 Hz over USB. BLE and every split
// peripheral stay at 125 Hz so slower host/split transports cannot build a
// FIFO of stale mouse reports.
const USB_REPORT_INTERVAL: Duration = Duration::from_millis(4);
const TRANSPORT_SAFE_REPORT_INTERVAL: Duration = Duration::from_millis(8);
const MOTION_ACCUM_LIMIT: i32 = (i8::MAX as i32) * 2;
const MAX_MOTION_READS_PER_WAKE: usize = 16;
const SLEEP_MOTION_THRESHOLD: u32 = 2;
const SLEEP_MOTION_WINDOW: Duration = Duration::from_millis(20);
const DEFAULT_CPI: u16 = 1000;

pub type K04Trackball = Pmw3610<BitBangSpiBus<Output<'static>, Flex<'static>>, Output<'static>, Input<'static>>;

pub fn new_trackball(
    id: u8,
    sck: Output<'static>,
    sdio: Flex<'static>,
    cs: Output<'static>,
    motion: Input<'static>,
) -> K04Trackball {
    let spi = BitBangSpiBus::new(sck, sdio);
    let config = Pmw3610Config {
        res_cpi: DEFAULT_CPI as i16,
        swap_xy: true,
        invert_x: false,
        invert_y: false,
        force_awake: false,
        smart_mode: true,
    };
    Pmw3610::new(id, spi, cs, Some(motion), config)
}

pub fn new_trackball_from_pins(
    id: u8,
    sck: Peri<'static, embassy_nrf::peripherals::P0_01>,
    sdio: Peri<'static, embassy_nrf::peripherals::P0_00>,
    cs: Peri<'static, embassy_nrf::peripherals::P0_05>,
    motion: Peri<'static, embassy_nrf::peripherals::P1_09>,
) -> K04Trackball {
    new_trackball(
        id,
        Output::new(sck, Level::High, OutputDrive::Standard),
        Flex::new(sdio),
        Output::new(cs, Level::High, OutputDrive::Standard),
        Input::new(motion, Pull::Up),
    )
}

pub struct Trackball {
    trackball: K04Trackball,
    device_id: u8,
    is_central: bool,
    ready: bool,
    acc_x: i32,
    acc_y: i32,
    last_report: Instant,
    sleep_motion_deadline: Option<Instant>,
    next_probe: Instant,
    next_health_check: Instant,
    unavailable_since: Option<Instant>,
    current_cpi: u16,
}

impl Trackball {
    // This source is shared by separate central and peripheral binaries, so
    // each binary intentionally leaves one constructor unused.
    #[allow(dead_code)]
    pub fn new_central(trackball: K04Trackball, device_id: u8) -> Self {
        Self::new(trackball, device_id, true)
    }

    #[allow(dead_code)]
    pub fn new_peripheral(trackball: K04Trackball, device_id: u8) -> Self {
        Self::new(trackball, device_id, false)
    }

    fn new(trackball: K04Trackball, device_id: u8, is_central: bool) -> Self {
        Self {
            trackball,
            device_id,
            is_central,
            ready: false,
            acc_x: 0,
            acc_y: 0,
            last_report: Instant::MIN,
            sleep_motion_deadline: None,
            next_probe: Instant::MIN,
            next_health_check: Instant::MIN,
            unavailable_since: None,
            current_cpi: DEFAULT_CPI,
        }
    }

    async fn run_loop(&mut self) -> ! {
        loop {
            let selection = module_settings::module_selection(self.device_id);
            if selection != module_settings::ModuleSelection::Trackball {
                self.deactivate();
                let _ = module_settings::wait_for_module_selection_change(self.device_id, selection).await;
                continue;
            }

            let sleeping = module_settings::module_sleeping();
            if !self.ready {
                // Do not probe an unavailable sensor while the keyboard is
                // asleep. A configured PMW3610 follows MOTION below instead.
                if sleeping {
                    match select(
                        module_settings::wait_for_module_selection_change(
                            self.device_id,
                            module_settings::ModuleSelection::Trackball,
                        ),
                        module_settings::wait_for_module_sleep_change(sleeping),
                    )
                    .await
                    {
                        Either::First(_) => self.deactivate(),
                        Either::Second(_) => self.resume_from_sleep(),
                    }
                    continue;
                }

                let now = Instant::now();
                if now < self.next_probe {
                    match select3(
                        Timer::at(self.next_probe),
                        module_settings::wait_for_module_selection_change(
                            self.device_id,
                            module_settings::ModuleSelection::Trackball,
                        ),
                        module_settings::wait_for_module_sleep_change(sleeping),
                    )
                    .await
                    {
                        Either3::First(_) => {}
                        Either3::Second(_) => {
                            self.deactivate();
                            continue;
                        }
                        Either3::Third(next_sleeping) => {
                            if next_sleeping {
                                self.park_for_sleep();
                            } else {
                                self.resume_from_sleep();
                            }
                            continue;
                        }
                    }
                }

                // Avoid initializing in the small race between the retry
                // deadline and a sleep-state update.
                if module_settings::module_sleeping() {
                    self.park_for_sleep();
                    continue;
                }

                if self.trackball.init().await.is_ok() {
                    let now = Instant::now();
                    self.current_cpi = module_settings::ball_cpi(self.device_id);
                    let _ = self.trackball.set_resolution(self.current_cpi).await;
                    self.ready = true;
                    self.acc_x = 0;
                    self.acc_y = 0;
                    self.last_report = now;
                    self.next_health_check = now + HEALTH_CHECK_INTERVAL;
                    self.unavailable_since = None;
                } else {
                    self.mark_unavailable(Instant::now());
                    continue;
                }
            }

            let sleeping = module_settings::module_sleeping();
            if !sleeping {
                self.apply_configured_cpi().await;
            }
            let report_interval = self.report_interval();
            let report_deadline = self
                .has_reportable_motion(sleeping)
                .then_some(self.last_report + report_interval);
            let deadline = if sleeping {
                match (report_deadline, self.sleep_motion_deadline) {
                    (Some(report), Some(noise)) => Some(report.min(noise)),
                    (Some(report), None) => Some(report),
                    (None, noise) => noise,
                }
            } else {
                Some(
                    report_deadline
                        .map(|report| report.min(self.next_health_check))
                        .unwrap_or(self.next_health_check),
                )
            };

            let motion_or_deadline = async {
                match (self.trackball.motion_gpio(), deadline) {
                    (Some(gpio), Some(deadline)) => {
                        matches!(select(gpio.wait_for_low(), Timer::at(deadline)).await, Either::First(_))
                    }
                    (Some(gpio), None) => {
                        gpio.wait_for_low().await;
                        true
                    }
                    (None, Some(deadline)) => {
                        Timer::at(deadline).await;
                        false
                    }
                    (None, None) => core::future::pending::<bool>().await,
                }
            };
            let motion_woke = match select3(
                motion_or_deadline,
                module_settings::wait_for_module_selection_change(
                    self.device_id,
                    module_settings::ModuleSelection::Trackball,
                ),
                module_settings::wait_for_module_sleep_change(sleeping),
            )
            .await
            {
                Either3::First(motion_woke) => motion_woke,
                Either3::Second(_) => {
                    self.deactivate();
                    continue;
                }
                Either3::Third(next_sleeping) => {
                    if next_sleeping {
                        self.park_for_sleep();
                    } else {
                        self.resume_from_sleep();
                    }
                    continue;
                }
            };

            if motion_woke {
                let mut reads = 0usize;
                while reads < MAX_MOTION_READS_PER_WAKE && self.trackball.motion_pending() {
                    reads += 1;
                    match self.trackball.read_motion().await {
                        Ok(motion) => {
                            let now = Instant::now();
                            if sleeping && self.acc_x == 0 && self.acc_y == 0 && (motion.dx != 0 || motion.dy != 0) {
                                self.sleep_motion_deadline = Some(now + SLEEP_MOTION_WINDOW);
                            }
                            self.acc_x = clamp_motion_accum(self.acc_x.saturating_add(motion.dx as i32));
                            self.acc_y = clamp_motion_accum(self.acc_y.saturating_add(motion.dy as i32));
                            if self.has_reportable_motion(sleeping)
                                && now.duration_since(self.last_report) >= report_interval
                            {
                                self.send_accumulated_motion();
                                self.last_report = now;
                            }
                        }
                        Err(_) => {
                            self.mark_unavailable(Instant::now());
                            break;
                        }
                    }
                }
                if reads == MAX_MOTION_READS_PER_WAKE && self.trackball.motion_pending() {
                    // A stuck-low/noisy MOTION line must not monopolize the
                    // executor. The next pass resumes immediately if data is
                    // still pending.
                    Timer::after(Duration::from_micros(50)).await;
                }
            }

            if !self.ready {
                continue;
            }

            let now = Instant::now();
            if !sleeping && now >= self.next_health_check {
                self.next_health_check = now + HEALTH_CHECK_INTERVAL;
                if !self.trackball.is_configured().await {
                    self.mark_unavailable(now);
                    continue;
                }
                self.apply_configured_cpi().await;
            }

            if sleeping && self.sleep_motion_deadline.is_some_and(|deadline| now >= deadline) {
                if self.has_reportable_motion(true) {
                    self.send_accumulated_motion();
                    self.last_report = now;
                } else {
                    // A single +/-1 sample that was not followed by motion in
                    // the short confirmation window is settling noise.
                    self.acc_x = 0;
                    self.acc_y = 0;
                    self.sleep_motion_deadline = None;
                }
            } else if self.has_reportable_motion(sleeping) && now.duration_since(self.last_report) >= report_interval {
                self.send_accumulated_motion();
                self.last_report = now;
            }
        }
    }

    fn has_reportable_motion(&self, sleeping: bool) -> bool {
        if sleeping {
            self.acc_x.unsigned_abs() >= SLEEP_MOTION_THRESHOLD || self.acc_y.unsigned_abs() >= SLEEP_MOTION_THRESHOLD
        } else {
            self.acc_x != 0 || self.acc_y != 0
        }
    }

    fn park_for_sleep(&mut self) {
        // PMW3610 enters Rest3 on its own when force-awake is disabled. Stop
        // timer-driven SPI traffic and leave only the MOTION line armed.
        self.acc_x = 0;
        self.acc_y = 0;
        self.last_report = Instant::now();
        self.sleep_motion_deadline = None;
        self.next_health_check = Instant::MIN;
    }

    fn resume_from_sleep(&mut self) {
        // Discard a lone +/-1 sample retained while sleeping. Deliberate
        // accumulated motion has already been published to trigger the wake.
        if !self.has_reportable_motion(true) {
            self.acc_x = 0;
            self.acc_y = 0;
        }
        self.sleep_motion_deadline = None;
        if self.ready {
            self.next_health_check = Instant::now() + HEALTH_CHECK_INTERVAL;
        }
    }

    fn report_interval(&self) -> Duration {
        if self.is_central
            && matches!(
                rmk::state::current_connection_status().decide_active(),
                Some(ConnectionType::Usb)
            )
        {
            USB_REPORT_INTERVAL
        } else {
            TRANSPORT_SAFE_REPORT_INTERVAL
        }
    }

    async fn apply_configured_cpi(&mut self) {
        let configured_cpi = module_settings::ball_cpi(self.device_id);
        if configured_cpi != self.current_cpi && self.trackball.set_resolution(configured_cpi).await.is_ok() {
            self.current_cpi = configured_cpi;
        }
    }

    fn mark_unavailable(&mut self, now: Instant) {
        self.ready = false;
        let unavailable_since = *self.unavailable_since.get_or_insert(now);
        let retry_interval = if now.duration_since(unavailable_since) < FAST_PROBE_WINDOW {
            FAST_PROBE_INTERVAL
        } else {
            SLOW_PROBE_INTERVAL
        };
        self.next_probe = now + retry_interval;
        self.next_health_check = Instant::MIN;
        self.acc_x = 0;
        self.acc_y = 0;
        self.sleep_motion_deadline = None;
    }

    fn deactivate(&mut self) {
        self.ready = false;
        self.acc_x = 0;
        self.acc_y = 0;
        self.last_report = Instant::MIN;
        self.sleep_motion_deadline = None;
        self.next_probe = Instant::MIN;
        self.next_health_check = Instant::MIN;
        self.unavailable_since = None;
    }

    fn send_accumulated_motion(&mut self) {
        if self.acc_x == 0 && self.acc_y == 0 {
            return;
        }

        let report_x = self.acc_x.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
        let report_y = self.acc_y.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
        self.acc_x -= report_x as i32;
        self.acc_y -= report_y as i32;
        if self.acc_x == 0 && self.acc_y == 0 {
            self.sleep_motion_deadline = None;
        }

        publish_event(PointingEvent {
            device_id: self.device_id,
            axes: [
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::X,
                    value: report_x,
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Y,
                    value: report_y,
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Z,
                    value: 0,
                },
            ],
        });
    }
}

struct NeverSub;
pub struct NeverEvent;

impl EventSubscriber for NeverSub {
    type Event = NeverEvent;

    async fn next_event(&mut self) -> NeverEvent {
        core::future::pending().await
    }
}

impl Runnable for Trackball {
    async fn run(&mut self) -> ! {
        self.run_loop().await
    }
}

impl Processor for Trackball {
    type Event = NeverEvent;

    fn subscriber() -> impl EventSubscriber<Event = NeverEvent> {
        NeverSub
    }

    async fn process(&mut self, _: NeverEvent) {}

    async fn process_loop(&mut self) -> ! {
        self.run().await
    }
}

fn clamp_motion_accum(value: i32) -> i32 {
    value.clamp(-MOTION_ACCUM_LIMIT, MOTION_ACCUM_LIMIT)
}
