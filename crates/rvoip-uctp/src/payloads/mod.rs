//! Typed payload structs for each `MessageType` variant.
//!
//! Decode flow: parse to `UctpEnvelope<serde_json::Value>` first
//! (forward-compat), then call `env.decode_payload::<T>()` against one of
//! the structs in this module.
//!
//! Payload shapes mirror the JSON examples in CONVERSATION_PROTOCOL.md
//! §5–§9, §11. They use `#[serde(deny_unknown_fields = false)]` (i.e.,
//! the default) so wire-level additions don't break decoding.

pub mod auth;
pub mod capability;
pub mod connection;
pub mod control;
pub mod conversation;
pub mod message;
pub mod session;
pub mod stream;
