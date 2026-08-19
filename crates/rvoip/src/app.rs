//! High-level app/gateway API.
//!
//! This module composes the lower-level `rvoip-core`, SIP, and WebRTC
//! surfaces into a product-shaped server runtime. It is intentionally above
//! `rvoip-core`: the core crate remains transport-agnostic, while this facade
//! module is allowed to own adapter startup, SIP registrar resolution, browser
//! signaling, message callbacks, assignment policy, and voice escalation.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect},
    routing::get,
    Router,
};
use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest, RejectReason};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config as CoreConfig;
use rvoip_core::connection::{Direction, Transport as CoreTransport};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{
    BridgeId, ConnectionId, ConversationId, MessageId, ParticipantId, SessionId, TenantId,
};
use rvoip_core::inbound_admission::InboundAdmission;
use rvoip_core::message::{ContentType, Message, MessageOrigin, MessageRecipients};
use rvoip_core::operational_events::{OperationalEvent, OperationalEventStreamHealth};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::store::MessageFilter;
use rvoip_core_traits::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance,
};
use rvoip_sip::server::contact_resolver::{
    ContactRequest, ContactResolver, RegistrarContactResolver, ResolvedContact,
};
use rvoip_sip::{
    Config as LowSipConfig, IpNet, SipAdapter, SipInboundContextPolicy, SipListenerAuthPolicy,
    UnifiedCoordinator,
};
use rvoip_webrtc::{
    WebRtcAdapter, WebRtcConfig as LowWebRtcConfig, WebRtcServer, WebRtcServerBuilder,
};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, Mutex};

/// Result type returned by the high-level app API.
pub type AppResult<T> = std::result::Result<T, AppError>;

/// Error type returned by the high-level app API.
#[derive(Debug, Error)]
pub enum AppError {
    /// A configured socket address could not be parsed.
    #[error("invalid bind address `{addr}`: {source}")]
    InvalidBind {
        /// The string address supplied by the caller.
        addr: String,
        /// The parser error.
        source: std::net::AddrParseError,
    },

    /// A requested transport is not available in the first app-layer runtime.
    #[error("unsupported app transport: {0}")]
    UnsupportedTransport(&'static str),

    /// A role, capability, or routing decision was rejected by policy.
    #[error("policy rejected request: {0}")]
    Policy(String),

    /// The assigned employee cannot currently be reached for voice.
    #[error("no routeable voice contact for `{0}`")]
    NoVoiceContact(String),

    /// The configured WebRTC server did not expose a WS signaling address.
    #[error("WebRTC WS signaling address is unavailable")]
    MissingWebRtcWsAddress,

    /// A WebRTC service failed.
    #[error("WebRTC error: {0}")]
    WebRtc(String),

    /// A SIP service failed.
    #[error("SIP error: {0}")]
    Sip(String),

    /// A SIP registrar contact lookup failed.
    #[error("SIP contact resolution failed: {0}")]
    ContactResolution(String),

    /// A core orchestration operation failed.
    #[error(transparent)]
    Core(#[from] rvoip_core::RvoipError),

    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Logical application role.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Role {
    /// A customer/end-user connecting to the app.
    Customer,
    /// An employee/agent serving customers.
    Employee,
}

/// Application-level capability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    /// Text messages.
    Text,
    /// Realtime voice.
    Voice,
    /// Realtime video.
    Video,
}

/// High-level transport family used by app routing policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Transport {
    /// Browser/native WebRTC.
    WebRtc,
    /// SIP signaling and RTP media.
    Sip,
    /// UCTP over one of its substrates.
    Uctp,
}

/// Static HTTP server configuration for app demos and browser clients.
#[derive(Clone, Debug)]
pub struct HttpConfig {
    bind: String,
    static_root: Option<PathBuf>,
}

impl HttpConfig {
    /// Bind a static HTTP service to `addr`.
    pub fn bind(addr: impl Into<String>) -> Self {
        Self {
            bind: addr.into(),
            static_root: None,
        }
    }

    /// Serve files from `root`.
    pub fn serve_static(mut self, root: impl Into<PathBuf>) -> Self {
        self.static_root = Some(root.into());
        self
    }
}

/// WebRTC server configuration for the app layer.
#[derive(Clone, Debug)]
pub struct WebRtcConfig {
    ws_bind: String,
    role_capabilities: RoleCapabilities,
    escalation_command: String,
}

impl WebRtcConfig {
    /// Bind WebRTC WebSocket signaling to `addr`.
    pub fn ws(addr: impl Into<String>) -> Self {
        Self {
            ws_bind: addr.into(),
            role_capabilities: RoleCapabilities::default(),
            escalation_command: "CALL_ASSIGNED_EMPLOYEE".into(),
        }
    }

    /// Allow `role` to use the supplied capabilities over WebRTC.
    pub fn allow<I>(mut self, role: Role, capabilities: I) -> Self
    where
        I: IntoIterator<Item = Capability>,
    {
        self.role_capabilities.allow(role, capabilities);
        self
    }

    /// Configure the inbound text command that asks the app to start voice.
    pub fn escalation_command(mut self, command: impl Into<String>) -> Self {
        self.escalation_command = command.into();
        self
    }
}

/// SIP server and registrar configuration for the app layer.
#[derive(Clone, Debug)]
pub struct SipConfig {
    bind: String,
    domain: String,
    sip_advertised_addr: Option<SocketAddr>,
    media_public_addr: Option<SocketAddr>,
    role_capabilities: RoleCapabilities,
    registrar_users: HashMap<String, String>,
    tenant: Option<String>,
    /// `(CIDR, subject)` pairs. A request whose source IP falls inside a
    /// trusted CIDR is admitted with the corresponding principal.
    trusted_trunks: Vec<(String, String)>,
    /// `X-*` headers to capture into the inbound context.
    captured_headers: Vec<String>,
    /// Playout smoothing and packet-loss concealment for inbound audio.
    playout: Option<rvoip_sip::PlayoutConfig>,
    /// SRTP posture: whether to offer it, and whether to insist on it.
    srtp: SipMediaSecurity,
}

/// How much media encryption a SIP listener insists on.
///
/// The default is `Disabled`, which is what every release before this did
/// and what a LAN PBX or a lab wants. A trunk crossing the public internet
/// should be `Required`: `Preferred` will silently carry a call in the clear
/// when the far end declines, which is exactly the case where the operator
/// most needs to know.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SipMediaSecurity {
    /// Carry media as plaintext RTP.
    #[default]
    Disabled,
    /// Offer SRTP, but carry the call unencrypted if the peer declines.
    Preferred,
    /// Offer SRTP and fail the call rather than fall back to plaintext RTP.
    Required,
}

impl SipConfig {
    /// Smooth inbound audio and conceal lost packets.
    ///
    /// A carrier trunk delivers audio in bursts and loses packets; without
    /// this those are heard directly as clicks and dropouts. It costs the
    /// buffer's depth in added latency, which is why it is opt-in rather
    /// than assumed.
    #[must_use]
    pub fn playout(
        mut self,
        config: rvoip_sip::PlayoutConfig,
    ) -> Self {
        self.playout = Some(config);
        self
    }

    /// Set the media-encryption posture for this listener.
    #[must_use]
    pub const fn media_security(mut self, security: SipMediaSecurity) -> Self {
        self.srtp = security;
        self
    }

    /// Bind the SIP listener/registrar to `addr`.
    pub fn bind(addr: impl Into<String>) -> Self {
        Self {
            bind: addr.into(),
            domain: "callcenter.local".into(),
            sip_advertised_addr: None,
            media_public_addr: None,
            role_capabilities: RoleCapabilities::default(),
            registrar_users: HashMap::new(),
            tenant: None,
            trusted_trunks: Vec::new(),
            captured_headers: Vec::new(),
            // Both off by default: unchanged behaviour for every existing
            // caller, and a deployment that wants smoothing or encryption
            // says so rather than inheriting it.
            playout: None,
            srtp: SipMediaSecurity::Disabled,
        }
    }

    /// Ownership namespace for every identity this listener admits.
    ///
    /// Required by [`Self::trusted_trunk`]: an enabled listener policy without
    /// a tenant fails closed rather than admitting an unowned principal.
    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Trust calls arriving from `cidr` and give them `subject` as their
    /// identity.
    ///
    /// This is the carrier-trunk model: a trunk authenticates by source
    /// address rather than by digest, so nothing else gives those calls a
    /// principal. Without a principal the SIP adapter captures no inbound
    /// context at all, which means **the dialed number is unavailable** and
    /// DID-based routing cannot work.
    ///
    /// `cidr` is parsed at build time; an invalid one fails startup rather
    /// than silently trusting nothing.
    pub fn trusted_trunk(mut self, cidr: impl Into<String>, subject: impl Into<String>) -> Self {
        self.trusted_trunks.push((cidr.into(), subject.into()));
        self
    }

    /// Capture these headers from the inbound INVITE into the connection's
    /// context, readable via `Orchestrator::take_inbound_context`.
    ///
    /// Only `X-*` headers are eligible. `From`, `To` and `P-Asserted-Identity`
    /// are rejected by design — a peer-supplied identity header is a claim, not
    /// a fact, and admitting one would let whoever is on the trunk influence
    /// routing.
    pub fn capture_headers<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.captured_headers
            .extend(names.into_iter().map(Into::into));
        self
    }

    /// Set the SIP AOR domain/realm.
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
        self
    }

    /// Advertise a concrete peer-facing SIP address while retaining the
    /// configured bind address.
    ///
    /// The address is used for SIP Via/Contact generation, and its IP is used
    /// for RTP SDP. This is required when binding SIP to an unspecified
    /// address such as `0.0.0.0`.
    pub fn advertised_addr(mut self, address: SocketAddr) -> Self {
        self.sip_advertised_addr = Some(address);
        self
    }

    /// Override the peer-facing RTP address placed in SDP.
    ///
    /// Port `0` keeps each locally allocated RTP port while replacing only
    /// the advertised IP. When omitted, [`Self::advertised_addr`] supplies
    /// its IP as the media default.
    pub fn media_public_addr(mut self, address: SocketAddr) -> Self {
        self.media_public_addr = Some(address);
        self
    }

    /// Allow `role` to use the supplied capabilities over SIP.
    pub fn allow<I>(mut self, role: Role, capabilities: I) -> Self
    where
        I: IntoIterator<Item = Capability>,
    {
        self.role_capabilities.allow(role, capabilities);
        self
    }

    /// Configure demo registrar users as `(username, password)` pairs.
    pub fn registrar_users<I, U, P>(mut self, users: I) -> Self
    where
        I: IntoIterator<Item = (U, P)>,
        U: Into<String>,
        P: Into<String>,
    {
        self.registrar_users = users
            .into_iter()
            .map(|(user, password)| (user.into(), password.into()))
            .collect();
        self
    }
}

/// UCTP server configuration placeholder for app routing policy.
#[derive(Clone, Debug)]
pub struct UctpConfig {
    bind: String,
}

impl UctpConfig {
    /// Bind a future UCTP service to `addr`.
    pub fn bind(addr: impl Into<String>) -> Self {
        Self { bind: addr.into() }
    }
}

/// Employee admission policy.
#[derive(Clone, Debug, Default)]
pub struct EmployeePolicy {
    employees: HashSet<String>,
}

impl EmployeePolicy {
    /// Allow exactly the named employees.
    pub fn named<I, S>(employees: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            employees: employees.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns true when `employee` is allowed.
    pub fn allows(&self, employee: &str) -> bool {
        self.employees.is_empty() || self.employees.contains(employee)
    }

    fn first(&self) -> Option<String> {
        self.employees.iter().next().cloned()
    }
}

/// Customer admission policy.
#[derive(Clone, Debug)]
pub struct CustomerPolicy {
    transports: HashSet<Transport>,
}

impl CustomerPolicy {
    /// Allow customers to use only SIP.
    pub fn sip_only() -> Self {
        Self::only([Transport::Sip])
    }

    /// Allow customers to use only WebRTC.
    pub fn webrtc_only() -> Self {
        Self::only([Transport::WebRtc])
    }

    /// Allow customers to use exactly the supplied transports.
    pub fn only<I>(transports: I) -> Self
    where
        I: IntoIterator<Item = Transport>,
    {
        Self {
            transports: transports.into_iter().collect(),
        }
    }

    /// Returns true when customers may use `transport`.
    pub fn allows(&self, transport: Transport) -> bool {
        self.transports.contains(&transport)
    }
}

impl Default for CustomerPolicy {
    fn default() -> Self {
        Self::webrtc_only()
    }
}

/// Conversation assignment policy.
#[derive(Clone, Debug)]
pub enum AssignmentPolicy {
    /// Always assign conversations to the named employee.
    Fixed(String),
}

impl AssignmentPolicy {
    /// Always assign conversations to `employee`.
    pub fn fixed(employee: impl Into<String>) -> Self {
        Self::Fixed(employee.into())
    }

    fn assigned_employee(&self) -> String {
        match self {
            Self::Fixed(employee) => employee.clone(),
        }
    }
}

/// Voice transport preference policy.
#[derive(Clone, Debug)]
pub struct VoiceRoutingPolicy {
    transports: Vec<Transport>,
}

impl VoiceRoutingPolicy {
    /// Prefer the supplied transport order when escalating to voice.
    pub fn prefer<I>(transports: I) -> Self
    where
        I: IntoIterator<Item = Transport>,
    {
        Self {
            transports: transports.into_iter().collect(),
        }
    }
}

impl Default for VoiceRoutingPolicy {
    fn default() -> Self {
        Self::prefer([Transport::Sip, Transport::WebRtc, Transport::Uctp])
    }
}

/// Message delivered to a high-level app callback.
#[derive(Clone, Debug)]
pub struct AppMessage {
    /// Core message identifier.
    pub message_id: MessageId,
    /// Conversation that received the message.
    pub conversation_id: ConversationId,
    /// Text body decoded from the inbound message.
    pub text: String,
}

/// Evidence for an established media bridge.
#[derive(Clone, Debug)]
pub struct BridgeEvidence {
    /// Core bridge identifier.
    pub bridge_id: BridgeId,
    /// Customer-side connection.
    pub customer_connection: ConnectionId,
    /// Employee-side connection.
    pub employee_connection: ConnectionId,
    /// Contact URI or active connection used for the employee leg.
    pub employee_route: String,
}

/// High-level app event stream.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// An inbound customer call was admitted to the app conversation.
    InboundCallAccepted {
        /// Transport-independent connection identifier used by bridges and
        /// extensions such as `rvoip::vapi`.
        connection_id: ConnectionId,
        /// Transport that accepted the caller.
        transport: Transport,
    },
    /// A conversation was created or attached for a customer.
    ConversationStarted {
        /// Conversation identifier.
        conversation_id: ConversationId,
        /// Assigned employee user id.
        assigned_employee: String,
    },
    /// A message arrived in the app conversation.
    MessageReceived {
        /// Conversation identifier.
        conversation_id: ConversationId,
        /// Message identifier.
        message_id: MessageId,
        /// Message text.
        text: String,
    },
    /// Conversation assignment changed.
    AssignmentChanged {
        /// Conversation identifier.
        conversation_id: ConversationId,
        /// Assigned employee user id.
        assigned_employee: String,
    },
    /// A customer asked to escalate to voice.
    EscalationRequested {
        /// Conversation identifier.
        conversation_id: ConversationId,
        /// Assigned employee user id.
        assigned_employee: String,
    },
    /// A voice bridge was established.
    CallEstablished {
        /// Conversation identifier.
        conversation_id: ConversationId,
        /// Bridge evidence.
        evidence: BridgeEvidence,
    },
    /// Voice escalation failed.
    CallFailed {
        /// Conversation identifier.
        conversation_id: ConversationId,
        /// Assigned employee user id.
        assigned_employee: String,
        /// Failure reason.
        reason: String,
    },
}

/// Resolved addresses for a running app.
#[derive(Clone, Debug, Default)]
pub struct RvoipAppAddresses {
    /// Static HTTP address, when configured.
    pub http: Option<SocketAddr>,
    /// WebRTC WS signaling address, when configured.
    pub webrtc_ws: Option<SocketAddr>,
    /// SIP listener/registrar address, when configured.
    pub sip: Option<SocketAddr>,
}

/// Resolved voice target for an employee.
#[derive(Clone, Debug)]
pub enum ResolvedVoiceContact {
    /// A SIP AOR resolved through the live registrar.
    Sip {
        /// Employee SIP address-of-record.
        aor: String,
        /// Dialable registered contact URI.
        contact_uri: String,
    },
    /// A currently active transport connection.
    ActiveConnection {
        /// Transport family.
        transport: Transport,
        /// Connection identifier.
        connection_id: ConnectionId,
    },
}

/// Context passed to message callbacks.
#[derive(Clone)]
pub struct ConversationContext {
    state: Arc<AppState>,
}

impl ConversationContext {
    /// Core conversation id.
    pub fn conversation_id(&self) -> ConversationId {
        self.state.conversation_id.clone()
    }

    /// Assigned employee user id.
    pub fn assigned_employee(&self) -> String {
        self.state.assigned_employee.clone()
    }

    /// Send a text reply from the assigned employee/system to the customer.
    pub async fn reply(&self, _from: impl Into<String>, text: impl Into<String>) -> AppResult<()> {
        self.state.send_text_to_customer(text.into()).await
    }

    /// Escalate the current conversation to voice with the assigned employee.
    pub async fn escalate_to_voice(&self) -> AppResult<BridgeEvidence> {
        self.state.escalate_to_assigned_employee().await
    }
}

type BoxedHandlerFuture = Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'static>>;
type MessageHandler =
    Arc<dyn Fn(ConversationContext, AppMessage) -> BoxedHandlerFuture + Send + Sync + 'static>;

fn default_message_handler() -> MessageHandler {
    Arc::new(|_, _| Box::pin(async { Ok(()) }))
}

/// How this app delivers inbound admission and lifecycle to its owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum IngressMode {
    /// The historical path: the app admits inbound connections itself using
    /// its assignment policy, and reads lifecycle from the observational
    /// broadcast. Convenient, and lossy under lag by construction.
    Convenience,
    /// The owning application is the single correctness consumer: it decides
    /// every inbound admission through the gate and reads lifecycle from the
    /// backpressured operational stream. Losing either receiver degrades the
    /// runtime and stops new admission instead of dropping work.
    Authoritative,
}

/// Requests the authoritative ingress composition on [`RvoipAppBuilder`].
///
/// The two correctness primitives must be installed before any adapter is
/// registered, which the convenience `build` path cannot do on its own —
/// this configuration is how an application asks for that ordering.
#[derive(Clone, Copy, Debug)]
pub struct AuthoritativeIngressConfig {
    /// Bounded number of undecided inbound admissions. Backpressure, not
    /// loss: adapters wait for a permit.
    pub admission_capacity: usize,
    /// How long one presented admission may remain undecided.
    pub admission_decision_timeout: Duration,
    /// Bounded capacity of the authoritative operational event stream.
    pub operational_capacity: usize,
}

impl AuthoritativeIngressConfig {
    /// Bounded capacities with a decision deadline.
    #[must_use]
    pub const fn new(
        admission_capacity: usize,
        admission_decision_timeout: Duration,
        operational_capacity: usize,
    ) -> Self {
        Self {
            admission_capacity,
            admission_decision_timeout,
            operational_capacity,
        }
    }
}

impl Default for AuthoritativeIngressConfig {
    fn default() -> Self {
        Self {
            admission_capacity: 64,
            admission_decision_timeout: Duration::from_secs(5),
            operational_capacity: 256,
        }
    }
}

/// The correctness receivers, taken exactly once by the owning application.
///
/// Dropping either receiver is not a way to opt out: core treats the loss as
/// a degraded runtime and refuses new admission, which is the whole point of
/// asking for this mode.
#[derive(Debug)]
pub struct AuthoritativeIngress {
    /// Undecided inbound connections awaiting this application's policy.
    pub admissions: mpsc::Receiver<InboundAdmission>,
    /// Lossless connection lifecycle for the same runtime.
    pub operational: mpsc::Receiver<OperationalEvent>,
}

/// Queryable readiness of the ingress path.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct IngressHealth {
    /// Which delivery contract this app was built with.
    pub mode: IngressMode,
    /// Core's view of the authoritative stream.
    pub operational_stream: OperationalEventStreamHealth,
    /// False once a correctness boundary has been lost. A readiness probe
    /// should fail on this rather than keep taking calls.
    pub admits_new_work: bool,
}

/// Result of a bounded drain.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DrainOutcome {
    /// Whether every retained lifecycle task finished within the budget.
    pub completed: bool,
    /// Tasks still retained when the budget expired.
    pub remaining_tasks: usize,
}

/// Builder for [`RvoipApp`].
pub struct RvoipAppBuilder {
    http: Option<HttpConfig>,
    webrtc: Option<WebRtcConfig>,
    sip: Option<SipConfig>,
    uctp: Option<UctpConfig>,
    employees: EmployeePolicy,
    customers: CustomerPolicy,
    assignment: Option<AssignmentPolicy>,
    voice_routing: VoiceRoutingPolicy,
    on_message: MessageHandler,
    authoritative_ingress: Option<AuthoritativeIngressConfig>,
}

impl RvoipAppBuilder {
    /// Configure the optional static HTTP server.
    pub fn http(mut self, config: HttpConfig) -> Self {
        self.http = Some(config);
        self
    }

    /// Configure WebRTC signaling.
    pub fn webrtc(mut self, config: WebRtcConfig) -> Self {
        self.webrtc = Some(config);
        self
    }

    /// Configure SIP signaling and registration.
    pub fn sip(mut self, config: SipConfig) -> Self {
        self.sip = Some(config);
        self
    }

    /// Configure UCTP admission policy.
    pub fn uctp(mut self, config: UctpConfig) -> Self {
        self.uctp = Some(config);
        self
    }

    /// Configure employees.
    pub fn employees(mut self, policy: EmployeePolicy) -> Self {
        self.employees = policy;
        self
    }

    /// Configure customers.
    pub fn customers(mut self, policy: CustomerPolicy) -> Self {
        self.customers = policy;
        self
    }

    /// Configure assignment.
    pub fn assignment(mut self, policy: AssignmentPolicy) -> Self {
        self.assignment = Some(policy);
        self
    }

    /// Configure voice routing.
    pub fn voice_routing(mut self, policy: VoiceRoutingPolicy) -> Self {
        self.voice_routing = policy;
        self
    }

    /// Take authoritative ownership of inbound admission and lifecycle.
    ///
    /// `build` then installs the inbound admission gate and the operational
    /// event stream **before** registering any adapter — the ordering core
    /// requires and the convenience path cannot express — and hands both
    /// receivers back through [`RvoipApp::take_authoritative_ingress`].
    ///
    /// In this mode the app stops admitting inbound connections on the
    /// application's behalf: every inbound connection is presented as an
    /// [`InboundAdmission`] ticket for the owner to accept or reject, and
    /// the normalized inbound event follows acceptance. Assignment and
    /// voice-routing policy still apply to what the owner accepts.
    pub fn authoritative_ingress(mut self, config: AuthoritativeIngressConfig) -> Self {
        self.authoritative_ingress = Some(config);
        self
    }

    /// Configure the async message callback.
    pub fn on_message<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(ConversationContext, AppMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        self.on_message = Arc::new(move |ctx, msg| Box::pin(handler(ctx, msg)));
        self
    }

    /// Build and start configured services.
    pub async fn build(self) -> AppResult<RvoipApp> {
        if let Some(uctp) = self.uctp {
            let _bind = uctp.bind;
            return Err(AppError::UnsupportedTransport(
                "automatic UCTP service startup is not wired into rvoip::app yet",
            ));
        }

        let assignment = self
            .assignment
            .or_else(|| self.employees.first().map(AssignmentPolicy::fixed))
            .ok_or_else(|| AppError::Policy("no employee assignment policy configured".into()))?;
        let assigned_employee = assignment.assigned_employee();
        if !self.employees.allows(&assigned_employee) {
            return Err(AppError::Policy(format!(
                "assigned employee `{assigned_employee}` is not allowed"
            )));
        }

        let orchestrator = Orchestrator::new(CoreConfig::default());
        // Both correctness primitives reject installation once an adapter is
        // registered, so they are installed here — before any listener can
        // exist — which is the composition the convenience path could not
        // offer and the reason this builder option exists.
        let authoritative_ingress = match self.authoritative_ingress {
            Some(config) => {
                let admissions = orchestrator.install_inbound_admission_gate(
                    config.admission_capacity,
                    config.admission_decision_timeout,
                )?;
                let operational =
                    orchestrator.install_operational_event_stream(config.operational_capacity)?;
                Some(AuthoritativeIngress {
                    admissions,
                    operational,
                })
            }
            None => None,
        };
        let ingress_mode = if authoritative_ingress.is_some() {
            IngressMode::Authoritative
        } else {
            IngressMode::Convenience
        };
        // Subscribe before any adapter starts a listener so an immediate
        // inbound call cannot race app admission-loop startup.
        let core_events = orchestrator.subscribe_events();
        let directory = Arc::new(Directory::default());
        let mut addresses = RvoipAppAddresses::default();
        let mut sip_coordinator = None;
        let mut contact_resolver = None;
        let mut webrtc_server = None;
        let mut webrtc_adapter = None;

        if let Some(sip) = self.sip {
            let employee_voice = sip
                .role_capabilities
                .allows(Role::Employee, Capability::Voice);
            let customer_voice = self.customers.allows(Transport::Sip)
                && sip
                    .role_capabilities
                    .allows(Role::Customer, Capability::Voice);
            if !employee_voice && !customer_voice {
                return Err(AppError::Policy(
                    "SIP is configured but neither employee nor customer voice is allowed".into(),
                ));
            }
            let sip_addr = parse_socket_addr(&sip.bind)?;
            if sip_addr.ip().is_unspecified() && sip.sip_advertised_addr.is_none() {
                return Err(AppError::Policy(
                    "a concrete SIP advertised address is required for an unspecified bind".into(),
                ));
            }
            let sip_addr = resolve_udp_bind_addr(sip_addr)?;
            let low_sip = make_low_sip_config(&sip, sip_addr);

            // A listener auth policy is what gives an inbound INVITE a
            // principal, and the SIP adapter captures no inbound context
            // without one — no dialed number, no custom headers. Left
            // unconfigured the listener stays disabled, preserving the
            // previous behaviour.
            let coordinator = if sip.trusted_trunks.is_empty() {
                UnifiedCoordinator::new(low_sip).await
            } else {
                let tenant = sip.tenant.clone().ok_or_else(|| {
                    AppError::Policy(
                        "a tenant is required when trusted trunks are configured: \
                         every admitted principal must have an owner"
                            .into(),
                    )
                })?;
                let mut policy = SipListenerAuthPolicy::enabled_for_tenant(&tenant)
                    .map_err(|error| AppError::Policy(error.to_string()))?;
                for (cidr, subject) in &sip.trusted_trunks {
                    let parsed: IpNet = cidr.parse().map_err(|_| {
                        AppError::Policy(format!(
                            "trusted trunk CIDR {cidr:?} is not valid, e.g. 203.0.113.0/24"
                        ))
                    })?;
                    policy =
                        policy.with_trusted_cidr(parsed, trusted_trunk_principal(subject, &tenant));
                }
                UnifiedCoordinator::new_with_listener_auth(low_sip, policy).await
            }
            .map_err(|error| AppError::Sip(error.to_string()))?;

            let registrar = coordinator
                .start_registration_server(&sip.domain, sip.registrar_users)
                .await
                .map_err(|error| AppError::Sip(error.to_string()))?;

            // The default context policy captures no headers at all, so the
            // allowlist has to be installed explicitly or `metadata()` is
            // always empty. The Request-URI routing hint comes through either
            // way, provided a principal exists.
            let adapter = if sip.captured_headers.is_empty() {
                SipAdapter::new(Arc::clone(&coordinator)).await
            } else {
                let policy =
                    SipInboundContextPolicy::new(&sip.captured_headers).map_err(|error| {
                        AppError::Policy(format!(
                            "inbound header allowlist rejected: {error:?}. \
                         Only X-* headers are eligible."
                        ))
                    })?;
                SipAdapter::new_with_inbound_context_policy(Arc::clone(&coordinator), policy).await
            }
            .map_err(|error| AppError::Sip(error.to_string()))?;
            orchestrator.register(adapter as Arc<dyn ConnectionAdapter>)?;
            if employee_voice {
                for employee in &self.employees.employees {
                    directory.add_sip_aor(employee, format!("sip:{employee}@{}", sip.domain));
                }
            }
            addresses.sip = Some(sip_addr);
            contact_resolver = Some(RegistrarContactResolver::new(registrar));
            sip_coordinator = Some(coordinator);
        }

        let escalation_command = if let Some(webrtc) = self.webrtc {
            if !self.customers.allows(Transport::WebRtc) {
                return Err(AppError::Policy(
                    "WebRTC is configured but customers are not allowed to use WebRTC".into(),
                ));
            }
            let customer_text = webrtc
                .role_capabilities
                .allows(Role::Customer, Capability::Text);
            let customer_voice = webrtc
                .role_capabilities
                .allows(Role::Customer, Capability::Voice);
            if !customer_text && !customer_voice {
                return Err(AppError::Policy(
                    "WebRTC customer text or voice is required for the app runtime".into(),
                ));
            }
            let mut config = LowWebRtcConfig::loopback();
            config.trickle_ice = false;
            let server = WebRtcServerBuilder::new(config)
                .with_ws(webrtc.ws_bind)
                .build()
                .await
                .map_err(|error| AppError::WebRtc(error.to_string()))?;
            let adapter = server.adapter();
            let ws_addr = server.ws_addr().ok_or(AppError::MissingWebRtcWsAddress)?;
            orchestrator.register(adapter.clone() as Arc<dyn ConnectionAdapter>)?;
            addresses.webrtc_ws = Some(ws_addr);
            webrtc_adapter = Some(adapter);
            webrtc_server = Some(server);
            webrtc.escalation_command
        } else {
            "CALL_ASSIGNED_EMPLOYEE".into()
        };

        let conversation_id = orchestrator
            .open_conversation(
                TenantId::new(),
                ConversationPolicy::default(),
                HashMap::from([
                    ("assigned_employee".to_string(), assigned_employee.clone()),
                    ("app_layer".to_string(), "rvoip::app".to_string()),
                ]),
            )
            .await?;
        let session_id = orchestrator
            .start_session(conversation_id.clone(), SessionMedium::Mixed, vec![])
            .await?;
        let customer_participant = ParticipantId::new();
        let employee_participant = ParticipantId::new();
        // Retain the initial receiver until the application subscribes. This
        // preserves events emitted between listener startup and the caller's
        // first `subscribe_events()` call.
        let (app_events, initial_app_events) = broadcast::channel(64);

        let state = Arc::new(AppState {
            orchestrator,
            directory,
            contact_resolver,
            webrtc_adapter: webrtc_adapter.clone(),
            conversation_id,
            session_id,
            customer_participant,
            employee_participant,
            assigned_employee,
            customers: self.customers,
            customer_connection: Mutex::new(None),
            bridge: Mutex::new(None),
            app_events: app_events.clone(),
            message_handler: self.on_message,
            voice_routing: self.voice_routing,
            escalation_command,
            ingress_mode,
            observational_loss: AtomicBool::new(false),
        });
        spawn_app_event_loop(Arc::clone(&state), core_events);

        let mut http_task = None;
        if let Some(http) = self.http {
            let ws_url = addresses
                .webrtc_ws
                .map(|addr| format!("ws://{addr}"))
                .unwrap_or_default();
            let (addr, task) = spawn_static_server(http, ws_url).await?;
            addresses.http = Some(addr);
            http_task = Some(task);
        }

        Ok(RvoipApp {
            state,
            initial_events: StdMutex::new(Some(initial_app_events)),
            authoritative_ingress: StdMutex::new(authoritative_ingress),
            _webrtc_server: webrtc_server,
            _sip_coordinator: sip_coordinator,
            _http_task: http_task,
            addresses,
        })
    }
}

impl Default for RvoipAppBuilder {
    fn default() -> Self {
        Self {
            http: None,
            webrtc: None,
            sip: None,
            uctp: None,
            employees: EmployeePolicy::default(),
            customers: CustomerPolicy::default(),
            assignment: None,
            voice_routing: VoiceRoutingPolicy::default(),
            on_message: default_message_handler(),
            authoritative_ingress: None,
        }
    }
}

/// Running high-level rvoip gateway app.
pub struct RvoipApp {
    state: Arc<AppState>,
    initial_events: StdMutex<Option<broadcast::Receiver<AppEvent>>>,
    authoritative_ingress: StdMutex<Option<AuthoritativeIngress>>,
    _webrtc_server: Option<WebRtcServer>,
    _sip_coordinator: Option<Arc<UnifiedCoordinator>>,
    _http_task: Option<tokio::task::JoinHandle<()>>,
    addresses: RvoipAppAddresses,
}

impl RvoipApp {
    /// Start building an app.
    pub fn builder() -> RvoipAppBuilder {
        RvoipAppBuilder::default()
    }

    /// Take the correctness receivers, once, when built with
    /// [`RvoipAppBuilder::authoritative_ingress`].
    ///
    /// Returns `None` in convenience mode, and `None` on any later call —
    /// there is exactly one correctness consumer by construction.
    pub fn take_authoritative_ingress(&self) -> Option<AuthoritativeIngress> {
        self.authoritative_ingress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// Current ingress readiness. Fail a readiness probe when
    /// [`IngressHealth::admits_new_work`] is false: a lost correctness
    /// boundary means this runtime can no longer promise that an accepted
    /// call was seen exactly once.
    pub fn ingress_health(&self) -> IngressHealth {
        let operational_stream = self.state.orchestrator.operational_event_stream_health();
        let stream_ok = !matches!(operational_stream, OperationalEventStreamHealth::Degraded);
        let observational_ok = !self.state.observational_loss.load(Ordering::Relaxed);
        IngressHealth {
            mode: self.state.ingress_mode,
            operational_stream,
            admits_new_work: stream_ok && observational_ok,
        }
    }

    /// Stop taking new work and wait, within a budget, for retained
    /// connection lifecycle tasks to converge.
    ///
    /// Terminal: adapter registration and new connection-side work are
    /// rejected once this begins. The outcome reports honestly whether the
    /// budget was enough rather than blocking forever.
    pub async fn drain(&self, budget: Duration) -> DrainOutcome {
        let orchestrator = &self.state.orchestrator;
        orchestrator.drain_prepared_outbound_connections().await;
        let completed =
            tokio::time::timeout(budget, orchestrator.drain_connection_lifecycle_tasks())
                .await
                .is_ok();
        DrainOutcome {
            completed,
            remaining_tasks: orchestrator.connection_lifecycle_task_count(),
        }
    }

    /// Subscribe to high-level app events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<AppEvent> {
        self.initial_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| self.state.app_events.subscribe())
    }

    /// Resolved service addresses.
    pub fn addresses(&self) -> RvoipAppAddresses {
        self.addresses.clone()
    }

    /// Underlying orchestrator for diagnostics and advanced escape hatches.
    pub fn orchestrator(&self) -> Arc<Orchestrator> {
        Arc::clone(&self.state.orchestrator)
    }

    /// WebRTC adapter, when WebRTC is configured.
    pub fn webrtc_adapter(&self) -> Option<Arc<WebRtcAdapter>> {
        self.state.webrtc_adapter.clone()
    }

    /// Register an already-authenticated employee connection in the app directory.
    pub async fn register_employee_connection(
        &self,
        employee: impl Into<String>,
        transport: Transport,
        connection_id: ConnectionId,
    ) -> AppResult<()> {
        let employee = employee.into();
        match transport {
            Transport::Sip => Err(AppError::Policy(
                "SIP employee reachability must come from REGISTER/contact resolution".into(),
            )),
            Transport::WebRtc | Transport::Uctp => {
                self.state
                    .directory
                    .add_active_connection(&employee, transport, connection_id);
                Ok(())
            }
        }
    }

    /// Resolve the employee voice route using the configured routing policy.
    pub async fn resolve_employee_voice_contact(
        &self,
        employee: impl AsRef<str>,
    ) -> AppResult<ResolvedVoiceContact> {
        self.state
            .resolve_employee_voice_contact(employee.as_ref())
            .await
    }

    /// Escalate the assigned conversation to voice.
    pub async fn escalate_assigned_voice(&self) -> AppResult<BridgeEvidence> {
        self.state.escalate_to_assigned_employee().await
    }

    /// Run until Ctrl-C.
    pub async fn run(&self) -> AppResult<()> {
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct RoleCapabilities {
    allowed: HashMap<Role, HashSet<Capability>>,
}

impl RoleCapabilities {
    fn allow<I>(&mut self, role: Role, capabilities: I)
    where
        I: IntoIterator<Item = Capability>,
    {
        self.allowed.entry(role).or_default().extend(capabilities);
    }

    fn allows(&self, role: Role, capability: Capability) -> bool {
        self.allowed
            .get(&role)
            .map(|caps| caps.contains(&capability))
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug)]
enum DirectoryContact {
    SipAor(String),
    ActiveConnection {
        transport: Transport,
        connection_id: ConnectionId,
    },
}

#[derive(Default)]
struct Directory {
    contacts: StdMutex<HashMap<String, Vec<DirectoryContact>>>,
}

impl Directory {
    fn add_sip_aor(&self, employee: &str, aor: String) {
        self.contacts
            .lock()
            .expect("directory lock poisoned")
            .entry(employee.to_string())
            .or_default()
            .push(DirectoryContact::SipAor(aor));
    }

    fn add_active_connection(
        &self,
        employee: &str,
        transport: Transport,
        connection_id: ConnectionId,
    ) {
        self.contacts
            .lock()
            .expect("directory lock poisoned")
            .entry(employee.to_string())
            .or_default()
            .push(DirectoryContact::ActiveConnection {
                transport,
                connection_id,
            });
    }

    fn resolve(&self, employee: &str, policy: &VoiceRoutingPolicy) -> Option<DirectoryContact> {
        let contacts = self.contacts.lock().expect("directory lock poisoned");
        let contacts = contacts.get(employee)?;
        for preferred in &policy.transports {
            if let Some(contact) = contacts.iter().find(|contact| match contact {
                DirectoryContact::SipAor(_) => preferred == &Transport::Sip,
                DirectoryContact::ActiveConnection { transport, .. } => transport == preferred,
            }) {
                return Some(contact.clone());
            }
        }
        None
    }
}

struct AppState {
    orchestrator: Arc<Orchestrator>,
    directory: Arc<Directory>,
    contact_resolver: Option<RegistrarContactResolver>,
    webrtc_adapter: Option<Arc<WebRtcAdapter>>,
    conversation_id: ConversationId,
    session_id: SessionId,
    customer_participant: ParticipantId,
    employee_participant: ParticipantId,
    assigned_employee: String,
    customers: CustomerPolicy,
    customer_connection: Mutex<Option<ConnectionId>>,
    bridge: Mutex<Option<BridgeEvidence>>,
    app_events: broadcast::Sender<AppEvent>,
    message_handler: MessageHandler,
    voice_routing: VoiceRoutingPolicy,
    escalation_command: String,
    ingress_mode: IngressMode,
    /// Sticky: the observational stream dropped events this app was relying
    /// on. Reported through [`RvoipApp::ingress_health`] so a readiness probe
    /// can fail instead of the runtime quietly serving from a gap.
    observational_loss: AtomicBool,
}

impl AppState {
    async fn send_text_to_customer(&self, body: String) -> AppResult<()> {
        let Some(conn) = self.customer_connection.lock().await.clone() else {
            return Ok(());
        };
        let message = Message {
            id: MessageId::new(),
            conversation_id: self.conversation_id.clone(),
            origin: MessageOrigin::Ai(self.employee_participant.clone()),
            from_participant: self.employee_participant.clone(),
            to: MessageRecipients::Participants(vec![self.customer_participant.clone()]),
            direction: Direction::Outbound,
            content_type: ContentType::Text,
            body: Bytes::from(body),
            attachments: vec![],
            in_reply_to: None,
            timestamp: Utc::now(),
        };
        self.orchestrator
            .send_message_to_connection(conn, message)
            .await?;
        Ok(())
    }

    async fn resolve_employee_voice_contact(
        &self,
        employee: &str,
    ) -> AppResult<ResolvedVoiceContact> {
        match self.directory.resolve(employee, &self.voice_routing) {
            Some(DirectoryContact::SipAor(aor)) => {
                let resolver = self
                    .contact_resolver
                    .as_ref()
                    .ok_or_else(|| AppError::NoVoiceContact(employee.to_string()))?;
                let contact = resolver
                    .resolve_contact(&ContactRequest::Registered { aor: aor.clone() })
                    .await
                    .map_err(|error| AppError::ContactResolution(error.to_string()))?;
                Ok(ResolvedVoiceContact::Sip {
                    aor,
                    contact_uri: contact.uri,
                })
            }
            Some(DirectoryContact::ActiveConnection {
                transport,
                connection_id,
            }) => Ok(ResolvedVoiceContact::ActiveConnection {
                transport,
                connection_id,
            }),
            None => Err(AppError::NoVoiceContact(employee.to_string())),
        }
    }

    async fn escalate_to_assigned_employee(&self) -> AppResult<BridgeEvidence> {
        if let Some(existing) = self.bridge.lock().await.clone() {
            return Ok(existing);
        }

        let Some(customer_connection) = self.customer_connection.lock().await.clone() else {
            let reason = "no active customer WebRTC connection".to_string();
            let _ = self.app_events.send(AppEvent::CallFailed {
                conversation_id: self.conversation_id.clone(),
                assigned_employee: self.assigned_employee.clone(),
                reason: reason.clone(),
            });
            return Err(AppError::Policy(reason));
        };

        let _ = self.app_events.send(AppEvent::EscalationRequested {
            conversation_id: self.conversation_id.clone(),
            assigned_employee: self.assigned_employee.clone(),
        });

        let resolved = match self
            .directory
            .resolve(&self.assigned_employee, &self.voice_routing)
        {
            Some(contact) => contact,
            None => {
                let reason = format!(
                    "{} has no configured voice contacts",
                    self.assigned_employee
                );
                let _ = self.app_events.send(AppEvent::CallFailed {
                    conversation_id: self.conversation_id.clone(),
                    assigned_employee: self.assigned_employee.clone(),
                    reason: reason.clone(),
                });
                return Err(AppError::NoVoiceContact(self.assigned_employee.clone()));
            }
        };

        let evidence = match resolved {
            DirectoryContact::SipAor(aor) => {
                let resolver = self
                    .contact_resolver
                    .as_ref()
                    .ok_or_else(|| AppError::NoVoiceContact(self.assigned_employee.clone()))?;
                let contact = resolver
                    .resolve_contact(&ContactRequest::Registered { aor: aor.clone() })
                    .await
                    .map_err(|error| {
                        let reason = error.to_string();
                        let _ = self.app_events.send(AppEvent::CallFailed {
                            conversation_id: self.conversation_id.clone(),
                            assigned_employee: self.assigned_employee.clone(),
                            reason: reason.clone(),
                        });
                        AppError::ContactResolution(reason)
                    })?;
                self.originate_sip_and_bridge(customer_connection, contact)
                    .await?
            }
            DirectoryContact::ActiveConnection {
                transport,
                connection_id,
            } => {
                let bridge_id = self
                    .orchestrator
                    .bridge_connections(customer_connection.clone(), connection_id.clone())
                    .await?;
                BridgeEvidence {
                    bridge_id,
                    customer_connection,
                    employee_connection: connection_id.clone(),
                    employee_route: format!("{transport:?}:{connection_id}"),
                }
            }
        };

        *self.bridge.lock().await = Some(evidence.clone());
        let _ = self
            .send_text_to_customer(
                "Voice bridge established with the assigned employee.".to_string(),
            )
            .await;
        let _ = self.app_events.send(AppEvent::CallEstablished {
            conversation_id: self.conversation_id.clone(),
            evidence: evidence.clone(),
        });
        Ok(evidence)
    }

    async fn originate_sip_and_bridge(
        &self,
        customer_connection: ConnectionId,
        contact: ResolvedContact,
    ) -> AppResult<BridgeEvidence> {
        let mut connected_events = self.orchestrator.subscribe_events();
        let handle = self
            .orchestrator
            .originate_connection(OriginateRequest {
                session_id: self.session_id.clone(),
                participant_id: self.employee_participant.clone(),
                target: contact.uri.clone(),
                direction: Direction::Outbound,
                capabilities: CapabilityDescriptor::default(),
                transport: Some(CoreTransport::Sip),
                context: Default::default(),
            })
            .await?;
        let employee_connection = handle.connection.id.clone();
        wait_for_core_connection_connected(
            &mut connected_events,
            &employee_connection,
            Duration::from_secs(10),
        )
        .await?;
        let bridge_id = self
            .orchestrator
            .bridge_connections(customer_connection.clone(), employee_connection.clone())
            .await?;
        Ok(BridgeEvidence {
            bridge_id,
            customer_connection,
            employee_connection,
            employee_route: contact.uri,
        })
    }
}

fn spawn_app_event_loop(state: Arc<AppState>, core_events: broadcast::Receiver<Event>) {
    tokio::spawn(async move {
        if let Err(error) = run_app_event_loop(state, core_events).await {
            tracing::warn!(error = %error, "rvoip app event loop stopped");
        }
    });
}

async fn run_app_event_loop(
    state: Arc<AppState>,
    mut events: broadcast::Receiver<Event>,
) -> AppResult<()> {
    loop {
        match events.recv().await {
            // In authoritative mode the owning application already decided
            // this connection through the admission gate — the normalized
            // event only follows its accept — so the app must not admit it
            // a second time on the owner's behalf.
            Ok(Event::ConnectionInbound { connection_id, .. })
                if state.ingress_mode == IngressMode::Convenience =>
            {
                if let Err(error) = handle_inbound_connection(&state, connection_id.clone()).await {
                    // A caller can CANCEL or disconnect while application
                    // policy is accepting it. That call fails independently;
                    // later callers must still reach the admission loop.
                    tracing::warn!(
                        %connection_id,
                        error = %error,
                        "failed to route inbound app connection"
                    );
                }
            }
            Ok(Event::MessageReceived {
                message_id,
                conversation_id,
                ..
            }) if conversation_id == state.conversation_id => {
                if let Err(error) = handle_message_received(&state, message_id.clone()).await {
                    tracing::warn!(
                        %message_id,
                        error = %error,
                        "failed to handle inbound app message"
                    );
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                // A warning is not a contract. Record the loss so
                // `ingress_health` stops claiming this runtime saw
                // everything; in authoritative mode admission itself is
                // unaffected because it rides the gate, not this bus.
                state.observational_loss.store(true, Ordering::Relaxed);
                tracing::error!(
                    skipped,
                    mode = ?state.ingress_mode,
                    "rvoip app event receiver lagged; ingress reported degraded"
                );
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}

async fn handle_inbound_connection(
    state: &Arc<AppState>,
    connection_id: ConnectionId,
) -> AppResult<()> {
    let transport = match state.orchestrator.connection_transport(&connection_id)? {
        CoreTransport::Sip => Transport::Sip,
        CoreTransport::WebRtc => Transport::WebRtc,
        _ => {
            state
                .orchestrator
                .route_inbound_connection(
                    connection_id,
                    InboundAction::Reject {
                        reason: RejectReason::NotAcceptable,
                    },
                )
                .await?;
            return Ok(());
        }
    };
    if !state.customers.allows(transport) {
        state
            .orchestrator
            .route_inbound_connection(
                connection_id,
                InboundAction::Reject {
                    reason: RejectReason::Forbidden,
                },
            )
            .await?;
        return Ok(());
    }
    if transport == Transport::WebRtc {
        let Some(adapter) = &state.webrtc_adapter else {
            return Ok(());
        };
        if !adapter.routes().contains_key(&connection_id) {
            return Ok(());
        }
    }

    state
        .orchestrator
        .route_inbound_connection(
            connection_id.clone(),
            InboundAction::Accept {
                session_id: state.session_id.clone(),
                participant_id: state.customer_participant.clone(),
            },
        )
        .await?;
    *state.customer_connection.lock().await = Some(connection_id.clone());
    let _ = state.app_events.send(AppEvent::InboundCallAccepted {
        connection_id,
        transport,
    });
    let _ = state.app_events.send(AppEvent::ConversationStarted {
        conversation_id: state.conversation_id.clone(),
        assigned_employee: state.assigned_employee.clone(),
    });
    let _ = state.app_events.send(AppEvent::AssignmentChanged {
        conversation_id: state.conversation_id.clone(),
        assigned_employee: state.assigned_employee.clone(),
    });
    Ok(())
}

async fn handle_message_received(state: &Arc<AppState>, message_id: MessageId) -> AppResult<()> {
    let Some(text) = message_text(state, &message_id).await else {
        return Ok(());
    };

    let _ = state.app_events.send(AppEvent::MessageReceived {
        conversation_id: state.conversation_id.clone(),
        message_id: message_id.clone(),
        text: text.clone(),
    });

    if text.trim().eq_ignore_ascii_case(&state.escalation_command) {
        if let Err(error) = state.escalate_to_assigned_employee().await {
            let _ = state
                .send_text_to_customer(format!(
                    "Voice escalation failed for {}: {error}",
                    state.assigned_employee
                ))
                .await;
        }
        return Ok(());
    }

    let ctx = ConversationContext {
        state: Arc::clone(state),
    };
    let msg = AppMessage {
        message_id,
        conversation_id: state.conversation_id.clone(),
        text,
    };
    (state.message_handler)(ctx, msg).await
}

async fn message_text(state: &AppState, message_id: &MessageId) -> Option<String> {
    let page = state
        .orchestrator
        .list_messages(
            state.conversation_id.clone(),
            MessageFilter::default(),
            None,
        )
        .await
        .ok()?;
    page.messages
        .into_iter()
        .find(|message| &message.id == message_id)
        .map(|message| String::from_utf8_lossy(&message.body).into_owned())
}

async fn wait_for_core_connection_connected(
    events: &mut broadcast::Receiver<Event>,
    connection_id: &ConnectionId,
    timeout: Duration,
) -> AppResult<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionConnected {
                connection_id: id, ..
            })) if &id == connection_id => return Ok(()),
            Ok(Ok(Event::ConnectionFailed {
                connection_id: id,
                detail,
                ..
            })) if &id == connection_id => {
                return Err(AppError::Policy(format!(
                    "connection failed before bridge: {detail}"
                )));
            }
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err(AppError::Policy("core event channel closed".into()));
            }
            Err(_) => {}
        }
    }
    Err(AppError::Policy(format!(
        "timed out waiting for {connection_id} to connect"
    )))
}

#[derive(Clone)]
struct StaticState {
    root: PathBuf,
    ws_url: String,
}

async fn spawn_static_server(
    config: HttpConfig,
    ws_url: String,
) -> AppResult<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let bind = config.bind;
    let root = config.static_root.unwrap_or_else(|| PathBuf::from("."));
    let state = StaticState { root, ws_url };
    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/customer.html") }))
        .route("/customer.html", get(serve_customer_html))
        .with_state(state);
    let listener = TcpListener::bind(bind.as_str()).await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::warn!(error = %error, "rvoip app static HTTP server stopped");
        }
    });
    Ok((addr, task))
}

async fn serve_customer_html(State(state): State<StaticState>) -> impl IntoResponse {
    let path = state.root.join("customer.html");
    match tokio::fs::read_to_string(&path).await {
        Ok(template) => {
            let body = template.replace("__RVOIP_WS_URL__", &state.ws_url);
            let mut headers = HeaderMap::new();
            headers.insert(
                "content-type",
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            (StatusCode::OK, headers, body).into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("failed to read customer page: {error}")),
        )
            .into_response(),
    }
}

/// The identity a call arriving from a trusted trunk is admitted with.
///
/// Trust here derives from network location, not from a presented credential.
/// `AuthenticationMethod` has no variant for that, so `ApiKey` is used as the
/// closest available sense — an out-of-band arrangement rather than a
/// challenge-response — matching how rvoip's own listener tests construct a
/// trusted-CIDR principal. The scope is the minimum needed to attach a call.
#[cfg(feature = "sip")]
fn trusted_trunk_principal(subject: &str, tenant: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: subject.to_owned(),
        tenant: Some(tenant.to_owned()),
        scopes: vec!["call:attach".to_owned()],
        issuer: Some("rvoip-app-trusted-trunk".to_owned()),
        expires_at: None,
        method: AuthenticationMethod::ApiKey,
        assurance: IdentityAssurance::Anonymous,
    }
}

fn make_low_sip_config(config: &SipConfig, bind: SocketAddr) -> LowSipConfig {
    let mut low = LowSipConfig::on("rvoip-gateway", bind.ip(), bind.port());
    low.playout = config.playout;
    // `srtp_required` without `offer_srtp` is rejected as an invalid policy
    // downstream, so the two move together.
    match config.srtp {
        SipMediaSecurity::Disabled => {}
        SipMediaSecurity::Preferred => low.offer_srtp = true,
        SipMediaSecurity::Required => {
            low.offer_srtp = true;
            low.srtp_required = true;
        }
    }
    if let Some(advertised) = config.sip_advertised_addr {
        low = low
            .with_sip_advertised_addr(advertised)
            .with_media_public_addr(SocketAddr::new(advertised.ip(), 0));
    }
    if let Some(media_public) = config.media_public_addr {
        low = low.with_media_public_addr(media_public);
    }
    low
}

fn parse_socket_addr(addr: &str) -> AppResult<SocketAddr> {
    addr.parse().map_err(|source| AppError::InvalidBind {
        addr: addr.to_string(),
        source,
    })
}

fn resolve_udp_bind_addr(addr: SocketAddr) -> AppResult<SocketAddr> {
    if addr.port() != 0 {
        return Ok(addr);
    }
    let socket = std::net::UdpSocket::bind(addr)?;
    Ok(socket.local_addr()?)
}

#[cfg(test)]
mod tests {

    /// Media posture set on the builder must reach the coordinator, or a
    /// deployment that asked for encryption silently carries calls in clear.
    #[cfg(feature = "sip")]
    #[test]
    fn media_posture_reaches_the_low_level_config() {
        use rvoip_sip::PlayoutConfig;

        let bind: SocketAddr = "127.0.0.1:5060".parse().expect("addr");

        // Default: unchanged from every release before this.
        let plain = make_low_sip_config(&SipConfig::bind("127.0.0.1:5060"), bind);
        assert!(plain.playout.is_none());
        assert!(!plain.offer_srtp);
        assert!(!plain.srtp_required);

        let smoothed = make_low_sip_config(
            &SipConfig::bind("127.0.0.1:5060").playout(PlayoutConfig {
                target_depth_frames: 4,
                ..PlayoutConfig::default()
            }),
            bind,
        );
        assert_eq!(
            smoothed.playout.expect("playout reaches the coordinator").target_depth_frames,
            4
        );

        // Preferred offers but tolerates plaintext; required refuses it.
        let preferred = make_low_sip_config(
            &SipConfig::bind("127.0.0.1:5060").media_security(SipMediaSecurity::Preferred),
            bind,
        );
        assert!(preferred.offer_srtp);
        assert!(
            !preferred.srtp_required,
            "preferred must fall back, or it is not preferred"
        );

        let required = make_low_sip_config(
            &SipConfig::bind("127.0.0.1:5060").media_security(SipMediaSecurity::Required),
            bind,
        );
        assert!(required.offer_srtp && required.srtp_required);
    }
    use super::*;

    #[test]
    fn role_capabilities_gate_transport_capability() {
        let cfg = WebRtcConfig::ws("127.0.0.1:0")
            .allow(Role::Customer, [Capability::Text, Capability::Voice]);

        assert!(cfg
            .role_capabilities
            .allows(Role::Customer, Capability::Text));
        assert!(!cfg
            .role_capabilities
            .allows(Role::Employee, Capability::Text));
    }

    #[test]
    fn customer_policy_selects_the_inbound_transport() {
        let sip = CustomerPolicy::sip_only();
        assert!(sip.allows(Transport::Sip));
        assert!(!sip.allows(Transport::WebRtc));

        let both = CustomerPolicy::only([Transport::Sip, Transport::WebRtc]);
        assert!(both.allows(Transport::Sip));
        assert!(both.allows(Transport::WebRtc));
    }

    #[test]
    fn sip_advertisement_preserves_wildcard_bind_and_sets_media_ip() {
        let advertised = "192.0.2.10:5060".parse().expect("advertised address");
        let config = SipConfig::bind("0.0.0.0:5060").advertised_addr(advertised);
        let low = make_low_sip_config(&config, "0.0.0.0:5060".parse().expect("bind"));

        assert_eq!(
            low.bind_addr,
            "0.0.0.0:5060".parse::<SocketAddr>().expect("wildcard bind")
        );
        assert_eq!(
            low.local_ip,
            "0.0.0.0".parse::<std::net::IpAddr>().expect("wildcard IP")
        );
        assert_eq!(low.sip_advertised_addr, Some(advertised));
        assert_eq!(
            low.media_public_addr,
            Some("192.0.2.10:0".parse::<SocketAddr>().expect("media address"))
        );
    }

    #[test]
    fn explicit_media_advertisement_overrides_sip_advertised_ip() {
        let config = SipConfig::bind("0.0.0.0:5060")
            .advertised_addr("192.0.2.10:5060".parse().expect("SIP address"))
            .media_public_addr("198.51.100.20:0".parse().expect("media address"));
        let low = make_low_sip_config(&config, "0.0.0.0:5060".parse().expect("bind"));

        assert_eq!(
            low.media_public_addr,
            Some(
                "198.51.100.20:0"
                    .parse::<SocketAddr>()
                    .expect("media address")
            )
        );
    }

    #[test]
    fn fixed_assignment_must_be_allowed_employee() {
        let allowed = EmployeePolicy::named(["alice"]);
        assert!(allowed.allows("alice"));
        assert!(!allowed.allows("bob"));
    }

    #[test]
    fn directory_uses_voice_routing_order() {
        let directory = Directory::default();
        let sip_conn = "sip:alice@callcenter.local".to_string();
        let webrtc_conn = ConnectionId::new();
        directory.add_sip_aor("alice", sip_conn.clone());
        directory.add_active_connection("alice", Transport::WebRtc, webrtc_conn.clone());

        let policy = VoiceRoutingPolicy::prefer([Transport::WebRtc, Transport::Sip]);
        match directory.resolve("alice", &policy) {
            Some(DirectoryContact::ActiveConnection { connection_id, .. }) => {
                assert_eq!(connection_id, webrtc_conn);
            }
            other => panic!("expected WebRTC active connection, got {other:?}"),
        }

        let policy = VoiceRoutingPolicy::prefer([Transport::Sip, Transport::WebRtc]);
        match directory.resolve("alice", &policy) {
            Some(DirectoryContact::SipAor(aor)) => assert_eq!(aor, sip_conn),
            other => panic!("expected SIP AOR, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn uctp_service_startup_fails_explicitly_until_wired() {
        let result = RvoipApp::builder()
            .uctp(UctpConfig::bind("127.0.0.1:0"))
            .employees(EmployeePolicy::named(["alice"]))
            .assignment(AssignmentPolicy::fixed("alice"))
            .build()
            .await;

        match result {
            Err(AppError::UnsupportedTransport(_)) => {}
            Err(other) => panic!("expected unsupported transport, got {other}"),
            Ok(_) => panic!("UCTP service startup should be explicitly unsupported"),
        }
    }

    #[tokio::test]
    async fn wildcard_sip_bind_requires_advertised_address() {
        let result = RvoipApp::builder()
            .customers(CustomerPolicy::sip_only())
            .sip(SipConfig::bind("0.0.0.0:5060").allow(Role::Customer, [Capability::Voice]))
            .employees(EmployeePolicy::named(["vapi-agent"]))
            .assignment(AssignmentPolicy::fixed("vapi-agent"))
            .build()
            .await;

        match result {
            Err(AppError::Policy(reason)) => {
                assert!(reason.contains("advertised address"), "{reason}");
            }
            Err(other) => panic!("expected advertised-address policy error, got {other}"),
            Ok(_) => panic!("wildcard SIP bind must fail without an advertised address"),
        }
    }

    /// Build one app with adapters actually registered, in authoritative
    /// mode. If the builder did not install both correctness primitives
    /// before adapter registration, core would have refused installation and
    /// this build would fail — so a successful build with both receivers in
    /// hand *is* the ordering proof.
    async fn authoritative_app(config: AuthoritativeIngressConfig) -> RvoipApp {
        RvoipApp::builder()
            .customers(CustomerPolicy::sip_only())
            .sip(
                SipConfig::bind("127.0.0.1:0")
                    .domain("test.local")
                    .allow(Role::Customer, [Capability::Voice]),
            )
            .employees(EmployeePolicy::named(["agent"]))
            .assignment(AssignmentPolicy::fixed("agent"))
            .authoritative_ingress(config)
            .build()
            .await
            .expect("build authoritative app")
    }

    #[tokio::test]
    async fn authoritative_ingress_installs_before_adapters_and_hands_over_receivers() {
        let app = authoritative_app(AuthoritativeIngressConfig::default()).await;

        let ingress = app
            .take_authoritative_ingress()
            .expect("authoritative receivers");
        assert!(!ingress.admissions.is_closed());

        let health = app.ingress_health();
        assert_eq!(health.mode, IngressMode::Authoritative);
        assert_eq!(
            health.operational_stream,
            OperationalEventStreamHealth::Healthy
        );
        assert!(health.admits_new_work);
    }

    #[tokio::test]
    async fn authoritative_ingress_has_exactly_one_consumer() {
        let app = authoritative_app(AuthoritativeIngressConfig::default()).await;

        assert!(app.take_authoritative_ingress().is_some());
        assert!(
            app.take_authoritative_ingress().is_none(),
            "a second correctness consumer would defeat the boundary"
        );
    }

    #[tokio::test]
    async fn capacity_one_stream_is_a_supported_configuration() {
        // The register's acceptance matrix runs at capacity one, where every
        // delivery is a backpressure decision.
        let app = authoritative_app(AuthoritativeIngressConfig::new(
            1,
            Duration::from_secs(1),
            1,
        ))
        .await;

        let ingress = app
            .take_authoritative_ingress()
            .expect("authoritative receivers");
        drop(ingress);
        assert_eq!(app.ingress_health().mode, IngressMode::Authoritative);
    }

    #[tokio::test]
    async fn losing_the_operational_receiver_degrades_readiness() {
        let app = authoritative_app(AuthoritativeIngressConfig::default()).await;
        let ingress = app
            .take_authoritative_ingress()
            .expect("authoritative receivers");

        assert!(app.ingress_health().admits_new_work);

        // Losing the correctness receiver must be observable as degraded
        // readiness, not a log line the operator never reads.
        drop(ingress.operational);
        let _ = app
            .state
            .orchestrator
            .open_conversation(
                TenantId::new(),
                ConversationPolicy::default(),
                HashMap::new(),
            )
            .await;

        let health = app.ingress_health();
        assert_eq!(
            health.operational_stream,
            OperationalEventStreamHealth::Degraded
        );
        assert!(
            !health.admits_new_work,
            "a degraded runtime must stop claiming it can take new calls"
        );
    }

    #[tokio::test]
    async fn convenience_mode_keeps_the_legacy_contract() {
        let app = RvoipApp::builder()
            .customers(CustomerPolicy::sip_only())
            .sip(
                SipConfig::bind("127.0.0.1:0")
                    .domain("test.local")
                    .allow(Role::Customer, [Capability::Voice]),
            )
            .employees(EmployeePolicy::named(["agent"]))
            .assignment(AssignmentPolicy::fixed("agent"))
            .build()
            .await
            .expect("build convenience app");

        assert!(app.take_authoritative_ingress().is_none());
        let health = app.ingress_health();
        assert_eq!(health.mode, IngressMode::Convenience);
        assert_eq!(
            health.operational_stream,
            OperationalEventStreamHealth::NotInstalled
        );
        assert!(health.admits_new_work);
    }

    #[tokio::test]
    async fn drain_is_bounded_and_reports_its_outcome() {
        let app = authoritative_app(AuthoritativeIngressConfig::default()).await;
        let _ingress = app.take_authoritative_ingress();

        // An idle runtime converges well inside the budget; the point is
        // that the join point exists and answers honestly either way.
        let outcome = app.drain(Duration::from_secs(5)).await;
        assert!(outcome.completed, "idle drain should finish in budget");
        assert_eq!(outcome.remaining_tasks, 0);
    }

    #[tokio::test]
    async fn unregistered_sip_employee_fails_contact_resolution() {
        let app = RvoipApp::builder()
            .sip(
                SipConfig::bind("127.0.0.1:0")
                    .domain("callcenter.local")
                    .allow(Role::Employee, [Capability::Voice])
                    .registrar_users([("alice", "password123")]),
            )
            .employees(EmployeePolicy::named(["alice"]))
            .assignment(AssignmentPolicy::fixed("alice"))
            .build()
            .await
            .expect("build SIP app");

        let result = app.resolve_employee_voice_contact("alice").await;
        match result {
            Err(AppError::ContactResolution(reason)) => {
                assert!(
                    reason.contains("no live contacts")
                        || reason.contains("User not found")
                        || reason.contains("user-not-found"),
                    "{reason}"
                );
            }
            Err(other) => panic!("expected contact resolution failure, got {other}"),
            Ok(contact) => panic!("unexpected contact resolution: {contact:?}"),
        }
    }

    #[tokio::test]
    async fn sip_customer_inbound_emits_accepted_call() {
        let app = RvoipApp::builder()
            .customers(CustomerPolicy::sip_only())
            .sip(
                SipConfig::bind("127.0.0.1:0")
                    .domain("test.local")
                    .allow(Role::Customer, [Capability::Voice]),
            )
            .employees(EmployeePolicy::named(["vapi-agent"]))
            .assignment(AssignmentPolicy::fixed("vapi-agent"))
            .build()
            .await
            .expect("build SIP customer app");
        let mut app_events = app.subscribe_events();
        let gateway = app.addresses().sip.expect("SIP listener");

        let caller_addr = resolve_udp_bind_addr("127.0.0.1:0".parse().expect("loopback socket"))
            .expect("allocate caller port");
        let caller = UnifiedCoordinator::new(LowSipConfig::on(
            "caller",
            caller_addr.ip(),
            caller_addr.port(),
        ))
        .await
        .expect("start SIP caller");
        let call_id = caller
            .invite(
                Some(format!("sip:caller@{caller_addr}")),
                format!("sip:vapi@{gateway}"),
            )
            .send()
            .await
            .expect("send SIP INVITE");

        let (connection_id, transport) = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match app_events.recv().await {
                    Ok(AppEvent::InboundCallAccepted {
                        connection_id,
                        transport,
                    }) => break (connection_id, transport),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        panic!("app event stream closed before inbound acceptance")
                    }
                }
            }
        })
        .await
        .expect("inbound SIP customer was not accepted");

        assert_eq!(transport, Transport::Sip);
        assert_eq!(
            app.orchestrator()
                .connection_transport(&connection_id)
                .expect("accepted connection transport"),
            CoreTransport::Sip
        );
        assert!(
            app.orchestrator().session_of(&connection_id).is_some(),
            "accepted caller must be routed into the app session"
        );

        let _ = caller.bye(&call_id).send().await;
        let _ = caller.shutdown_gracefully(None).await;
    }

    #[tokio::test]
    async fn webrtc_voice_customer_emits_accepted_call() {
        use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};

        let app = RvoipApp::builder()
            .customers(CustomerPolicy::webrtc_only())
            .webrtc(WebRtcConfig::ws("127.0.0.1:0").allow(Role::Customer, [Capability::Voice]))
            .employees(EmployeePolicy::named(["vapi-agent"]))
            .assignment(AssignmentPolicy::fixed("vapi-agent"))
            .build()
            .await
            .expect("build WebRTC customer app");
        let mut app_events = app.subscribe_events();
        let adapter = app.webrtc_adapter().expect("WebRTC adapter");

        let mut peer_config = LowWebRtcConfig::loopback();
        peer_config.trickle_ice = false;
        let caller = RvoipPeerConnection::new(&peer_config, PeerRole::Offerer)
            .await
            .expect("create WebRTC caller");
        let offer = caller
            .create_offer_and_gather()
            .await
            .expect("create WebRTC offer");
        let offered_connection = adapter
            .apply_remote_offer(&offer)
            .await
            .expect("apply WebRTC offer");
        let answer = adapter
            .local_sdp(&offered_connection)
            .expect("WebRTC answer");
        caller
            .set_remote_answer(&answer)
            .await
            .expect("apply WebRTC answer");

        let accepted_connection = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match app_events.recv().await {
                    Ok(AppEvent::InboundCallAccepted {
                        connection_id,
                        transport: Transport::WebRtc,
                    }) => break connection_id,
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        panic!("app event stream closed before WebRTC acceptance")
                    }
                }
            }
        })
        .await
        .expect("inbound WebRTC customer was not accepted");

        assert_eq!(accepted_connection, offered_connection);
        assert_eq!(
            app.orchestrator()
                .connection_transport(&accepted_connection)
                .expect("accepted connection transport"),
            CoreTransport::WebRtc
        );
        assert!(
            app.orchestrator()
                .session_of(&accepted_connection)
                .is_some(),
            "accepted WebRTC caller must be routed into the app session"
        );

        let _ = app
            .orchestrator()
            .end_connection(accepted_connection, rvoip_core::adapter::EndReason::Normal)
            .await;
        let _ = caller.close().await;
    }
}
