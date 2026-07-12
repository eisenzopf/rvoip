use crate::capability::CapabilityDescriptor;
use crate::connection::{Connection, Direction, Transport};
use crate::data::DataMessage;
use crate::identity::{AuthenticatedPrincipal, IdentityAssurance, Jwk, PrincipalOwnershipKey};
use crate::ids::{ConnectionId, ParticipantId, PlaybackId, SessionId};
use crate::stream::QualitySnapshot;
use std::fmt;
use thiserror::Error;
use tokio::sync::oneshot;
use zeroize::Zeroize;

/// Maximum UTF-8 size of an adapter-owned inbound routing hint.
pub const MAX_INBOUND_ROUTING_HINT_BYTES: usize = 2_048;
/// Maximum number of ordered metadata fields retained for one inbound leg.
pub const MAX_INBOUND_METADATA_FIELDS: usize = 32;
/// Maximum UTF-8 size of one inbound metadata field name.
pub const MAX_INBOUND_METADATA_NAME_BYTES: usize = 128;
/// Maximum UTF-8 size of one inbound metadata field value.
pub const MAX_INBOUND_METADATA_VALUE_BYTES: usize = 4_096;
/// Maximum aggregate UTF-8 size of inbound metadata names and values.
pub const MAX_INBOUND_METADATA_BYTES: usize = 16 * 1_024;

/// Validation failure for adapter-supplied inbound connection context.
///
/// Variants deliberately contain no user-controlled text so formatting an
/// error cannot disclose a routing secret or signaling metadata value.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum InboundContextError {
    #[error("the authenticated principal has no tenant")]
    MissingTenant,
    #[error("the authenticated principal has expired")]
    ExpiredPrincipal,
    #[error("the inbound routing hint is empty")]
    EmptyRoutingHint,
    #[error("the inbound routing hint is too large")]
    RoutingHintTooLarge,
    #[error("the inbound routing hint contains forbidden control characters")]
    InvalidRoutingHint,
    #[error("there are too many inbound metadata fields")]
    TooManyMetadataFields,
    #[error("an inbound metadata name is invalid")]
    InvalidMetadataName,
    #[error("an inbound metadata name is too large")]
    MetadataNameTooLarge,
    #[error("an inbound metadata value is too large")]
    MetadataValueTooLarge,
    #[error("an inbound metadata value contains forbidden characters")]
    InvalidMetadataValue,
    #[error("the aggregate inbound metadata is too large")]
    MetadataTooLarge,
}

/// Opaque, redacted routing material captured before an inbound adapter
/// normalizes its signaling event.
///
/// This value intentionally implements neither `Display` nor serialization.
/// Callers must explicitly opt in to reading it through [`Self::expose_secret`].
#[derive(Eq, PartialEq)]
pub struct InboundRoutingHint(String);

impl InboundRoutingHint {
    pub fn new(value: impl Into<String>) -> Result<Self, InboundContextError> {
        let value = value.into();
        if value.is_empty() {
            return Err(InboundContextError::EmptyRoutingHint);
        }
        if value.len() > MAX_INBOUND_ROUTING_HINT_BYTES {
            return Err(InboundContextError::RoutingHintTooLarge);
        }
        if value.chars().any(char::is_control) {
            return Err(InboundContextError::InvalidRoutingHint);
        }
        Ok(Self(value))
    }

    /// Reveal the routing hint to the policy layer that will consume it.
    /// Avoid formatting or logging the returned value.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Transfer the owned routing material to the policy layer.
    ///
    /// The returned `String` remains sensitive and its final owner must
    /// zeroize it. The wrapper's `Drop` implementation clears any value that
    /// is not explicitly transferred.
    pub fn into_secret(mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

impl Drop for InboundRoutingHint {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for InboundRoutingHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InboundRoutingHint([redacted])")
    }
}

/// Ordered, duplicate-preserving signaling metadata captured for an inbound
/// connection. Field values are redacted from `Debug` output.
#[derive(Default, Eq, PartialEq)]
pub struct InboundSignalingMetadata {
    entries: Vec<(String, String)>,
}

impl InboundSignalingMetadata {
    pub fn new<I, N, V>(entries: I) -> Result<Self, InboundContextError>
    where
        I: IntoIterator<Item = (N, V)>,
        N: Into<String>,
        V: Into<String>,
    {
        let entries = entries
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect::<Vec<_>>();
        if entries.len() > MAX_INBOUND_METADATA_FIELDS {
            return Err(InboundContextError::TooManyMetadataFields);
        }

        let mut aggregate_bytes = 0usize;
        let mut normalized = Vec::with_capacity(entries.len());
        for (name, value) in entries {
            if name.len() > MAX_INBOUND_METADATA_NAME_BYTES {
                return Err(InboundContextError::MetadataNameTooLarge);
            }
            if name.is_empty() || !name.bytes().all(is_metadata_name_byte) {
                return Err(InboundContextError::InvalidMetadataName);
            }
            if value.len() > MAX_INBOUND_METADATA_VALUE_BYTES {
                return Err(InboundContextError::MetadataValueTooLarge);
            }
            if value
                .chars()
                .any(|character| character != '\t' && character.is_control())
            {
                return Err(InboundContextError::InvalidMetadataValue);
            }
            aggregate_bytes = aggregate_bytes
                .checked_add(name.len())
                .and_then(|total| total.checked_add(value.len()))
                .ok_or(InboundContextError::MetadataTooLarge)?;
            if aggregate_bytes > MAX_INBOUND_METADATA_BYTES {
                return Err(InboundContextError::MetadataTooLarge);
            }
            normalized.push((name.to_ascii_lowercase(), value));
        }
        Ok(Self {
            entries: normalized,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterate over metadata values. Avoid formatting or logging the values.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries.iter().filter_map(move |(candidate, value)| {
            candidate
                .eq_ignore_ascii_case(name)
                .then_some(value.as_str())
        })
    }
}

impl fmt::Debug for InboundSignalingMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InboundSignalingMetadata")
            .field("field_count", &self.entries.len())
            .finish()
    }
}

impl Drop for InboundSignalingMetadata {
    fn drop(&mut self) {
        for (name, value) in &mut self.entries {
            name.zeroize();
            value.zeroize();
        }
    }
}

const fn is_metadata_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Principal- and transport-bound context for one inbound connection.
///
/// Adapters may expose this exactly once. The Orchestrator retains it until
/// an authenticated owner consumes it or the connection terminates.
pub struct InboundConnectionContext {
    connection_id: ConnectionId,
    transport: Transport,
    owner: PrincipalOwnershipKey,
    routing_hint: Option<InboundRoutingHint>,
    metadata: InboundSignalingMetadata,
}

impl InboundConnectionContext {
    pub fn new(
        connection_id: ConnectionId,
        transport: Transport,
        principal: &AuthenticatedPrincipal,
        routing_hint: Option<InboundRoutingHint>,
        metadata: InboundSignalingMetadata,
    ) -> Result<Self, InboundContextError> {
        if principal.tenant.as_deref().is_none_or(str::is_empty) {
            return Err(InboundContextError::MissingTenant);
        }
        if principal.is_expired() {
            return Err(InboundContextError::ExpiredPrincipal);
        }
        Ok(Self {
            connection_id,
            transport,
            owner: principal.ownership_key(),
            routing_hint,
            metadata,
        })
    }

    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    pub const fn transport(&self) -> Transport {
        self.transport
    }

    pub fn routing_hint(&self) -> Option<&InboundRoutingHint> {
        self.routing_hint.as_ref()
    }

    /// Take the routing hint exactly once for an owning policy boundary.
    /// Untaken routing material is zeroized when this context is dropped.
    pub fn take_routing_hint(&mut self) -> Option<InboundRoutingHint> {
        self.routing_hint.take()
    }

    pub fn metadata(&self) -> &InboundSignalingMetadata {
        &self.metadata
    }

    pub fn is_bound_to(
        &self,
        connection_id: &ConnectionId,
        transport: Transport,
        principal: &AuthenticatedPrincipal,
    ) -> bool {
        !principal.is_expired()
            && self.connection_id == *connection_id
            && self.transport == transport
            && self.owner == principal.ownership_key()
    }
}

impl fmt::Debug for InboundConnectionContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InboundConnectionContext")
            .field("connection_id", &self.connection_id)
            .field("transport", &self.transport)
            .field("owner", &"[redacted]")
            .field("has_routing_hint", &self.routing_hint.is_some())
            .field("metadata_field_count", &self.metadata.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterKind {
    /// UCTP-native (QUIC, WebTransport, WebSocket).
    Substrate,
    /// Gateway to a foreign protocol (SIP, WebRTC).
    Interop,
}

#[derive(Clone, Debug)]
pub struct OriginateRequest {
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    pub target: String,
    pub direction: Direction,
    pub capabilities: CapabilityDescriptor,
    /// P6 — transport selector. When `Some`, the Orchestrator
    /// dispatches the originate through the adapter registered for
    /// this transport. When `None`, the "first registered adapter"
    /// fallback applies (single-adapter deployments).
    pub transport: Option<Transport>,
}

#[derive(Clone, Debug)]
pub struct ConnectionHandle {
    pub connection: Connection,
}

#[derive(Clone, Debug)]
pub enum RejectReason {
    Busy,
    Decline,
    NotFound,
    Forbidden,
    NotAcceptable,
    ServerError,
    Custom { code: u16, phrase: String },
}

#[derive(Clone, Debug)]
pub enum EndReason {
    Normal,
    Cancelled,
    Failed { detail: String },
    Timeout,
    BridgeTorn,
}

#[derive(Clone, Debug)]
pub enum TransferTarget {
    Uri(String),
    Connection(ConnectionId),
    Session(SessionId),
}

/// Handle returned by adapter playback paths that lets callers stop an
/// in-flight playback.
#[derive(Debug)]
pub struct PlaybackHandle {
    id: PlaybackId,
    cancel_tx: oneshot::Sender<()>,
}

impl PlaybackHandle {
    /// Adapter helper: build a handle + the matching cancel receiver.
    pub fn new(id: PlaybackId) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self { id, cancel_tx: tx }, rx)
    }

    pub fn id(&self) -> &PlaybackId {
        &self.id
    }

    /// Best-effort cancellation. Returns `Err` only when the adapter's
    /// playback task already exited.
    pub fn cancel(self) -> std::result::Result<(), &'static str> {
        self.cancel_tx
            .send(())
            .map_err(|_| "playback already ended")
    }
}

#[derive(Clone, Debug)]
pub struct SignatureHeaders {
    pub signature: String,
    pub signature_input: String,
    pub signature_key: Option<Jwk>,
    pub signature_agent: Option<Jwk>,
}

/// Adapter-native event surface. `rvoip-core` normalizes these into the
/// orchestration event vocabulary; consumers wanting protocol-native
/// access can subscribe directly to the adapter.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AdapterEvent {
    InboundConnection {
        connection: Connection,
    },
    Connected {
        connection_id: ConnectionId,
    },
    Authenticated {
        connection_id: ConnectionId,
        identity_id: String,
        participant_id: String,
        assurance: IdentityAssurance,
    },
    /// Additive full-principal authentication event. The legacy
    /// `Authenticated` variant remains unchanged for source compatibility.
    PrincipalAuthenticated {
        connection_id: ConnectionId,
        participant_id: String,
        principal: AuthenticatedPrincipal,
    },
    Ended {
        connection_id: ConnectionId,
        reason: EndReason,
    },
    Failed {
        connection_id: ConnectionId,
        detail: String,
    },
    Dtmf {
        connection_id: ConnectionId,
        digits: String,
        duration_ms: u32,
    },
    Quality {
        connection_id: ConnectionId,
        snapshot: QualitySnapshot,
    },
    Message {
        connection_id: ConnectionId,
        text: String,
    },
    DataMessage {
        connection_id: ConnectionId,
        message: DataMessage,
    },
    StepUpResponse {
        connection_id: ConnectionId,
        method: String,
        credential: String,
    },
    Native {
        kind: &'static str,
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{AuthenticationMethod, IdentityAssurance};

    fn principal(tenant: Option<&str>) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: "routing-owner".into(),
            tenant: tenant.map(str::to_owned),
            scopes: vec!["call:attach".into()],
            issuer: Some("https://issuer.invalid".into()),
            expires_at: None,
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::Identified {
                credential_kind: crate::identity::CredentialKind::Oidc,
            },
        }
    }

    #[test]
    fn inbound_context_is_bound_and_debug_redacted() {
        let connection_id = ConnectionId::new();
        let owner = principal(Some("tenant-a"));
        let hint = InboundRoutingHint::new("attachment-secret").unwrap();
        let metadata = InboundSignalingMetadata::new([
            ("X-Correlation-Id", "correlation-secret"),
            ("X-Correlation-Id", "second-secret"),
        ])
        .unwrap();
        let context = InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &owner,
            Some(hint),
            metadata,
        )
        .unwrap();

        assert!(context.is_bound_to(&connection_id, Transport::Sip, &owner));
        assert_eq!(
            context.routing_hint().unwrap().expose_secret(),
            "attachment-secret"
        );
        assert_eq!(
            context
                .metadata()
                .values("x-correlation-id")
                .collect::<Vec<_>>(),
            vec!["correlation-secret", "second-secret"]
        );

        let debug = format!("{context:?}");
        assert!(!debug.contains("attachment-secret"));
        assert!(!debug.contains("correlation-secret"));
        assert!(!debug.contains("second-secret"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn inbound_routing_hint_transfers_owned_secret_exactly_once() {
        let connection_id = ConnectionId::new();
        let owner = principal(Some("tenant-a"));
        let mut context = InboundConnectionContext::new(
            connection_id,
            Transport::WebRtc,
            &owner,
            Some(InboundRoutingHint::new("attachment-secret").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap();

        let mut secret = context.take_routing_hint().unwrap().into_secret();
        assert_eq!(secret, "attachment-secret");
        assert!(context.take_routing_hint().is_none());
        secret.zeroize();
        assert!(secret.is_empty());
    }

    #[test]
    fn inbound_context_rejects_unscoped_or_unsafe_values() {
        assert_eq!(
            InboundConnectionContext::new(
                ConnectionId::new(),
                Transport::Sip,
                &principal(None),
                None,
                InboundSignalingMetadata::default(),
            )
            .unwrap_err(),
            InboundContextError::MissingTenant
        );
        assert_eq!(
            InboundRoutingHint::new("secret\r\nheader").unwrap_err(),
            InboundContextError::InvalidRoutingHint
        );
        assert_eq!(
            InboundSignalingMetadata::new([("x-safe", "unsafe\r\nvalue")]).unwrap_err(),
            InboundContextError::InvalidMetadataValue
        );
    }
}
