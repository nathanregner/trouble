use core::cell::RefCell;

use bt_hci::cmd::link_control::Inquiry;
use bt_hci::controller::ControllerCmdSync;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use heapless::Deque;
use trouble_host::prelude::*;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

pub async fn run<C>(controller: C)
where
    C: Controller + ControllerCmdSync<Inquiry>,
{
    info!("Starting Bluetooth Classic Scanner");

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources);

    let Host {
        central, mut runner, ..
    } = stack.build();

    let printer = Printer {
        seen: RefCell::new(Deque::new()),
    };

    let mut scanner = Scanner::new(central);

    let _ = join(runner.run_with_handler(&printer), async {
        // Wait a bit for initialization
        Timer::after(Duration::from_millis(500)).await;

        info!("Starting continuous Bluetooth Classic inquiry...");

        // Continuous inquiry loop
        loop {
            // Perform Bluetooth Classic inquiry
            // LAP: GIAC 0x9E8B33 in little-endian = [0x33, 0x8B, 0x9E]
            // inquiry_length: 0x08 = 10.24 seconds (units of 1.28s)
            // num_responses: 0x00 = unlimited
            match scanner.inquiry([0x33, 0x8b, 0x9e], 0x08, 0x00).await {
                Ok(_session) => {
                    // Inquiry running, wait for it to complete
                    Timer::after(Duration::from_secs(11)).await;
                }
                Err(_e) => {
                    error!("Inquiry command failed");
                    Timer::after(Duration::from_secs(1)).await;
                }
            }
        }
    })
    .await;
}

struct Printer {
    seen: RefCell<Deque<BdAddr, 128>>,
}

impl EventHandler for Printer {
    fn on_inquiry_result(&self, result: &bt_hci::event::InquiryResult) {
        let mut seen = self.seen.borrow_mut();
        for item in result.iter() {
            if seen.iter().find(|b| b.raw() == item.bd_addr.raw()).is_none() {
                let target_addr = BdAddr::new([0x66, 0x8a, 0x9f, 0xe2, 0x1f, 0x00]);
                if item.bd_addr == target_addr {
                    if let Some(class) = item.class_of_device {
                        error!(
                            "discovered: {:x} | Class: {:02x}{:02x}{:02x}",
                            MacAddress(item.bd_addr),
                            class[0],
                            class[1],
                            class[2]
                        );
                    } else {
                        error!("discovered: {:x}", MacAddress(item.bd_addr));
                    }
                } else {
                    if let Some(class) = item.class_of_device {
                        info!(
                            "discovered: {:x} | Class: {:02x}{:02x}{:02x}",
                            MacAddress(item.bd_addr),
                            class[0],
                            class[1],
                            class[2]
                        );
                    } else {
                        info!("discovered: {:x}", MacAddress(item.bd_addr));
                    }
                }
                if seen.is_full() {
                    seen.pop_front();
                }
                seen.push_back(item.bd_addr).unwrap();
            }
        }
    }

    fn on_inquiry_complete(&self, complete: &bt_hci::event::InquiryComplete) {
        if let Err(e) = complete.status.to_result() {
            warn!("Inquiry complete with error: {:?}", e);
        } else {
            info!("Inquiry cycle completed");
        }
    }
}

struct MacAddress(BdAddr);

impl defmt::Format for MacAddress {
    fn format(&self, fmt: defmt::Formatter) {
        let octets = self.0.raw();
        defmt::write!(
            fmt,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            octets[0],
            octets[1],
            octets[2],
            octets[3],
            octets[4],
            octets[5],
        )
    }
}
