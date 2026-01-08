#[cfg(not(target_os = "macos"))]
compile_error!("Only MacOS is supported");

use std::io;

use bt_hci::transport::{self, WithIndicator};
use bt_hci::{ControllerToHostPacket, FromHciBytes as _, HostToControllerPacket, WriteHci as _};
use hidapi::{HidApi, HidError};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
    FromHciBytes(bt_hci::FromHciBytesError),
    Hid(HidError),
    Io(io::Error),
}

impl core::error::Error for Error {}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl From<bt_hci::FromHciBytesError> for Error {
    fn from(e: bt_hci::FromHciBytesError) -> Self {
        Self::FromHciBytes(e)
    }
}

impl From<HidError> for Error {
    fn from(e: HidError) -> Self {
        Self::Hid(e)
    }
}

#[derive(Clone)]
pub struct Transport {
    tx: mpsc::Sender<DeviceCmd>,
}

// Internal message types for the worker thread
enum DeviceCmd {
    Write {
        data: Vec<u8>,
        resp: oneshot::Sender<Result<(), Error>>,
    },
    Read {
        resp: oneshot::Sender<Result<Vec<u8>, Error>>,
    },
}

impl Transport {
    pub fn new() -> Result<Self, Error> {
        pub const VENDOR_ID: u16 = 0x54c;
        pub const PRODUCT_ID_OLD: u16 = 0x5c4;

        let api = HidApi::new().map_err(Error::Hid)?;
        // We wrap the device in an Arc to share it between the two dedicated threads
        let device = api.open(VENDOR_ID, PRODUCT_ID_OLD).map_err(Error::Hid)?;

        let (tx, mut rx) = mpsc::channel(32);

        // Spawn the blocking worker thread
        std::thread::spawn(move || {
            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    DeviceCmd::Write { data, resp } => {
                        let res = device.write(&data).map(|_| ()).map_err(Error::Hid);
                        let _ = resp.send(res);
                    }
                    DeviceCmd::Read { resp } => {
                        let mut buf = [0u8; 1024]; // Standard HCI MTU size
                        let res = device.read(&mut buf).map(|n| buf[..n].to_vec()).map_err(Error::Hid);
                        let _ = resp.send(res);
                    }
                }
            }
        });

        Ok(Self { tx })
    }
}

impl transport::Transport for Transport {
    async fn read<'a>(&self, rx: &'a mut [u8]) -> Result<ControllerToHostPacket<'a>, Self::Error> {
        eprintln!("read");
        let (resp_tx, resp_rx) = oneshot::channel();

        self.tx.send(DeviceCmd::Read { resp: resp_tx }).await.unwrap();

        let data = resp_rx.await.unwrap()?;

        let len = data.len().min(rx.len());
        rx[..len].copy_from_slice(&data[..len]);

        Ok(ControllerToHostPacket::from_hci_bytes_complete(&rx[..len])?)
    }

    async fn write<T: HostToControllerPacket>(&self, val: &T) -> Result<(), Self::Error> {
        eprintln!("write");
        let mut buf = Vec::<u8>::new();
        WithIndicator::new(val).write_hci(&mut buf).unwrap();

        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(DeviceCmd::Write {
                data: buf,
                resp: resp_tx,
            })
            .await
            .unwrap();
        resp_rx.await.unwrap()
    }
}

impl embedded_io::ErrorType for Transport {
    type Error = Error;
}
