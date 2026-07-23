//! Thin wrapper over `webrtc_ice::agent::Agent` — a real ICE 1.2 (RFC 8445)
//! agent, same webrtc-rs "production" 0.17.x lineage already used
//! elsewhere in this workspace for DTLS and SRTP interop testing.
//!
//! This module only drives ICE gathering and connectivity checks and
//! hands back the negotiated remote address; it has no notion of SDP —
//! same split already used by `rvoip-rtp-core::dtls_srtp` for the
//! DTLS-SRTP handshake. The caller is responsible for carrying
//! [`IceAgent::local_credentials`] and gathered candidates into an
//! outgoing `a=ice-ufrag`/`a=ice-pwd`/`a=candidate` SDP, and for parsing
//! the peer's equivalent lines back into [`IceCandidate`] values to feed
//! into [`IceAgent::add_remote_candidate`].

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::sync::mpsc;
use webrtc_ice::agent::agent_config::AgentConfig;
use webrtc_ice::agent::Agent as InnerAgent;
use webrtc_ice::candidate::{candidate_base::unmarshal_candidate, Candidate, CandidateType};
use webrtc_ice::network_type::NetworkType;
use webrtc_ice::udp_mux::UDPMux;
use webrtc_ice::udp_network::UDPNetwork;
use webrtc_ice::url::Url;

use crate::bridge::{SharedIceMux, SharedIceSocket};
use crate::candidate::{CandidateKind, IceCandidate};
use crate::error::{Error, Result};

/// Which side of the ICE exchange this agent plays. Analogous to
/// `DtlsRole` in `rvoip-media-core` — the offerer is conventionally
/// controlling, the answerer controlled (RFC 8445 doesn't mandate this,
/// but it's the near-universal convention and keeps role resolution
/// simple, matching how `a=setup:actpass` resolves for DTLS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    Controlling,
    Controlled,
}

/// A real ICE agent for one media stream. Gathers host and
/// server-reflexive candidates (relay/TURN is out of scope for now) and
/// runs RFC 8445 connectivity checks against a remote peer's candidates.
pub struct IceAgent {
    inner: InnerAgent,
    role: IceRole,
    /// Set only when this agent was built via
    /// [`IceAgent::new_with_shared_socket`] — the caller's demux loop
    /// feeds inbound STUN datagrams in through
    /// [`IceAgent::handle_incoming_stun`], which forwards to this.
    mux: Option<Arc<SharedIceMux>>,
}

fn to_ice_candidate(c: &Arc<dyn Candidate + Send + Sync>) -> Result<IceCandidate> {
    let address: IpAddr = c
        .address()
        .parse()
        .map_err(|e| Error::InvalidCandidate(format!("unparseable candidate address: {e}")))?;
    let kind = match c.candidate_type() {
        CandidateType::Host => CandidateKind::Host,
        CandidateType::ServerReflexive => CandidateKind::ServerReflexive,
        CandidateType::PeerReflexive => CandidateKind::PeerReflexive,
        CandidateType::Relay => CandidateKind::Relay,
        CandidateType::Unspecified => {
            return Err(Error::InvalidCandidate(
                "candidate has an unspecified type".to_string(),
            ))
        }
    };
    let related = c.related_address();
    Ok(IceCandidate {
        foundation: c.foundation(),
        component: c.component() as u32,
        transport: "udp".to_string(),
        priority: c.priority(),
        address,
        port: c.port(),
        kind,
        related_address: related
            .as_ref()
            .and_then(|r| r.address.parse::<IpAddr>().ok()),
        related_port: related.as_ref().map(|r| r.port),
    })
}

fn parse_stun_servers(stun_servers: &[String]) -> Result<(Vec<Url>, Vec<CandidateType>)> {
    let urls = stun_servers
        .iter()
        .map(|s| Url::parse_url(s))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Agent(format!("invalid STUN server URL: {e}")))?;

    let mut candidate_types = vec![CandidateType::Host];
    if !urls.is_empty() {
        candidate_types.push(CandidateType::ServerReflexive);
    }
    Ok((urls, candidate_types))
}

impl IceAgent {
    /// Create a new agent. `stun_servers` are `stun:host:port` URLs (same
    /// convention as `rvoip-sip`'s `Config::stun_server`); an empty list
    /// means host candidates only (no server-reflexive gathering).
    ///
    /// Gathers host candidates over ephemeral sockets it binds itself.
    /// Use [`IceAgent::new_with_shared_socket`] to instead multiplex
    /// connectivity checks over a socket the caller already owns (e.g.
    /// `rvoip-rtp-core`'s shared RTP/RTCP/DTLS/STUN socket).
    pub async fn new(role: IceRole, stun_servers: &[String]) -> Result<Self> {
        Self::new_with_ip_filter(role, stun_servers, None).await
    }

    /// Same as [`IceAgent::new`], but restricts host candidate gathering
    /// to IP addresses the given filter accepts. Mainly useful for tests
    /// on multi-homed machines (restrict to loopback so the test doesn't
    /// depend on which of several real network interfaces the OS
    /// happens to prefer).
    pub async fn new_with_ip_filter(
        role: IceRole,
        stun_servers: &[String],
        ip_filter: Option<Arc<dyn Fn(std::net::IpAddr) -> bool + Send + Sync>>,
    ) -> Result<Self> {
        let (urls, candidate_types) = parse_stun_servers(stun_servers)?;

        let include_loopback = ip_filter.is_some();
        let ip_filter: webrtc_ice::agent::agent_config::IpFilterFn = match ip_filter {
            Some(f) => Box::new(move |ip| f(ip)),
            None => Box::new(|_| true),
        };

        let config = AgentConfig {
            urls,
            network_types: vec![NetworkType::Udp4],
            candidate_types,
            is_controlling: matches!(role, IceRole::Controlling),
            ip_filter: Arc::new(Some(ip_filter)),
            include_loopback,
            ..Default::default()
        };

        let inner = InnerAgent::new(config).await?;
        Ok(Self {
            inner,
            role,
            mux: None,
        })
    }

    /// Create a new agent that multiplexes its connectivity checks over a
    /// socket the caller already owns and reads from — e.g.
    /// `rvoip-rtp-core`'s shared RTP/RTCP/DTLS/STUN socket, via its own
    /// RFC 7983 demux — instead of binding dedicated ephemeral sockets.
    ///
    /// The caller must forward every inbound datagram it classifies as
    /// STUN to [`IceAgent::handle_incoming_stun`]; this agent never reads
    /// `socket` itself; it only writes through it.
    pub async fn new_with_shared_socket(
        role: IceRole,
        stun_servers: &[String],
        socket: Arc<dyn SharedIceSocket>,
    ) -> Result<Self> {
        Self::new_with_shared_socket_and_ip_filter(role, stun_servers, socket, None).await
    }

    /// Same as [`IceAgent::new_with_shared_socket`], but restricts host
    /// candidate gathering to IP addresses the given filter accepts —
    /// see [`IceAgent::new_with_ip_filter`] for why this matters on
    /// multi-homed machines (mainly tests).
    pub async fn new_with_shared_socket_and_ip_filter(
        role: IceRole,
        stun_servers: &[String],
        socket: Arc<dyn SharedIceSocket>,
        ip_filter: Option<Arc<dyn Fn(std::net::IpAddr) -> bool + Send + Sync>>,
    ) -> Result<Self> {
        let (urls, candidate_types) = parse_stun_servers(stun_servers)?;

        let mux = SharedIceMux::new(socket);

        let include_loopback = ip_filter.is_some();
        let ip_filter: webrtc_ice::agent::agent_config::IpFilterFn = match ip_filter {
            Some(f) => Box::new(move |ip| f(ip)),
            None => Box::new(|_| true),
        };

        let config = AgentConfig {
            urls,
            network_types: vec![NetworkType::Udp4],
            candidate_types,
            is_controlling: matches!(role, IceRole::Controlling),
            udp_network: UDPNetwork::Muxed(mux.clone() as Arc<dyn UDPMux + Send + Sync>),
            ip_filter: Arc::new(Some(ip_filter)),
            include_loopback,
            ..Default::default()
        };

        let inner = InnerAgent::new(config).await?;
        Ok(Self {
            inner,
            role,
            mux: Some(mux),
        })
    }

    /// Feed in one inbound datagram the caller's own demux loop has
    /// already classified as STUN. A no-op if this agent wasn't built via
    /// [`IceAgent::new_with_shared_socket`].
    pub async fn handle_incoming_stun(&self, buf: &[u8], addr: SocketAddr) {
        if let Some(mux) = &self.mux {
            mux.handle_incoming(buf, addr).await;
        }
    }

    /// Our local `(ufrag, pwd)` for the outgoing `a=ice-ufrag`/`a=ice-pwd`
    /// SDP lines.
    pub async fn local_credentials(&self) -> (String, String) {
        self.inner.get_local_user_credentials().await
    }

    /// Start gathering local candidates and wait for gathering to
    /// complete (non-trickle: this crate doesn't support incrementally
    /// sending candidates as they're found, only the full batch once
    /// done). Host + server-reflexive candidates typically resolve in
    /// well under a second, which is why this is safe to await before
    /// building the outgoing SDP offer/answer — unlike
    /// [`IceAgent::connect`], gathering never depends on the remote
    /// peer.
    pub async fn gather_candidates(&self) -> Result<Vec<IceCandidate>> {
        let (tx, mut rx) = mpsc::channel::<Option<Arc<dyn Candidate + Send + Sync>>>(32);
        self.inner.on_candidate(Box::new(move |c| {
            let tx = tx.clone();
            Box::pin(async move {
                let _ = tx.send(c).await;
            })
        }));

        self.inner.gather_candidates()?;

        let mut candidates = Vec::new();
        while let Some(maybe_candidate) = rx.recv().await {
            match maybe_candidate {
                Some(c) => candidates.push(to_ice_candidate(&c)?),
                None => break, // gathering complete
            }
        }

        if candidates.is_empty() {
            return Err(Error::NoCandidatesGathered);
        }
        Ok(candidates)
    }

    /// Feed in one of the peer's candidates (parsed from its SDP
    /// `a=candidate:` line).
    pub fn add_remote_candidate(&self, candidate: &IceCandidate) -> Result<()> {
        let parsed = unmarshal_candidate(&candidate.to_sdp_line())
            .map_err(|e| Error::InvalidCandidate(e.to_string()))?;
        let arc: Arc<dyn Candidate + Send + Sync> = Arc::new(parsed);
        self.inner.add_remote_candidate(&arc)?;
        Ok(())
    }

    /// Run connectivity checks against the peer's credentials and
    /// candidates already fed via [`IceAgent::add_remote_candidate`],
    /// blocking until a candidate pair is selected (or ICE fails).
    ///
    /// Must be run in the background (`tokio::spawn`), never awaited
    /// inline before the caller's own SDP offer/answer has actually been
    /// sent: the controlled side's checks can't succeed until the
    /// controlling side has started its own checks, which (for a fresh
    /// call) only happens once each side has both sent its own SDP and
    /// received the peer's — blocking either side's SDP send on this
    /// would deadlock the same way an early version of this session's
    /// DTLS-SRTP handshake wiring did.
    pub async fn connect(&self, remote_ufrag: String, remote_pwd: String) -> Result<SocketAddr> {
        let (_cancel_tx, cancel_rx) = mpsc::channel(1);
        // `dial`/`accept` each return a distinct opaque `impl Conn` type, so
        // they can't share one match-arm binding; we don't need the
        // returned Conn here (only the side effect of running checks to
        // completion), so just await each branch on its own.
        match self.role {
            IceRole::Controlling => {
                self.inner.dial(cancel_rx, remote_ufrag, remote_pwd).await?;
            }
            IceRole::Controlled => {
                self.inner
                    .accept(cancel_rx, remote_ufrag, remote_pwd)
                    .await?;
            }
        }
        let pair = self
            .inner
            .get_selected_candidate_pair()
            .ok_or(Error::ConnectFailed)?;
        Ok(pair.remote.addr())
    }

    pub async fn close(&self) -> Result<()> {
        self.inner.close().await?;
        Ok(())
    }
}
