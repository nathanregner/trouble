use std::error::Error;

use bt_hci::controller::ExternalController;
use bt_hci_darwin::Transport;
use trouble_example_apps::{high_throughput_ble_l2cap_central, BigAlloc};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let transport = Transport::new()?;
    let controller = ExternalController::<_, 8>::new(transport);
    high_throughput_ble_l2cap_central::run::<_, BigAlloc>(controller).await;
    Ok(())
}
