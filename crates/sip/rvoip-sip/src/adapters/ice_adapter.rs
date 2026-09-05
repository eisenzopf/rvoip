//! ICE wiring between SDP negotiation and the sans-io agent.
//!
//! The agent lives in `rvoip-ice-core` and knows nothing about SDP or
//! sockets. This module is the io and signaling glue: it extracts the
//! peer's ICE material from parsed SDP, contributes ours to offers and
//! answers, and runs one pump task per media session that shuttles STUN
//! datagrams between the agent and the RTP socket's event bus. When the
//! agent nominates a pair, the pump retargets the media flow — the same
//! `establish_media_flow` the SDP path uses, so ICE refines the destination
//! rather than inventing a parallel one.

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use rvoip_ice_core::{
    AgentConfig, Candidate, CandidateKind, Credentials, IceAgent, IceEvent, IceRole,
};
use rvoip_media_core::relay::controller::MediaSessionController;
use rvoip_media_core::DialogId;
use rvoip_rtp_core::traits::RtpEvent;
use rvoip_rtp_core::transport::RtpTransport;
use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};
use tokio::sync::mpsc;

use crate::state_table::types::SessionId;

/// Whether and how this endpoint runs ICE (RFC 8445).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SipIcePolicy {
    /// No ICE: today's behavior, byte for byte.
    #[default]
    Disabled,
    /// RFC 8445 §2.5 ice-lite: answer checks, never send them. Correct only
    /// on a genuinely reachable (public or 1:1-NAT-advertised) address.
    Lite,
    /// The full agent: gather, check, nominate.
    Full,
}

/// The peer's ICE material, as its SDP declared it.
#[derive(Clone, Debug)]
pub struct RemoteIce {
    /// Peer credentials.
    pub credentials: Credentials,
    /// Peer candidates (component 1, UDP).
    pub candidates: Vec<Candidate>,
    /// Whether the peer declared `a=ice-lite`.
    pub lite: bool,
    /// RFC 8839 ice-mismatch: the default `c=`/`m=` destination is not among
    /// the candidates, which means a middlebox rewrote the SDP after the
    /// peer built it. ICE must stand down for this call — the rewritten
    /// default is the only address the middlebox will relay.
    pub mismatch: bool,
}

/// Our ICE material for one session's SDP.
#[derive(Clone, Debug)]
pub struct IceMaterial {
    /// Our ufrag.
    pub ufrag: String,
    /// Our password.
    pub pwd: String,
    /// Whether we are lite (drives `a=ice-lite`).
    pub lite: bool,
    /// Candidates to write as `a=candidate` lines.
    pub candidates: Vec<Candidate>,
}

enum IceCommand {
    SetRemote {
        credentials: Credentials,
        candidates: Vec<Candidate>,
        remote_lite: bool,
    },
}

struct IceRuntime {
    commands: mpsc::Sender<IceCommand>,
    material: IceMaterial,
    task: tokio::task::AbortHandle,
}

/// One ICE runtime per media session, owned by the media adapter.
#[derive(Default)]
pub(crate) struct IceRuntimes {
    runtimes: DashMap<SessionId, IceRuntime>,
}

impl std::fmt::Debug for IceRuntimes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IceRuntimes")
            .field("active", &self.runtimes.len())
            .finish()
    }
}

impl IceRuntimes {
    /// Create (or return) the session's ICE runtime and the material its
    /// SDP should carry. Returns `None` — with a warning — when the media
    /// transport is not reachable, in which case the SDP simply goes out
    /// without ICE and the call proceeds on the classic path.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn ensure_local(
        &self,
        session_id: &SessionId,
        dialog_id: &DialogId,
        controller: &Arc<MediaSessionController>,
        policy: SipIcePolicy,
        offerer: bool,
        host_addr: SocketAddr,
        public_addr: Option<SocketAddr>,
    ) -> Option<IceMaterial> {
        if let Some(existing) = self.runtimes.get(session_id) {
            return Some(existing.material.clone());
        }
        let Some(transport) = controller.ice_transport(dialog_id).await else {
            tracing::warn!(
                session = %session_id.0,
                "ICE requested but the media transport is not reachable; offering without ICE"
            );
            return None;
        };

        let credentials = Credentials::generate();
        let lite = policy == SipIcePolicy::Lite;
        let mut candidates = Vec::new();
        if lite {
            // Lite has exactly one candidate: the reachable address. On a
            // gateway behind 1:1 NAT that is the advertised address, carried
            // as a host candidate — the lite deployment shape.
            let reachable = public_addr.unwrap_or(host_addr);
            candidates.push(Candidate::host(reachable, 1, 65_535));
        } else {
            candidates.push(Candidate::host(host_addr, 1, 65_535));
            if let Some(public) = public_addr {
                if public != host_addr {
                    candidates.push(Candidate::server_reflexive(
                        public,
                        host_addr,
                        // The discovery server is not retained down here;
                        // the foundation only needs to distinguish bases.
                        SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 3478),
                        1,
                        65_534,
                    ));
                }
            }
        }

        let config = if lite {
            AgentConfig::lite(credentials.clone())
        } else {
            AgentConfig::full(
                if offerer {
                    IceRole::Controlling
                } else {
                    IceRole::Controlled
                },
                credentials.clone(),
            )
        };
        let mut agent = IceAgent::new(config);
        for candidate in &candidates {
            agent.add_local_candidate(candidate.clone());
        }

        let material = IceMaterial {
            ufrag: credentials.ufrag,
            pwd: credentials.pwd,
            lite,
            candidates,
        };
        let (commands_tx, commands_rx) = mpsc::channel(8);
        let task = tokio::spawn(run_ice_pump(
            session_id.0.clone(),
            dialog_id.clone(),
            Arc::clone(controller),
            transport,
            agent,
            commands_rx,
        ));
        self.runtimes.insert(
            session_id.clone(),
            IceRuntime {
                commands: commands_tx,
                material: material.clone(),
                task: task.abort_handle(),
            },
        );
        Some(material)
    }

    /// Hand the peer's material to the session's agent. Returns whether a
    /// runtime existed to receive it.
    pub(crate) async fn apply_remote(&self, session_id: &SessionId, remote: RemoteIce) -> bool {
        let Some(runtime) = self.runtimes.get(session_id) else {
            return false;
        };
        runtime
            .commands
            .send(IceCommand::SetRemote {
                credentials: remote.credentials,
                candidates: remote.candidates,
                remote_lite: remote.lite,
            })
            .await
            .is_ok()
    }

    /// Whether a runtime exists for this session.
    pub(crate) fn is_active(&self, session_id: &SessionId) -> bool {
        self.runtimes.contains_key(session_id)
    }

    /// Tear a session's runtime down (peer declined ICE, or the session
    /// ended). Idempotent.
    pub(crate) fn stop(&self, session_id: &SessionId) {
        if let Some((_, runtime)) = self.runtimes.remove(session_id) {
            runtime.task.abort();
        }
    }
}

/// The per-session pump: agent on one side, RTP socket on the other.
async fn run_ice_pump(
    session_label: String,
    dialog_id: DialogId,
    controller: Arc<MediaSessionController>,
    transport: Arc<dyn RtpTransport>,
    mut agent: IceAgent,
    mut commands: mpsc::Receiver<IceCommand>,
) {
    let mut stun_events = transport.subscribe();
    loop {
        agent.handle_timeout(Instant::now());
        while let Some(transmit) = agent.poll_transmit() {
            if let Err(error) = transport
                .send_stun_bytes(&transmit.payload, transmit.to)
                .await
            {
                tracing::trace!(session = %session_label, %error, "ICE check send failed");
            }
        }
        while let Some(event) = agent.poll_event() {
            match event {
                IceEvent::Selected { local_base, remote } => {
                    tracing::info!(
                        session = %session_label,
                        %local_base,
                        %remote,
                        "ICE nominated a pair; retargeting media"
                    );
                    if let Err(error) = controller.establish_media_flow(&dialog_id, remote).await {
                        tracing::warn!(
                            session = %session_label,
                            %error,
                            "media retarget after ICE nomination failed"
                        );
                    }
                    // Post-nomination re-INVITE (RFC 8839 §4.4) when the
                    // nominated pair differs from the SDP default is the
                    // coordinator's follow-up; the media path itself is
                    // already correct from this retarget.
                }
                IceEvent::ConsentExpired => {
                    tracing::warn!(
                        session = %session_label,
                        "ICE consent expired: the peer stopped answering; media may be black-holed"
                    );
                }
                IceEvent::StateChanged(state) => {
                    tracing::debug!(session = %session_label, ?state, "ICE state");
                    if state == rvoip_ice_core::IceState::Failed {
                        tracing::warn!(
                            session = %session_label,
                            "ICE failed on every pair; call continues on the SDP default path"
                        );
                    }
                }
                IceEvent::PairValidated { local_base, remote } => {
                    tracing::debug!(session = %session_label, %local_base, %remote, "ICE pair validated");
                }
                IceEvent::RoleChanged(role) => {
                    tracing::debug!(session = %session_label, ?role, "ICE role repaired");
                }
            }
        }
        let deadline = agent
            .poll_timeout(Instant::now())
            .unwrap_or_else(|| Instant::now() + Duration::from_millis(500));
        tokio::select! {
            command = commands.recv() => match command {
                Some(IceCommand::SetRemote { credentials, candidates, remote_lite }) => {
                    if remote_lite {
                        // RFC 8445 §6.1.1: full beside lite is controlling,
                        // regardless of who offered.
                        agent.set_role(IceRole::Controlling);
                    }
                    agent.set_remote_credentials(credentials);
                    for candidate in candidates {
                        agent.add_remote_candidate(candidate);
                    }
                }
                None => break,
            },
            event = stun_events.recv() => match event {
                Ok(RtpEvent::StunPacket { local, source, payload }) => {
                    let _ = agent.handle_packet(Instant::now(), local, source, &payload);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::debug!(session = %session_label, skipped, "ICE pump lagged the RTP event bus");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            () = tokio::time::sleep_until(deadline.into()) => {}
        }
    }
    tracing::debug!(session = %session_label, "ICE pump ended");
}

/// Extract the peer's ICE material from parsed SDP, or `None` when the peer
/// did not offer ICE at all.
pub(crate) fn extract_remote_ice(sdp: &SdpSession) -> Option<RemoteIce> {
    let mut ufrag = None;
    let mut pwd = None;
    let mut lite = false;
    let mut candidates = Vec::new();
    let mut default_addr: Option<SocketAddr> = None;

    let mut scan = |attributes: &[ParsedAttribute]| {
        for attribute in attributes {
            match attribute {
                ParsedAttribute::IceUfrag(value) => ufrag = Some(value.clone()),
                ParsedAttribute::IcePwd(value) => pwd = Some(value.clone()),
                ParsedAttribute::IceLite => lite = true,
                ParsedAttribute::Candidate(candidate) => {
                    if candidate.component_id != 1
                        || !candidate.transport.eq_ignore_ascii_case("udp")
                    {
                        continue;
                    }
                    let Ok(ip) = IpAddr::from_str(&candidate.connection_address) else {
                        continue;
                    };
                    let addr = SocketAddr::new(ip, candidate.port);
                    let Some(kind) = CandidateKind::from_sdp_type(&candidate.candidate_type) else {
                        continue;
                    };
                    candidates.push(Candidate {
                        foundation: candidate.foundation.clone(),
                        component: 1,
                        priority: candidate.priority,
                        addr,
                        kind,
                        base: addr,
                        related: None,
                    });
                }
                _ => {}
            }
        }
    };

    scan(&sdp.generic_attributes);
    for media in &sdp.media_descriptions {
        scan(&media.generic_attributes);
        if default_addr.is_none() {
            let connection = media
                .connection_info
                .as_ref()
                .or(sdp.connection_info.as_ref());
            if let Some(connection) = connection {
                if let Ok(ip) = IpAddr::from_str(&connection.connection_address) {
                    default_addr = Some(SocketAddr::new(ip, media.port));
                }
            }
        }
    }

    let credentials = Credentials {
        ufrag: ufrag?,
        pwd: pwd?,
    };
    // RFC 8839 §4.3: a default destination missing from the candidate set
    // means something between the peers rewrote the SDP. Detect, do not run.
    let mismatch = match default_addr {
        Some(default) if !candidates.is_empty() => {
            !candidates.iter().any(|candidate| candidate.addr == default)
        }
        _ => false,
    };
    Some(RemoteIce {
        credentials,
        candidates,
        lite,
        mismatch,
    })
}

/// Format one candidate as the value of an `a=candidate:` line (RFC 8839).
pub(crate) fn format_candidate(candidate: &Candidate) -> String {
    let mut line = format!(
        "{} {} UDP {} {} {} typ {}",
        candidate.foundation,
        candidate.component,
        candidate.priority,
        candidate.addr.ip(),
        candidate.addr.port(),
        candidate.kind.sdp_type(),
    );
    if let Some(related) = candidate.related {
        line.push_str(&format!(" raddr {} rport {}", related.ip(), related.port()));
    }
    line
}

#[cfg(test)]
mod tests {

    /// The io chain end to end, over real sockets: a full ICE peer on a raw
    /// UDP socket traverses to our lite runtime, whose checks flow through
    /// the real `UdpRtpTransport` — STUN demuxed off the media port onto the
    /// event bus, into the pump, answered by the agent, and sent back
    /// through `send_stun_bytes`. The peer completing proves every link.
    #[tokio::test]
    async fn the_pump_answers_a_full_peer_over_real_sockets() {
        use rvoip_ice_core::{AgentConfig, IceAgent, IceRole, IceState};
        use rvoip_media_core::relay::controller::MediaConfig;

        let controller = Arc::new(MediaSessionController::new());
        let dialog = DialogId::new("ice-pump-io-test");
        controller
            .start_media(
                dialog.clone(),
                MediaConfig {
                    local_addr: "127.0.0.1:0".parse().unwrap(),
                    remote_addr: None,
                    preferred_codec: None,
                    parameters: Default::default(),
                },
            )
            .await
            .expect("start media");
        let info = controller
            .get_session_info(&dialog)
            .await
            .expect("session info");
        let our_addr = SocketAddr::new(
            "127.0.0.1".parse().unwrap(),
            info.rtp_port.expect("rtp port allocated"),
        );

        let runtimes = IceRuntimes::default();
        let session_id = SessionId("ice-pump-io-test".into());
        let material = runtimes
            .ensure_local(
                &session_id,
                &dialog,
                &controller,
                SipIcePolicy::Lite,
                false,
                our_addr,
                None,
            )
            .await
            .expect("lite material");
        assert!(material.lite);
        assert_eq!(material.candidates.len(), 1);

        // The peer: a full controlling agent driven by hand on a raw socket.
        let peer_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer_socket.local_addr().unwrap();
        let mut peer = IceAgent::new(AgentConfig::full(
            IceRole::Controlling,
            Credentials::generate(),
        ));
        peer.add_local_candidate(Candidate::host(peer_addr, 1, 65_535));
        peer.set_remote_credentials(Credentials {
            ufrag: material.ufrag.clone(),
            pwd: material.pwd.clone(),
        });
        for candidate in &material.candidates {
            peer.add_remote_candidate(candidate.clone());
        }

        assert!(
            runtimes
                .apply_remote(
                    &session_id,
                    RemoteIce {
                        credentials: peer.local_credentials().clone(),
                        candidates: vec![Candidate::host(peer_addr, 1, 65_535)],
                        lite: false,
                        mismatch: false,
                    },
                )
                .await
        );

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut buffer = [0_u8; 1500];
        while tokio::time::Instant::now() < deadline && peer.state() != IceState::Completed {
            peer.handle_timeout(Instant::now());
            while let Some(transmit) = peer.poll_transmit() {
                peer_socket
                    .send_to(&transmit.payload, transmit.to)
                    .await
                    .unwrap();
            }
            while peer.poll_event().is_some() {}
            if let Ok(Ok((size, from))) = tokio::time::timeout(
                Duration::from_millis(20),
                peer_socket.recv_from(&mut buffer),
            )
            .await
            {
                let _ = peer.handle_packet(Instant::now(), peer_addr, from, &buffer[..size]);
            }
        }
        assert_eq!(
            peer.state(),
            IceState::Completed,
            "the full peer must complete against our lite pump over real sockets"
        );
        let (_, selected_remote) = peer.selected_pair().expect("peer selected");
        assert_eq!(
            selected_remote, our_addr,
            "media lands on our real RTP port"
        );
        runtimes.stop(&session_id);
    }
    use super::*;

    fn parse(sdp: &str) -> SdpSession {
        SdpSession::from_str(sdp).expect("test SDP parses")
    }

    const BASE: &str =
        "v=0\r\no=- 1 1 IN IP4 198.51.100.7\r\ns=-\r\nc=IN IP4 198.51.100.7\r\nt=0 0\r\n";

    #[test]
    fn a_peer_without_ice_yields_none() {
        let sdp = parse(&format!("{BASE}m=audio 5004 RTP/AVP 0\r\n"));
        assert!(extract_remote_ice(&sdp).is_none());
    }

    #[test]
    fn ice_material_is_extracted_with_candidates() {
        let sdp = parse(&format!(
            "{BASE}m=audio 5004 RTP/AVP 0\r\n\
             a=ice-ufrag:8hhY\r\na=ice-pwd:asd88fgpdd777uzjYhagZg1x\r\n\
             a=candidate:1 1 UDP 2130706431 198.51.100.7 5004 typ host\r\n\
             a=candidate:2 1 UDP 1694498815 203.0.113.9 40000 typ srflx raddr 10.0.0.2 rport 5004\r\n"
        ));
        let remote = extract_remote_ice(&sdp).expect("ice present");
        assert_eq!(remote.credentials.ufrag, "8hhY");
        assert_eq!(remote.candidates.len(), 2);
        assert!(!remote.lite);
        assert!(!remote.mismatch, "default 198.51.100.7:5004 is a candidate");
    }

    #[test]
    fn a_rewritten_default_is_flagged_as_mismatch() {
        // The default destination is not among the candidates: a middlebox
        // rewrote c=/m= after the peer built its SDP.
        let sdp = parse(&format!(
            "{BASE}m=audio 6000 RTP/AVP 0\r\n\
             a=ice-ufrag:8hhY\r\na=ice-pwd:asd88fgpdd777uzjYhagZg1x\r\n\
             a=candidate:1 1 UDP 2130706431 192.0.2.55 5004 typ host\r\n"
        ));
        let remote = extract_remote_ice(&sdp).expect("ice present");
        assert!(remote.mismatch);
    }

    #[test]
    fn lite_and_tcp_candidates_are_handled() {
        let sdp = parse(&format!(
            "{BASE}a=ice-lite\r\nm=audio 5004 RTP/AVP 0\r\n\
             a=ice-ufrag:srvr\r\na=ice-pwd:server-password-of-22char\r\n\
             a=candidate:1 1 UDP 2130706431 198.51.100.7 5004 typ host\r\n\
             a=candidate:9 1 TCP 1000000 198.51.100.7 5004 typ host\r\n\
             a=candidate:8 2 UDP 1000001 198.51.100.7 5005 typ host\r\n"
        ));
        let remote = extract_remote_ice(&sdp).expect("ice present");
        assert!(remote.lite);
        assert_eq!(
            remote.candidates.len(),
            1,
            "TCP and component-2 candidates are outside v1 scope and skipped"
        );
    }

    #[test]
    fn candidate_lines_roundtrip_through_our_own_parser() {
        let srflx = Candidate::server_reflexive(
            "203.0.113.9:40000".parse().unwrap(),
            "10.0.0.2:5004".parse().unwrap(),
            "192.0.2.250:3478".parse().unwrap(),
            1,
            65_534,
        );
        let line = format_candidate(&srflx);
        let sdp = parse(&format!(
            "{BASE}m=audio 40000 RTP/AVP 0\r\n\
             a=ice-ufrag:aaaa\r\na=ice-pwd:password-that-is-22-chars\r\n\
             a=candidate:{line}\r\nc=IN IP4 203.0.113.9\r\n"
        ));
        let remote = extract_remote_ice(&sdp).expect("ice present");
        assert_eq!(remote.candidates.len(), 1);
        assert_eq!(remote.candidates[0].addr, srflx.addr);
        assert_eq!(remote.candidates[0].kind, CandidateKind::ServerReflexive);
    }
}
