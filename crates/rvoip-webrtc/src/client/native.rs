//! Native WebRTC client surface (INTERFACE_DESIGN §15.3).

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::ids::{ConnectionId, SessionId};
use webrtc::peer_connection::{RTCIceCandidateInit, RTCSdpType, RTCSessionDescription};

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::peer::{PeerRole, RvoipPeerConnection};

/// Thin newtype over webrtc-rs SDP offer.
#[derive(Clone, Debug)]
pub struct Offer(pub String);

/// Thin newtype over webrtc-rs SDP answer.
#[derive(Clone, Debug)]
pub struct Answer(pub String);

/// Thin newtype over ICE candidate init JSON.
#[derive(Clone, Debug)]
pub struct IceCandidate(pub String);

/// Outbound call target (thin until `rvoip-client` exists).
#[derive(Clone, Debug)]
pub enum CallTarget {
    Uri(String),
    Participant(String),
}

/// Session medium — audio-only in v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionMedium {
    Audio,
}

/// Handle returned from [`WebRtcClient::call`].
#[derive(Clone)]
pub struct SessionHandle {
    session_id: SessionId,
    connection_id: ConnectionId,
    target: CallTarget,
    medium: SessionMedium,
    answer: Answer,
    peer: Arc<RvoipPeerConnection>,
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

    /// Wait until ICE/DTLS reaches connected.
    pub async fn wait_connected(&self, timeout: Duration) -> Result<()> {
        self.peer.wait_connected(timeout).await
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

    /// Place an outbound call: create offer, exchange via signaler, apply answer.
    pub async fn call<S: Signaler>(
        &self,
        signaler: &S,
        target: CallTarget,
        medium: SessionMedium,
    ) -> Result<SessionHandle> {
        if medium != SessionMedium::Audio {
            return Err(WebRtcError::NotImplemented("video calls (audio-only v1)"));
        }

        let offer_sdp = self.peer.create_offer_and_gather().await?;
        let answer = signaler.send_offer(&Offer(offer_sdp)).await?;
        self.peer.set_remote_answer(&answer.0).await?;

        Ok(SessionHandle {
            session_id: self.session_id.clone(),
            connection_id: self.connection_id.clone(),
            target,
            medium,
            answer,
            peer: Arc::clone(&self.peer),
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
