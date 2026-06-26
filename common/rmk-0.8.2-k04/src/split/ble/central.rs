use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer, with_timeout};
use embedded_storage_async::nor_flash::NorFlash;
use heapless::{Vec, VecView};
use trouble_host::prelude::*;
#[cfg(feature = "controller")]
use {
    crate::channel::{CONTROLLER_CHANNEL, send_controller_event},
    crate::event::ControllerEvent,
};

use crate::ble::{SLEEPING_STATE, update_ble_phy, update_conn_params};
use crate::channel::FLASH_CHANNEL;
#[cfg(feature = "storage")]
use crate::split::ble::PeerAddress;
use crate::split::driver::{PeripheralManager, SplitDriverError, SplitReader, SplitWriter};
use crate::split::{SPLIT_MESSAGE_MAX_SIZE, SplitMessage};
use crate::storage::{FlashOperationMessage, Storage};
use crate::{CONNECTION_STATE, SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS};

pub(crate) static STACK_STARTED: Signal<crate::RawMutex, bool> = Signal::new();
pub(crate) static PERIPHERAL_FOUND: Signal<crate::RawMutex, (u8, BdAddr)> = Signal::new();

// Signals and mutex for syncing scanning state between scanning task and peripheral manager
static START_SCANNING: Signal<crate::RawMutex, ()> = Signal::new();
static STOP_SCANNING: Signal<crate::RawMutex, ()> = Signal::new();
static SCANNING_MUTEX: Mutex<crate::RawMutex, ()> = Mutex::new(());
static CLEAR_PEER_PENDING: AtomicU32 = AtomicU32::new(0);
static UNCOMMITTED_PEER_CANDIDATES: AtomicU32 = AtomicU32::new(0);

/// Sleep management signal for BLE Split Central
///
/// This signal serves dual purposes for sleep management:
/// - `signal(true)`: Indicates central has entered sleep mode
/// - `signal(false)`: Indicates activity detected, wake up or reset sleep timer
pub(crate) static CENTRAL_SLEEP: Signal<crate::RawMutex, bool> = Signal::new();

const SPLIT_SERVICE_UUID: [u8; 16] = [
    // uuid: 4dd5fbaa-18e5-4b07-bf0a-353698659946
    70u8, 153u8, 101u8, 152u8, 54u8, 53u8, 10u8, 191u8, 7u8, 75u8, 229u8, 24u8, 170u8, 251u8, 213u8,
    77u8,
];
const SPLIT_COMPANY_ID: u16 = 0xe118;

const DEFAULT_SLEEP_TIMEOUT_SECONDS: u64 = 30 * 60;
const SLEEP_TIMEOUT_SECONDS: [u64; 10] = [
    10 * 60,
    15 * 60,
    20 * 60,
    30 * 60,
    45 * 60,
    60 * 60,
    2 * 60 * 60,
    3 * 60 * 60,
    4 * 60 * 60,
    5 * 60 * 60,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PeripheralSessionResult {
    KeepPeer,
    ClearPeer,
    ClearPeerDelivered,
    ProductCheckFailed,
}

/// Gatt service used in split central to send split message to peripheral
#[gatt_service(uuid = "4dd5fbaa-18e5-4b07-bf0a-353698659946")]
struct SplitBleCentralService {
    #[characteristic(uuid = "0e6313e3-bd0b-45c2-8d2e-37a2e8128bc3", read, notify)]
    message_to_central: [u8; SPLIT_MESSAGE_MAX_SIZE],

    #[characteristic(uuid = "4b3514fb-cae4-4d38-a097-3a2a3d1c3b9c", write_without_response, read, notify)]
    message_to_peripheral: [u8; SPLIT_MESSAGE_MAX_SIZE],
}

/// Gatt server in split peripheral
#[gatt_server]
struct BleSplitCentralServer {
    service: SplitBleCentralService,
}

pub async fn scan_peripherals<
    'a,
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    stack: &'a Stack<'a, C, DefaultPacketPool>,
    addrs: &RefCell<VecView<Option<[u8; 6]>>>,
) {
    loop {
        // Wait unitil `START_SCANNING` is signaled
        START_SCANNING.wait().await;
        // Check whether the scanning is needed, aka there's empty slot in the addr list.
        let need_scan = !addrs.borrow().iter().all(|a| a.is_some());
        if need_scan {
            let scanning_fut = async {
                loop {
                    let Host { central, .. } = stack.build();
                    wait_for_stack_started().await;
                    let mut scanner = Scanner::new(central);
                    let scan_config = ScanConfig {
                        active: false,
                        ..Default::default()
                    };
                    let _guard = SCANNING_MUTEX.lock().await;
                    if let Ok(_session) = scanner.scan(&scan_config).await {
                        info!("Start scanning peripherals");
                        STOP_SCANNING.wait().await;
                        info!("Stop scanning");
                    }
                }
            };
            let update_addrs_fut = async {
                loop {
                    let (found_peripheral_id, addr) = PERIPHERAL_FOUND.wait().await;
                    let scanned_addr = addr.into_inner();
                    if let Some(Some(stored_addr)) = addrs.borrow_mut().get_mut(found_peripheral_id as usize)
                        && *stored_addr == scanned_addr
                    {
                        continue;
                    }

                    info!("Scanned new peripheral {:?}", scanned_addr);
                    let mut slot_updated = false;
                    if let Some(slot) = addrs.borrow_mut().get_mut(found_peripheral_id as usize)
                        && slot.is_none()
                    {
                        // Use scanned addresses as runtime candidates. Persist only
                        // after the split product handshake succeeds.
                        *slot = Some(scanned_addr);
                        slot_updated = true;
                    }

                    if slot_updated {
                        mark_uncommitted_peer_candidate(found_peripheral_id as usize);
                    }

                    if addrs.borrow().iter().all(|a| a.is_some()) {
                        break;
                    }
                }
            };

            // Scan until all peripherals are scanned
            // TODO: Timeout?
            select(scanning_fut, update_addrs_fut).await;
        }
    }
}

/// Read peripheral addresses from storage.
///
/// # Arguments
///
/// * `storage` - The storage to read peripheral addresses from
pub async fn read_peripheral_addresses<
    const PERI_NUM: usize,
    F: NorFlash,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
>(
    storage: &mut Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>,
) -> RefCell<Vec<Option<[u8; 6]>, PERI_NUM>> {
    let mut peripheral_addresses: heapless::Vec<Option<[u8; 6]>, PERI_NUM> = heapless::Vec::new();
    for id in 0..PERI_NUM {
        if let Ok(Some(peer_address)) = storage.read_peer_address(id as u8).await
            && peer_address.is_valid
        {
            peripheral_addresses.push(Some(peer_address.address)).unwrap();
            continue;
        }
        peripheral_addresses.push(None).unwrap();
    }
    RefCell::new(peripheral_addresses)
}

// When no peripheral address is saved, the central should first scan for peripheral.
// This handler is used to handle the scan result.
pub(crate) struct ScanHandler {}

impl EventHandler for ScanHandler {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            if let Some(peripheral_id) = split_peripheral_id_from_advertisement(report.data) {
                info!("Found split peripheral: id={:?}, addr={:?}", peripheral_id, report.addr);
                PERIPHERAL_FOUND.signal((peripheral_id, report.addr));
                break;
            }
        }
    }
}

fn split_peripheral_id_from_advertisement(data: &[u8]) -> Option<u8> {
    let mut has_split_service = false;
    let mut matching_product_peripheral_id = None;
    let mut offset = 0usize;

    while offset < data.len() {
        let len = data[offset] as usize;
        if len == 0 {
            break;
        }
        let end = offset + 1 + len;
        if end > data.len() || len < 1 {
            break;
        }

        let ad_type = data[offset + 1];
        let payload = &data[offset + 2..end];
        match ad_type {
            0x07 if payload == &SPLIT_SERVICE_UUID => {
                has_split_service = true;
            }
            0xff if payload.len() >= 5 => {
                let company_id = u16::from_le_bytes([payload[0], payload[1]]);
                let product_id = u16::from_le_bytes([payload[2], payload[3]]);
                if company_id == SPLIT_COMPANY_ID && product_id == crate::SPLIT_PRODUCT_ID {
                    matching_product_peripheral_id = Some(payload[4]);
                }
            }
            _ => {}
        }

        offset = end;
    }

    if has_split_service {
        matching_product_peripheral_id
    } else {
        None
    }
}

pub(crate) async fn run_ble_peripheral_manager<
    'a,
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
>(
    peri_id: usize,
    addrs: &RefCell<VecView<Option<[u8; 6]>>>,
    stack: &'a Stack<'a, C, DefaultPacketPool>,
) {
    trace!("SPLIT_MESSAGE_MAX_SIZE: {}", SPLIT_MESSAGE_MAX_SIZE);

    #[cfg(feature = "controller")]
    let mut controller_pub = unwrap!(CONTROLLER_CHANNEL.publisher());
    #[cfg(all(feature = "controller", feature = "storage"))]
    let mut controller_sub = unwrap!(CONTROLLER_CHANNEL.subscriber());

    loop {
        #[cfg(all(feature = "controller", feature = "storage"))]
        if poll_clear_peer(&mut controller_sub) {
            clear_saved_peripheral_peer(peri_id, addrs, &mut controller_pub, true).await;
        }

        // Check until the address is available
        let address = loop {
            #[cfg(all(feature = "controller", feature = "storage"))]
            if poll_clear_peer(&mut controller_sub) {
                clear_saved_peripheral_peer(peri_id, addrs, &mut controller_pub, true).await;
            }
            if let Some(Some(addr)) = addrs.borrow().get(peri_id) {
                break Address::random(*addr);
            }
            #[cfg(feature = "controller")]
            send_controller_event(&mut controller_pub, ControllerEvent::SplitPeripheralUnpaired(peri_id));
            if !START_SCANNING.signaled() {
                START_SCANNING.signal(());
            }
            // Check again after 500ms
            embassy_time::Timer::after_millis(500).await;
        };
        info!("Peripheral peer address: {:?}", address);

        let Host { mut central, .. } = stack.build();
        let config = ConnectConfig {
            connect_params: defaul_central_conn_param(),
            scan_config: ScanConfig {
                filter_accept_list: &[(address.kind, &address.addr)],
                ..Default::default()
            },
        };
        wait_for_stack_started().await;

        #[cfg(feature = "controller")]
        send_controller_event(&mut controller_pub, ControllerEvent::SplitPeripheral(peri_id, false));

        // Connect to peripheral
        match with_timeout(Duration::from_secs(5), async {
            if let Ok(_guard) = SCANNING_MUTEX.try_lock() {
                info!("Start connecting to peripheral {}", peri_id);
                central.connect(&config).await
            } else {
                STOP_SCANNING.signal(());
                let _guard = SCANNING_MUTEX.lock().await;
                // Wait a little bit to ensure that the scanning has been fully stopped
                embassy_time::Timer::after_millis(100).await;
                info!("Start connecting to peripheral {}", peri_id);
                central.connect(&config).await
            }
        })
        .await
        {
            Ok(Ok(conn)) => {
                info!("Connected to peripheral {}", peri_id);

                #[cfg(feature = "controller")]
                send_controller_event(&mut controller_pub, ControllerEvent::SplitPeripheral(peri_id, true));

                let clear_peer_on_connect = take_clear_peer_pending(peri_id);
                match run_central_manager_task::<_, _, ROW, COL, ROW_OFFSET, COL_OFFSET>(
                    peri_id,
                    address.addr.into_inner(),
                    clear_peer_on_connect,
                    stack,
                    &conn,
                )
                .await
                {
                    Ok(PeripheralSessionResult::KeepPeer) => {}
                    Ok(PeripheralSessionResult::ClearPeer) => {
                        clear_clear_peer_pending(peri_id);
                        #[cfg(all(feature = "controller", feature = "storage"))]
                        clear_saved_peripheral_peer(peri_id, addrs, &mut controller_pub, true).await;
                    }
                    Ok(PeripheralSessionResult::ClearPeerDelivered) => {
                        #[cfg(all(feature = "controller", feature = "storage"))]
                        clear_saved_peripheral_peer(peri_id, addrs, &mut controller_pub, false).await;
                    }
                    Ok(PeripheralSessionResult::ProductCheckFailed) => {
                        warn!("Peripheral {} product check failed, keeping saved address", peri_id);
                        drop_uncommitted_peer_candidate(peri_id, addrs, &mut controller_pub);
                    }
                    Err(e) => {
                        #[cfg(feature = "defmt")]
                        let e = defmt::Debug2Format(&e);
                        error!("BLE central error: {:?}", e);
                        drop_uncommitted_peer_candidate(peri_id, addrs, &mut controller_pub);
                    }
                }
            }
            Ok(Err(e)) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("Connect to peripheral {} error: {:?}", peri_id, e);
                drop_uncommitted_peer_candidate(peri_id, addrs, &mut controller_pub);
            }
            Err(_) => {
                // Connect to peripheral timeout
                warn!("Connect to peripheral {} timeout, keeping saved address", peri_id);
                drop_uncommitted_peer_candidate(peri_id, addrs, &mut controller_pub);
            }
        }
        // Reconnect after 500ms
        embassy_time::Timer::after_millis(500).await;
    }
}

#[cfg(all(feature = "controller", feature = "storage"))]
fn poll_clear_peer(controller_sub: &mut crate::channel::ControllerSub) -> bool {
    let mut requested = false;
    while let Some(event) = controller_sub.try_next_message_pure() {
        if let ControllerEvent::ClearPeer = event {
            requested = true;
        }
    }
    requested
}

#[cfg(all(feature = "controller", feature = "storage"))]
async fn clear_saved_peripheral_peer(
    peri_id: usize,
    addrs: &RefCell<VecView<Option<[u8; 6]>>>,
    controller_pub: &mut crate::channel::ControllerPub,
    clear_on_next_connect: bool,
) {
    if let Some(addr) = addrs.borrow_mut().get_mut(peri_id) {
        *addr = None;
    }
    clear_uncommitted_peer_candidate(peri_id);
    if clear_on_next_connect {
        mark_clear_peer_pending(peri_id);
    } else {
        clear_clear_peer_pending(peri_id);
    }
    FLASH_CHANNEL
        .send(FlashOperationMessage::PeerAddress(PeerAddress::new(
            peri_id as u8,
            false,
            [0; 6],
        )))
        .await;
    send_controller_event(controller_pub, ControllerEvent::SplitPeripheralUnpaired(peri_id));
    if !START_SCANNING.signaled() {
        START_SCANNING.signal(());
    }
}

fn bit_for_peri(peri_id: usize) -> u32 {
    1u32 << peri_id.min(31)
}

fn mark_clear_peer_pending(peri_id: usize) {
    CLEAR_PEER_PENDING.fetch_or(bit_for_peri(peri_id), Ordering::AcqRel);
}

fn take_clear_peer_pending(peri_id: usize) -> bool {
    let bit = bit_for_peri(peri_id);
    CLEAR_PEER_PENDING.fetch_and(!bit, Ordering::AcqRel) & bit != 0
}

fn clear_clear_peer_pending(peri_id: usize) {
    CLEAR_PEER_PENDING.fetch_and(!bit_for_peri(peri_id), Ordering::AcqRel);
}

fn mark_uncommitted_peer_candidate(peri_id: usize) {
    UNCOMMITTED_PEER_CANDIDATES.fetch_or(bit_for_peri(peri_id), Ordering::AcqRel);
}

fn clear_uncommitted_peer_candidate(peri_id: usize) {
    UNCOMMITTED_PEER_CANDIDATES.fetch_and(!bit_for_peri(peri_id), Ordering::AcqRel);
}

fn take_uncommitted_peer_candidate(peri_id: usize) -> bool {
    let bit = bit_for_peri(peri_id);
    UNCOMMITTED_PEER_CANDIDATES.fetch_and(!bit, Ordering::AcqRel) & bit != 0
}

fn drop_uncommitted_peer_candidate(
    peri_id: usize,
    addrs: &RefCell<VecView<Option<[u8; 6]>>>,
    controller_pub: &mut crate::channel::ControllerPub,
) {
    if take_uncommitted_peer_candidate(peri_id) {
        if let Some(addr) = addrs.borrow_mut().get_mut(peri_id) {
            *addr = None;
        }
        send_controller_event(controller_pub, ControllerEvent::SplitPeripheralUnpaired(peri_id));
        if !START_SCANNING.signaled() {
            START_SCANNING.signal(());
        }
    }
}

fn defaul_central_conn_param() -> ConnectParams {
    ConnectParams {
        min_connection_interval: Duration::from_micros(7500),
        max_connection_interval: Duration::from_micros(7500),
        max_latency: 0,
        supervision_timeout: Duration::from_secs(5),
        ..Default::default()
    }
}

async fn run_central_manager_task<
    'a,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
>(
    id: usize,
    peer_address: [u8; 6],
    clear_peer_on_connect: bool,
    stack: &'a Stack<'a, C, P>,
    conn: &Connection<'a, P>,
) -> Result<PeripheralSessionResult, BleHostError<C::Error>> {
    let client = GattClient::<C, P, 10>::new(stack, conn).await?;

    // Use 2M Phy
    update_ble_phy(stack, conn).await;

    info!("Updating connection parameters for peripheral");
    update_conn_params(stack, conn, &defaul_central_conn_param()).await;

    match select3(
        ble_central_task(&client, conn),
        run_peripheral_manager::<_, _, ROW, COL, ROW_OFFSET, COL_OFFSET>(
            id,
            peer_address,
            clear_peer_on_connect,
            &client,
        ),
        sleep_manager_task(stack, conn),
    )
    .await
    {
        Either3::First(e) => e.map(|_| PeripheralSessionResult::KeepPeer),
        Either3::Second(e) => e,
        Either3::Third(e) => e.map(|_| PeripheralSessionResult::KeepPeer),
    }
}

async fn ble_central_task<'a, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool>(
    client: &GattClient<'a, C, P, 10>,
    conn: &Connection<'a, P>,
) -> Result<(), BleHostError<C::Error>> {
    // Simply monitor connection status
    let conn_check = async {
        while conn.is_connected() {
            Timer::after_secs(5).await;
        }
    };

    match select(client.task(), conn_check).await {
        Either::First(e) => e,
        Either::Second(_) => {
            info!("Connection lost");
            Ok(())
        }
    }
}

async fn run_peripheral_manager<
    'a,
    C: Controller + ControllerCmdAsync<LeSetPhy>,
    P: PacketPool,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
>(
    id: usize,
    peer_address: [u8; 6],
    clear_peer_on_connect: bool,
    client: &GattClient<'a, C, P, 10>,
) -> Result<PeripheralSessionResult, BleHostError<C::Error>> {
    let services = client
        .services_by_uuid(&Uuid::new_long(SPLIT_SERVICE_UUID))
        .await?;
    info!("Services found");
    if let Some(service) = services.first() {
        let message_to_central = client
            .characteristic_by_uuid::<[u8; SPLIT_MESSAGE_MAX_SIZE]>(
                service,
                // uuid: 0e6313e3-bd0b-45c2-8d2e-37a2e8128bc3
                &Uuid::Uuid128([
                    195u8, 139u8, 18u8, 232u8, 162u8, 55u8, 46u8, 141u8, 194u8, 69u8, 11u8, 189u8, 227u8, 19u8, 99u8,
                    14u8,
                ]),
            )
            .await?;
        info!("Message to central found");
        let message_to_peripheral = client
            .characteristic_by_uuid::<[u8; SPLIT_MESSAGE_MAX_SIZE]>(
                service,
                // uuid: 4b3514fb-cae4-4d38-a097-3a2a3d1c3b9c
                &Uuid::Uuid128([
                    156u8, 59u8, 28u8, 61u8, 42u8, 58u8, 151u8, 160u8, 56u8, 77u8, 228u8, 202u8, 251u8, 20u8, 53u8,
                    75u8,
                ]),
            )
            .await?;
        info!("Subscribing notifications");
        let listener = client.subscribe(&message_to_central, false).await?;
        let mut split_ble_driver = BleSplitCentralDriver::new(listener, message_to_peripheral, client);
        if !validate_split_product(&mut split_ble_driver).await {
            return Ok(PeripheralSessionResult::ProductCheckFailed);
        }
        if clear_peer_on_connect {
            debug!("Deliver pending ClearPeer to peripheral {}", id);
            if let Err(e) = split_ble_driver.write(&SplitMessage::ClearPeer).await {
                warn!("Pending ClearPeer delivery failed: {:?}", e);
                mark_clear_peer_pending(id);
                return Ok(PeripheralSessionResult::ClearPeer);
            }
            return Ok(PeripheralSessionResult::ClearPeerDelivered);
        }
        #[cfg(feature = "storage")]
        FLASH_CHANNEL
            .send(FlashOperationMessage::PeerAddress(PeerAddress::new(
                id as u8,
                true,
                peer_address,
            )))
            .await;
        clear_uncommitted_peer_candidate(id);
        let peripheral_manager = PeripheralManager::<ROW, COL, ROW_OFFSET, COL_OFFSET, _>::new(split_ble_driver, id);
        if peripheral_manager.run().await {
            return Ok(PeripheralSessionResult::ClearPeer);
        }
        info!("Peripheral manager stopped");
        return Ok(PeripheralSessionResult::KeepPeer);
    };
    Ok(PeripheralSessionResult::ProductCheckFailed)
}

async fn validate_split_product<T: SplitReader + SplitWriter>(driver: &mut T) -> bool {
    if let Err(e) = driver.write(&SplitMessage::ProductId(crate::SPLIT_PRODUCT_ID)).await {
        warn!("Split product check write failed: {:?}", e);
        return false;
    }

    match with_timeout(Duration::from_millis(1500), driver.read()).await {
        Ok(Ok(SplitMessage::ProductId(product_id))) if product_id == crate::SPLIT_PRODUCT_ID => true,
        Ok(Ok(SplitMessage::ProductId(product_id))) => {
            warn!("Split product id mismatch: got {}, expected {}", product_id, crate::SPLIT_PRODUCT_ID);
            false
        }
        Ok(Ok(message)) => {
            warn!("Unexpected split product check response: {:?}", message);
            false
        }
        Ok(Err(e)) => {
            warn!("Split product check read failed: {:?}", e);
            false
        }
        Err(_) => {
            warn!("Split product check timeout");
            false
        }
    }
}

/// Ble central driver which reads and writes the split message.
///
/// Different from serial, BLE split message is processed in a separate service.
/// The BLE service should keep running, it processes the split message in the callback, which is not async.
/// It's impossible to implement `SplitReader` or `SplitWriter` for BLE service,
/// so we need this wrapper to forward split message to channel.
pub(crate) struct BleSplitCentralDriver<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> {
    // Listener for split message from peripheral
    listener: NotificationListener<'b, 512>,
    // Characteristic to send split message to peripheral
    message_to_peripheral: Characteristic<[u8; SPLIT_MESSAGE_MAX_SIZE]>,
    // Client
    client: &'c GattClient<'a, C, P, 10>,
    // Cached connection state
    connection_state: bool,
}

impl<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> BleSplitCentralDriver<'a, 'b, 'c, C, P> {
    pub(crate) fn new(
        listener: NotificationListener<'b, 512>,
        message_to_peripheral: Characteristic<[u8; SPLIT_MESSAGE_MAX_SIZE]>,
        client: &'c GattClient<'a, C, P, 10>,
    ) -> Self {
        Self {
            listener,
            message_to_peripheral,
            client,
            connection_state: CONNECTION_STATE.load(Ordering::Acquire),
        }
    }
}

impl<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> SplitReader
    for BleSplitCentralDriver<'a, 'b, 'c, C, P>
{
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        let data = self.listener.next().await;
        let message = postcard::from_bytes(data.as_ref()).map_err(|_| SplitDriverError::DeserializeError)?;
        info!("Received split message: {:?}", message);

        // Update last activity time when receiving key events from peripheral
        match &message {
            SplitMessage::Key(_) => {
                debug!("Key activity detected from peripheral");
                update_activity_time();
            }
            SplitMessage::Event(_) => {
                debug!("Event activity detected from peripheral");
                update_activity_time();
            }
            _ => {}
        }

        Ok(message)
    }
}

impl<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> SplitWriter
    for BleSplitCentralDriver<'a, 'b, 'c, C, P>
{
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        if let SplitMessage::ConnectionState(state) = message {
            // ConnectionState changed, update cached state and notify peripheral
            if self.connection_state != *state {
                self.connection_state = *state;
            }
        }
        // Always sync the connection state to peripheral since central doesn't know the CONNECTION_STATE of the peripheral.
        let mut buf = [0_u8; SPLIT_MESSAGE_MAX_SIZE];
        match postcard::to_slice(&message, &mut buf) {
            Ok(_bytes) => {
                if let Err(e) = self
                    .client
                    .write_characteristic_without_response(&self.message_to_peripheral, &buf)
                    .await
                {
                    if let BleHostError::BleHost(Error::NotFound) = e {
                        error!("Peripheral disconnected");
                        return Err(SplitDriverError::Disconnected);
                    }
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("BLE message_to_peripheral_write error: {:?}", e);
                }
            }
            Err(e) => error!("Postcard serialize split message error: {}", e),
        };

        Ok(SPLIT_MESSAGE_MAX_SIZE)
    }
}

/// Wait for the BLE stack to start.
///
/// If the BLE stack has been started, wait 500ms then quit.
pub(crate) async fn wait_for_stack_started() {
    loop {
        if STACK_STARTED.signaled() {
            embassy_time::Timer::after_millis(500).await;
            break;
        }
        embassy_time::Timer::after_millis(500).await;
    }
}

/// Sleep manager task for connection between split central and peripheral
/// Handles sleep timeout and connection parameter adjustments using event-driven approach
async fn sleep_manager_task<
    'a,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
>(
    stack: &'a Stack<'a, C, P>,
    conn: &Connection<'a, P>,
) -> Result<(), BleHostError<C::Error>> {
    let mut sleep_timeout_seconds = default_sleep_timeout_seconds();

    info!(
        "Sleep manager started with {}s timeout",
        sleep_timeout_seconds
    );

    #[cfg(feature = "controller")]
    let mut controller_pub = unwrap!(CONTROLLER_CHANNEL.publisher());
    #[cfg(feature = "controller")]
    let mut controller_sub = unwrap!(CONTROLLER_CHANNEL.subscriber());

    loop {
        if !SLEEPING_STATE.load(Ordering::Acquire) {
            loop {
                // Wait for timeout, activity (false signal), forced sleep, or settings changes.
                #[cfg(feature = "controller")]
                match select3(
                    Timer::after_secs(sleep_timeout_seconds),
                    CENTRAL_SLEEP.wait(),
                    controller_sub.next_message_pure(),
                )
                .await
                {
                    Either3::First(_) => {
                        // Timeout: enter sleep mode
                        break;
                    }
                    Either3::Second(signal_value) => {
                        // Received signal - if false, it means activity detected
                        if !signal_value {
                            debug!("Activity detected, resetting sleep timeout");
                            continue;
                        }
                        // True, enter sleep mode
                        break;
                    }
                    Either3::Third(event) => {
                        if let ControllerEvent::SleepTimeout(index) = event {
                            sleep_timeout_seconds = sleep_timeout_seconds_from_index(index);
                            debug!("Sleep timeout updated to {}s", sleep_timeout_seconds);
                        }
                        continue;
                    }
                }

                #[cfg(not(feature = "controller"))]
                match select(Timer::after_secs(sleep_timeout_seconds), CENTRAL_SLEEP.wait()).await {
                    Either::First(_) => {
                        // Timeout: enter sleep mode
                        break;
                    }
                    Either::Second(signal_value) => {
                        // Received signal - if false, it means activity detected
                        if !signal_value {
                            debug!("Activity detected, resetting sleep timeout");
                            continue;
                        }
                        // True, enter sleep mode
                        break;
                    }
                }
            }

            // Timeout or received true from CENTRAL_SLEEP signal, enter sleep mode
            info!("Entering sleep mode");

            // Connection parameters are different when central is broadcasting and connected to host
            let conn_params = if CONNECTION_STATE.load(Ordering::Acquire) {
                // Connected, the connection interval is 20ms
                ConnectParams {
                    min_connection_interval: Duration::from_millis(20),
                    max_connection_interval: Duration::from_millis(20),
                    max_latency: 200, // 4s
                    supervision_timeout: Duration::from_secs(9),
                    ..Default::default()
                }
            } else {
                // Advertising ,the connection interval can be longer
                ConnectParams {
                    min_connection_interval: Duration::from_millis(200),
                    max_connection_interval: Duration::from_millis(200),
                    max_latency: 25, // 5s
                    supervision_timeout: Duration::from_secs(11),
                    ..Default::default()
                }
            };

            // Update connection parameters
            update_conn_params(stack, conn, &conn_params).await;
            SLEEPING_STATE.store(true, Ordering::Release);
            #[cfg(feature = "controller")]
            send_controller_event(&mut controller_pub, ControllerEvent::Sleep(true));
        } else {
            // Wait for activity to wake up (false signal means activity/wakeup)
            let signal_value = CENTRAL_SLEEP.wait().await;
            if !signal_value {
                info!("Waking up from sleep mode due to activity");
                SLEEPING_STATE.store(false, Ordering::Release);
                #[cfg(feature = "controller")]
                send_controller_event(&mut controller_pub, ControllerEvent::Sleep(false));

                // Restore normal connection parameters
                update_conn_params(stack, conn, &defaul_central_conn_param()).await;
            }
        }
    }
}

/// Update the activity time to indicate user activity
/// This function triggers activity wakeup signal for sleep management
pub(crate) fn update_activity_time() {
    crate::channel::signal_activity();
    CENTRAL_SLEEP.signal(false);
    debug!("Activity detected, signaling wakeup");
}

fn default_sleep_timeout_seconds() -> u64 {
    match SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS {
        0 => DEFAULT_SLEEP_TIMEOUT_SECONDS,
        seconds => u64::from(seconds),
    }
}

#[cfg(feature = "controller")]
fn sleep_timeout_seconds_from_index(index: u8) -> u64 {
    SLEEP_TIMEOUT_SECONDS[usize::from(index.min(9))]
}
