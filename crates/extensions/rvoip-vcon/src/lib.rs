//! `rvoip-vcon` — vCon document builder + store.
//!
//! vCon ("Virtualized Conversation") is the IETF-WG container format
//! for a recorded conversation's metadata + media references. Per
//! `docs/CONVERSATION_PROTOCOL.md` §7.6 and
//! `INTERFACE_DESIGN.md` §3.9, every UCTP-family adapter emits a
//! `RecordingComplete` event at `session.ended` carrying a
//! `VconRef`; this crate is what actually produces the vCon document
//! that the `VconRef::Local { uuid }` resolves to.
//!
//! ## Surface
//!
//! - [`Vcon`] / [`Party`] / [`Dialog`] / [`Attachment`] — the spec
//!   data types, `serde`-shaped to round-trip the wire JSON.
//! - [`VconBuilder`] — fluent constructor; consumes rvoip-core
//!   session metadata (participants, timeline, codec) and produces a
//!   `Vcon`.
//! - [`VconStore`] trait + [`MemoryVconStore`] — pluggable persistence
//!   indexed by `uuid::Uuid`. Production deployments swap in disk /
//!   S3 / database stores; tests and the v0 demo use the in-memory
//!   one.
//! - [`sign_jws`] — optional JWS signature wrapper using the same
//!   `jsonwebtoken` stack as `rvoip-auth-core`. Signing is opt-in for
//!   v0.x — deployments without a configured signing key fall back to
//!   plain unsigned vCons.
//!
//! ## What this is NOT (yet)
//!
//! - JWE encryption (vCon §4.4) — opt-in v0.x followup once a real
//!   recipient-key model is wired up.
//! - Redaction lineage (vCon §6) — append-only redactions are a
//!   v1 feature; this crate exposes the data model but doesn't
//!   automatically redact PII.
//! - HTTPS-resolvable vCon URIs (`VconRef::Url`) — the variant is
//!   reserved in `rvoip_core::vcon::VconRef` but this crate only
//!   produces `Local { uuid }` references in v0.x.

pub mod builder;
pub mod store;
pub mod types;

pub use builder::VconBuilder;
pub use store::{MemoryVconStore, VconStore, VconStoreError};
pub use types::{Attachment, Dialog, DialogKind, Party, Vcon, VconError};

/// JWS-sign a `Vcon` using the supplied HMAC secret (HS256) or
/// RSA/EC PEM. Returns the compact JWS string the consumer stores or
/// transmits. Verification helpers live in [`crate::builder`].
///
/// Optional — deployments that don't need signed vCons skip this
/// entirely and persist the [`Vcon`] JSON directly.
pub fn sign_jws(
    vcon: &Vcon,
    encoding_key: &jsonwebtoken::EncodingKey,
    algorithm: jsonwebtoken::Algorithm,
) -> Result<String, VconError> {
    let header = jsonwebtoken::Header::new(algorithm);
    jsonwebtoken::encode(&header, vcon, encoding_key).map_err(|e| VconError::Sign(e.to_string()))
}
