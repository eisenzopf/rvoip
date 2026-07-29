//! vCon core document model, builder, signing, and persistence.
//!
//! The model is pinned to `draft-ietf-vcon-vcon-core` working-group
//! commit `2342aba64bdb71d9e80ab6e274a3921e2b1c769e`.
//! Unsigned vCons are represented by [`Vcon`]. Signed vCons use JWS
//! General JSON Serialization through [`SignedVcon`]; signing is always
//! explicit and this crate does not implement JWE encryption.

pub mod builder;
mod hash;
mod jws;
pub mod store;
pub mod types;

pub use builder::VconBuilder;
pub use hash::{content_hash, encode_base64url};
pub use jws::{
    append_signature, sign_jws, verify_jws, verify_jws_with, CertificateReference, JwsHeader,
    JwsProtectedHeader, JwsSignature, SignedVcon, TrustedKey,
};
pub use store::{MemoryVconStore, VconStore, VconStoreError};
pub use types::{
    Amended, Analysis, Attachment, CivicAddress, ContentEncoding, ContentHashes, Dialog,
    DialogKind, Disposition, ExtraFields, IndexReferences, Party, PartyChannel, PartyHistory,
    PartyHistoryEvent, PartyIndices, Redacted, SessionId, SessionIdChannel, SessionIds, Vcon,
    VconError,
};
