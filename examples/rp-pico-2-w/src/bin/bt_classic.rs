#![no_std]
#![no_main]

use bt_hci::cmd::controller_baseband::{HostBufferSize, Reset, SetEventMask};
use bt_hci::cmd::info::{ReadBdAddr, ReadLocalSupportedFeatures};
use bt_hci::cmd::le::{LeReadBufferSize, LeSetRandomAddr};
use bt_hci::cmd::link_control::Inquiry;
use bt_hci::cmd::SyncCmd;
use bt_hci::controller::{Controller, ControllerCmdSync};
use bt_hci::event::{EventPacket, ExtendedInquiryResult, InquiryComplete, InquiryResult};
use bt_hci::param::{BdAddr, EventMask, RemainingBytes};
use bt_hci::{ControllerToHostPacket, FromHciBytes, HostToControllerPacket, PacketKind};
use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::{bind_interrupts, install_core0_stack_guard};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use heapless::FnvIndexSet;
use static_cell::StaticCell;
use trouble_host::prelude::ExternalController;
use trouble_host::Address;
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

const MAX_HCI_PACKET_LEN: usize = 259;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn cyw43_task(runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    defmt::expect!(install_core0_stack_guard(), "MPU already configured");

    let p = embassy_rp::init(Default::default());

    let fw = cyw43_firmware::CYW43_43439A0;
    let clm = cyw43_firmware::CYW43_43439A0_CLM;
    let btfw = cyw43_firmware::CYW43_43439A0_BTFW;

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        RM2_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (_net_device, bt_device, mut control, runner) = cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));
    control.init(clm).await;

    // cortex_m::asm::bkpt();

    let controller: ExternalController<_, 10> = ExternalController::new(bt_device);

    // Shared state for inquiry coordination
    static INQUIRY_COMPLETE: StaticCell<Channel<CriticalSectionRawMutex, (), 1>> = StaticCell::new();
    static DISCOVERED: StaticCell<Mutex<CriticalSectionRawMutex, FnvIndexSet<BdAddr, 32>>> = StaticCell::new();

    let inquiry_complete_channel = INQUIRY_COMPLETE.init(Channel::new());
    let discovered_devices = DISCOVERED.init(Mutex::new(FnvIndexSet::new()));

    select(
        run_controller(&controller, inquiry_complete_channel),
        run_rx(&controller, inquiry_complete_channel, discovered_devices),
    )
    .await;
    defmt::error!("EXITED");
}

async fn run_controller<C>(
    controller: &C,
    inquiry_complete_channel: &Channel<CriticalSectionRawMutex, (), 1>,
) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: ControllerCmdSync<LeReadBufferSize>,
    C: ControllerCmdSync<LeSetRandomAddr>,
    C: ControllerCmdSync<ReadBdAddr>,
    C: ControllerCmdSync<Reset>,
    C: ControllerCmdSync<SetEventMask>,
    C: ControllerCmdSync<Inquiry>,
    C: ControllerCmdSync<HostBufferSize>,
    C: ControllerCmdSync<ReadLocalSupportedFeatures>,
    C: ControllerCmdSync<WriteScanEnable>,
    C: ControllerCmdSync<WriteInquiryMode>,
    C: ControllerCmdSync<WriteClassOfDevice>,
    C: ControllerCmdSync<WriteInquiryScanActivity>,
    C: ControllerCmdSync<WritePageScanActivity>,
    C: ControllerCmdSync<WriteInquiryScanType>,
{
    defmt::info!("resetting...");
    Reset::new().exec(controller).await?;

    const ACL_LEN: u16 = 255;
    const ACL_N: u16 = 1;
    info!(
        "[host] configuring host buffers ({} packets of size {})",
        ACL_N, ACL_LEN,
    );
    HostBufferSize::new(ACL_LEN, 0, ACL_N, 0).exec(controller).await?;

    // defmt::info!("setting random address...");
    // let addr = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    // LeSetRandomAddr::new(addr.addr).exec(controller).await?;

    info!("set event mask");
    SetEventMask::new(
        EventMask::new()
            .enable_conn_request(true)
            .enable_conn_complete(true)
            .enable_hardware_error(true)
            .enable_disconnection_complete(true)
            .enable_encryption_change_v1(true)
            .enable_inquiry_complete(true)
            .enable_inquiry_result(true)
            .enable_ext_inquiry_result(true),
    )
    .exec(controller)
    .await?;

    // let _ret = LeReadBufferSize::new().exec(controller).await?;

    info!("reading local BD_ADDR...");
    let device_address = ReadBdAddr::new().exec(controller).await?;
    info!(
        "Local BD_ADDR: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        device_address.raw()[0],
        device_address.raw()[1],
        device_address.raw()[2],
        device_address.raw()[3],
        device_address.raw()[4],
        device_address.raw()[5],
    );

    info!("reading local supported features...");
    let features = ReadLocalSupportedFeatures::new().exec(controller).await?;
    info!("Features: {}", features);

    // Set device class (Computer - Desktop workstation)
    info!("setting class of device...");
    write_class_of_device(controller, [0x00, 0x01, 0x04]).await?;

    // Set inquiry scan activity (interval and window in 0.625ms units)
    // Default: interval=0x1000 (2.56s), window=0x0012 (11.25ms)
    info!("setting inquiry scan activity...");
    WriteInquiryScanActivity::new(0x1000, 0x0012).exec(controller).await?;

    // Set page scan activity
    info!("setting page scan activity...");
    WritePageScanActivity::new(0x0800, 0x0012).exec(controller).await?;

    // Set inquiry scan type (0x00 = standard, 0x01 = interlaced)
    info!("setting inquiry scan type...");
    WriteInquiryScanType::new(0x01).exec(controller).await?;

    // Set inquiry mode to standard (with RSSI would be 0x01, but controller doesn't support extended)
    info!("setting inquiry mode to 0x01 (RSSI)...");
    write_inquiry_mode(controller, 0x01).await?;

    // Enable inquiry and page scan
    // 0x00 = No scans, 0x01 = Inquiry scan only, 0x02 = Page scan only, 0x03 = Both
    info!("enabling inquiry and page scan...");
    write_scan_enable(controller, 0x03).await?;

    info!("Starting continuous Bluetooth Classic inquiry...");

    loop {
        // Send Inquiry command
        // LAP: [0x9e, 0x8b, 0x33] = General/Unlimited Inquiry Access Code
        // inquiry_length: 0x08 = 10.24 seconds
        // num_responses: 0x00 = unlimited
        Inquiry::new([0x9e, 0x8b, 0x33], 0x08, 0x00).exec(controller).await?;

        // Wait for InquiryComplete event
        inquiry_complete_channel.receive().await;

        // Brief delay before next inquiry
        Timer::after(Duration::from_millis(100)).await;
    }
}

pub async fn run_rx<C>(
    controller: &C,
    inquiry_complete_channel: &Channel<CriticalSectionRawMutex, (), 1>,
    discovered_devices: &Mutex<CriticalSectionRawMutex, FnvIndexSet<BdAddr, 32>>,
) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: Controller,
{
    let mut buf = [0u8; MAX_HCI_PACKET_LEN];
    loop {
        match controller.read(&mut buf).await {
            Ok(packet) => match packet {
                ControllerToHostPacket::Event(event) => {
                    handle_event(event, inquiry_complete_channel, discovered_devices).await;
                }
                packet => {
                    defmt::debug!("Ignoring packet {}", packet);
                }
            },
            Err(_) => error!("rx error"),
        }
    }
}

// Raw HCI command implementations for missing bt-hci commands
// Using the bt_hci cmd! macro

use bt_hci::cmd;

cmd! {
    /// WriteScanEnable command (OGF: 0x03, OCF: 0x001A)
    WriteScanEnable(CONTROL_BASEBAND, 0x001A) {
        WriteScanEnableParams {
            scan_enable: u8,
        }
        Return = ();
    }
}

cmd! {
    /// WriteInquiryMode command (OGF: 0x03, OCF: 0x0045)
    WriteInquiryMode(CONTROL_BASEBAND, 0x0045) {
        WriteInquiryModeParams {
            inquiry_mode: u8,
        }
        Return = ();
    }
}

cmd! {
    /// WriteClassOfDevice command (OGF: 0x03, OCF: 0x0024)
    WriteClassOfDevice(CONTROL_BASEBAND, 0x0024) {
        WriteClassOfDeviceParams {
            class_of_device: [u8; 3],
        }
        Return = ();
    }
}

cmd! {
    /// WriteInquiryScanActivity command (OGF: 0x03, OCF: 0x001E)
    WriteInquiryScanActivity(CONTROL_BASEBAND, 0x001E) {
        WriteInquiryScanActivityParams {
            inquiry_scan_interval: u16,
            inquiry_scan_window: u16,
        }
        Return = ();
    }
}

cmd! {
    /// WritePageScanActivity command (OGF: 0x03, OCF: 0x001C)
    WritePageScanActivity(CONTROL_BASEBAND, 0x001C) {
        WritePageScanActivityParams {
            page_scan_interval: u16,
            page_scan_window: u16,
        }
        Return = ();
    }
}

cmd! {
    /// WriteInquiryScanType command (OGF: 0x03, OCF: 0x0043)
    WriteInquiryScanType(CONTROL_BASEBAND, 0x0043) {
        WriteInquiryScanTypeParams {
            scan_type: u8,
        }
        Return = ();
    }
}

async fn write_scan_enable<C>(controller: &C, mode: u8) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: ControllerCmdSync<WriteScanEnable>,
{
    WriteScanEnable::new(mode).exec(controller).await?;
    Timer::after(Duration::from_millis(50)).await;
    Ok(())
}

async fn write_inquiry_mode<C>(controller: &C, mode: u8) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: ControllerCmdSync<WriteInquiryMode>,
{
    WriteInquiryMode::new(mode).exec(controller).await?;
    Timer::after(Duration::from_millis(50)).await;
    Ok(())
}

async fn write_class_of_device<C>(controller: &C, class: [u8; 3]) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: ControllerCmdSync<WriteClassOfDevice>,
{
    WriteClassOfDevice::new(class).exec(controller).await?;
    Timer::after(Duration::from_millis(50)).await;
    Ok(())
}

async fn handle_event(
    event: EventPacket<'_>,
    inquiry_complete_channel: &Channel<CriticalSectionRawMutex, (), 1>,
    discovered_devices: &Mutex<CriticalSectionRawMutex, FnvIndexSet<BdAddr, 32>>,
) {
    use bt_hci::event::EventKind;

    match event.kind {
        EventKind::InquiryResult => {
            info!("Received InquiryResult event!");
            // Handle standard inquiry results (fallback if Extended mode doesn't work)
            match InquiryResult::from_hci_bytes_complete(event.data) {
                Ok(result) => {
                    info!("Parsed InquiryResult: {} responses", result.num_responses);
                    let mut devices = discovered_devices.lock().await;
                    for item in result.iter() {
                        if !devices.contains(&item.bd_addr) {
                            if let Some(class) = item.class_of_device {
                                info!(
                                    "Discovered (standard): {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} | Class: {:02x}{:02x}{:02x}",
                                    item.bd_addr.raw()[0],
                                    item.bd_addr.raw()[1],
                                    item.bd_addr.raw()[2],
                                    item.bd_addr.raw()[3],
                                    item.bd_addr.raw()[4],
                                    item.bd_addr.raw()[5],
                                    class[0],
                                    class[1],
                                    class[2],
                                );
                            } else {
                                info!(
                                    "Discovered (standard): {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                                    item.bd_addr.raw()[0],
                                    item.bd_addr.raw()[1],
                                    item.bd_addr.raw()[2],
                                    item.bd_addr.raw()[3],
                                    item.bd_addr.raw()[4],
                                    item.bd_addr.raw()[5],
                                );
                            }
                            devices.insert(item.bd_addr).ok();
                        }
                    }
                }
                Err(e) => warn!("Failed to parse InquiryResult: {:?}", e),
            }
        }

        EventKind::ExtendedInquiryResult => {
            info!("Received ExtendedInquiryResult event!");
            match ExtendedInquiryResult::from_hci_bytes_complete(event.data) {
                Ok(result) => {
                    info!("Parsed ExtendedInquiryResult");
                    // Check if already seen
                    let mut devices = discovered_devices.lock().await;
                    if devices.contains(&result.bd_addr) {
                        return;
                    }

                    // Parse device name from EIR data
                    let name = parse_eir_device_name(result.eir_data);

                    // Print discovery
                    info!(
                        "Discovered: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} | RSSI: {} dBm | Class: {:02x}{:02x}{:02x} | Name: {}",
                        result.bd_addr.raw()[0],
                        result.bd_addr.raw()[1],
                        result.bd_addr.raw()[2],
                        result.bd_addr.raw()[3],
                        result.bd_addr.raw()[4],
                        result.bd_addr.raw()[5],
                        result.rssi,
                        result.class_of_device[0],
                        result.class_of_device[1],
                        result.class_of_device[2],
                        name.as_deref().unwrap_or("<unknown>")
                    );

                    // Mark as seen
                    devices.insert(result.bd_addr).ok();
                }
                Err(e) => warn!("Failed to parse ExtendedInquiryResult: {:?}", e),
            }
        }

        EventKind::InquiryComplete => match InquiryComplete::from_hci_bytes_complete(event.data) {
            Ok(complete) => {
                if let Err(e) = complete.status.to_result() {
                    warn!("Inquiry error: {:?}", e);
                } else {
                    debug!("Inquiry cycle completed");
                }
                inquiry_complete_channel.try_send(()).ok();
            }
            Err(e) => warn!("Failed to parse InquiryComplete: {:?}", e),
        },

        _ => {
            // Log ALL events to see if we're missing something
            info!("Received event: {:?} (kind: {})", event.kind, event.kind);
        }
    }
}

fn parse_eir_device_name(eir_data: RemainingBytes) -> Option<heapless::String<64>> {
    let mut data = eir_data.as_ref();

    while !data.is_empty() {
        if data.len() < 1 {
            break;
        }

        let length = data[0] as usize;
        if length == 0 {
            break; // End of EIR data
        }

        if data.len() < 1 + length {
            break; // Incomplete element
        }

        let element_type = data[1];
        let value = &data[2..1 + length];

        // 0x08 = Shortened Local Name, 0x09 = Complete Local Name
        if element_type == 0x08 || element_type == 0x09 {
            if let Ok(name) = core::str::from_utf8(value) {
                if let Ok(owned_name) = heapless::String::try_from(name) {
                    return Some(owned_name);
                }
            }
        }

        data = &data[1 + length..];
    }

    None
}

pub async fn run_tx<C>(_controller: &C) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: Controller,
{
    Ok(())
}
