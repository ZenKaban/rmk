use embassy_futures::select::{select, Either};
use embassy_nrf::gpio::{Flex, Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::Peri;
use embassy_time::{Duration, Instant, Timer};
use rmk::core_traits::Runnable;
use rmk::driver::bitbang_spi::BitBangSpiBus;
use rmk::event::{publish_event, Axis, AxisEvent, AxisValType, EventSubscriber, PointingEvent};
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
const REPORT_INTERVAL: Duration = Duration::from_millis(12);
const MOTION_ACCUM_LIMIT: i32 = (i8::MAX as i32) * 2;
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
    ready: bool,
    acc_x: i32,
    acc_y: i32,
    last_report: Instant,
    next_probe: Instant,
    next_health_check: Instant,
    unavailable_since: Option<Instant>,
    current_cpi: u16,
}

impl Trackball {
    pub fn new(trackball: K04Trackball, device_id: u8) -> Self {
        Self {
            trackball,
            device_id,
            ready: false,
            acc_x: 0,
            acc_y: 0,
            last_report: Instant::MIN,
            next_probe: Instant::MIN,
            next_health_check: Instant::MIN,
            unavailable_since: None,
            current_cpi: DEFAULT_CPI,
        }
    }

    async fn run_loop(&mut self) -> ! {
        loop {
            if !self.ready {
                let now = Instant::now();
                if now < self.next_probe {
                    Timer::at(self.next_probe).await;
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

            self.apply_configured_cpi().await;
            let report_deadline = (self.acc_x != 0 || self.acc_y != 0).then_some(self.last_report + REPORT_INTERVAL);
            let deadline = report_deadline
                .map(|report| report.min(self.next_health_check))
                .unwrap_or(self.next_health_check);

            let motion_woke = if let Some(gpio) = self.trackball.motion_gpio() {
                matches!(select(gpio.wait_for_low(), Timer::at(deadline)).await, Either::First(_))
            } else {
                Timer::at(deadline).await;
                false
            };

            if motion_woke {
                while self.trackball.motion_pending() {
                    match self.trackball.read_motion().await {
                        Ok(motion) => {
                            self.acc_x = clamp_motion_accum(self.acc_x.saturating_add(motion.dx as i32));
                            self.acc_y = clamp_motion_accum(self.acc_y.saturating_add(motion.dy as i32));
                        }
                        Err(_) => {
                            self.mark_unavailable(Instant::now());
                            break;
                        }
                    }
                }
            }

            if !self.ready {
                continue;
            }

            let now = Instant::now();
            if now >= self.next_health_check {
                self.next_health_check = now + HEALTH_CHECK_INTERVAL;
                if !self.trackball.is_configured().await {
                    self.mark_unavailable(now);
                    continue;
                }
                self.apply_configured_cpi().await;
            }

            if (self.acc_x != 0 || self.acc_y != 0) && now.duration_since(self.last_report) >= REPORT_INTERVAL {
                self.send_accumulated_motion();
                self.last_report = now;
            }
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
    }

    fn send_accumulated_motion(&mut self) {
        if self.acc_x == 0 && self.acc_y == 0 {
            return;
        }

        let report_x = self.acc_x.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
        let report_y = self.acc_y.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
        self.acc_x -= report_x as i32;
        self.acc_y -= report_y as i32;

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
