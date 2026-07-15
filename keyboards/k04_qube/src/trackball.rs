use embassy_nrf::gpio::{Flex, Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::Peri;
use embassy_time::{Duration, Instant, Timer};
use rmk::driver::bitbang_spi::BitBangSpiBus;
use rmk::event::{publish_event, Axis, AxisEvent, AxisValType, LayerChangeEvent, PointingEvent};
use rmk::input_device::pmw3610::{Pmw3610, Pmw3610Config};
use rmk::input_device::pointing::PointingDriver;
use rmk::macros::processor;

use crate::module_settings;

const PROBE_INTERVAL_MS: u32 = 250;
const REPORT_INTERVAL_MS: u32 = 12;
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

#[processor(subscribe = [LayerChangeEvent], poll_interval = 12)]
pub struct Trackball {
    trackball: K04Trackball,
    device_id: u8,
    ready: bool,
    acc_x: i32,
    acc_y: i32,
    last_report_ms: u32,
    next_probe_ms: u32,
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
            last_report_ms: 0,
            next_probe_ms: 0,
            current_cpi: DEFAULT_CPI,
        }
    }

    async fn on_layer_change_event(&mut self, _event: LayerChangeEvent) {}

    async fn poll(&mut self) {
        if !self.ready {
            let now = now_ms();
            if self.next_probe_ms != 0 && now.wrapping_sub(self.next_probe_ms) < u32::MAX / 2 {
                return;
            }

            match self.trackball.init().await {
                Ok(()) => {
                    self.current_cpi = module_settings::ball_cpi(self.device_id);
                    let _ = self.trackball.set_resolution(self.current_cpi).await;
                    self.ready = true;
                    self.acc_x = 0;
                    self.acc_y = 0;
                    self.last_report_ms = now_ms();
                }
                Err(_) => {
                    self.next_probe_ms = now.wrapping_add(PROBE_INTERVAL_MS);
                    Timer::after(Duration::from_millis(1)).await;
                    return;
                }
            }
        }

        let configured_cpi = module_settings::ball_cpi(self.device_id);
        if configured_cpi != self.current_cpi && self.trackball.set_resolution(configured_cpi).await.is_ok() {
            self.current_cpi = configured_cpi;
        }

        while self.trackball.motion_pending() {
            match self.trackball.read_motion().await {
                Ok(motion) => {
                    self.acc_x = clamp_motion_accum(self.acc_x.saturating_add(motion.dx as i32));
                    self.acc_y = clamp_motion_accum(self.acc_y.saturating_add(motion.dy as i32));
                }
                Err(_) => {
                    self.ready = false;
                    self.next_probe_ms = now_ms().wrapping_add(PROBE_INTERVAL_MS);
                    self.acc_x = 0;
                    self.acc_y = 0;
                    return;
                }
            }
        }

        let now = now_ms();
        if now.wrapping_sub(self.last_report_ms) >= REPORT_INTERVAL_MS {
            self.send_accumulated_motion();
            self.last_report_ms = now;
        }
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

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}

fn clamp_motion_accum(value: i32) -> i32 {
    value.clamp(-MOTION_ACCUM_LIMIT, MOTION_ACCUM_LIMIT)
}
