// k04-modules-v0.0.55: hot-retry PMW3610 trackball task with bounded motion backlog.
// RMK 0.8.2's generated Pmw3610Device stops forever after failed init, so K04
// owns the sensor task here and keeps probing for replaceable modules.

use embassy_nrf::gpio::{Flex, Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::Peri;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use rmk::driver::bitbang_spi::BitBangSpiBus;
use rmk::event::Event;
use rmk::input_device::pmw3610::{Pmw3610, Pmw3610Config};

use crate::vial_settings::{
    ball_cpi, claim_pointing_module, kind_index, pointing_module_claimed_by_other,
    pointing_module_enabled, release_pointing_module, side_index,
    wait_pointing_module_selection_change, ModuleKind, ModuleSide,
};

const PROBE_INTERVAL_MS: u64 = 250;
const MOTION_WAIT_CHECK_MS: u64 = 100;
// Match Ergohaven ZMK PMW3610 report cadence to reduce BLE mouse traffic.
const REPORT_INTERVAL_MS: u32 = 12;
const MOTION_ACCUM_LIMIT: i32 = (i8::MAX as i32) * 2;
const CUSTOM_MAGIC: [u8; 4] = *b"K04P";
const CUSTOM_MOTION: u8 = 3;

pub type K04Trackball =
    Pmw3610<BitBangSpiBus<Output<'static>, Flex<'static>>, Output<'static>, Input<'static>>;

pub fn new_trackball(
    sck: Output<'static>,
    sdio: Flex<'static>,
    cs: Output<'static>,
    motion: Input<'static>,
) -> K04Trackball {
    let spi = BitBangSpiBus::new(sck, sdio);
    let config = Pmw3610Config {
        res_cpi: 1000,
        swap_xy: true,
        invert_x: false,
        invert_y: false,
        force_awake: false,
        smart_mode: true,
    };
    Pmw3610::new(spi, cs, Some(motion), config)
}

pub fn new_trackball_from_pins(
    sck: Peri<'static, embassy_nrf::peripherals::P0_01>,
    sdio: Peri<'static, embassy_nrf::peripherals::P0_00>,
    cs: Peri<'static, embassy_nrf::peripherals::P0_05>,
    motion: Peri<'static, embassy_nrf::peripherals::P1_09>,
) -> K04Trackball {
    new_trackball(
        Output::new(sck, Level::High, OutputDrive::Standard),
        Flex::new(sdio),
        Output::new(cs, Level::High, OutputDrive::Standard),
        Input::new(motion, Pull::Up),
    )
}

#[embassy_executor::task]
pub async fn trackball_task(mut trackball: K04Trackball, side: ModuleSide) {
    let mut ready = false;
    let mut acc_x = 0i32;
    let mut acc_y = 0i32;
    let mut last_report_ms = now_ms();
    let mut applied_cpi = 0u16;

    loop {
        if !pointing_module_enabled(side, ModuleKind::Ball) {
            release_pointing_module(ModuleKind::Ball);
            ready = false;
            acc_x = 0;
            acc_y = 0;
            wait_pointing_module_selection_change(side, ModuleKind::Ball).await;
            continue;
        }

        if pointing_module_claimed_by_other(ModuleKind::Ball) {
            ready = false;
            acc_x = 0;
            acc_y = 0;
            Timer::after(Duration::from_millis(50)).await;
            continue;
        }

        if !ready {
            release_pointing_module(ModuleKind::Ball);
            match trackball.init().await {
                Ok(()) => {
                    if !claim_pointing_module(ModuleKind::Ball) {
                        Timer::after(Duration::from_millis(50)).await;
                        continue;
                    }
                    applied_cpi = ball_cpi(side);
                    let _ = trackball.set_resolution(applied_cpi).await;
                    ready = true;
                    ::defmt::info!("K04 PMW3610 initialized");
                }
                Err(_e) => {
                    release_pointing_module(ModuleKind::Ball);
                    Timer::after(Duration::from_millis(PROBE_INTERVAL_MS)).await;
                    continue;
                }
            }
        }

        let configured_cpi = ball_cpi(side);
        if configured_cpi != applied_cpi {
            if trackball.set_resolution(configured_cpi).await.is_ok() {
                applied_cpi = configured_cpi;
            }
        }

        let _ = with_timeout(
            Duration::from_millis(MOTION_WAIT_CHECK_MS),
            trackball.wait_for_motion(),
        )
        .await;
        if !pointing_module_enabled(side, ModuleKind::Ball) {
            release_pointing_module(ModuleKind::Ball);
            ready = false;
            acc_x = 0;
            acc_y = 0;
            continue;
        }

        while trackball.motion_pending() {
            match trackball.read_motion().await {
                Ok(motion) => {
                    acc_x = clamp_motion_accum(acc_x.saturating_add(motion.dx as i32));
                    acc_y = clamp_motion_accum(acc_y.saturating_add(motion.dy as i32));

                    let now = now_ms();
                    if now.wrapping_sub(last_report_ms) >= REPORT_INTERVAL_MS
                        && send_accumulated_motion(&mut acc_x, &mut acc_y, side).await
                    {
                        last_report_ms = now;
                    }
                }
                Err(_e) => {
                    send_accumulated_motion(&mut acc_x, &mut acc_y, side).await;
                    acc_x = 0;
                    acc_y = 0;
                    ready = false;
                    release_pointing_module(ModuleKind::Ball);
                    Timer::after(Duration::from_millis(PROBE_INTERVAL_MS)).await;
                    continue;
                }
            }
        }

        let now = now_ms();
        if now.wrapping_sub(last_report_ms) >= REPORT_INTERVAL_MS
            && send_accumulated_motion(&mut acc_x, &mut acc_y, side).await
        {
            last_report_ms = now;
        }
    }
}

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}

fn clamp_motion_accum(value: i32) -> i32 {
    value.clamp(-MOTION_ACCUM_LIMIT, MOTION_ACCUM_LIMIT)
}

async fn send_accumulated_motion(acc_x: &mut i32, acc_y: &mut i32, side: ModuleSide) -> bool {
    if *acc_x == 0 && *acc_y == 0 {
        return false;
    }

    let report_x = (*acc_x).clamp(i8::MIN as i32, i8::MAX as i32) as i16;
    let report_y = (*acc_y).clamp(i8::MIN as i32, i8::MAX as i32) as i16;
    let event = custom_motion(side, ModuleKind::Ball, report_x, report_y);

    if send_motion_event_latest(event) {
        *acc_x -= report_x as i32;
        *acc_y -= report_y as i32;
        true
    } else {
        false
    }
}

fn send_motion_event_latest(event: Event) -> bool {
    match rmk::channel::EVENT_CHANNEL.try_send(event) {
        Ok(()) => true,
        Err(rmk::channel::channel::TrySendError::Full(event)) => {
            let _ = rmk::channel::EVENT_CHANNEL.try_receive();
            rmk::channel::EVENT_CHANNEL.try_send(event).is_ok()
        }
    }
}

fn custom_motion(side: ModuleSide, kind: ModuleKind, x: i16, y: i16) -> Event {
    let mut data = [0u8; 16];
    data[0..4].copy_from_slice(&CUSTOM_MAGIC);
    data[4] = CUSTOM_MOTION;
    data[5] = side_index(side);
    data[6] = kind_index(kind);
    data[7..9].copy_from_slice(&x.to_be_bytes());
    data[9..11].copy_from_slice(&y.to_be_bytes());
    Event::Custom(data)
}
