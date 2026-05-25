//! Native WebRTC client surface (INTERFACE_DESIGN §15.3).

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::ids::{ConnectionId, SessionId};
use webrtc::data_channel::DataChannel;
use webrtc::peer_connection::{RTCIceCandidateInit, RTCSdpType, RTCSessionDescription};

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::peer::{PeerRole, RvoipPeerConnection};

/// Thin newtype over webrtc-rs SDP offer.
#[derive(Clone, Debug)]
pub struct Offer(pub String);

/// Thin newtype over webrtc-rs SDP answer.
#[derive(Clone, Debug)]
pub struct Answer {
    pub sdp: String,
    /// Server-side [`ConnectionId`] when signaling returns one (WebSocket answer).
    pub connection_id: Option<String>,
}

impl Answer {
    pub fn new(sdp: impl Into<String>) -> Self {
        Self {
            sdp: sdp.into(),
            connection_id: None,
        }
    }
}

/// Thin newtype over ICE candidate init JSON.
#[derive(Clone, Debug)]
pub struct IceCandidate(pub String);

/// Outbound call target (thin until `rvoip-client` exists).
#[derive(Clone, Debug)]
pub enum CallTarget {
    Uri(String),
    Participant(String),
}

/// Session medium.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionMedium {
    Audio,
    Video,
    AudioVideo,
}

/// Handle returned from [`WebRtcClient::call`].
///
/// Cloning a handle increments the refcount on the underlying peer connection;
/// the connection closes only when the last clone is dropped. For deterministic
/// teardown call [`SessionHandle::close`] explicitly — `Drop` is best-effort.
#[derive(Clone)]
pub struct SessionHandle {
    session_id: SessionId,
    connection_id: ConnectionId,
    target: CallTarget,
    medium: SessionMedium,
    answer: Answer,
    peer: Arc<RvoipPeerConnection>,
    data_channel: Arc<dyn DataChannel>,
    /// When all clones drop, the strong count hits 1 here and the Drop impl
    /// fires a detached close on the underlying peer.
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl SessionHandle {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    pub fn target(&self) -> &CallTarget {
        &self.target
    }

    pub fn medium(&self) -> SessionMedium {
        self.medium
    }

    pub fn answer(&self) -> &Answer {
        &self.answer
    }

    pub fn peer(&self) -> &Arc<RvoipPeerConnection> {
        &self.peer
    }

    pub fn data_channel(&self) -> &Arc<dyn DataChannel> {
        &self.data_channel
    }

    /// Wait until ICE/DTLS reaches connected.
    pub async fn wait_connected(&self, timeout: Duration) -> Result<()> {
        self.peer.wait_connected(timeout).await
    }

    /// Explicitly close the peer connection. Idempotent — subsequent calls
    /// (or `Drop`) are no-ops.
    pub async fn close(&self) -> Result<()> {
        if self
            .closed
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Ok(());
        }
        self.peer.close().await
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        // Only the last clone runs the actual close — `peer` Arc refcount tells
        // us if anyone else still holds the peer (the comprehensive checks
        // hand the peer around via `session.peer()`).
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        if Arc::strong_count(&self.peer) <= 1 {
            // Best-effort detached close — Drop is sync.
            let peer = Arc::clone(&self.peer);
            let closed = Arc::clone(&self.closed);
            tokio::spawn(async move {
                if !closed.swap(true, std::sync::atomic::Ordering::AcqRel) {
                    let _ = peer.close().await;
                }
            });
        }
    }
}

/// Signaling transport abstraction (WebSocket JSON, WHIP, custom).
#[async_trait::async_trait]
pub trait Signaler: Send + Sync {
    async fn send_offer(&self, offer: &Offer) -> Result<Answer>;
    async fn send_answer(&self, answer: &Answer) -> Result<()>;
    async fn send_ice(&self, candidate: &IceCandidate) -> Result<()>;
}

pub struct WebRtcClient {
    config: WebRtcConfig,
    signaler_uri: String,
    peer: Arc<RvoipPeerConnection>,
    session_id: SessionId,
    connection_id: ConnectionId,
}

impl WebRtcClient {
    /// Connect using WebRTC configuration and a signaling URI (used by custom signalers).
    pub async fn connect(config: WebRtcConfig, signaler_uri: impl Into<String>) -> Result<Arc<Self>> {
        let peer = RvoipPeerConnection::new(&config, PeerRole::Offerer).await?;
        Ok(Arc::new(Self {
            config,
            signaler_uri: signaler_uri.into(),
            peer,
            session_id: SessionId::new(),
            connection_id: ConnectionId::new(),
        }))
    }

    pub fn signaler_uri(&self) -> &str {
        &self.signaler_uri
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    pub fn capabilities(&self) -> CapabilityDescriptor {
        self.config.capabilities.clone()
    }

    /// Place an outbound call: add tracks + data channel, create offer, exchange via signaler.
    pub async fn call<S: Signaler>(
        &self,
        signaler: &S,
        target: CallTarget,
        medium: SessionMedium,
    ) -> Result<SessionHandle> {
        let data_channel =
            crate::client::comprehensive::prepare_offer_media(&self.peer, medium).await?;

        let offer_sdp = self.peer.create_offer_and_gather().await?;
        let answer = signaler.send_offer(&Offer(offer_sdp)).await?;
        self.peer.set_remote_answer(&answer.sdp).await?;

        Ok(SessionHandle {
            session_id: self.session_id.clone(),
            connection_id: self.connection_id.clone(),
            target,
            medium,
            answer,
            peer: Arc::clone(&self.peer),
            data_channel,
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub fn peer(&self) -> &Arc<RvoipPeerConnection> {
        &self.peer
    }

    pub fn parse_offer(sdp: &str) -> Result<RTCSessionDescription> {
        crate::sdp::parse_sdp(sdp, RTCSdpType::Offer)
    }

    pub fn parse_answer(sdp: &str) -> Result<RTCSessionDescription> {
        crate::sdp::parse_sdp(sdp, RTCSdpType::Answer)
    }

    pub fn parse_ice_candidate(json: &str) -> Result<RTCIceCandidateInit> {
        serde_json::from_str(json)
            .map_err(|e| WebRtcError::Signaling(format!("ice candidate json: {e}")))
    }
}
