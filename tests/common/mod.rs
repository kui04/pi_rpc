use std::time::Duration;

use pi_rpc::client::Config;
use pi_rpc::error::{Error, ErrorKind, Result};
use pi_rpc::transport::Transport;
use tokio::sync::mpsc;

pub struct Duplex {
    pub out: mpsc::UnboundedSender<String>,
    pub inc: mpsc::UnboundedReceiver<String>,
}

pub struct DuplexTx {
    out: mpsc::UnboundedSender<String>,
}

pub struct DuplexRx {
    inc: mpsc::UnboundedReceiver<String>,
}

pub struct Peer {
    pub inc: mpsc::UnboundedReceiver<String>,
    pub out: mpsc::UnboundedSender<String>,
}

pub fn pair() -> (Duplex, Peer) {
    let (c2p_tx, c2p_rx) = mpsc::unbounded_channel();
    let (p2c_tx, p2c_rx) = mpsc::unbounded_channel();
    (
        Duplex {
            out: c2p_tx,
            inc: p2c_rx,
        },
        Peer {
            inc: c2p_rx,
            out: p2c_tx,
        },
    )
}

impl pi_rpc::transport::TransportSend for DuplexTx {
    async fn send(&mut self, line: String) -> Result<()> {
        self.out
            .send(line)
            .map_err(|_| Error::new(ErrorKind::Transport, "peer gone"))
    }

    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl pi_rpc::transport::TransportRecv for DuplexRx {
    async fn recv(&mut self) -> Result<Option<String>> {
        Ok(self.inc.recv().await)
    }
}

impl Transport for Duplex {
    type Tx = DuplexTx;
    type Rx = DuplexRx;

    fn split(self) -> (DuplexTx, DuplexRx) {
        (DuplexTx { out: self.out }, DuplexRx { inc: self.inc })
    }
}

pub fn fast_config() -> Config {
    Config::default()
        .with_request_timeout(Duration::from_millis(300))
        .with_wait_timeout(Duration::from_millis(300))
        .with_event_capacity(64)
        .with_max_line(1024 * 1024)
}
