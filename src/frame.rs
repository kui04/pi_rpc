//! LF-only JSONL framing.
//!
//! Source: pi `f1b2e77`
//! `packages/coding-agent/src/modes/rpc/jsonl.ts:10-21`
//! (`serializeJsonLine`, `attachJsonlLineReader`).
//! Rules: `packages/coding-agent/docs/rpc.md:50-56` (Framing).
//!
//! Two strict deviations: the cap also bounds the remainder (upstream only
//! knows whole frames), and `end` rejects a trailing partial instead of
//! emitting it (`jsonl.ts:43-49`). Both fail loudly over parsing halves.

use serde::Serialize;

use crate::error::{Error, ErrorKind};

/// Upper bound for one JSONL record, mirroring the 16 MiB CBOR frame cap.
pub const DEFAULT_MAX_LINE: usize = 16 * 1024 * 1024;

/// Consumed prefix dropped at once past this size.
const DRAIN_AT: usize = 65536;

/// Spare capacity shed after a drain past this size.
const SHRINK_ABOVE: usize = 1048576;

/// Serialize one record plus a single LF terminator.
pub fn encode_line<T: Serialize>(value: &T) -> Result<String, Error> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    Ok(line)
}

/// Incremental LF-only splitter for arbitrary byte chunks.
///
/// Split points are `\n` bytes only. A `\r` directly before `\n` is stripped.
/// Unicode separators such as U+2028 inside strings never split.
///
/// A head offset marks consumed bytes instead of draining per line, so
/// small-chunk feeds stay linear. The consumed prefix drops past 64 KiB.
///
/// A scan cursor resumes each search where the last one found no LF, so a
/// long line fed in small chunks scans once, not once per chunk.
#[derive(Debug)]
pub struct Decoder {
    buf: Vec<u8>,
    head: usize,
    scan: usize,
    max_line: usize,
}

impl Decoder {
    /// New decoder with the default line cap.
    pub fn new() -> Self {
        Self::with_max_line(DEFAULT_MAX_LINE)
    }

    /// New decoder with an explicit line cap in bytes.
    pub fn with_max_line(max_line: usize) -> Self {
        Self {
            buf: Vec::with_capacity(8192),
            head: 0,
            scan: 0,
            max_line,
        }
    }

    /// Feed bytes, return each complete line without its terminator.
    ///
    /// Splits first. Each complete line trips the cap on its own size;
    /// only the trailing remainder without LF may also trip it, so an
    /// endless line can never grow past the cap.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, Error> {
        self.buf.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(rel) = memchr::memchr(b'\n', &self.buf[self.scan..]) {
            let end = self.scan + rel;
            let mut raw = &self.buf[self.head..end];
            if raw.last() == Some(&b'\r') {
                raw = &raw[..raw.len() - 1];
            }
            if raw.len() > self.max_line {
                return Err(Error::new(
                    ErrorKind::Protocol,
                    format!(
                        "line of {} bytes exceeds cap of {}",
                        raw.len(),
                        self.max_line
                    ),
                ));
            }
            let text = std::str::from_utf8(raw)
                .map_err(|err| {
                    Error::new(ErrorKind::Protocol, format!("line is not UTF-8: {err}"))
                })?
                .to_string();
            lines.push(text);
            self.head = end + 1;
            self.scan = self.head;
        }
        self.scan = self.buf.len();
        if self.head >= DRAIN_AT {
            self.buf.drain(..self.head);
            let dropped = self.head;
            self.head = 0;
            self.scan -= dropped;
            if self.buf.capacity() > SHRINK_ABOVE {
                self.buf.shrink_to_fit();
            }
        }
        if self.buf.len() - self.head > self.max_line {
            return Err(Error::new(
                ErrorKind::Protocol,
                format!(
                    "buffered {} bytes past cap of {} without LF",
                    self.buf.len() - self.head,
                    self.max_line
                ),
            ));
        }
        Ok(lines)
    }

    /// Reject a trailing partial line at clean end of stream.
    ///
    /// Stricter than upstream on purpose: `jsonl.ts` emits the residual
    /// buffer as a final line, but a line without LF is corruption here,
    /// so it fails instead of parsing half a record.
    pub fn end(&mut self) -> Result<(), Error> {
        if self.head == self.buf.len() {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Protocol,
                format!(
                    "{} trailing bytes after last LF",
                    self.buf.len() - self.head
                ),
            ))
        }
    }

    /// Drop all buffered bytes and cursors after an error.
    ///
    /// Errors are fatal for the buffered line: `head` and `scan` freeze
    /// at the bad line, so every later `push` would fail again. Call this
    /// to resync, knowing the bad line is gone.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.head = 0;
        self.scan = 0;
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_appends_single_lf() {
        let line = encode_line(&serde_json::json!({"type": "abort"})).unwrap();
        assert_eq!(line, "{\"type\":\"abort\"}\n");
    }
}
