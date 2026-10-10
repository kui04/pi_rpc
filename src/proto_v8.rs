//! Reserve for the experimental pi protocol, version 8. No code in v1.
//!
//! Source: pi `f1b2e77`
//! `packages/protocol/src/protocol.ts` (hello handshake, routed targets,
//! request and cancel envelopes, subscription updates, attachment route),
//! `packages/protocol/src/framing.ts` (u32 big-endian length prefix,
//! 16 MiB default cap).
//!
//! Rules for v2: one frame is a four-byte big-endian length plus one
//! definite-length CBOR item. Envelopes reject unknown properties.
//! Opaque payloads stay strict JSON. Transports keep byte order.
//! Peer auth is out of scope upstream and stays out here.
