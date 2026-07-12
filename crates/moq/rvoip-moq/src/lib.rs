//! Stable rvoip models and lifecycle adapters for MOQT broadcasting.
//!
//! Bridgefu and other rvoip applications only use the types exported by this
//! crate. The draft-specific `moq-rs` implementation is kept behind a private
//! wire adapter so a future, wire-incompatible MOQT draft is contained here.

mod authorization;
mod catalog;
mod catalog_subscriber;
mod catalog_subscriber_wire;
mod catalog_subscription;
mod compatibility;
mod error;
mod group;
mod loc;
mod namespace;
mod publisher;
#[cfg(feature = "relay-admission")]
mod relay_admission;
#[cfg(feature = "relay-runtime")]
mod relay_runtime;
mod replay;
mod session_lease;
mod wire;

pub use authorization::{
    MoqAction, MoqAuthorizationError, MoqAuthorizationGrant, MoqAuthorizationRequest,
    MoqAuthorizer, MoqPeerIdentity, MoqResource, MoqRevocationChecker, MoqRevocationError,
    MoqRevocationStatus, SecureMoqAuthorizer,
};
pub use catalog::{MsfCatalog, MsfCatalogError, MsfCatalogState, MsfTrack, MSF_CATALOG_VERSION};
pub use catalog_subscriber::{
    MoqCatalogApplyOutcome, MoqCatalogDeliveryMode, MoqCatalogObject, MoqCatalogStateMachine,
    MoqCatalogSubscriberConfig, MoqCatalogSubscriberConfigError, MoqCatalogSubscriberFailure,
    MoqCatalogSubscriberLifecycle, MoqCatalogSubscriptionSnapshot, MoqCatalogUpdate,
    MoqCatalogValidationError, MoqEndOfGroupEvidence, MoqSubscriberCredential,
    MoqSubscriberCredentialError, MoqSubscriberCredentialProvider, MoqSubscriberCredentialRequest,
    DEFAULT_MAX_CATALOG_BYTES, MAX_CATALOG_ATTEMPT_TIMEOUT, MAX_CATALOG_BYTES,
    MAX_CATALOG_RECONNECT_ATTEMPTS, MAX_CATALOG_RECONNECT_BACKOFF, MAX_CATALOG_RECONNECT_DEADLINE,
    MAX_MOQ_SUBSCRIBER_CREDENTIAL_BYTES,
};
pub use catalog_subscription::{MoqCatalogSubscriber, MoqCatalogSubscriberTlsConfig};
pub use compatibility::{
    MoqCompatibility, MoqCompatibilityError, MoqProtocolVersion, LOC_DRAFT, LOC_DRAFT_NUMBER,
    MOQT_DRAFT, MOQT_DRAFT_NUMBER, MOQT_NEGOTIATED_PROTOCOL, MSF_DRAFT, MSF_DRAFT_NUMBER,
    TARGET_MOQT_DRAFT,
};
pub use error::{MoqError, MoqRelayFailure};
pub use group::{InMemoryMoqGroupIdAllocator, MoqGroupIdAllocationError, MoqGroupIdAllocator};
pub use loc::{
    validate_opus_20ms_mono, LocAudioObject, LocError, LocOpusPacketizer, LocPacketizedFrame,
    LocProperty, LocTimestampDiscontinuity, LOC_TIMESCALE_PROPERTY, LOC_TIMESTAMP_PROPERTY,
    OPUS_CHANNELS, OPUS_FRAME_DURATION_MS, OPUS_RTP_TIMESTAMP_STEP, OPUS_SAMPLE_RATE,
};
pub use namespace::{MoqNamespace, MoqNamespaceError, NamespaceComponent};
#[cfg(feature = "insecure-development")]
pub use publisher::MoqRelayDevelopmentMode;
pub use publisher::{
    MoqBroadcastPublisher, MoqPublisherConfig, MoqRelayClient, MoqRelayConnectionPolicy,
    MoqRelayHealthIssue, MoqRelayHealthSnapshot, MoqRelayPeerIdentity, MoqRelayPublication,
    MoqRelaySubstratePolicy, MoqRelayTlsConfig,
};
#[cfg(feature = "relay-admission")]
pub use relay_admission::{
    MoqRelayAdmissionConfig, MoqRelayAdmissionSubstrate, RvoipMoqRelayAdmission,
};
#[cfg(feature = "relay-runtime")]
pub use relay_runtime::{
    MoqRelayDeploymentMode, MoqRelayListenerKind, MoqRelayPublisherBinding, MoqRelayResourceLimits,
    MoqRelayRuntime, MoqRelayRuntimeConfig, MoqRelayRuntimeError, MoqRelayRuntimeLifecycle,
    MoqRelayRuntimeLimits, MoqRelayRuntimeSecurity, MoqRelayRuntimeSnapshot,
    MoqRelayRuntimeTimeouts, MoqRelayServerTlsConfig, MoqRelayTopology,
};
pub use replay::{
    BoundedMemoryMoqReplayStore, MoqReplayError, MoqSessionId, MoqTokenBinding,
    MoqTokenReplayStore, MAX_MOQ_SESSION_ID_BYTES,
};
pub use session_lease::{
    BoundedMemoryMoqSessionLeaseStore, MoqSessionLease, MoqSessionLeaseBinding,
    MoqSessionLeaseClose, MoqSessionLeaseError, MoqSessionLeaseLimits, MoqSessionLeaseSnapshot,
    MoqSessionLeaseStore,
};

/// Canonical audio track used by the Bridgefu 1.0 broadcast profile.
pub const AUDIO_TRACK: &str = "audio/main";

/// MSF catalog track for the canonical audio publication.
pub const CATALOG_TRACK: &str = "catalog";
