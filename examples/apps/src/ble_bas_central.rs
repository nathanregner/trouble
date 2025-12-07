use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use trouble_host::{prelude::*, scan};

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att + CoC

pub async fn run<C>(controller: C)
where
    C: Controller,
{
    // Using a fixed "random" address can be useful for testing. In real scenarios, one would
    // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
    let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    info!("Our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut central,
        mut runner,
        ..
    } = stack.build();

    let addr = BdAddr::new([0x00, 0x1F, 0xE2, 0x9F, 0x8A, 0x66]);
    let config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig {
            filter_accept_list: &[
                (AddrKind::PUBLIC, &addr),
                (AddrKind::RANDOM, &addr),
                (AddrKind::RESOLVABLE_PRIVATE_OR_PUBLIC, &addr),
                (AddrKind::RESOLVABLE_PRIVATE_OR_RANDOM, &addr),
                (AddrKind::ANONYMOUS_ADV, &addr),
            ],
            ..Default::default()
        },
    };

    info!("Scanning for peripheral...");
    let _ = join(runner.run(), async {
        let mut scanner = Scanner::new(central);
        loop {
            info!("Scanning...");
            let result = scanner.scan(&ScanConfig { ..Default::default() }).await;

            let session = match result {
                Ok(res) => res,
                Err(err) => {
                    match err {
                        BleHostError::Controller(err) => warn!("Controller error"),
                        BleHostError::BleHost(err) => warn!("BleHost: {}", err),
                    }
                    continue;
                }
            };
            session;
        }
    })
    .await;
}
