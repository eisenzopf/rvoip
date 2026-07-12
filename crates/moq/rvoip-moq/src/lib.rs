//! Stable rvoip models and lifecycle adapters for MOQT broadcasting.
//!
//! Bridgefu and other rvoip applications only use the types exported by this
//! crate. The draft-specific `moq-rs` implementation is kept behind a private
//! wire adapter so a future, wire-incompatible MOQT draft is contained here.

mod authorization;
mod catalog;
mod compatibility;
mod error;
mod loc;
mod namespace;
mod publisher;
mod replay;
mod wire;

pub use authorization::{
    MoqAction, MoqAuthorizationError, MoqAuthorizationGrant, MoqAuthorizationRequest,
    MoqAuthorizer, MoqPeerIdentity, MoqResource, MoqRevocationChecker, MoqRevocationError,
    MoqRevocationStatus, SecureMoqAuthorizer,
};
pub use catalog::{MsfCatalog, MsfCatalogError, MsfTrack, MSF_CATALOG_VERSION};
pub use compatibility::{
    MoqCompatibility, MoqCompatibilityError, MoqProtocolVersion, LOC_DRAFT, LOC_DRAFT_NUMBER,
    MOQT_DRAFT, MOQT_DRAFT_NUMBER, MSF_DRAFT, MSF_DRAFT_NUMBER, TARGET_MOQT_DRAFT,
};
pub use error::MoqError;
pub use loc::{
    validate_opus_20ms_mono, LocAudioObject, LocError, LocOpusPacketizer, LocPacketizedFrame,
    LocProperty, LocTimestampDiscontinuity, LOC_TIMESCALE_PROPERTY, LOC_TIMESTAMP_PROPERTY,
    OPUS_CHANNELS, OPUS_FRAME_DURATION_MS, OPUS_RTP_TIMESTAMP_STEP, OPUS_SAMPLE_RATE,
};
pub use namespace::{MoqNamespace, MoqNamespaceError, NamespaceComponent};
pub use publisher::{
    MoqBroadcastPublisher, MoqPublisherConfig, MoqRelayClient, MoqRelayPublication,
    MoqRelayTlsConfig,
};
pub use replay::{
    BoundedMemoryMoqReplayStore, MoqReplayError, MoqSessionId, MoqTokenBinding,
    MoqTokenReplayStore, MAX_MOQ_SESSION_ID_BYTES,
};

/// Canonical audio track used by the Bridgefu 1.0 broadcast profile.
pub const AUDIO_TRACK: &str = "audio/main";

/// MSF catalog track for the canonical audio publication.
pub const CATALOG_TRACK: &str = "catalog";
