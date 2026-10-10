#![forbid(unsafe_code)]
//! Transport-free wrapper for the pi coding-agent RPC protocol.
//!
//! The crate owns protocol logic: framing, typed records, and a client that
//! multiplexes commands and events. It owns no socket, no process, no TLS.
//! Users supply a transport over any byte path such as stdio pipes, an HTTP
//! stream, or an SSH channel.
//!
//! Pinned upstream: pi `f1b2e77f5b13b2a199b1052cb79c235451afe7d7`
//! (see `Cargo.toml [package.metadata.pi_rpc]`). Each module cites the exact
//! source file and lines it mirrors, so reviewers can check every shape.
//!
//! # Compat
//!
//! Reads ignore unknown fields, and unknown record types map to catch-all
//! variants, so newer pi versions keep parsing. Writes use the exact pinned
//! shape. A pi update means a new pinned hash, a new review, and a new
//! golden run, never silent drift.
//!
//! Three normalizations apply on write: missing and explicit-null optionals
//! both omit; wire integers may print as floats (values stay equal).
//! Unknown records replay verbatim at every depth, including inside lists,
//! trees, and event payloads.
//!
//! Strict write drops unknown fields on known records: a future field pi
//! sends still parses (reads ignore it) but does not survive re-encode.
//! Only unknown records keep every byte.
//!
//! Unmatched lines never reach listeners: responses without a waiter,
//! malformed lines, and bad events count under [`Client::drop_counts`](client::Client::drop_counts)
//! instead. This differs from upstream on purpose; the counts make drift
//! visible where upstream stays silent.
//!
//! # Errors
//!
//! Fallible methods share one [`Error`](error::Error) type instead of per-method errors.
//! Read [`ErrorKind`](error::ErrorKind) for the five cases: `Transport` for broken paths,
//! `Protocol` for bad records, `Command` for rejected commands, `Timeout`
//! for missed deadlines, `Closed` for ended streams. `Error::message`
//! carries the detail; `source()` keeps the cause where one exists.
//!
//! # Transport guide
//!
//! Implement [`Transport`](transport::Transport) with three rules: move one full record per call
//! without its LF terminator, read in a loop without stopping (a stalled
//! reader stalls pi), and keep stderr out of band. Byte-stream backends
//! frame with [`Decoder`](frame::Decoder). The client wraps every wait in a deadline and
//! fails pending work as `Closed` when the path ends.
//!
//! # Numbers
//!
//! Wire integers decode to `f64` and print back as floats (`200000` may
//! write as `200000.0`). Values compare equal numerically; only the text
//! differs. Pi (JS numbers) sees no difference.
//!
//! # Example
//!
//! ```rust
//! use std::collections::VecDeque;
//! use std::sync::{Arc, Mutex};
//! use pi_rpc::client::Client;
//! use pi_rpc::error::Result;
//! use pi_rpc::transport::{Transport, TransportRecv, TransportSend};
//!
//! struct Script {
//!     lines: VecDeque<String>,
//!     sent: Vec<String>,
//!     ready: Arc<tokio::sync::Notify>,
//! }
//!
//! struct ScriptTx {
//!     shared: Arc<Mutex<Script>>,
//! }
//!
//! struct ScriptRx {
//!     shared: Arc<Mutex<Script>>,
//! }
//! impl TransportSend for ScriptTx {
//!     async fn send(&mut self, line: String) -> Result<()> {
//!         let shared = self.shared.clone();
//!         shared.lock().unwrap().sent.push(line);
//!         shared.lock().unwrap().ready.notify_one();
//!         Ok(())
//!     }
//!     async fn close(&mut self) -> Result<()> {
//!         Ok(())
//!     }
//! }
//! impl TransportRecv for ScriptRx {
//!     async fn recv(&mut self) -> Result<Option<String>> {
//!         // Toy rule: answers follow commands, like the real stream.
//!         loop {
//!             let ready = {
//!                 let mut script = self.shared.lock().unwrap();
//!                 if !script.sent.is_empty() {
//!                     break Ok(script.lines.pop_front());
//!                 }
//!                 script.ready.clone()
//!             };
//!             ready.notified().await;
//!         }
//!     }
//! }
//! impl Transport for Script {
//!     type Tx = ScriptTx;
//!     type Rx = ScriptRx;
//!
//!     fn split(self) -> (ScriptTx, ScriptRx) {
//!         let shared = Arc::new(Mutex::new(self));
//!         (
//!             ScriptTx {
//!                 shared: shared.clone(),
//!             },
//!             ScriptRx { shared },
//!         )
//!     }
//! }
//!
//! let rt = tokio::runtime::Builder::new_current_thread()
//!     .enable_all()
//!     .build()
//!     .unwrap();
//! rt.block_on(async {
//!     let script = Script {
//!         lines: VecDeque::from([
//!             r#"{"id":"req_1","type":"response","command":"get_state","success":true,"data":{"thinkingLevel":"medium","isStreaming":false,"isCompacting":false,"steeringMode":"all","followUpMode":"one-at-a-time","sessionId":"s1","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}"#.to_string(),
//!         ]),
//!         sent: Vec::new(),
//!         ready: Arc::new(tokio::sync::Notify::new()),
//!     };
//!     let client = Client::new(script);
//!     let state = client.get_state().await.unwrap();
//!     assert_eq!(state.session_id, "s1");
//!     client.close().await.unwrap();
//! });
//! ```

pub mod client;
pub mod command;
pub mod entry;
pub mod error;
pub mod event;
pub mod extui;
pub mod frame;
pub mod message;
pub mod proto_v8;
pub mod response;
pub mod transport;
pub mod types;

/// Re-exported for backend and handler impls, which need the attribute.
/// Foreign re-exports are exempt from the single-path rule.
#[doc(inline)]
pub use async_trait::async_trait;
