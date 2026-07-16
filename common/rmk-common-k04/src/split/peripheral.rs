#[cfg(feature = "_ble")]
use bt_hci::{cmd::le::LeSetPhy, controller::ControllerCmdAsync};
use embassy_futures::select::select3;
#[cfg(not(feature = "_ble"))]
use embedded_io_async::{Read, Write};
#[cfg(all(feature = "_ble", feature = "storage"))]
use {super::ble::PeerAddress, crate::channel::FLASH_CHANNEL};
#[cfg(feature = "controller")]
use {
    crate::channel::{CONTROLLER_CHANNEL, send_controller_event},
    crate::event::ControllerEvent,
};
#[cfg(feature = "_ble")]
use {crate::storage::Storage, embedded_storage_async::nor_flash::NorFlash, trouble_host::prelude::*};

use super::SplitMessage;
use super::driver::{SplitReader, SplitWriter};
use crate::CONNECTION_STATE;
use crate::channel::{EVENT_CHANNEL, KEY_EVENT_CHANNEL};
#[cfg(not(feature = "_ble"))]
use crate::split::serial::SerialSplitDriver;
use crate::state::ConnectionState;

/// Run the split peripheral service.
///
/// # Arguments
///
/// * `id` - (optional) The id of the peripheral
/// * `stack` - (optional) The TrouBLE stack
/// * `serial` - (optional) serial port used to send peripheral split message. This argument is enabled only for serial split now
/// * `storage` - (optional) The storage to save the central address
pub async fn run_rmk_split_peripheral<
    'a,
    #[cfg(feature = "_ble")] C: Controller + ControllerCmdAsync<LeSetPhy>,
    #[cfg(not(feature = "_ble"))] S: Write + Read,
    #[cfg(feature = "_ble")] F: NorFlash,
    #[cfg(feature = "_ble")] const ROW: usize,
    #[cfg(feature = "_ble")] const COL: usize,
    #[cfg(feature = "_ble")] const NUM_LAYER: usize,
    #[cfg(feature = "_ble")] const NUM_ENCODER: usize,
>(
    #[cfg(feature = "_ble")] id: usize,
    #[cfg(feature = "_ble")] stack: &'a Stack<'a, C, DefaultPacketPool>,
    #[cfg(feature = "_ble")] storage: &mut Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>,
    #[cfg(not(feature = "_ble"))] serial: S,
) {
    #[cfg(not(feature = "_ble"))]
    {
        let mut peripheral = SplitPeripheral::new(SerialSplitDriver::new(serial));
        loop {
            peripheral.run().await;
        }
    }

    #[cfg(feature = "_ble")]
    crate::split::ble::peripheral::initialize_nrf_ble_split_peripheral_and_run(id, stack, storage).await;
}

/// The split peripheral instance.
pub(crate) struct SplitPeripheral<S: SplitWriter + SplitReader> {
    split_driver: S,
}

#[cfg(all(feature = "_ble", feature = "controller"))]
fn k04_battery_percent_from_adc(val: u16) -> u8 {
    const ADC_DIVIDER_MEASURED: i32 = 782;
    const ADC_DIVIDER_TOTAL: i32 = 1185;
    let val = val as i32;
    if val > 4755_i32 * ADC_DIVIDER_MEASURED / ADC_DIVIDER_TOTAL {
        100
    } else if val < 4055_i32 * ADC_DIVIDER_MEASURED / ADC_DIVIDER_TOTAL {
        0
    } else {
        ((val * ADC_DIVIDER_TOTAL / ADC_DIVIDER_MEASURED - 4055) / 7) as u8
    }
}

impl<S: SplitWriter + SplitReader> SplitPeripheral<S> {
    pub(crate) fn new(split_driver: S) -> Self {
        Self { split_driver }
    }

    /// Run the peripheral keyboard service.
    ///
    /// The peripheral uses the general matrix, does scanning and send the key events through `SplitWriter`.
    /// If also receives split messages from the central through `SplitReader`.
    pub(crate) async fn run(&mut self) -> bool {
        CONNECTION_STATE.store(ConnectionState::Connected.into(), core::sync::atomic::Ordering::Release);

        #[cfg(feature = "controller")]
        let mut controller_pub = unwrap!(CONTROLLER_CHANNEL.publisher());

        loop {
            match select3(
                self.split_driver.read(),
                KEY_EVENT_CHANNEL.receive(),
                EVENT_CHANNEL.receive(),
            )
            .await
            {
                embassy_futures::select::Either3::First(m) => match m {
                    // Currently only handle the central state message
                    Ok(split_message) => match split_message {
                        SplitMessage::ConnectionState(state) => {
                            trace!("Received connection state update: {}", state);
                            CONNECTION_STATE.store(state, core::sync::atomic::Ordering::Release);
                        }
                        #[cfg(all(feature = "_ble", feature = "storage"))]
                        SplitMessage::ClearPeer => {
                            // Clear the peer address
                            FLASH_CHANNEL
                                .send(crate::storage::FlashOperationMessage::PeerAddress(PeerAddress::new(
                                    0, false, [0; 6],
                                )))
                                .await;
                            CONNECTION_STATE.store(false, core::sync::atomic::Ordering::Release);
                            return true;
                        }
                        SplitMessage::KeyboardIndicator(indicator) => {
                            // Publish KeyboardIndicator to CONTROLLER_CHANNEL
                            #[cfg(feature = "controller")]
                            send_controller_event(
                                &mut controller_pub,
                                ControllerEvent::KeyboardIndicator(rmk_types::led_indicator::LedIndicator::from_bits(
                                    indicator,
                                )),
                            );
                        }
                        SplitMessage::Layer(layer) => {
                            // Publish Layer to CONTROLLER_CHANNEL
                            #[cfg(feature = "controller")]
                            send_controller_event(&mut controller_pub, ControllerEvent::Layer(layer));
                        }
                        SplitMessage::DeviceSettings(settings) => {
                            #[cfg(feature = "controller")]
                            send_controller_event(&mut controller_pub, ControllerEvent::DeviceSettings(settings));
                        }
                        SplitMessage::BatteryLevelRequest => {
                            #[cfg(feature = "_nrf_ble")]
                            crate::input_device::adc::request_battery_adc_sample();
                            #[cfg(feature = "controller")]
                            send_controller_event(&mut controller_pub, ControllerEvent::BatteryLevelRequest);
                        }
                        SplitMessage::ProductId(_central_product_id) => {
                            self.split_driver.write(&SplitMessage::ProductId(crate::SPLIT_PRODUCT_ID)).await.ok();
                        }
                        _ => (),
                    },
                    Err(e) => {
                        error!("Split message read error: {:?}", e);
                        if let crate::split::driver::SplitDriverError::Disconnected = e {
                            break;
                        }
                    }
                },
                embassy_futures::select::Either3::Second(e) => {
                    #[cfg(feature = "_ble")]
                    crate::channel::signal_activity();
                    // Only send the key event if the connection is established
                    if CONNECTION_STATE.load(core::sync::atomic::Ordering::Acquire) {
                        debug!("Writing split key event to central");
                        self.split_driver.write(&SplitMessage::Key(e)).await.ok();
                    } else {
                        debug!("Connection not established, skipping key event");
                    }
                }
                embassy_futures::select::Either3::Third(e) => match e {
                    #[cfg(all(feature = "_ble", feature = "controller"))]
                    crate::event::Event::Battery(val) => {
                        let battery_percent = k04_battery_percent_from_adc(val);
                        send_controller_event(&mut controller_pub, ControllerEvent::BatteryAdc(val));
                        send_controller_event(&mut controller_pub, ControllerEvent::Battery(battery_percent));
                        self.split_driver
                            .write(&SplitMessage::BatteryLevel(battery_percent))
                            .await
                            .ok();
                    }
                    e => {
                        #[cfg(feature = "_ble")]
                        crate::channel::signal_activity();
                        if CONNECTION_STATE.load(core::sync::atomic::Ordering::Acquire) {
                            debug!("Writing split event to central: {:?}", e);
                            self.split_driver.write(&SplitMessage::Event(e)).await.ok();
                        } else {
                            debug!("Connection not established, skipping event");
                        }
                    }
                },
            }
        }
        false
    }
}
