#![no_std]
#![no_main]

use bt_hci::cmd::controller_baseband::Reset;
use bt_hci::cmd::info::ReadBdAddr;
use bt_hci::cmd::le::{LeSetAdvSetRandomAddr, LeSetRandomAddr};
use bt_hci::cmd::SyncCmd;
use bt_hci::controller::{Controller, ControllerCmdSync};
use bt_hci::ControllerToHostPacket;
use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use static_cell::StaticCell;
use trouble_host::prelude::ExternalController;
use trouble_host::Address;
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

bind_interrupts!(
    struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
    }
);

#[embassy_executor::task]
async fn cyw43_task(runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
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

    let controller: ExternalController<_, 10> = ExternalController::new(bt_device);
    if let Err(err) = controller(&controller).await {
        defmt::error!("{}", err);
    }
    defmt::info!("init done");
}

async fn run_controller<C>(controller: &C) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: ControllerCmdSync<Reset>,
    C: ControllerCmdSync<LeSetRandomAddr>,
    C: ControllerCmdSync<ReadBdAddr>,
{
    defmt::info!("resetting...");
    Reset::new().exec(controller).await?;

    // defmt::info!("setting random address...");
    // let addr = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    // LeSetRandomAddr::new(addr.addr).exec(controller).await?;

    defmt::info!("reading...");
    let device_address = ReadBdAddr::new().exec(controller).await?;
    defmt::dbg!(device_address);

    core::future::pending::<()>().await;
    Ok(())
}

pub async fn run_rx<C>(controller: &C) -> Result<(), bt_hci::cmd::Error<C::Error>>
where
    C: Controller,
{
    const MAX_HCI_PACKET_LEN: usize = 259;
    loop {
        // Task handling receiving data from the controller.
        let mut rx = [0u8; MAX_HCI_PACKET_LEN];
        let result = controller.read(&mut rx).await;
        match result {
            Ok(p) => info!("[host] read {}", p),
            Err(_) => {
                error!("[host] rx::run error");
            }
        }
    }
}
