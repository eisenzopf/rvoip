//! rvoip-owned contracts and validation for an MSF catalog subscriber.
//!
//! The network adapter deliberately feeds the validator below with primitive
//! metadata. Draft-specific `moq-rs` readers and session handles therefore do
//! not escape this crate, and applications can consume catalog state through a
//! bounded `watch`-style snapshot model.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_core_traits::broadcast::{BroadcastHealthDescriptor, BroadcastSubstrate};
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    MoqNamespace, MoqProtocolVersion, MoqRelayPeerIdentity, MoqRelaySubstratePolicy, MsfCatalog,
    MsfCatalogState, CATALOG_TRACK,
};

/// Maximum SETUP authorization value accepted by the pinned wire engine.
pub const MAX_MOQ_SUBSCRIBER_CREDENTIAL_BYTES: usize = 4 * 1024;

/// Default upper bound for one MSF catalog object.
pub const DEFAULT_MAX_CATALOG_BYTES: usize = 64 * 1024;

/// Hard safety cap for a single decoded MSF catalog object.
pub const MAX_CATALOG_BYTES: usize = 1024 * 1024;

/// Hard cap on reconnect attempts retained in one subscriber configuration.
pub const MAX_CATALOG_RECONNECT_ATTEMPTS: u32 = 100;

/// Hard cap on one connection attempt.
pub const MAX_CATALOG_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Hard cap on a complete reconnect window.
pub const MAX_CATALOG_RECONNECT_DEADLINE: Duration = Duration::from_secs(60 * 60);

/// Hard cap on an individual reconnect delay.
pub const MAX_CATALOG_RECONNECT_BACKOFF: Duration = Duration::from_secs(5 * 60);

const MAX_ENDPOINT_BYTES: usize = 2 * 1024;
const DEFAULT_DEDUPE_ENTRIES: usize = 256;

/// Opaque, single-use credential for one outbound MOQT SETUP.
///
/// It is intentionally not `Clone`, does not serialize, redacts `Debug`, and
/// overwrites its owned buffer when dropped. A reconnect must ask the provider
/// for another value; the eventual transport supervisor also rejects a value
/// reused by the same subscription.
pub struct MoqSubscriberCredential {
    bytes: Vec<u8>,
}

impl MoqSubscriberCredential {
    pub fn new(mut bytes: Vec<u8>) -> Result<Self, MoqSubscriberCredentialError> {
        if bytes.is_empty() {
            return Err(MoqSubscriberCredentialError::Empty);
        }
        if bytes.len() > MAX_MOQ_SUBSCRIBER_CREDENTIAL_BYTES {
            let actual = bytes.len();
            bytes.zeroize();
            return Err(MoqSubscriberCredentialError::TooLong {
                maximum: MAX_MOQ_SUBSCRIBER_CREDENTIAL_BYTES,
                actual,
            });
        }
        Ok(Self { bytes })
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Transfer the credential to the private wire adapter while preserving
    /// zeroization when that adapter-owned buffer is dropped.
    #[allow(dead_code)] // Used by the follow-on network adapter, never by applications.
    pub(crate) fn into_wire_bytes(mut self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(std::mem::take(&mut self.bytes))
    }

    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(&self.bytes).into()
    }
}

impl fmt::Debug for MoqSubscriberCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MoqSubscriberCredential")
            .field("bytes", &format_args!("<redacted:{}>", self.len()))
            .finish()
    }
}

impl Drop for MoqSubscriberCredential {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqSubscriberCredentialError {
    #[error("MOQT subscriber credential is empty")]
    Empty,
    #[error("MOQT subscriber credential is {actual} bytes; maximum is {maximum}")]
    TooLong { maximum: usize, actual: usize },
    #[error("MOQT subscriber credential provider is unavailable")]
    Unavailable,
    #[error("MOQT subscriber credential request was denied")]
    Denied,
}

/// Redaction-safe context supplied whenever a fresh SETUP token is required.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqSubscriberCredentialRequest {
    /// Canonical endpoint without query credentials or fragments.
    pub endpoint_uri: String,
    pub namespace: MoqNamespace,
    /// Zero for the first connection, one for its first reconnect, and so on.
    pub reconnect_attempt: u32,
    pub substrate: MoqRelaySubstratePolicy,
}

/// Issues a fresh, single-use SETUP credential for every connection attempt.
#[async_trait]
pub trait MoqSubscriberCredentialProvider: Send + Sync {
    async fn issue(
        &self,
        request: MoqSubscriberCredentialRequest,
    ) -> Result<MoqSubscriberCredential, MoqSubscriberCredentialError>;
}

/// Strict configuration for one catalog subscription.
#[derive(Clone)]
pub struct MoqCatalogSubscriberConfig {
    pub endpoint: Url,
    pub namespace: MoqNamespace,
    pub substrate: MoqRelaySubstratePolicy,
    pub max_catalog_bytes: usize,
    pub attempt_timeout: Duration,
    /// Total replacement-connection budget for this managed handle.
    ///
    /// The count does not reset after a successful reconnect, so a flapping
    /// peer cannot create an unbounded number of sessions. The deadline below
    /// does reset after success and bounds each individual outage window.
    pub max_reconnect_attempts: u32,
    pub reconnect_initial_backoff: Duration,
    pub reconnect_max_backoff: Duration,
    pub reconnect_deadline: Duration,
}

impl fmt::Debug for MoqCatalogSubscriberConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let endpoint = if self.endpoint.username().is_empty()
            && self.endpoint.password().is_none()
            && self.endpoint.query().is_none()
            && self.endpoint.fragment().is_none()
        {
            self.endpoint.as_str()
        } else {
            "<redacted-invalid-endpoint>"
        };
        formatter
            .debug_struct("MoqCatalogSubscriberConfig")
            .field("endpoint", &endpoint)
            .field("namespace", &self.namespace)
            .field("substrate", &self.substrate)
            .field("max_catalog_bytes", &self.max_catalog_bytes)
            .field("attempt_timeout", &self.attempt_timeout)
            .field("max_reconnect_attempts", &self.max_reconnect_attempts)
            .field("reconnect_initial_backoff", &self.reconnect_initial_backoff)
            .field("reconnect_max_backoff", &self.reconnect_max_backoff)
            .field("reconnect_deadline", &self.reconnect_deadline)
            .finish()
    }
}

impl MoqCatalogSubscriberConfig {
    pub fn new(endpoint: Url, namespace: MoqNamespace) -> Self {
        Self {
            endpoint,
            namespace,
            substrate: MoqRelaySubstratePolicy::WebTransport,
            max_catalog_bytes: DEFAULT_MAX_CATALOG_BYTES,
            attempt_timeout: Duration::from_secs(10),
            max_reconnect_attempts: 5,
            reconnect_initial_backoff: Duration::from_millis(100),
            reconnect_max_backoff: Duration::from_secs(5),
            reconnect_deadline: Duration::from_secs(30),
        }
    }

    /// Validate a canonical, credential-free target and bounded lifecycle.
    ///
    /// Token subscribers are scoped by the exact `/{tenant}/{broadcast}` path.
    /// Query parameters are forbidden because credentials belong in SETUP and
    /// URLs are routinely copied into diagnostics.
    pub fn validate(&self) -> Result<(), MoqCatalogSubscriberConfigError> {
        if self.endpoint.as_str().len() > MAX_ENDPOINT_BYTES {
            return Err(MoqCatalogSubscriberConfigError::EndpointTooLong {
                maximum: MAX_ENDPOINT_BYTES,
            });
        }
        if self.endpoint.scheme() != "moqt" {
            return Err(MoqCatalogSubscriberConfigError::UnsupportedScheme);
        }
        if self.endpoint.host_str().is_none_or(str::is_empty) {
            return Err(MoqCatalogSubscriberConfigError::MissingAuthority);
        }
        if !self.endpoint.username().is_empty() || self.endpoint.password().is_some() {
            return Err(MoqCatalogSubscriberConfigError::UserInfoForbidden);
        }
        if self.endpoint.query().is_some() {
            return Err(MoqCatalogSubscriberConfigError::QueryForbidden);
        }
        if self.endpoint.fragment().is_some() {
            return Err(MoqCatalogSubscriberConfigError::FragmentForbidden);
        }
        if self.substrate == MoqRelaySubstratePolicy::Auto {
            return Err(MoqCatalogSubscriberConfigError::ExactSubstrateRequired);
        }
        let expected = format!("/{}", self.namespace.as_str());
        if self.endpoint.path() != expected {
            return Err(MoqCatalogSubscriberConfigError::PathMismatch {
                expected,
                actual: self.endpoint.path().to_owned(),
            });
        }
        if !(1..=MAX_CATALOG_BYTES).contains(&self.max_catalog_bytes) {
            return Err(MoqCatalogSubscriberConfigError::InvalidCatalogLimit {
                maximum: MAX_CATALOG_BYTES,
            });
        }
        if self.attempt_timeout.is_zero() || self.attempt_timeout > MAX_CATALOG_ATTEMPT_TIMEOUT {
            return Err(MoqCatalogSubscriberConfigError::InvalidAttemptTimeout {
                maximum: MAX_CATALOG_ATTEMPT_TIMEOUT,
            });
        }
        if self.max_reconnect_attempts > MAX_CATALOG_RECONNECT_ATTEMPTS {
            return Err(MoqCatalogSubscriberConfigError::TooManyReconnectAttempts {
                maximum: MAX_CATALOG_RECONNECT_ATTEMPTS,
            });
        }
        if self.reconnect_deadline > MAX_CATALOG_RECONNECT_DEADLINE
            || (self.max_reconnect_attempts > 0 && self.reconnect_deadline.is_zero())
        {
            return Err(MoqCatalogSubscriberConfigError::InvalidReconnectDeadline {
                maximum: MAX_CATALOG_RECONNECT_DEADLINE,
            });
        }
        if self.reconnect_initial_backoff > MAX_CATALOG_RECONNECT_BACKOFF
            || self.reconnect_max_backoff > MAX_CATALOG_RECONNECT_BACKOFF
            || (self.max_reconnect_attempts > 0
                && (self.reconnect_initial_backoff.is_zero()
                    || self.reconnect_max_backoff.is_zero()))
        {
            return Err(MoqCatalogSubscriberConfigError::InvalidReconnectBackoff {
                maximum: MAX_CATALOG_RECONNECT_BACKOFF,
            });
        }
        if self.reconnect_initial_backoff > self.reconnect_max_backoff {
            return Err(MoqCatalogSubscriberConfigError::InvalidBackoffOrder);
        }
        if self.max_reconnect_attempts > 0 && self.reconnect_max_backoff > self.reconnect_deadline {
            return Err(MoqCatalogSubscriberConfigError::BackoffExceedsDeadline);
        }
        if self.max_reconnect_attempts > 0 && self.attempt_timeout > self.reconnect_deadline {
            return Err(MoqCatalogSubscriberConfigError::AttemptExceedsReconnectDeadline);
        }
        Ok(())
    }

    pub fn credential_request(
        &self,
        reconnect_attempt: u32,
    ) -> Result<MoqSubscriberCredentialRequest, MoqCatalogSubscriberConfigError> {
        self.validate()?;
        if reconnect_attempt > self.max_reconnect_attempts {
            return Err(
                MoqCatalogSubscriberConfigError::ReconnectAttemptOutOfRange {
                    configured: self.max_reconnect_attempts,
                    requested: reconnect_attempt,
                },
            );
        }
        Ok(MoqSubscriberCredentialRequest {
            endpoint_uri: self.endpoint.to_string(),
            namespace: self.namespace.clone(),
            reconnect_attempt,
            substrate: self.substrate,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqCatalogSubscriberConfigError {
    #[error("catalog subscriber endpoint exceeds {maximum} bytes")]
    EndpointTooLong { maximum: usize },
    #[error("catalog subscriber endpoint must use canonical moqt://")]
    UnsupportedScheme,
    #[error("catalog subscriber endpoint is missing an authority")]
    MissingAuthority,
    #[error("catalog subscriber endpoint cannot contain user information")]
    UserInfoForbidden,
    #[error("catalog subscriber endpoint cannot contain a query")]
    QueryForbidden,
    #[error("catalog subscriber endpoint cannot contain a fragment")]
    FragmentForbidden,
    #[error("catalog subscriber must bind one exact raw-QUIC or WebTransport substrate")]
    ExactSubstrateRequired,
    #[error("catalog subscriber path mismatch: expected {expected:?}, got {actual:?}")]
    PathMismatch { expected: String, actual: String },
    #[error("catalog payload limit must be between 1 and {maximum} bytes")]
    InvalidCatalogLimit { maximum: usize },
    #[error("catalog subscriber attempt timeout must be positive and at most {maximum:?}")]
    InvalidAttemptTimeout { maximum: Duration },
    #[error("catalog subscriber reconnect attempts exceed {maximum}")]
    TooManyReconnectAttempts { maximum: u32 },
    #[error("catalog subscriber reconnect deadline must be positive and at most {maximum:?}")]
    InvalidReconnectDeadline { maximum: Duration },
    #[error("catalog subscriber reconnect backoff must be positive and at most {maximum:?}")]
    InvalidReconnectBackoff { maximum: Duration },
    #[error("catalog subscriber initial reconnect backoff exceeds its maximum")]
    InvalidBackoffOrder,
    #[error("catalog subscriber maximum reconnect backoff exceeds its reconnect deadline")]
    BackoffExceedsDeadline,
    #[error("catalog subscriber attempt timeout exceeds its reconnect deadline")]
    AttemptExceedsReconnectDeadline,
    #[error(
        "catalog subscriber reconnect attempt {requested} exceeds configured maximum {configured}"
    )]
    ReconnectAttemptOutOfRange { configured: u32, requested: u32 },
}

/// One validated, state-changing catalog object.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoqCatalogUpdate {
    pub sequence: u64,
    pub group_id: u64,
    pub object_id: u64,
    pub received_at: DateTime<Utc>,
    pub catalog: MsfCatalog,
}

/// Bounded operational failure categories safe for APIs and metric labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqCatalogSubscriberFailure {
    CredentialUnavailable,
    CredentialDenied,
    CredentialReused,
    ConnectFailed,
    ConnectTimeout,
    PeerUnauthenticated,
    ProtocolMismatch,
    SetupFailed,
    SubscribeFailed,
    InvalidTrack,
    InvalidCatalog,
    PayloadTooLarge,
    StreamEnded,
    ReconnectExhausted,
    TaskFailed,
}

impl std::fmt::Display for MoqCatalogSubscriberFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::CredentialUnavailable => "credential-unavailable",
            Self::CredentialDenied => "credential-denied",
            Self::CredentialReused => "credential-reused",
            Self::ConnectFailed => "connect-failed",
            Self::ConnectTimeout => "connect-timeout",
            Self::PeerUnauthenticated => "peer-unauthenticated",
            Self::ProtocolMismatch => "protocol-mismatch",
            Self::SetupFailed => "setup-failed",
            Self::SubscribeFailed => "subscribe-failed",
            Self::InvalidTrack => "invalid-track",
            Self::InvalidCatalog => "invalid-catalog",
            Self::PayloadTooLarge => "payload-too-large",
            Self::StreamEnded => "stream-ended",
            Self::ReconnectExhausted => "reconnect-exhausted",
            Self::TaskFailed => "task-failed",
        };
        formatter.write_str(value)
    }
}

/// Subscriber lifecycle, independent of any draft-specific session handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqCatalogSubscriberLifecycle {
    Starting,
    Connecting,
    Subscribing,
    Live,
    Reconnecting,
    PermanentlyCompleted,
    Draining,
    Closed,
    Failed,
}

impl MoqCatalogSubscriberLifecycle {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::PermanentlyCompleted | Self::Closed | Self::Failed
        )
    }
}

/// Latest-state snapshot intended to back a `tokio::sync::watch` channel.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoqCatalogSubscriptionSnapshot {
    pub endpoint_uri: String,
    pub namespace: MoqNamespace,
    pub protocol_version: MoqProtocolVersion,
    pub lifecycle: MoqCatalogSubscriberLifecycle,
    pub lifecycle_since: DateTime<Utc>,
    pub health: BroadcastHealthDescriptor,
    pub latest: Option<MoqCatalogUpdate>,
    pub failure: Option<MoqCatalogSubscriberFailure>,
    pub reconnects: u32,
    pub substrate: Option<BroadcastSubstrate>,
    pub negotiated_protocol: Option<String>,
    pub peer_identity: Option<MoqRelayPeerIdentity>,
}

impl MoqCatalogSubscriptionSnapshot {
    pub fn is_terminal(&self) -> bool {
        self.lifecycle.is_terminal()
    }
}

/// Detailed catalog validation failures; no wire-engine type appears here.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqCatalogValidationError {
    #[error("catalog object was delivered for a different namespace")]
    NamespaceMismatch,
    #[error("catalog object was delivered on a different track")]
    TrackMismatch,
    #[error("catalog profile requires subgroup zero")]
    InvalidSubgroup,
    #[error("catalog profile requires Object zero")]
    InvalidObject,
    #[error(
        "catalog object must assert FIRST_OBJECT and live subgroup delivery must assert END_OF_GROUP"
    )]
    InvalidSubgroupFlags,
    #[error("catalog objects cannot carry extension headers")]
    ExtensionHeadersForbidden,
    #[error("catalog object declared {declared} bytes but delivered {actual}")]
    PayloadLengthMismatch { declared: u64, actual: usize },
    #[error("catalog object is {actual} bytes; configured maximum is {maximum}")]
    PayloadTooLarge { maximum: usize, actual: usize },
    #[error("catalog coordinate regressed")]
    CoordinateRegression,
    #[error("catalog coordinate was reused with different content")]
    ConflictingDuplicate,
    #[error("catalog JSON is invalid")]
    InvalidJson,
    #[error("catalog does not match the canonical MSF profile")]
    InvalidProfile,
    #[error("catalog publication changed after permanent completion")]
    UpdateAfterCompletion,
    #[error("catalog update sequence overflowed")]
    SequenceOverflow,
}

/// Evidence available for the catalog object's group boundary.
///
/// Draft-19 subgroup delivery carries `END_OF_GROUP`, while FETCH delivery
/// does not have a field for it. The unknown state is therefore valid only
/// when a standards-compliant FETCH adapter explicitly reports it; it must
/// never be promoted to a signaled boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoqEndOfGroupEvidence {
    Signaled,
    NotSignaled,
    UnknownFromFetch,
}

/// Decoded catalog object supplied by a private MOQT wire adapter.
///
/// This contains only rvoip-owned primitives. It is public so an independent
/// transport adapter can use the same validation core without importing or
/// exposing the pinned `moq-rs` types.
#[derive(Clone, Copy, Debug)]
pub struct MoqCatalogObject<'a> {
    pub namespace: &'a str,
    pub track: &'a str,
    pub group_id: u64,
    pub subgroup_id: u64,
    pub object_id: u64,
    pub first_object: bool,
    pub end_of_group: MoqEndOfGroupEvidence,
    pub extension_header_count: usize,
    pub declared_payload_len: u64,
    pub payload: &'a [u8],
    pub received_at: DateTime<Utc>,
}

/// Result of applying one decoded object to the catalog state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub enum MoqCatalogApplyOutcome {
    Update(MoqCatalogUpdate),
    /// The same coordinate and payload was already accepted.
    Duplicate,
}

/// Bounded, connection-independent catalog state machine.
///
/// A FETCH stream and a live SUBSCRIBE stream may both deliver an object. The
/// machine accepts that exact overlap once, rejects coordinate conflicts, and
/// never permits catalog state to move backwards or reopen after completion.
pub struct MoqCatalogStateMachine {
    namespace: MoqNamespace,
    max_catalog_bytes: usize,
    last_group_id: Option<u64>,
    seen: BTreeMap<u64, [u8; 32]>,
    latest: Option<MoqCatalogUpdate>,
    sequence: u64,
    completed: bool,
}

impl MoqCatalogStateMachine {
    pub fn new(
        config: &MoqCatalogSubscriberConfig,
    ) -> Result<Self, MoqCatalogSubscriberConfigError> {
        config.validate()?;
        Ok(Self::from_validated_parts(
            config.namespace.clone(),
            config.max_catalog_bytes,
        ))
    }

    fn from_validated_parts(namespace: MoqNamespace, max_catalog_bytes: usize) -> Self {
        Self {
            namespace,
            max_catalog_bytes,
            last_group_id: None,
            seen: BTreeMap::new(),
            latest: None,
            sequence: 0,
            completed: false,
        }
    }

    pub fn apply(
        &mut self,
        object: MoqCatalogObject<'_>,
    ) -> Result<MoqCatalogApplyOutcome, MoqCatalogValidationError> {
        if object.namespace != self.namespace.as_str() {
            return Err(MoqCatalogValidationError::NamespaceMismatch);
        }
        if object.track != CATALOG_TRACK {
            return Err(MoqCatalogValidationError::TrackMismatch);
        }
        if object.subgroup_id != 0 {
            return Err(MoqCatalogValidationError::InvalidSubgroup);
        }
        if object.object_id != 0 {
            return Err(MoqCatalogValidationError::InvalidObject);
        }
        if !object.first_object || object.end_of_group == MoqEndOfGroupEvidence::NotSignaled {
            return Err(MoqCatalogValidationError::InvalidSubgroupFlags);
        }
        if object.extension_header_count != 0 {
            return Err(MoqCatalogValidationError::ExtensionHeadersForbidden);
        }
        let actual_payload_len = u64::try_from(object.payload.len()).unwrap_or(u64::MAX);
        if object.declared_payload_len != actual_payload_len {
            return Err(MoqCatalogValidationError::PayloadLengthMismatch {
                declared: object.declared_payload_len,
                actual: object.payload.len(),
            });
        }
        if object.payload.len() > self.max_catalog_bytes {
            return Err(MoqCatalogValidationError::PayloadTooLarge {
                maximum: self.max_catalog_bytes,
                actual: object.payload.len(),
            });
        }

        let fingerprint: [u8; 32] = Sha256::digest(object.payload).into();
        if let Some(previous) = self.seen.get(&object.group_id) {
            return if previous == &fingerprint {
                Ok(MoqCatalogApplyOutcome::Duplicate)
            } else {
                Err(MoqCatalogValidationError::ConflictingDuplicate)
            };
        }
        if self
            .last_group_id
            .is_some_and(|previous| object.group_id < previous)
        {
            return Err(MoqCatalogValidationError::CoordinateRegression);
        }
        if self.completed {
            return Err(MoqCatalogValidationError::UpdateAfterCompletion);
        }

        let json: serde_json::Value = serde_json::from_slice(object.payload)
            .map_err(|_| MoqCatalogValidationError::InvalidJson)?;
        let catalog: MsfCatalog =
            serde_json::from_value(json).map_err(|_| MoqCatalogValidationError::InvalidProfile)?;
        catalog
            .validate_for(&self.namespace)
            .map_err(|_| MoqCatalogValidationError::InvalidProfile)?;

        let next_sequence = self
            .sequence
            .checked_add(1)
            .ok_or(MoqCatalogValidationError::SequenceOverflow)?;
        self.completed = catalog.state() == MsfCatalogState::PermanentlyCompleted;
        self.sequence = next_sequence;
        let update = MoqCatalogUpdate {
            sequence: next_sequence,
            group_id: object.group_id,
            object_id: object.object_id,
            received_at: object.received_at,
            catalog,
        };
        self.latest = Some(update.clone());
        self.record_group(object.group_id, fingerprint);
        Ok(MoqCatalogApplyOutcome::Update(update))
    }

    pub const fn completed(&self) -> bool {
        self.completed
    }

    pub const fn update_count(&self) -> u64 {
        self.sequence
    }

    pub fn latest(&self) -> Option<&MoqCatalogUpdate> {
        self.latest.as_ref()
    }

    fn record_group(&mut self, group_id: u64, fingerprint: [u8; 32]) {
        self.last_group_id = Some(group_id);
        self.seen.insert(group_id, fingerprint);
        while self.seen.len() > DEFAULT_DEDUPE_ENTRIES {
            if let Some(oldest) = self.seen.keys().next().copied() {
                self.seen.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_core_traits::broadcast::BroadcastHealthStatus;

    fn namespace() -> MoqNamespace {
        MoqNamespace::new("tenant", "broadcast").unwrap()
    }

    fn config() -> MoqCatalogSubscriberConfig {
        MoqCatalogSubscriberConfig::new(
            Url::parse("moqt://relay.example/tenant/broadcast").unwrap(),
            namespace(),
        )
    }

    fn live(generated_at: i64) -> Vec<u8> {
        MsfCatalog::opus_audio(&namespace(), 24_000, Some("en".into()), generated_at)
            .unwrap()
            .to_json_bytes()
            .unwrap()
    }

    fn completed(generated_at: i64) -> Vec<u8> {
        MsfCatalog::permanently_completed(generated_at)
            .to_json_bytes()
            .unwrap()
    }

    fn envelope(payload: &[u8], group_id: u64) -> MoqCatalogObject<'_> {
        MoqCatalogObject {
            namespace: "tenant/broadcast",
            track: CATALOG_TRACK,
            group_id,
            subgroup_id: 0,
            object_id: 0,
            first_object: true,
            end_of_group: MoqEndOfGroupEvidence::Signaled,
            extension_header_count: 0,
            declared_payload_len: payload.len() as u64,
            payload,
            received_at: Utc::now(),
        }
    }

    #[test]
    fn credential_is_bounded_redacted_and_transfers_to_zeroizing_storage() {
        assert_eq!(
            MoqSubscriberCredential::new(Vec::new()).unwrap_err(),
            MoqSubscriberCredentialError::Empty
        );
        let too_long = vec![7; MAX_MOQ_SUBSCRIBER_CREDENTIAL_BYTES + 1];
        assert!(matches!(
            MoqSubscriberCredential::new(too_long),
            Err(MoqSubscriberCredentialError::TooLong { .. })
        ));
        let credential = MoqSubscriberCredential::new(b"top-secret".to_vec()).unwrap();
        assert_eq!(credential.len(), 10);
        assert!(!credential.is_empty());
        let debug = format!("{credential:?}");
        assert!(debug.contains("redacted:10"));
        assert!(!debug.contains("top-secret"));
        let wire_bytes = credential.into_wire_bytes();
        assert_eq!(wire_bytes.as_slice(), b"top-secret");
    }

    #[test]
    fn config_requires_exact_credential_free_namespace_path() {
        config().validate().unwrap();
        let request = config().credential_request(2).unwrap();
        assert_eq!(
            request.endpoint_uri,
            "moqt://relay.example/tenant/broadcast"
        );
        assert_eq!(request.reconnect_attempt, 2);
        assert_eq!(request.namespace, namespace());

        let cases = [
            (
                "https://relay.example/tenant/broadcast",
                MoqCatalogSubscriberConfigError::UnsupportedScheme,
            ),
            (
                "moqt://user@relay.example/tenant/broadcast",
                MoqCatalogSubscriberConfigError::UserInfoForbidden,
            ),
            (
                "moqt://relay.example/tenant/broadcast?token=secret",
                MoqCatalogSubscriberConfigError::QueryForbidden,
            ),
            (
                "moqt://relay.example/tenant/broadcast#local",
                MoqCatalogSubscriberConfigError::FragmentForbidden,
            ),
        ];
        for (uri, expected) in cases {
            let mut candidate = config();
            candidate.endpoint = Url::parse(uri).unwrap();
            assert_eq!(candidate.validate().unwrap_err(), expected);
        }
        let mut wrong_path = config();
        wrong_path.endpoint = Url::parse("moqt://relay.example/other/broadcast").unwrap();
        assert!(matches!(
            wrong_path.validate(),
            Err(MoqCatalogSubscriberConfigError::PathMismatch { .. })
        ));

        let mut ambiguous_substrate = config();
        ambiguous_substrate.substrate = MoqRelaySubstratePolicy::Auto;
        assert_eq!(
            ambiguous_substrate.validate().unwrap_err(),
            MoqCatalogSubscriberConfigError::ExactSubstrateRequired
        );
    }

    #[test]
    fn config_rejects_unbounded_or_incoherent_lifecycle_values() {
        let mut candidate = config();
        candidate.max_catalog_bytes = 0;
        assert!(matches!(
            candidate.validate(),
            Err(MoqCatalogSubscriberConfigError::InvalidCatalogLimit { .. })
        ));
        candidate = config();
        candidate.attempt_timeout = Duration::ZERO;
        assert!(matches!(
            candidate.validate(),
            Err(MoqCatalogSubscriberConfigError::InvalidAttemptTimeout { .. })
        ));
        candidate = config();
        candidate.max_reconnect_attempts = MAX_CATALOG_RECONNECT_ATTEMPTS + 1;
        assert!(matches!(
            candidate.validate(),
            Err(MoqCatalogSubscriberConfigError::TooManyReconnectAttempts { .. })
        ));
        candidate = config();
        candidate.reconnect_deadline = MAX_CATALOG_RECONNECT_DEADLINE + Duration::from_secs(1);
        assert!(matches!(
            candidate.validate(),
            Err(MoqCatalogSubscriberConfigError::InvalidReconnectDeadline { .. })
        ));
        candidate = config();
        candidate.reconnect_deadline = Duration::from_secs(1);
        assert_eq!(
            candidate.validate().unwrap_err(),
            MoqCatalogSubscriberConfigError::BackoffExceedsDeadline
        );
        candidate = config();
        candidate.reconnect_initial_backoff = Duration::from_secs(2);
        candidate.reconnect_max_backoff = Duration::from_secs(1);
        assert_eq!(
            candidate.validate().unwrap_err(),
            MoqCatalogSubscriberConfigError::InvalidBackoffOrder
        );

        let mut secret = config();
        secret.endpoint =
            Url::parse("moqt://relay.example/tenant/broadcast?token=top-secret").unwrap();
        let debug = format!("{secret:?}");
        assert!(debug.contains("redacted-invalid-endpoint"));
        assert!(!debug.contains("top-secret"));
    }

    #[test]
    fn validator_accepts_live_then_terminal_and_dedupes_exact_coordinate() {
        let live = live(10);
        let terminal = completed(11);
        let mut validator = MoqCatalogStateMachine::new(&config()).unwrap();

        let first = validator.apply(envelope(&live, 4)).unwrap();
        let MoqCatalogApplyOutcome::Update(first) = first else {
            panic!("first catalog must update state")
        };
        assert_eq!(first.sequence, 1);
        assert_eq!(first.catalog.state(), MsfCatalogState::Live);
        assert!(matches!(
            validator.apply(envelope(&live, 4)).unwrap(),
            MoqCatalogApplyOutcome::Duplicate
        ));

        let second = validator.apply(envelope(&terminal, 5)).unwrap();
        let MoqCatalogApplyOutcome::Update(second) = second else {
            panic!("terminal catalog must update state")
        };
        assert_eq!(second.sequence, 2);
        assert!(validator.completed());
        assert_eq!(validator.update_count(), 2);
        assert_eq!(validator.latest(), Some(&second));
        assert_eq!(
            second.catalog.state(),
            MsfCatalogState::PermanentlyCompleted
        );
    }

    #[test]
    fn validator_rejects_cross_namespace_headers_and_oversized_payloads() {
        let payload = live(10);
        let mutations: Vec<fn(&mut MoqCatalogObject<'_>)> = vec![
            |object| object.namespace = "other/broadcast",
            |object| object.track = "audio/main",
            |object| object.subgroup_id = 1,
            |object| object.object_id = 1,
            |object| object.first_object = false,
            |object| object.end_of_group = MoqEndOfGroupEvidence::NotSignaled,
            |object| object.extension_header_count = 1,
            |object| object.declared_payload_len += 1,
        ];
        for mutate in mutations {
            let mut validator = MoqCatalogStateMachine::new(&config()).unwrap();
            let mut object = envelope(&payload, 1);
            mutate(&mut object);
            assert!(validator.apply(object).is_err());
        }

        let mut limited = config();
        limited.max_catalog_bytes = payload.len() - 1;
        let mut validator = MoqCatalogStateMachine::new(&limited).unwrap();
        assert!(matches!(
            validator.apply(envelope(&payload, 1)),
            Err(MoqCatalogValidationError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn validator_accepts_unknown_group_end_only_for_fetch_evidence() {
        let payload = live(10);
        let mut validator = MoqCatalogStateMachine::new(&config()).unwrap();
        let mut fetched = envelope(&payload, 1);
        fetched.end_of_group = MoqEndOfGroupEvidence::UnknownFromFetch;
        assert!(matches!(
            validator.apply(fetched).unwrap(),
            MoqCatalogApplyOutcome::Update(_)
        ));

        let mut invalid = MoqCatalogStateMachine::new(&config()).unwrap();
        let mut live_without_boundary = envelope(&payload, 1);
        live_without_boundary.end_of_group = MoqEndOfGroupEvidence::NotSignaled;
        assert_eq!(
            invalid.apply(live_without_boundary).unwrap_err(),
            MoqCatalogValidationError::InvalidSubgroupFlags
        );
    }

    #[test]
    fn validator_rejects_regression_conflicts_and_updates_after_completion() {
        let initial = live(10);
        let later = live(11);
        let terminal = completed(12);
        let mut validator = MoqCatalogStateMachine::new(&config()).unwrap();
        let _ = validator.apply(envelope(&initial, 10)).unwrap();

        assert_eq!(
            validator.apply(envelope(&later, 9)).unwrap_err(),
            MoqCatalogValidationError::CoordinateRegression
        );
        assert_eq!(
            validator.apply(envelope(&later, 10)).unwrap_err(),
            MoqCatalogValidationError::ConflictingDuplicate
        );
        let _ = validator.apply(envelope(&terminal, 11)).unwrap();
        assert_eq!(
            validator.apply(envelope(&later, 12)).unwrap_err(),
            MoqCatalogValidationError::UpdateAfterCompletion
        );
    }

    #[test]
    fn validator_accepts_terminal_only_and_same_timestamp_completion() {
        let mut terminal_only = MoqCatalogStateMachine::new(&config()).unwrap();
        let result = terminal_only.apply(envelope(&completed(10), 1)).unwrap();
        assert!(matches!(result, MoqCatalogApplyOutcome::Update(_)));
        assert!(terminal_only.completed());

        let mut live_then_terminal = MoqCatalogStateMachine::new(&config()).unwrap();
        let _ = live_then_terminal.apply(envelope(&live(10), 1)).unwrap();
        let _ = live_then_terminal
            .apply(envelope(&completed(10), 2))
            .unwrap();
        assert!(live_then_terminal.completed());
    }

    #[test]
    fn validator_distinguishes_invalid_json_from_invalid_profile() {
        let mut validator = MoqCatalogStateMachine::new(&config()).unwrap();
        assert_eq!(
            validator.apply(envelope(b"{", 1)).unwrap_err(),
            MoqCatalogValidationError::InvalidJson
        );
        let unsupported = br#"{"version":"draft-99","generatedAt":1,"tracks":[]}"#;
        assert_eq!(
            validator.apply(envelope(unsupported, 1)).unwrap_err(),
            MoqCatalogValidationError::InvalidProfile
        );

        let other_namespace = MoqNamespace::new("other", "broadcast").unwrap();
        let cross_tenant = MsfCatalog::opus_audio(&other_namespace, 24_000, None, 1)
            .unwrap()
            .to_json_bytes()
            .unwrap();
        assert_eq!(
            validator.apply(envelope(&cross_tenant, 1)).unwrap_err(),
            MoqCatalogValidationError::InvalidProfile
        );
    }

    #[test]
    fn snapshot_is_serializable_and_terminal_state_is_explicit() {
        let snapshot = MoqCatalogSubscriptionSnapshot {
            endpoint_uri: "moqt://relay.example/tenant/broadcast".into(),
            namespace: namespace(),
            protocol_version: MoqProtocolVersion::PINNED,
            lifecycle: MoqCatalogSubscriberLifecycle::Closed,
            lifecycle_since: Utc::now(),
            health: BroadcastHealthDescriptor {
                status: BroadcastHealthStatus::Closed,
                issues: Vec::new(),
                active_subscribers: None,
                subscriber_capacity: None,
                checked_at: Utc::now(),
            },
            latest: None,
            failure: None,
            reconnects: 1,
            substrate: Some(BroadcastSubstrate::WebTransport),
            negotiated_protocol: Some("moqt-19".into()),
            peer_identity: None,
        };
        assert!(snapshot.is_terminal());
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("tenant/broadcast"));
        assert!(!json.contains("top-secret"));
    }
}
