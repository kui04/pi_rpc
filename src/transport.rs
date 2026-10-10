//! Byte-path abstraction. Own design; I/O contract mirrors
//! `RpcClient` startup and shutdown behavior at
//! `packages/coding-agent/src/modes/rpc/rpc-client.ts:118-225`
//! (`start`, `stop`, stdin errors, exit handling).
//!
//! A backend moves one full JSON record per call, without the LF
//! terminator. Backends over raw byte streams frame with
//! [`Decoder`](crate::frame::Decoder) and keep stderr out of band.
//!
//! Reads and writes split into separate halves, so a slow send never
//! stalls reads. Each half is owned by exactly one task.
//!
//! The halves spell `-> impl Future + Send` instead of `async_trait`:
//! no half is ever used as `dyn`, and the explicit bound keeps spawned
//! tasks `Send` with no per-call box. (`UiHandler` keeps the attribute
//! because handlers run boxed as trait objects.)

use crate::error::Result;

/// Write half of a transport: outbound records plus orderly shutdown.
pub trait TransportSend: Send + 'static {
    /// Send one record without its LF terminator.
    ///
    /// Must not block without bound: the loop awaits one send at a
    /// time. Backends with backpressure fail fast or shed instead of
    /// holding the call open.
    fn send(&mut self, line: String) -> impl Future<Output = Result<()>> + Send;

    /// Shut down the write side. Reads drain first where the path allows.
    fn close(&mut self) -> impl Future<Output = Result<()>> + Send;
}

/// Read half of a transport: inbound records until clean end.
pub trait TransportRecv: Send + 'static {
    /// Receive one record without its terminator. `None` means clean end.
    ///
    /// Must be cancel-safe: the reader task is aborted on shutdown, so a
    /// dropped `recv` future must lose no bytes. Keep all partial input
    /// inside `self` ([`Decoder`](crate::frame::Decoder) does this) and
    /// return only complete lines.
    fn recv(&mut self) -> impl Future<Output = Result<Option<String>>> + Send;
}

/// One transport over any byte path: stdio, HTTP stream, SSH channel.
///
/// Rules: reads run in a loop that never stops early; a stalled reader
/// stalls pi. No timeouts inside; the client wraps every wait with a
/// deadline. Keep stderr out of band.
///
/// Splitting keeps reads independent of writes. `send` still must not
/// block without bound (see [`TransportSend::send`]).
pub trait Transport: Send + 'static {
    /// Write half, owned by the loop task.
    type Tx: TransportSend;
    /// Read half, owned by the reader task.
    type Rx: TransportRecv;

    /// Split into independently owned halves.
    fn split(self) -> (Self::Tx, Self::Rx);
}
