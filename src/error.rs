//! Canonical error for this crate. Own design, no upstream mirror.
//!
//! Timeout default mirrors `RpcClient.send` (30s) at
//! `packages/coding-agent/src/modes/rpc/rpc-client.ts:583`.

use std::fmt;

/// Fallible result for every API in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure modes for every fallible API in this crate.
///
/// New kinds may arrive in later versions; match with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The byte path failed (write error, read error, peer gone).
    Transport,
    /// A line was not valid protocol (bad UTF-8, bad JSON, over cap).
    Protocol,
    /// Pi rejected the command, such as an unknown model.
    Command,
    /// A deadline passed before the matching record arrived.
    Timeout,
    /// The peer closed cleanly while work was still pending.
    Closed,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ErrorKind::Transport => "transport",
            ErrorKind::Protocol => "protocol",
            ErrorKind::Command => "command",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Closed => "closed",
        };
        f.write_str(name)
    }
}

/// Canonical error for this crate: a kind plus a human message.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct Error {
    kind: ErrorKind,
    message: String,
    #[source]
    cause: Option<std::sync::Arc<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    /// Build an error from a kind and a message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            cause: None,
        }
    }

    /// Which failure mode this is.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Human-readable detail.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self {
            kind: ErrorKind::Transport,
            message: err.to_string(),
            cause: Some(std::sync::Arc::new(err)),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self {
            kind: ErrorKind::Protocol,
            message: err.to_string(),
            cause: Some(std::sync::Arc::new(err)),
        }
    }
}

impl From<tokio::time::error::Elapsed> for Error {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        Self::new(ErrorKind::Timeout, "deadline passed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_names_stay_short() {
        assert_eq!(ErrorKind::Transport.to_string(), "transport");
        assert_eq!(ErrorKind::Closed.to_string(), "closed");
    }

    #[test]
    fn io_maps_to_transport() {
        let err = Error::from(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "pipe gone",
        ));
        assert_eq!(err.kind(), ErrorKind::Transport);
    }

    #[test]
    fn serde_maps_to_protocol() {
        let serde_err = serde_json::from_str::<serde_json::Value>("{oops").unwrap_err();
        let err = Error::from(serde_err);
        assert_eq!(err.kind(), ErrorKind::Protocol);
    }
}
