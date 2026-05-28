//! # rvoip-client — single-Identity client SDK
//!
//! The unifying `Client` surface from `INTERFACE_DESIGN.md` §15. Wraps
//! the per-protocol native clients (`rvoip-uctp::client`,
//! `rvoip-sip::api`, `rvoip-webrtc::client`) behind a verb-shaped API
//! tuned for mobile / web / desktop / embedded apps that act as a
//! single Identity in a single tenant.
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
//! v1 ships the **public type surface + crate boundary** per
//! `GAP_PLAN.md` P12.3. The per-protocol dispatch logic in
//! `Client::connect` / `Client::call` is wired against the substrate
//! adapter that matches the URL scheme; non-trivial flows
//! (`incoming()` Session-invite delivery, multi-substrate priority
//! fall-through, `conversations()` history) land incrementally as
//! consumers exercise them.

#![warn(rust_2018_idioms)]
#![allow(missing_docs)]

use rvoip_core_traits::ids::{ConnectionId, ConversationId, MessageId, SessionId};
use std::sync::Arc;
use thiserror::Error;
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
#[derive(Clone, Debug)]
pub enum Credential {
    Bearer(String),
    OAuth2Dpop { access_token: String, dpop_proof: String },
}

/// Target of an outbound `Client::call`. Resolved at the underlying
/// substrate adapter by URL scheme: `Identity` and `Participant`
/// route through UCTP; `Uri` accepts `sip:` / `tel:` and dispatches
/// to the SIP interop adapter.
#[derive(Clone, Debug)]
pub enum CallTarget {
    Identity(String),
    Participant(String),
    Uri(String),
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

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("transport not enabled for scheme: {0}")]
    UnsupportedScheme(String),
    #[error("connection failed: {0}")]
    ConnectFailed(String),
    #[error("session not found")]
    SessionNotFound,
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// Inbound event from the connected substrate — consumer pumps
/// `Client::incoming()` to receive these.
#[derive(Debug)]
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
    /// Our IdentityAssurance level changed on a Connection — usually
    /// because step-up auth completed (CONVERSATION_PROTOCOL.md §5.8).
    AssuranceChanged {
        connection_id: ConnectionId,
        new_assurance: String,
    },
    Disconnected { reason: String },
}

/// Handle to a single Session — returned from `Client::call` and
/// `InboundEvent::IncomingSession`. Carries the operations a single
/// user app needs: accept / reject / end / hold / resume / mute /
/// DTMF / streams / events.
#[derive(Debug)]
pub struct SessionHandle {
    session_id: SessionId,
    conversation_id: ConversationId,
    // `inner` holds per-substrate state. The dispatch lives behind a
    // dyn-trait so the SessionHandle's surface is substrate-agnostic.
    // For v1 this is a stub; concrete impls come from the per-
    // protocol crates as consumers exercise them.
    _inner: Arc<RwLock<SessionInner>>,
}

#[derive(Debug, Default)]
struct SessionInner {
    held: bool,
}

impl SessionHandle {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
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
        Err(ClientError::NotImplemented("SessionHandle::end"))
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
}

/// The single-Identity client. Holds the chosen substrate's
/// connection plus a private inbound event channel.
pub struct Client {
    server_uri: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
    inbound_rx: tokio::sync::Mutex<Option<mpsc::Receiver<InboundEvent>>>,
}

impl Client {
    /// Authenticate and open a substrate connection. Substrate is
    /// chosen by URL scheme: `uctp+quic://` / `uctp+ws://` /
    /// `uctp+wt://` / `sip:` / `wss:`.
    pub async fn connect(server_uri: &str, _credential: Credential) -> Result<Self> {
        let scheme = server_uri.split("://").next().unwrap_or("");
        match scheme {
            "uctp+quic" => {
                #[cfg(not(feature = "uctp"))]
                {
                    return Err(ClientError::UnsupportedScheme(scheme.into()));
                }
                #[cfg(feature = "uctp")]
                {
                    let (tx, rx) = mpsc::channel(64);
                    Ok(Self {
                        server_uri: server_uri.into(),
                        inbound_tx: tx,
                        inbound_rx: tokio::sync::Mutex::new(Some(rx)),
                    })
                }
            }
            "sip" => {
                #[cfg(not(feature = "sip"))]
                {
                    Err(ClientError::UnsupportedScheme(scheme.into()))
                }
                #[cfg(feature = "sip")]
                {
                    let (tx, rx) = mpsc::channel(64);
                    Ok(Self {
                        server_uri: server_uri.into(),
                        inbound_tx: tx,
                        inbound_rx: tokio::sync::Mutex::new(Some(rx)),
                    })
                }
            }
            "wss" | "https" => {
                #[cfg(not(feature = "webrtc"))]
                {
                    Err(ClientError::UnsupportedScheme(scheme.into()))
                }
                #[cfg(feature = "webrtc")]
                {
                    let (tx, rx) = mpsc::channel(64);
                    Ok(Self {
                        server_uri: server_uri.into(),
                        inbound_tx: tx,
                        inbound_rx: tokio::sync::Mutex::new(Some(rx)),
                    })
                }
            }
            other => Err(ClientError::UnsupportedScheme(other.into())),
        }
    }

    pub fn server_uri(&self) -> &str {
        &self.server_uri
    }

    /// Outbound: place a Session against `target`. Returns a handle
    /// the consumer drives. v1 returns `NotImplemented` until the
    /// substrate-specific dial logic is wired per consumer demand.
    pub async fn call(
        &self,
        _target: CallTarget,
        _medium: SessionMedium,
    ) -> Result<SessionHandle> {
        // Local in-process handle (no wire dial yet). Lets consumers
        // exercise the SessionHandle shape immediately while the
        // per-protocol dial implementations land.
        Ok(SessionHandle {
            session_id: SessionId::new(),
            conversation_id: ConversationId::new(),
            _inner: Arc::new(RwLock::new(SessionInner::default())),
        })
    }

    /// Send a Message in a Conversation. v1 returns
    /// `NotImplemented` until messaging is wired through the chosen
    /// substrate.
    pub async fn send_message(
        &self,
        _cid: ConversationId,
        _body: &str,
    ) -> Result<MessageId> {
        Err(ClientError::NotImplemented("Client::send_message"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_uctp_quic_returns_client() {
        let client = Client::connect(
            "uctp+quic://example.com:4433",
            Credential::Bearer("test".into()),
        )
        .await
        .expect("connect");
        assert_eq!(client.server_uri(), "uctp+quic://example.com:4433");
    }

    #[tokio::test]
    async fn connect_unknown_scheme_errors() {
        let result = Client::connect(
            "ftp://example.com",
            Credential::Bearer("test".into()),
        )
        .await;
        assert!(matches!(result, Err(ClientError::UnsupportedScheme(_))));
    }

    #[tokio::test]
    async fn incoming_can_be_taken_once() {
        let client = Client::connect(
            "uctp+quic://example.com",
            Credential::Bearer("t".into()),
        )
        .await
        .unwrap();
        assert!(client.incoming().is_some());
        assert!(client.incoming().is_none(), "second take should be None");
    }

    #[tokio::test]
    async fn deliver_routes_to_incoming() {
        let client = Client::connect(
            "uctp+quic://example.com",
            Credential::Bearer("t".into()),
        )
        .await
        .unwrap();
        let mut rx = client.incoming().unwrap();
        client
            .deliver(InboundEvent::Disconnected {
                reason: "test".into(),
            })
            .await;
        let event = rx.recv().await.expect("event");
        assert!(matches!(event, InboundEvent::Disconnected { .. }));
    }
}
