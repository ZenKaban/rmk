//! Ergohaven split battery (AIN7/P0_31, 1564k/2370k).
//!
//! Stock RMK `battery_adc_pin` is not used: its NrfAdc path calls
//! `calibrate().await`, which can hang under SoftDevice and leave the dongle
//! on "??" forever. This reader skips calibrate, times out samples, and
//! re-publishes BatteryStatusEvent every 2s so the split loop can forward.

use embassy_futures::select::select;
use embassy_nrf::interrupt;
use embassy_nrf::interrupt::InterruptExt;
use embassy_nrf::peripherals::{P0_31, SAADC};
use embassy_nrf::saadc::{self, Input as _, Saadc};
use embassy_nrf::{bind_interrupts, Peri};
use embassy_time::{with_timeout, Duration, Timer};
use rmk::core_traits::Runnable;
use rmk::event::{
    publish_event, BatteryStatusEvent, EventSubscriber, PeripheralBatteryRefreshEvent,
    SubscribableEvent,
};
use rmk::processor::Processor;
use rmk::types::battery::{BatteryStatus, ChargeState};

bind_interrupts!(struct SaadcIrqs {
    SAADC => saadc::InterruptHandler;
});

const MEASURED: i32 = 1564;
const TOTAL: i32 = 2370;

fn percent(val: u16) -> u8 {
    let val = val as i32;
    if val > 4755 * MEASURED / TOTAL {
        100
    } else if val < 4055 * MEASURED / TOTAL {
        0
    } else {
        ((val * TOTAL / MEASURED - 4055) / 7) as u8
    }
}

pub struct K04Battery {
    saadc: Saadc<'static, 1>,
}

impl K04Battery {
    pub fn new(saadc: Peri<'static, SAADC>, pin: Peri<'static, P0_31>) -> Self {
        interrupt::SAADC.set_priority(interrupt::Priority::P3);
        let mut channel = saadc::ChannelConfig::single_ended(pin.degrade_saadc());
        // K:04 uses a high-impedance battery divider. The default 10 us
        // acquisition can leave the SAADC sample capacitor biased high on
        // some halves, which shows up as a sticky 100%.
        channel.time = saadc::Time::_40US;
        Self {
            saadc: Saadc::new(saadc, SaadcIrqs, saadc::Config::default(), [channel]),
        }
    }

    async fn sample_raw(&mut self) -> Option<u16> {
        let mut sum = 0u32;
        let mut count = 0u32;

        for index in 0..5 {
            let mut buf = [0i16; 1];
            if with_timeout(Duration::from_millis(200), self.saadc.sample(&mut buf))
                .await
                .is_ok()
                && index > 0
            {
                sum += buf[0].max(0) as u32;
                count += 1;
            }
            Timer::after(Duration::from_millis(2)).await;
        }

        (count > 0).then_some((sum / count) as u16)
    }

    async fn publish_sample(&mut self) {
        let status = match self.sample_raw().await {
            Some(raw) => BatteryStatus::Available {
                charge_state: ChargeState::Unknown,
                level: Some(percent(raw)),
            },
            None => BatteryStatus::Unavailable,
        };
        publish_event(BatteryStatusEvent(status));
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

impl Runnable for K04Battery {
    async fn run(&mut self) -> ! {
        Timer::after(Duration::from_millis(1000)).await;
        let mut refresh_sub = PeripheralBatteryRefreshEvent::subscriber();
        loop {
            self.publish_sample().await;
            let _ = select(
                Timer::after(Duration::from_secs(2)),
                refresh_sub.next_event(),
            )
            .await;
        }
    }
}

impl Processor for K04Battery {
    type Event = NeverEvent;
    fn subscriber() -> impl EventSubscriber<Event = NeverEvent> {
        NeverSub
    }
    async fn process(&mut self, _: NeverEvent) {}
    async fn process_loop(&mut self) -> ! {
        self.run().await
    }
}
