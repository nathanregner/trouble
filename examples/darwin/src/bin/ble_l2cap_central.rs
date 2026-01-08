use std::error::Error;

use bt_hci::controller::ExternalController;
use bt_hci_darwin::Transport;
use trouble_example_apps::ble_l2cap_central;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let transport = Transport::new()?;
    let controller = ExternalController::<_, 8>::new(transport);
    ble_l2cap_central::run(controller).await;
    Ok(())
}
