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
use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config as CoreConfig;
use rvoip_core::connection::{Direction, Transport as CoreTransport};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{
    BridgeId, ConnectionId, ConversationId, MessageId, ParticipantId, SessionId, TenantId,
};
use rvoip_core::message::{ContentType, Message, MessageOrigin, MessageRecipients};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::store::MessageFilter;
use rvoip_sip::server::contact_resolver::{
    ContactRequest, ContactResolver, RegistrarContactResolver, ResolvedContact,
};
use rvoip_sip::{Config as LowSipConfig, SipAdapter, UnifiedCoordinator};
use rvoip_webrtc::{
    WebRtcAdapter, WebRtcConfig as LowWebRtcConfig, WebRtcServer, WebRtcServerBuilder,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};

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
    role_capabilities: RoleCapabilities,
    registrar_users: HashMap<String, String>,
}

impl SipConfig {
    /// Bind the SIP listener/registrar to `addr`.
    pub fn bind(addr: impl Into<String>) -> Self {
        Self {
            bind: addr.into(),
            domain: "callcenter.local".into(),
            role_capabilities: RoleCapabilities::default(),
            registrar_users: HashMap::new(),
        }
    }

    /// Set the SIP AOR domain/realm.
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
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
    /// Allow customers to use only WebRTC.
    pub fn webrtc_only() -> Self {
        Self {
            transports: HashSet::from([Transport::WebRtc]),
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
        let directory = Arc::new(Directory::default());
        let mut addresses = RvoipAppAddresses::default();
        let mut sip_coordinator = None;
        let mut contact_resolver = None;
        let mut webrtc_server = None;
        let mut webrtc_adapter = None;

        if let Some(sip) = self.sip {
            if !sip
                .role_capabilities
                .allows(Role::Employee, Capability::Voice)
            {
                return Err(AppError::Policy(
                    "SIP is configured but employee voice is not allowed".into(),
                ));
            }
            let sip_addr = parse_socket_addr(&sip.bind)?;
            let sip_addr = resolve_udp_bind_addr(sip_addr)?;
            let coordinator = UnifiedCoordinator::new(LowSipConfig::on(
                "rvoip-gateway",
                sip_addr.ip(),
                sip_addr.port(),
            ))
            .await
            .map_err(|error| AppError::Sip(error.to_string()))?;
            let registrar = coordinator
                .start_registration_server(&sip.domain, sip.registrar_users)
                .await
                .map_err(|error| AppError::Sip(error.to_string()))?;
            let adapter = SipAdapter::new(Arc::clone(&coordinator))
                .await
                .map_err(|error| AppError::Sip(error.to_string()))?;
            orchestrator.register(adapter as Arc<dyn ConnectionAdapter>)?;
            for employee in &self.employees.employees {
                directory.add_sip_aor(employee, format!("sip:{employee}@{}", sip.domain));
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
            if !webrtc
                .role_capabilities
                .allows(Role::Customer, Capability::Text)
            {
                return Err(AppError::Policy(
                    "WebRTC customer text is required for the app runtime".into(),
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
        let (app_events, _) = broadcast::channel(64);

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
            customer_connection: Mutex::new(None),
            bridge: Mutex::new(None),
            app_events: app_events.clone(),
            message_handler: self.on_message,
            voice_routing: self.voice_routing,
            escalation_command,
        });
        spawn_app_event_loop(Arc::clone(&state));

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
        }
    }
}

/// Running high-level rvoip gateway app.
pub struct RvoipApp {
    state: Arc<AppState>,
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

    /// Subscribe to high-level app events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<AppEvent> {
        self.state.app_events.subscribe()
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
    customer_connection: Mutex<Option<ConnectionId>>,
    bridge: Mutex<Option<BridgeEvidence>>,
    app_events: broadcast::Sender<AppEvent>,
    message_handler: MessageHandler,
    voice_routing: VoiceRoutingPolicy,
    escalation_command: String,
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

fn spawn_app_event_loop(state: Arc<AppState>) {
    tokio::spawn(async move {
        if let Err(error) = run_app_event_loop(state).await {
            tracing::warn!(error = %error, "rvoip app event loop stopped");
        }
    });
}

async fn run_app_event_loop(state: Arc<AppState>) -> AppResult<()> {
    let mut events = state.orchestrator.subscribe_events();
    loop {
        match events.recv().await {
            Ok(Event::ConnectionInbound { connection_id, .. }) => {
                handle_inbound_connection(&state, connection_id).await?;
            }
            Ok(Event::MessageReceived {
                message_id,
                conversation_id,
                ..
            }) if conversation_id == state.conversation_id => {
                handle_message_received(&state, message_id).await?;
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "rvoip app event receiver lagged");
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
    let Some(adapter) = &state.webrtc_adapter else {
        return Ok(());
    };
    if !adapter.routes().contains_key(&connection_id) {
        return Ok(());
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
    *state.customer_connection.lock().await = Some(connection_id);
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
                    reason.contains("no live contacts") || reason.contains("User not found"),
                    "{reason}"
                );
            }
            Err(other) => panic!("expected contact resolution failure, got {other}"),
            Ok(contact) => panic!("unexpected contact resolution: {contact:?}"),
        }
    }
}
