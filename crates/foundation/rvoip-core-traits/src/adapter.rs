use crate::capability::CapabilityDescriptor;
use crate::connection::{Connection, Direction, Transport};
use crate::data::DataMessage;
use crate::identity::{AuthenticatedPrincipal, IdentityAssurance, Jwk, PrincipalOwnershipKey};
use crate::ids::{ConnectionId, ParticipantId, PlaybackId, SessionId, TransferAttemptId};
use crate::stream::QualitySnapshot;
use std::any::Any;
use std::fmt;
use std::sync::Arc;
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
/// Maximum number of adapter-owned external identifiers on one activation.
pub const MAX_EXTERNAL_CONNECTION_REFERENCES: usize = 16;
/// Maximum UTF-8 size of an external identifier namespace.
pub const MAX_EXTERNAL_REFERENCE_KIND_BYTES: usize = 64;
/// Maximum UTF-8 size of one external identifier value.
pub const MAX_EXTERNAL_REFERENCE_VALUE_BYTES: usize = 4_096;

/// Validation failure for an adapter-owned external connection reference.
///
/// Variants deliberately contain no adapter-provided text so formatting an
/// error cannot disclose identifier values.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ExternalConnectionReferenceError {
    #[error("the external reference kind is empty")]
    EmptyKind,
    #[error("the external reference kind is too large")]
    KindTooLarge,
    #[error("the external reference kind is not a valid token")]
    InvalidKind,
    #[error("the external reference value is empty")]
    EmptyValue,
    #[error("the external reference value is too large")]
    ValueTooLarge,
    #[error("the external reference value contains forbidden control characters")]
    InvalidValue,
    #[error("there are too many external connection references")]
    TooManyReferences,
}

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

/// Opaque adapter-owned options for one outbound originate operation.
///
/// Core transports this value without inspecting it. An adapter defines its
/// own context type and recovers it with [`Self::downcast_ref`] or
/// [`Self::downcast_arc`]. The concrete type and value are intentionally
/// omitted from `Debug`; this type implements neither `Display` nor
/// serialization.
#[derive(Clone, Default)]
pub struct OriginateContext {
    inner: Option<Arc<dyn Any + Send + Sync>>,
}

impl OriginateContext {
    /// Wrap adapter-owned, immutable originate options.
    pub fn new<T>(value: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            inner: Some(Arc::new(value)),
        }
    }

    /// Whether this request carries no adapter-owned options.
    pub fn is_empty(&self) -> bool {
        self.inner.is_none()
    }

    /// Borrow the adapter-owned options when their concrete type matches.
    pub fn downcast_ref<T>(&self) -> Option<&T>
    where
        T: Any + Send + Sync,
    {
        self.inner.as_deref()?.downcast_ref::<T>()
    }

    /// Clone the shared adapter-owned options when their concrete type
    /// matches. A failed downcast does not consume or reveal the context.
    pub fn downcast_arc<T>(&self) -> Option<Arc<T>>
    where
        T: Any + Send + Sync,
    {
        Arc::clone(self.inner.as_ref()?).downcast::<T>().ok()
    }
}

impl fmt::Debug for OriginateContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OriginateContext")
            .field("present", &self.inner.is_some())
            .finish()
    }
}

#[derive(Clone)]
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
    /// Opaque, typed options interpreted only by the selected adapter.
    pub context: OriginateContext,
}

impl OriginateRequest {
    /// Build a transport-neutral outbound request with no adapter context.
    /// Select a transport with [`Self::with_transport`] when more than one
    /// adapter is registered.
    pub fn new(
        session_id: SessionId,
        participant_id: ParticipantId,
        target: impl Into<String>,
        direction: Direction,
        capabilities: CapabilityDescriptor,
    ) -> Self {
        Self {
            session_id,
            participant_id,
            target: target.into(),
            direction,
            capabilities,
            transport: None,
            context: OriginateContext::default(),
        }
    }

    /// Select the adapter transport for this request.
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Attach options owned and interpreted by the selected adapter.
    pub fn with_context<T>(mut self, context: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.context = OriginateContext::new(context);
        self
    }

    /// Attach an already type-erased adapter context.
    pub fn with_originate_context(mut self, context: OriginateContext) -> Self {
        self.context = context;
        self
    }
}

impl fmt::Debug for OriginateRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OriginateRequest")
            .field("session_id", &self.session_id)
            .field("participant_id", &self.participant_id)
            .field("target", &"[redacted]")
            .field("direction", &self.direction)
            .field("capabilities", &self.capabilities)
            .field("transport", &self.transport)
            .field("context_present", &!self.context.is_empty())
            .finish()
    }
}

/// A stable external identifier learned while activating an outbound route.
///
/// The identifier namespace is adapter-owned (for example an adapter may use
/// `sip.call-id`). Core treats both namespace and value as opaque. The value
/// is deliberately available only through [`Self::expose_secret`], and
/// formatting never reveals either component.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct ExternalConnectionReference {
    kind: Arc<str>,
    value: Arc<str>,
}

impl ExternalConnectionReference {
    pub fn new(
        kind: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Result<Self, ExternalConnectionReferenceError> {
        let kind = kind.into();
        if kind.is_empty() {
            return Err(ExternalConnectionReferenceError::EmptyKind);
        }
        if kind.len() > MAX_EXTERNAL_REFERENCE_KIND_BYTES {
            return Err(ExternalConnectionReferenceError::KindTooLarge);
        }
        let mut kind_bytes = kind.bytes();
        if !kind_bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !kind_bytes
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(ExternalConnectionReferenceError::InvalidKind);
        }

        let value = value.into();
        if value.is_empty() {
            return Err(ExternalConnectionReferenceError::EmptyValue);
        }
        if value.len() > MAX_EXTERNAL_REFERENCE_VALUE_BYTES {
            return Err(ExternalConnectionReferenceError::ValueTooLarge);
        }
        if value.chars().any(char::is_control) {
            return Err(ExternalConnectionReferenceError::InvalidValue);
        }
        Ok(Self { kind, value })
    }

    /// Adapter-owned identifier namespace. Do not derive routing policy from
    /// this untrusted string without an exact allowlist.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Explicitly reveal the external identifier to its durable owner.
    /// Avoid formatting or logging the returned value.
    pub fn expose_secret(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for ExternalConnectionReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExternalConnectionReference([redacted])")
    }
}

/// Adapter receipt produced only after outbound activation succeeds.
///
/// A receipt may contain more than one adapter-owned external identifier.
/// Core retains it unchanged and does not interpret identifier namespaces.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct OutboundActivation {
    external_references: Arc<[ExternalConnectionReference]>,
}

impl OutboundActivation {
    pub fn new(
        external_references: impl IntoIterator<Item = ExternalConnectionReference>,
    ) -> Result<Self, ExternalConnectionReferenceError> {
        let mut bounded = Vec::with_capacity(MAX_EXTERNAL_CONNECTION_REFERENCES);
        for external_reference in external_references {
            if bounded.len() == MAX_EXTERNAL_CONNECTION_REFERENCES {
                return Err(ExternalConnectionReferenceError::TooManyReferences);
            }
            bounded.push(external_reference);
        }
        Ok(Self {
            external_references: bounded.into(),
        })
    }

    pub fn with_external_reference(external_reference: ExternalConnectionReference) -> Self {
        Self {
            external_references: Arc::from([external_reference]),
        }
    }

    pub fn external_references(&self) -> &[ExternalConnectionReference] {
        &self.external_references
    }

    pub fn is_empty(&self) -> bool {
        self.external_references.is_empty()
    }
}

impl fmt::Debug for OutboundActivation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutboundActivation")
            .field("external_reference_count", &self.external_references.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct ConnectionHandle {
    pub connection: Connection,
    outbound_activation: OutboundActivation,
}

impl ConnectionHandle {
    /// Construct the provisional handle returned by
    /// `ConnectionAdapter::originate`.
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            outbound_activation: OutboundActivation::default(),
        }
    }

    /// Receipt made visible by core only after outbound activation and all
    /// post-activation liveness checks complete.
    pub fn outbound_activation(&self) -> &OutboundActivation {
        &self.outbound_activation
    }

    /// Core-only activation receipt attachment. This is public because
    /// `rvoip-core` and `rvoip-core-traits` are separate crates; adapters
    /// should construct provisional handles with [`Self::new`].
    #[doc(hidden)]
    pub fn attach_outbound_activation(&mut self, activation: OutboundActivation) {
        self.outbound_activation = activation;
    }

    /// Discard any receipt attached before core has activated the route.
    #[doc(hidden)]
    pub fn clear_outbound_activation(&mut self) {
        self.outbound_activation = OutboundActivation::default();
    }
}

impl fmt::Debug for ConnectionHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionHandle")
            .field("connection", &self.connection)
            .field(
                "external_reference_count",
                &self.outbound_activation.external_references.len(),
            )
            .finish()
    }
}

#[derive(Clone)]
pub enum RejectReason {
    Busy,
    Decline,
    NotFound,
    Forbidden,
    NotAcceptable,
    ServerError,
    Custom { code: u16, phrase: String },
}

impl fmt::Debug for RejectReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("Busy"),
            Self::Decline => formatter.write_str("Decline"),
            Self::NotFound => formatter.write_str("NotFound"),
            Self::Forbidden => formatter.write_str("Forbidden"),
            Self::NotAcceptable => formatter.write_str("NotAcceptable"),
            Self::ServerError => formatter.write_str("ServerError"),
            Self::Custom { code, phrase } => formatter
                .debug_struct("Custom")
                .field("code", code)
                .field("phrase_present", &!phrase.is_empty())
                .field("phrase_bytes", &phrase.len())
                .finish(),
        }
    }
}

#[derive(Clone)]
pub enum EndReason {
    Normal,
    Cancelled,
    Failed { detail: String },
    Timeout,
    BridgeTorn,
}

impl fmt::Debug for EndReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => formatter.write_str("Normal"),
            Self::Cancelled => formatter.write_str("Cancelled"),
            Self::Failed { detail } => formatter
                .debug_struct("Failed")
                .field("detail_present", &!detail.is_empty())
                .field("detail_bytes", &detail.len())
                .finish(),
            Self::Timeout => formatter.write_str("Timeout"),
            Self::BridgeTorn => formatter.write_str("BridgeTorn"),
        }
    }
}

#[derive(Clone)]
pub enum TransferTarget {
    Uri(String),
    Connection(ConnectionId),
    Session(SessionId),
}

impl fmt::Debug for TransferTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Uri(_) => "Uri([redacted])",
            Self::Connection(_) => "Connection([redacted])",
            Self::Session(_) => "Session([redacted])",
        })
    }
}

/// Transport-neutral asynchronous transfer status.
///
/// Returning successfully from an adapter's `transfer` operation means only
/// that it submitted the transfer request. Protocols such
/// as SIP report authoritative progress and completion later (for example via
/// REFER NOTIFY sipfrags); adapters surface those reports with
/// [`AdapterEvent::TransferStatus`].
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransferStatus {
    /// The remote endpoint accepted responsibility for processing the transfer.
    Accepted,
    /// A provisional status was reported for the transfer target.
    Progress { status_code: u16, reason: String },
    /// A final successful status was reported.
    Completed { status_code: u16, reason: String },
    /// A final failure status was reported.
    Failed { status_code: u16, reason: String },
}

impl fmt::Debug for TransferStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted => formatter.write_str("Accepted"),
            Self::Progress {
                status_code,
                reason,
            } => formatter
                .debug_struct("Progress")
                .field("status_code", status_code)
                .field("reason", &"[redacted]")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Completed {
                status_code,
                reason,
            } => formatter
                .debug_struct("Completed")
                .field("status_code", status_code)
                .field("reason", &"[redacted]")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Failed {
                status_code,
                reason,
            } => formatter
                .debug_struct("Failed")
                .field("status_code", status_code)
                .field("reason", &"[redacted]")
                .field("reason_bytes", &reason.len())
                .finish(),
        }
    }
}

/// Handle returned by adapter playback paths that lets callers stop an
/// in-flight playback.
pub struct PlaybackHandle {
    id: PlaybackId,
    cancel_tx: oneshot::Sender<()>,
}

impl fmt::Debug for PlaybackHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaybackHandle")
            .field("id", &self.id)
            .field("cancel_closed", &self.cancel_tx.is_closed())
            .finish()
    }
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

#[derive(Clone)]
pub struct SignatureHeaders {
    pub signature: String,
    pub signature_input: String,
    pub signature_key: Option<Jwk>,
    pub signature_agent: Option<Jwk>,
}

impl fmt::Debug for SignatureHeaders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignatureHeaders")
            .field("signature_present", &!self.signature.is_empty())
            .field("signature_bytes", &self.signature.len())
            .field("signature_input_present", &!self.signature_input.is_empty())
            .field("signature_input_bytes", &self.signature_input.len())
            .field("signature_key_present", &self.signature_key.is_some())
            .field("signature_agent_present", &self.signature_agent.is_some())
            .finish()
    }
}

/// Adapter-native event surface. `rvoip-core` normalizes these into the
/// orchestration event vocabulary; consumers wanting protocol-native
/// access can subscribe directly to the adapter.
#[derive(Clone)]
#[non_exhaustive]
pub enum AdapterEvent {
    InboundConnection {
        connection: Connection,
    },
    Connected {
        connection_id: ConnectionId,
    },
    /// Connection-scoped provisional signaling reported by an adapter.
    ///
    /// `early_media` is true only when the provisional response established
    /// a usable media description (for SIP, a 183 response carrying SDP).
    /// The raw transport description is intentionally not exposed through
    /// this transport-neutral event.
    Progress {
        connection_id: ConnectionId,
        status_code: u16,
        reason: String,
        early_media: bool,
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
    /// An asynchronous, protocol-authoritative transfer update.
    TransferStatus {
        connection_id: ConnectionId,
        /// Correlates this update with the exact submitted transfer when the
        /// adapter can provide transaction-safe correlation. Legacy adapters
        /// report `None`.
        attempt_id: Option<TransferAttemptId>,
        status: TransferStatus,
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

impl fmt::Debug for AdapterEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InboundConnection { .. } => formatter.write_str("InboundConnection"),
            Self::Connected { .. } => formatter.write_str("Connected"),
            Self::Progress {
                status_code,
                reason,
                early_media,
                ..
            } => formatter
                .debug_struct("Progress")
                .field("status_code", status_code)
                .field("reason_present", &!reason.is_empty())
                .field("reason_bytes", &reason.len())
                .field("early_media", early_media)
                .finish(),
            Self::Authenticated { .. } => formatter.write_str("Authenticated"),
            Self::PrincipalAuthenticated { .. } => formatter.write_str("PrincipalAuthenticated"),
            Self::Ended { .. } => formatter.write_str("Ended"),
            Self::Failed { detail, .. } => formatter
                .debug_struct("Failed")
                .field("detail_present", &!detail.is_empty())
                .field("detail_bytes", &detail.len())
                .finish(),
            Self::Dtmf {
                digits,
                duration_ms,
                ..
            } => formatter
                .debug_struct("Dtmf")
                .field("digit_count", &digits.chars().count())
                .field("duration_ms", duration_ms)
                .finish(),
            Self::Quality { .. } => formatter.write_str("Quality"),
            Self::Message { text, .. } => formatter
                .debug_struct("Message")
                .field("text_present", &!text.is_empty())
                .field("text_bytes", &text.len())
                .finish(),
            Self::DataMessage { message, .. } => formatter
                .debug_struct("DataMessage")
                .field("body_bytes", &message.bytes.len())
                .finish(),
            Self::TransferStatus {
                attempt_id, status, ..
            } => formatter
                .debug_struct("TransferStatus")
                .field("attempt_id_present", &attempt_id.is_some())
                .field("status", status)
                .finish(),
            Self::StepUpResponse {
                method, credential, ..
            } => formatter
                .debug_struct("StepUpResponse")
                .field("method_present", &!method.is_empty())
                .field("method_bytes", &method.len())
                .field("credential_present", &!credential.is_empty())
                .field("credential_bytes", &credential.len())
                .finish(),
            Self::Native { kind, detail } => formatter
                .debug_struct("Native")
                .field("kind", kind)
                .field("detail_present", &!detail.is_empty())
                .field("detail_bytes", &detail.len())
                .finish(),
        }
    }
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

    #[derive(Debug)]
    struct AdapterOwnedOptions {
        endpoint: String,
        bearer: String,
    }

    #[test]
    fn originate_context_round_trips_by_type_and_redacts_debug() {
        let context = OriginateContext::new(AdapterOwnedOptions {
            endpoint: "https://private.invalid/session".into(),
            bearer: "private-bearer".into(),
        });
        let cloned = context.clone();

        assert_eq!(
            context
                .downcast_ref::<AdapterOwnedOptions>()
                .unwrap()
                .endpoint,
            "https://private.invalid/session"
        );
        assert!(context.downcast_ref::<String>().is_none());
        let first = context.downcast_arc::<AdapterOwnedOptions>().unwrap();
        let second = cloned.downcast_arc::<AdapterOwnedOptions>().unwrap();
        assert!(Arc::ptr_eq(&first, &second));

        let request = OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: "sip:private-target@example.invalid".into(),
            direction: Direction::Outbound,
            capabilities: CapabilityDescriptor::default(),
            transport: Some(Transport::Sip),
            context,
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("private.invalid"));
        assert!(!debug.contains("private-bearer"));
        assert!(!debug.contains("AdapterOwnedOptions"));
        assert!(debug.contains("[redacted]"));
        assert_eq!(first.bearer, "private-bearer");

        let default_context = OriginateContext::default();
        assert!(default_context.is_empty());
        assert!(default_context
            .downcast_ref::<AdapterOwnedOptions>()
            .is_none());
    }

    #[test]
    fn external_references_are_bounded_and_redacted() {
        let reference =
            ExternalConnectionReference::new("sip.call-id", "private-call-id@example.invalid")
                .unwrap();
        assert_eq!(reference.kind(), "sip.call-id");
        assert_eq!(reference.expose_secret(), "private-call-id@example.invalid");
        let debug = format!("{reference:?}");
        assert!(!debug.contains("sip.call-id"));
        assert!(!debug.contains("private-call-id"));
        assert!(debug.contains("[redacted]"));

        let activation = OutboundActivation::new([reference.clone()]).unwrap();
        assert_eq!(activation.external_references(), &[reference.clone()]);
        let debug = format!("{activation:?}");
        assert!(!debug.contains("sip.call-id"));
        assert!(!debug.contains("private-call-id"));
        assert!(debug.contains("external_reference_count: 1"));

        assert_eq!(
            ExternalConnectionReference::new("", "value").unwrap_err(),
            ExternalConnectionReferenceError::EmptyKind
        );
        assert_eq!(
            ExternalConnectionReference::new("-invalid", "value").unwrap_err(),
            ExternalConnectionReferenceError::InvalidKind
        );
        assert_eq!(
            ExternalConnectionReference::new(
                "x".repeat(MAX_EXTERNAL_REFERENCE_KIND_BYTES + 1),
                "value"
            )
            .unwrap_err(),
            ExternalConnectionReferenceError::KindTooLarge
        );
        assert_eq!(
            ExternalConnectionReference::new("provider.id", "").unwrap_err(),
            ExternalConnectionReferenceError::EmptyValue
        );
        assert_eq!(
            ExternalConnectionReference::new(
                "provider.id",
                "x".repeat(MAX_EXTERNAL_REFERENCE_VALUE_BYTES + 1)
            )
            .unwrap_err(),
            ExternalConnectionReferenceError::ValueTooLarge
        );
        assert_eq!(
            ExternalConnectionReference::new("provider.id", "unsafe\r\nvalue").unwrap_err(),
            ExternalConnectionReferenceError::InvalidValue
        );

        let too_many = std::iter::repeat_n(reference, MAX_EXTERNAL_CONNECTION_REFERENCES + 1);
        assert_eq!(
            OutboundActivation::new(too_many).unwrap_err(),
            ExternalConnectionReferenceError::TooManyReferences
        );
    }

    #[test]
    fn signature_and_step_up_debug_never_render_credential_values() {
        const CANARY: &str = "adapter-credential-canary\r\nAuthorization: exposed";
        let headers = SignatureHeaders {
            signature: CANARY.into(),
            signature_input: CANARY.into(),
            signature_key: None,
            signature_agent: None,
        };
        let event = AdapterEvent::StepUpResponse {
            connection_id: ConnectionId::new(),
            method: "bearer".into(),
            credential: CANARY.into(),
        };

        for rendered in [format!("{headers:?}"), format!("{event:?}")] {
            assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
        }
        assert_eq!(headers.signature, CANARY);
        match event {
            AdapterEvent::StepUpResponse { credential, .. } => assert_eq!(credential, CANARY),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn transfer_status_debug_never_renders_peer_reason() {
        const CANARY: &str = "transfer-reason-canary\r\nAuthorization: exposed";
        let event = AdapterEvent::TransferStatus {
            connection_id: ConnectionId::new(),
            attempt_id: Some(TransferAttemptId::from_string(CANARY)),
            status: TransferStatus::Failed {
                status_code: 503,
                reason: CANARY.into(),
            },
        };

        let rendered = format!("{event:?}");
        assert!(
            !rendered.contains(CANARY),
            "transfer reason leaked: {rendered}"
        );
        assert!(rendered.contains("503"));
        assert!(rendered.contains("[redacted]"));
        assert!(rendered.contains("attempt_id_present"));
    }

    #[test]
    fn adapter_control_diagnostics_never_render_peer_values() {
        const CANARY: &str = "adapter-control-canary\r\nAuthorization: exposed";
        let values = [
            format!(
                "{:?}",
                RejectReason::Custom {
                    code: 488,
                    phrase: CANARY.into(),
                }
            ),
            format!(
                "{:?}",
                EndReason::Failed {
                    detail: CANARY.into()
                }
            ),
            format!("{:?}", TransferTarget::Uri(CANARY.into())),
            format!(
                "{:?}",
                TransferTarget::Connection(ConnectionId::from_string(CANARY))
            ),
        ];
        for debug in values {
            assert!(!debug.contains(CANARY));
        }
    }
}
