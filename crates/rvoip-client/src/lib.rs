//! # rvoip-client — single-Identity client SDK
//!
//! The unifying `Client` surface from `INTERFACE_DESIGN.md` §15. Wraps
//! the per-protocol native clients (`rvoip-uctp::client`,
//! `rvoip-sip::api`, `rvoip-webrtc::client`) behind a verb-shaped API
//! tuned for mobile / web / desktop / embedded apps that act as a
//! single Identity in a single tenant.
//!
//! ## Status — experimental UCTP QUIC client
//!
//! `Client::connect("uctp+quic://...")` performs the QUIC dial and UCTP
//! bearer handshake, `Client::call(..., SessionMedium::Voice)` sends a
//! `session.invite`, and `SessionHandle::end()` sends `session.end`.
//! SIP and WebRTC client dispatch are still explicit future work.
//!
//! To build a working SIP client today, use
//! [`rvoip-sip`](https://docs.rs/rvoip-sip) directly — `StreamPeer` /
//! `SessionHandle` for full control, or `Endpoint` for a PBX-account softphone.
//! This SDK will point there until its per-protocol dispatch lands.
//!
//! ## Why a separate crate?
//!
//! Per §15.1, the server-side `Orchestrator` surface is multi-tenant
//! and command/event-shaped — wrong fit for a client app driving one
//! user's calls. `Client` carries tenancy implicitly, picks one
//! substrate at construction time, and exposes `.call().await`
//! returning a `SessionHandle` rather than the command/event /
//! correlation-id dance the server uses.
//!
//! ## Quick start
//!
//! ```no_run
//! use rvoip_client::{Client, Credential, CallTarget, SessionMedium};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::connect(
//!     "uctp+quic://thelve.example.com:4433",
//!     Credential::Bearer("alice-token".into()),
//! ).await?;
//!
//! let session = client.call(
//!     CallTarget::Identity("id_bob".into()),
//!     SessionMedium::Voice,
//! ).await?;
//! # let _ = session;
//! # Ok(()) }
//! ```
//!
//! ## Status
//!
//! v1 ships a concrete **UCTP QUIC signaling happy path**. Non-trivial
//! flows (`incoming()` accept/reject, SIP/WebRTC dispatch,
//! multi-substrate priority fall-through, `conversations()` history)
//! land incrementally as consumers exercise them.

#![warn(rust_2018_idioms)]
#![allow(missing_docs)]

use rvoip_core_traits::ids::{ConnectionId, ConversationId, MessageId, SessionId};
use rvoip_core_traits::DataMessage;
#[cfg(feature = "uctp")]
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Per-protocol native client surface re-exports. Per
/// `INTERFACE_DESIGN.md` §15.3, developers who don't want the
/// unifying `Client` can reach for these directly.
#[cfg(feature = "sip")]
pub mod sip {
    pub use rvoip_sip::api;
}
#[cfg(feature = "webrtc")]
pub mod webrtc {
    pub use rvoip_webrtc::*;
}
#[cfg(feature = "uctp")]
pub mod uctp {
    pub use rvoip_uctp::*;
}

/// Credential the client presents at `connect` time. Concrete shape
/// matches the `Credential` enum in `rvoip-core-traits::identity`,
/// but is re-defined locally so the client crate doesn't depend on
/// the orchestrator's identity model directly.
#[derive(Clone)]
pub enum Credential {
    Bearer(String),
    OAuth2Dpop {
        access_token: String,
        dpop_proof: String,
    },
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bearer(token) => formatter
                .debug_struct("Bearer")
                .field("token_present", &!token.is_empty())
                .field("token_len", &token.len())
                .finish(),
            Self::OAuth2Dpop {
                access_token,
                dpop_proof,
            } => formatter
                .debug_struct("OAuth2Dpop")
                .field("access_token_present", &!access_token.is_empty())
                .field("access_token_len", &access_token.len())
                .field("dpop_proof_present", &!dpop_proof.is_empty())
                .field("dpop_proof_len", &dpop_proof.len())
                .finish(),
        }
    }
}

/// Options for `Client::connect_with_options`.
#[derive(Clone, Default)]
pub struct ClientOptions {
    /// Pre-bound QUIC endpoint. When absent, the client binds an ephemeral
    /// UDP socket on `0.0.0.0:0`.
    #[cfg(feature = "uctp")]
    pub quic_endpoint: Option<Arc<quinn::Endpoint>>,
    /// TLS client config for QUIC. Production defaults use WebPKI roots;
    /// tests and local development can pass a pinned self-signed config.
    #[cfg(feature = "uctp")]
    pub quic_client_config: Option<Arc<rustls::ClientConfig>>,
    /// Override the TLS server name. Defaults to the URI host.
    pub server_name: Option<String>,
    /// Device id advertised in `auth.hello`.
    pub device_id: Option<String>,
    /// Local participant id used in outgoing session payloads after auth.
    /// If absent, the server-issued participant id from `auth.session`
    /// is used.
    pub participant_id: Option<String>,
}

/// Target of an outbound `Client::call`. Resolved at the underlying
/// substrate adapter by URL scheme: `Identity` and `Participant`
/// route through UCTP; `Uri` accepts `sip:` / `tel:` and dispatches
/// to the SIP interop adapter.
#[derive(Clone)]
pub enum CallTarget {
    Identity(String),
    Participant(String),
    Uri(String),
}

impl std::fmt::Debug for CallTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Identity(_) => "identity",
            Self::Participant(_) => "participant",
            Self::Uri(_) => "uri",
        };
        formatter
            .debug_struct("CallTarget")
            .field("kind", &kind)
            .finish()
    }
}

/// Medium of a Session at start time. Mirror of
/// `rvoip-core-traits::SessionMedium` to avoid pulling the
/// orchestrator crate.
#[derive(Clone, Debug)]
pub enum SessionMedium {
    Voice,
    Video,
    VoiceVideo,
    TextChat,
    ScreenShare,
}

pub enum ClientError {
    UnsupportedScheme(String),
    ConnectFailed(String),
    InvalidUri(String),
    Protocol(String),
    SessionNotFound,
    NotImplemented(&'static str),
}

impl ClientError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::UnsupportedScheme(_) => "unsupported-scheme",
            Self::ConnectFailed(_) => "connect",
            Self::InvalidUri(_) => "invalid-uri",
            Self::Protocol(_) => "protocol",
            Self::SessionNotFound => "session-not-found",
            Self::NotImplemented(_) => "not-implemented",
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "client operation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl std::fmt::Debug for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for ClientError {}

pub type Result<T> = std::result::Result<T, ClientError>;

/// Inbound event from the connected substrate — consumer pumps
/// `Client::incoming()` to receive these.
#[non_exhaustive]
pub enum InboundEvent {
    /// Peer is inviting us into a new Session. Consumer either
    /// `accept`s the returned handle to enter the call or `reject`s
    /// it to refuse.
    IncomingSession(SessionHandle),
    /// A Message landed in one of our Conversations.
    Message {
        conversation_id: ConversationId,
        message_id: MessageId,
        from: String,
        body: String,
    },
    DataMessage {
        connection_id: ConnectionId,
        message: DataMessage,
    },
    /// Our IdentityAssurance level changed on a Connection — usually
    /// because step-up auth completed (CONVERSATION_PROTOCOL.md §5.8).
    AssuranceChanged {
        connection_id: ConnectionId,
        new_assurance: String,
    },
    Disconnected {
        reason: String,
    },
}

impl std::fmt::Debug for InboundEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncomingSession(_) => formatter.write_str("IncomingSession"),
            Self::Message { body, .. } => formatter
                .debug_struct("Message")
                .field("body_bytes", &body.len())
                .finish(),
            Self::DataMessage { message, .. } => formatter
                .debug_struct("DataMessage")
                .field("body_bytes", &message.bytes.len())
                .finish(),
            Self::AssuranceChanged { .. } => formatter.write_str("AssuranceChanged"),
            Self::Disconnected { reason } => formatter
                .debug_struct("Disconnected")
                .field("reason_bytes", &reason.len())
                .finish(),
        }
    }
}

/// Handle to a single Session — returned from `Client::call` and
/// `InboundEvent::IncomingSession`. Carries the operations a single
/// user app needs: accept / reject / end / hold / resume / mute /
/// DTMF / streams / events.
pub struct SessionHandle {
    session_id: SessionId,
    conversation_id: ConversationId,
    /// UCTP wire Connection identifier established by an outbound
    /// `connection.offer`. Inbound handles remain `None` until their accept
    /// path establishes a concrete substrate Connection.
    connection_id: Option<ConnectionId>,
    // `inner` holds per-substrate state. The dispatch lives behind a
    // dyn-trait so the SessionHandle's surface is substrate-agnostic.
    // For v1 this is a stub; concrete impls come from the per-
    // protocol crates as consumers exercise them.
    inner: Arc<RwLock<SessionInner>>,
}

struct SessionInner {
    #[allow(dead_code)] // call-hold state: written, not yet read
    held: bool,
    transport: SessionTransport,
    participant_id: String,
}

#[derive(Clone)]
enum SessionTransport {
    #[cfg(feature = "uctp")]
    UctpQuic {
        client: Arc<rvoip_quic::UctpQuicClient>,
    },
    Unsupported,
}

impl Default for SessionInner {
    fn default() -> Self {
        Self {
            held: false,
            transport: SessionTransport::Unsupported,
            participant_id: String::new(),
        }
    }
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("session_present", &!self.session_id.as_str().is_empty())
            .field(
                "conversation_present",
                &!self.conversation_id.as_str().is_empty(),
            )
            .field("connection_present", &self.connection_id.is_some())
            .finish_non_exhaustive()
    }
}

impl SessionHandle {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
    pub fn connection_id(&self) -> Option<&ConnectionId> {
        self.connection_id.as_ref()
    }
    pub async fn accept(self) -> Result<Self> {
        // v1 surface — per-substrate dispatch lands as consumers
        // exercise the path.
        Ok(self)
    }
    pub async fn reject(self, _reason: &str) -> Result<()> {
        Err(ClientError::NotImplemented("SessionHandle::reject"))
    }
    pub async fn end(&self) -> Result<()> {
        let inner = self.inner.read().await;
        match &inner.transport {
            #[cfg(feature = "uctp")]
            SessionTransport::UctpQuic { client } => {
                let payload = rvoip_uctp::payloads::session::SessionEnd {
                    by: inner.participant_id.clone(),
                    reason_code: 200,
                    reason: "normal-clearing".into(),
                };
                let env = rvoip_uctp::envelope::UctpEnvelope::new(
                    rvoip_uctp::types::MessageType::SessionEnd,
                    serde_json::to_value(payload)
                        .map_err(|e| ClientError::Protocol(e.to_string()))?,
                )
                .with_sid(self.session_id.to_string());
                client
                    .send(env)
                    .await
                    .map_err(|e| ClientError::ConnectFailed(e.to_string()))
            }
            SessionTransport::Unsupported => Err(ClientError::NotImplemented("SessionHandle::end")),
        }
    }
    pub async fn hold(&self) -> Result<()> {
        Err(ClientError::NotImplemented("SessionHandle::hold"))
    }
    pub async fn resume(&self) -> Result<()> {
        Err(ClientError::NotImplemented("SessionHandle::resume"))
    }
    pub async fn mute(&self) -> Result<()> {
        Err(ClientError::NotImplemented("SessionHandle::mute"))
    }
    pub async fn send_dtmf(&self, _digits: &str) -> Result<()> {
        Err(ClientError::NotImplemented("SessionHandle::send_dtmf"))
    }

    /// Send application data over this Session's established Connection.
    ///
    /// Outbound UCTP calls establish the wire Connection during
    /// [`Client::call`], so callers do not have to manually coordinate
    /// Conversation, Session, and Connection identifiers.
    pub async fn send_data_message(&self, message: DataMessage) -> Result<MessageId> {
        let connection_id = self
            .connection_id
            .as_ref()
            .ok_or(ClientError::NotImplemented(
                "SessionHandle::send_data_message requires an established Connection",
            ))?;
        let inner = self.inner.read().await;
        match &inner.transport {
            #[cfg(feature = "uctp")]
            SessionTransport::UctpQuic { client } => {
                send_uctp_data_message(
                    client,
                    &inner.participant_id,
                    connection_id,
                    &self.conversation_id,
                    Some(&self.session_id),
                    message,
                )
                .await
            }
            SessionTransport::Unsupported => Err(ClientError::NotImplemented(
                "SessionHandle::send_data_message",
            )),
        }
    }
}

/// The single-Identity client. Holds the chosen substrate's
/// connection plus a private inbound event channel.
pub struct Client {
    server_uri: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
    inbound_rx: tokio::sync::Mutex<Option<mpsc::Receiver<InboundEvent>>>,
    inner: ClientInner,
}

enum ClientInner {
    #[cfg(feature = "uctp")]
    UctpQuic {
        client: Arc<rvoip_quic::UctpQuicClient>,
        identity_id: String,
        participant_id: String,
    },
}

impl Client {
    /// Authenticate and open a substrate connection. v1 supports
    /// `uctp+quic://host:port`; SIP/WebRTC client dispatch is future work.
    pub async fn connect(server_uri: &str, credential: Credential) -> Result<Self> {
        Self::connect_with_options(server_uri, credential, ClientOptions::default()).await
    }

    /// Authenticate and open a substrate connection with explicit client
    /// options, primarily for pinned/self-signed QUIC TLS in tests and dev.
    pub async fn connect_with_options(
        server_uri: &str,
        credential: Credential,
        options: ClientOptions,
    ) -> Result<Self> {
        let scheme = server_uri.split("://").next().unwrap_or("");
        match scheme {
            "uctp+quic" => {
                #[cfg(not(feature = "uctp"))]
                {
                    return Err(ClientError::UnsupportedScheme(scheme.into()));
                }
                #[cfg(feature = "uctp")]
                {
                    let endpoint = match options.quic_endpoint.clone() {
                        Some(endpoint) => endpoint,
                        None => default_quic_endpoint()?,
                    };
                    let (server_addr, default_server_name) =
                        resolve_uctp_quic_uri(server_uri).await?;
                    let server_name = options.server_name.clone().unwrap_or(default_server_name);
                    let tls = options
                        .quic_client_config
                        .clone()
                        .unwrap_or_else(|| Arc::new(default_tls_config()));
                    let client = rvoip_quic::UctpQuicClient::connect(
                        &endpoint,
                        server_addr,
                        &server_name,
                        tls,
                    )
                    .await
                    .map_err(|e| ClientError::ConnectFailed(e.to_string()))?;
                    let mut wire_rx = client.take_inbound().ok_or_else(|| {
                        ClientError::Protocol("client inbound already taken".into())
                    })?;

                    run_bearer_handshake(&client, &mut wire_rx, &credential, &options).await?;
                    let auth_session = wait_for_auth_session(&mut wire_rx)
                        .await
                        .map_err(ClientError::Protocol)?;
                    let participant_id = options
                        .participant_id
                        .clone()
                        .unwrap_or_else(|| auth_session.participant_id.clone());

                    let (tx, rx) = mpsc::channel(64);
                    spawn_uctp_event_pump(
                        Arc::clone(&client),
                        wire_rx,
                        tx.clone(),
                        participant_id.clone(),
                    );
                    Ok(Self {
                        server_uri: server_uri.into(),
                        inbound_tx: tx,
                        inbound_rx: tokio::sync::Mutex::new(Some(rx)),
                        inner: ClientInner::UctpQuic {
                            client,
                            identity_id: auth_session.identity_id,
                            participant_id,
                        },
                    })
                }
            }
            "sip" | "wss" | "https" => Err(ClientError::NotImplemented(
                "rvoip-client only supports uctp+quic in this milestone",
            )),
            other => Err(ClientError::UnsupportedScheme(other.into())),
        }
    }

    pub fn server_uri(&self) -> &str {
        &self.server_uri
    }

    pub fn identity_id(&self) -> &str {
        match &self.inner {
            #[cfg(feature = "uctp")]
            ClientInner::UctpQuic { identity_id, .. } => identity_id,
        }
    }

    pub fn participant_id(&self) -> &str {
        match &self.inner {
            #[cfg(feature = "uctp")]
            ClientInner::UctpQuic { participant_id, .. } => participant_id,
        }
    }

    /// Outbound: place a Session against `target`. v1 supports voice
    /// over the UCTP QUIC signaling path only.
    pub async fn call(&self, target: CallTarget, medium: SessionMedium) -> Result<SessionHandle> {
        if !matches!(medium, SessionMedium::Voice) {
            return Err(ClientError::NotImplemented(
                "rvoip-client UCTP milestone supports SessionMedium::Voice only",
            ));
        }
        match &self.inner {
            #[cfg(feature = "uctp")]
            ClientInner::UctpQuic {
                client,
                participant_id,
                ..
            } => {
                let to = match target {
                    CallTarget::Identity(id) | CallTarget::Participant(id) => id,
                    CallTarget::Uri(_) => {
                        return Err(ClientError::NotImplemented(
                            "URI targets require SIP/WebRTC client dispatch",
                        ))
                    }
                };
                let session_id = SessionId::new();
                let conversation_id = ConversationId::new();
                let connection_id = ConnectionId::new();
                let payload = rvoip_uctp::payloads::session::SessionInvite {
                    from: participant_id.clone(),
                    to: vec![to],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities_offer: serde_json::Value::Object(Default::default()),
                };
                let env = rvoip_uctp::envelope::UctpEnvelope::new(
                    rvoip_uctp::types::MessageType::SessionInvite,
                    serde_json::to_value(payload)
                        .map_err(|e| ClientError::Protocol(e.to_string()))?,
                )
                .with_cid(conversation_id.to_string())
                .with_sid(session_id.to_string());
                client
                    .send(env)
                    .await
                    .map_err(|e| ClientError::ConnectFailed(e.to_string()))?;

                // Establish a real UCTP Connection immediately after the
                // invite. The two envelopes share one ordered signaling
                // stream, so the remote coordinator observes the Session
                // before binding this wire connid to its core Connection.
                let offer = rvoip_uctp::payloads::connection::ConnectionOffer {
                    by_participant: participant_id.clone(),
                    substrate: "quic".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![rvoip_uctp::payloads::connection::StreamOffer {
                        id: rvoip_core_traits::ids::StreamId::new().to_string(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                };
                let offer = rvoip_uctp::envelope::UctpEnvelope::new(
                    rvoip_uctp::types::MessageType::ConnectionOffer,
                    serde_json::to_value(offer)
                        .map_err(|error| ClientError::Protocol(error.to_string()))?,
                )
                .with_cid(conversation_id.to_string())
                .with_sid(session_id.to_string())
                .with_connid(connection_id.to_string());
                client
                    .send(offer)
                    .await
                    .map_err(|error| ClientError::ConnectFailed(error.to_string()))?;
                Ok(SessionHandle {
                    session_id,
                    conversation_id,
                    connection_id: Some(connection_id),
                    inner: Arc::new(RwLock::new(SessionInner {
                        held: false,
                        transport: SessionTransport::UctpQuic {
                            client: Arc::clone(client),
                        },
                        participant_id: participant_id.clone(),
                    })),
                })
            }
        }
    }

    /// Send a Message in a Conversation. v1 returns
    /// `NotImplemented` until messaging is wired through the chosen
    /// substrate.
    pub async fn send_message(&self, _cid: ConversationId, _body: &str) -> Result<MessageId> {
        Err(ClientError::NotImplemented("Client::send_message"))
    }

    pub async fn send_data_message(
        &self,
        connection_id: ConnectionId,
        conversation_id: ConversationId,
        message: DataMessage,
    ) -> Result<MessageId> {
        match &self.inner {
            #[cfg(feature = "uctp")]
            ClientInner::UctpQuic {
                client,
                participant_id,
                ..
            } => {
                send_uctp_data_message(
                    client,
                    participant_id,
                    &connection_id,
                    &conversation_id,
                    None,
                    message,
                )
                .await
            }
            #[cfg(not(feature = "uctp"))]
            _ => Err(ClientError::NotImplemented(
                "Client::send_data_message requires a data-capable substrate feature",
            )),
        }
    }

    /// Subscribe to inbound events. Consumer awaits on the returned
    /// receiver. Can only be called once per `Client`.
    pub fn incoming(&self) -> Option<mpsc::Receiver<InboundEvent>> {
        self.inbound_rx.try_lock().ok().and_then(|mut g| g.take())
    }

    /// Push an inbound event into the channel — used by the
    /// per-substrate background task that translates wire envelopes
    /// into `InboundEvent`s.
    #[doc(hidden)]
    pub async fn deliver(&self, event: InboundEvent) {
        let _ = self.inbound_tx.send(event).await;
    }

    /// Graceful close — drains the inbound channel and tears down
    /// the substrate connection. v1 stub.
    pub async fn close(self) -> Result<()> {
        drop(self.inbound_tx);
        Ok(())
    }
}

#[cfg(feature = "uctp")]
fn default_quic_endpoint() -> Result<Arc<quinn::Endpoint>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| ClientError::ConnectFailed(format!("bind UDP endpoint: {e}")))?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| ClientError::ConnectFailed(format!("create QUIC endpoint: {e}")))?;
    Ok(Arc::new(endpoint))
}

#[cfg(feature = "uctp")]
fn default_tls_config() -> rustls::ClientConfig {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

#[cfg(feature = "uctp")]
async fn resolve_uctp_quic_uri(uri: &str) -> Result<(SocketAddr, String)> {
    let authority = uri
        .strip_prefix("uctp+quic://")
        .ok_or_else(|| ClientError::InvalidUri(uri.into()))?
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if authority.is_empty() {
        return Err(ClientError::InvalidUri("missing host".into()));
    }

    let (host, port) = parse_host_port(authority)?;
    let lookup_host = host.clone();
    let mut addrs = tokio::net::lookup_host((lookup_host.as_str(), port))
        .await
        .map_err(|e| ClientError::ConnectFailed(format!("resolve {host}:{port}: {e}")))?;
    let addr = addrs
        .next()
        .ok_or_else(|| ClientError::ConnectFailed(format!("no addresses for {host}:{port}")))?;
    Ok((addr, host))
}

#[cfg(feature = "uctp")]
fn parse_host_port(authority: &str) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| ClientError::InvalidUri(authority.into()))?;
        let port = tail
            .strip_prefix(':')
            .map(str::parse)
            .transpose()
            .map_err(|_| ClientError::InvalidUri(format!("invalid port in {authority}")))?
            .unwrap_or(4433);
        return Ok((host.into(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Ok((
            host.into(),
            port.parse()
                .map_err(|_| ClientError::InvalidUri(format!("invalid port in {authority}")))?,
        )),
        _ => Ok((authority.into(), 4433)),
    }
}

#[cfg(feature = "uctp")]
async fn run_bearer_handshake(
    client: &Arc<rvoip_quic::UctpQuicClient>,
    wire_rx: &mut mpsc::Receiver<rvoip_uctp::envelope::UctpEnvelope>,
    credential: &Credential,
    options: &ClientOptions,
) -> Result<()> {
    let token = match credential {
        Credential::Bearer(token) => token.clone(),
        Credential::OAuth2Dpop { .. } => {
            return Err(ClientError::NotImplemented(
                "OAuth2 DPoP client handshake is not wired yet",
            ))
        }
    };
    let hello = rvoip_uctp::payloads::auth::AuthHello {
        device: rvoip_uctp::payloads::auth::Device {
            id: options
                .device_id
                .clone()
                .unwrap_or_else(|| "dev_rvoip_client".into()),
            kind: "desktop".into(),
            platform: "rvoip-client".into(),
            sdk_version: env!("CARGO_PKG_VERSION").into(),
        },
        auth_methods: vec!["bearer".into()],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    client
        .send(rvoip_uctp::envelope::UctpEnvelope::new(
            rvoip_uctp::types::MessageType::AuthHello,
            serde_json::to_value(hello).map_err(|e| ClientError::Protocol(e.to_string()))?,
        ))
        .await
        .map_err(|e| ClientError::ConnectFailed(e.to_string()))?;

    let challenge = wait_for_message(wire_rx, rvoip_uctp::types::MessageType::AuthChallenge)
        .await
        .map_err(ClientError::Protocol)?;
    let response = rvoip_uctp::payloads::auth::AuthResponse {
        method: "bearer".into(),
        credential: token,
        actor_token: None,
    };
    client
        .send(
            rvoip_uctp::envelope::UctpEnvelope::new(
                rvoip_uctp::types::MessageType::AuthResponse,
                serde_json::to_value(response).map_err(|e| ClientError::Protocol(e.to_string()))?,
            )
            .with_in_reply_to(challenge.id),
        )
        .await
        .map_err(|e| ClientError::ConnectFailed(e.to_string()))?;
    Ok(())
}

#[cfg(feature = "uctp")]
async fn wait_for_auth_session(
    wire_rx: &mut mpsc::Receiver<rvoip_uctp::envelope::UctpEnvelope>,
) -> std::result::Result<rvoip_uctp::payloads::auth::AuthSession, String> {
    let env = wait_for_message(wire_rx, rvoip_uctp::types::MessageType::AuthSession).await?;
    env.decode_payload()
        .map_err(|e| format!("decode auth.session: {e}"))
}

#[cfg(feature = "uctp")]
async fn wait_for_message(
    wire_rx: &mut mpsc::Receiver<rvoip_uctp::envelope::UctpEnvelope>,
    msg_type: rvoip_uctp::types::MessageType,
) -> std::result::Result<rvoip_uctp::envelope::UctpEnvelope, String> {
    loop {
        let env = tokio::time::timeout(std::time::Duration::from_secs(5), wire_rx.recv())
            .await
            .map_err(|_| format!("timed out waiting for {msg_type:?}"))?
            .ok_or_else(|| format!("connection closed waiting for {msg_type:?}"))?;
        if env.msg_type == rvoip_uctp::types::MessageType::Error {
            return Err("server returned a protocol error".into());
        }
        if env.msg_type == msg_type {
            return Ok(env);
        }
    }
}

#[cfg(feature = "uctp")]
async fn send_uctp_data_message(
    client: &Arc<rvoip_quic::UctpQuicClient>,
    participant_id: &str,
    connection_id: &ConnectionId,
    conversation_id: &ConversationId,
    session_id: Option<&SessionId>,
    message: DataMessage,
) -> Result<MessageId> {
    message
        .validate()
        .map_err(|error| ClientError::Protocol(format!("invalid data message: {error}")))?;
    let message_id = message.message_id.clone();
    let payload = rvoip_uctp::payloads::message::MessageSend::from_data_message(
        &message,
        participant_id,
        serde_json::json!("all"),
    )
    .map_err(|error| ClientError::Protocol(error.to_string()))?;
    let mut envelope = rvoip_uctp::envelope::UctpEnvelope::new(
        rvoip_uctp::types::MessageType::MessageSend,
        serde_json::to_value(payload).map_err(|error| ClientError::Protocol(error.to_string()))?,
    )
    .with_cid(conversation_id.to_string())
    .with_connid(connection_id.to_string());
    if let Some(session_id) = session_id {
        envelope = envelope.with_sid(session_id.to_string());
    }
    client
        .send(envelope)
        .await
        .map_err(|error| ClientError::ConnectFailed(error.to_string()))?;
    Ok(message_id)
}

#[cfg(feature = "uctp")]
fn spawn_uctp_event_pump(
    client: Arc<rvoip_quic::UctpQuicClient>,
    mut wire_rx: mpsc::Receiver<rvoip_uctp::envelope::UctpEnvelope>,
    inbound_tx: mpsc::Sender<InboundEvent>,
    participant_id: String,
) {
    tokio::spawn(async move {
        while let Some(env) = wire_rx.recv().await {
            match env.msg_type {
                rvoip_uctp::types::MessageType::SessionInvite => {
                    let session_id = env
                        .sid
                        .clone()
                        .map(SessionId::from_string)
                        .unwrap_or_else(SessionId::new);
                    let conversation_id = env
                        .cid
                        .clone()
                        .map(ConversationId::from_string)
                        .unwrap_or_else(ConversationId::new);
                    let handle = SessionHandle {
                        session_id,
                        conversation_id,
                        connection_id: env.connid.clone().map(ConnectionId::from_string),
                        inner: Arc::new(RwLock::new(SessionInner {
                            held: false,
                            transport: SessionTransport::UctpQuic {
                                client: Arc::clone(&client),
                            },
                            participant_id: participant_id.clone(),
                        })),
                    };
                    let _ = inbound_tx.send(InboundEvent::IncomingSession(handle)).await;
                }
                rvoip_uctp::types::MessageType::SessionEnd
                | rvoip_uctp::types::MessageType::SessionEnded => {
                    let _ = inbound_tx
                        .send(InboundEvent::Disconnected {
                            reason: "session ended".into(),
                        })
                        .await;
                }
                rvoip_uctp::types::MessageType::MessageSend => {
                    let Ok(payload) =
                        env.decode_payload::<rvoip_uctp::payloads::message::MessageSend>()
                    else {
                        continue;
                    };
                    let Ok(message) = payload.to_data_message() else {
                        continue;
                    };
                    let Some(connection_id) = env.connid.map(ConnectionId::from_string) else {
                        continue;
                    };
                    let _ = inbound_tx
                        .send(InboundEvent::DataMessage {
                            connection_id,
                            message,
                        })
                        .await;
                }
                _ => {}
            }
        }
        let _ = inbound_tx
            .send(InboundEvent::Disconnected {
                reason: "transport closed".into(),
            })
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[cfg(feature = "uctp")]
    const ALPN_UCTP: &[u8] = b"uctp/1";

    #[cfg(feature = "uctp")]
    fn install_crypto_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[cfg(feature = "uctp")]
    fn server_endpoint(
        addr: SocketAddr,
    ) -> (
        Arc<quinn::Endpoint>,
        rustls::pki_types::CertificateDer<'static>,
    ) {
        let (cert_der, key_der) =
            rvoip_uctp::substrate::self_signed_for_dev(&["localhost".into()]).expect("self_signed");
        let mut tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der)
            .expect("server tls");
        tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];

        let endpoint = rvoip_uctp::substrate::make_server_endpoint(
            addr,
            Arc::new(tls),
            quinn::TransportConfig::default(),
        )
        .expect("endpoint");
        (Arc::new(endpoint), cert_der)
    }

    #[cfg(feature = "uctp")]
    fn client_endpoint() -> Arc<quinn::Endpoint> {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        Arc::new(
            quinn::Endpoint::new(
                quinn::EndpointConfig::default(),
                None,
                socket,
                Arc::new(quinn::TokioRuntime),
            )
            .expect("client endpoint"),
        )
    }

    #[cfg(feature = "uctp")]
    async fn loopback_client() -> (Client, mpsc::Receiver<rvoip_quic::AdapterEvent>) {
        use rvoip_quic::ConnectionAdapter;

        install_crypto_provider();
        let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
        let server_addr = server_ep.local_addr().expect("local_addr");

        let mut routes =
            rvoip_uctp::substrate::dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP])
                .expect("dispatcher");
        let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

        let cfg =
            rvoip_quic::UctpQuicConfig::new(server_ep, accept_rx, rvoip_auth_core::bearer_stub());
        let adapter = rvoip_quic::UctpQuicAdapter::new(cfg)
            .await
            .expect("adapter");
        let events = adapter.subscribe_events();

        let client_cfg =
            rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");
        let client = Client::connect_with_options(
            &format!("uctp+quic://{server_addr}"),
            Credential::Bearer("test-token".into()),
            ClientOptions {
                quic_endpoint: Some(client_endpoint()),
                quic_client_config: Some(Arc::new(client_cfg)),
                server_name: Some("localhost".into()),
                ..ClientOptions::default()
            },
        )
        .await
        .expect("connect");
        (client, events)
    }

    #[cfg(feature = "uctp")]
    async fn loopback_orchestrator_client() -> (Client, Arc<rvoip_core::Orchestrator>) {
        install_crypto_provider();
        let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
        let server_addr = server_ep.local_addr().expect("local_addr");
        let mut routes =
            rvoip_uctp::substrate::dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP])
                .expect("dispatcher");
        let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");
        let adapter = rvoip_quic::UctpQuicAdapter::new(rvoip_quic::UctpQuicConfig::new(
            server_ep,
            accept_rx,
            rvoip_auth_core::bearer_stub(),
        ))
        .await
        .expect("adapter");
        let orchestrator = rvoip_core::Orchestrator::new(rvoip_core::Config::default());
        orchestrator.register(adapter).expect("register adapter");

        let client_cfg =
            rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");
        let client = Client::connect_with_options(
            &format!("uctp+quic://{server_addr}"),
            Credential::Bearer("test-token".into()),
            ClientOptions {
                quic_endpoint: Some(client_endpoint()),
                quic_client_config: Some(Arc::new(client_cfg)),
                server_name: Some("localhost".into()),
                ..ClientOptions::default()
            },
        )
        .await
        .expect("connect");
        (client, orchestrator)
    }

    #[tokio::test]
    async fn connect_unknown_scheme_errors() {
        let result = Client::connect("ftp://example.com", Credential::Bearer("test".into())).await;
        assert!(matches!(result, Err(ClientError::UnsupportedScheme(_))));
    }

    #[cfg(feature = "uctp")]
    #[tokio::test]
    async fn incoming_can_be_taken_once_after_real_connect() {
        let (client, _events) = loopback_client().await;
        assert!(client.incoming().is_some());
        assert!(client.incoming().is_none(), "second take should be None");
    }

    #[cfg(feature = "uctp")]
    #[tokio::test]
    async fn deliver_routes_to_incoming() {
        let (client, _events) = loopback_client().await;
        let mut rx = client.incoming().unwrap();
        client
            .deliver(InboundEvent::Disconnected {
                reason: "test".into(),
            })
            .await;
        let event = rx.recv().await.expect("event");
        assert!(matches!(event, InboundEvent::Disconnected { .. }));
    }

    #[cfg(feature = "uctp")]
    #[tokio::test]
    async fn uctp_quic_call_and_end_send_wire_events() {
        let (client, mut events) = loopback_client().await;
        let session = client
            .call(
                CallTarget::Participant("part_bob".into()),
                SessionMedium::Voice,
            )
            .await
            .expect("call");

        use rvoip_quic::AdapterEvent;
        let inbound = loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .expect("event timeout")
                .expect("event channel closed");
            if let AdapterEvent::InboundConnection { connection } = event {
                break connection;
            }
        };
        let canonical_session_id = inbound.session_id.to_string();
        let wire_session_id = session.session_id().to_string();
        assert_ne!(canonical_session_id, wire_session_id);
        assert!(canonical_session_id.ends_with(&format!(":{wire_session_id}")));

        session.end().await.expect("end");
    }

    #[cfg(feature = "uctp")]
    #[tokio::test]
    async fn data_message_api_preserves_binary_payload_and_rejects_unsupported_reliability() {
        let (client, mut events) = loopback_client().await;
        let session = client
            .call(
                CallTarget::Participant("part_bob".into()),
                SessionMedium::Voice,
            )
            .await
            .expect("call");
        let wire_connection_id = session
            .connection_id()
            .expect("outbound UCTP call has a wire connid")
            .clone();
        let core_connection_id = loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .expect("event timeout")
                .expect("event channel closed");
            if let rvoip_quic::AdapterEvent::InboundConnection { connection } = event {
                break connection.id;
            }
        };
        assert_ne!(core_connection_id, wire_connection_id);

        let message = DataMessage::reliable(
            "bridgefu.context.v1",
            "application/octet-stream",
            vec![0, 0xff, 7, 42],
        );
        let message_id = message.message_id.clone();
        assert_eq!(
            session
                .send_data_message(message.clone())
                .await
                .expect("send data message"),
            message_id
        );

        let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.expect("adapter event channel closed");
                if let rvoip_quic::AdapterEvent::DataMessage {
                    connection_id: received_connection,
                    message: received_message,
                } = event
                {
                    if received_connection == core_connection_id {
                        return received_message;
                    }
                }
            }
        })
        .await
        .expect("data-message adapter event timeout");
        assert_eq!(received, message);

        let mut unsupported = DataMessage::reliable("chat", "text/plain", "hello");
        unsupported.reliability = rvoip_core_traits::DataReliability::ReliableUnordered;
        assert!(matches!(
            session.send_data_message(unsupported).await,
            Err(ClientError::Protocol(_))
        ));
    }

    #[cfg(feature = "uctp")]
    #[tokio::test]
    async fn session_data_message_roundtrips_through_authenticated_orchestrator_route() {
        let (client, orchestrator) = loopback_orchestrator_client().await;
        let expected_subject = client.identity_id().to_string();
        let mut events = orchestrator.subscribe_events();
        let session = client
            .call(
                CallTarget::Participant("part_bob".into()),
                SessionMedium::Voice,
            )
            .await
            .expect("call");
        let wire_connection_id = session
            .connection_id()
            .expect("outbound UCTP call has a wire connid")
            .clone();

        let core_connection_id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(rvoip_core::Event::ConnectionInbound { connection_id, .. }) =
                    events.recv().await
                {
                    break connection_id;
                }
            }
        })
        .await
        .expect("inbound core Connection timeout");
        assert_ne!(core_connection_id, wire_connection_id);

        let message = DataMessage::reliable(
            "bridgefu.context.v1",
            "application/octet-stream",
            vec![0, 0xff, 7, 42],
        );
        session
            .send_data_message(message.clone())
            .await
            .expect("session data message");

        let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(rvoip_core::Event::DataMessageReceived {
                    connection_id,
                    message,
                    ..
                }) = events.recv().await
                {
                    if connection_id == core_connection_id {
                        break message;
                    }
                }
            }
        })
        .await
        .expect("orchestrator DataMessage timeout");
        assert_eq!(received, message);
        assert_eq!(
            orchestrator
                .connection_principal(&core_connection_id)
                .expect("retained route principal")
                .subject,
            expected_subject
        );
    }
}
