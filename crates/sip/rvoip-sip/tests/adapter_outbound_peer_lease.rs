//! An outbound SIP call that depends on a live peer connection through an
//! `OutboundPeerLease`: it is never dispatched once the peer has gone, and it
//! is torn down on the wire when the peer goes after dispatch.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{EndReason, OriginateRequest};
use rvoip_core::operational_events::OperationalEventKind;
use rvoip_core::{
    CapabilityDescriptor, Config as CoreConfig, ConnectionId, ConversationPolicy, Direction,
    Orchestrator, ParticipantId, RvoipError, SessionMedium, TenantId, Transport,
};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::{SipAdapter, SipOriginateContext};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Answers every INVITE with PCMU and reports each request's method and
/// user part (`peer` or `callback`).
async fn answering_uas() -> (
    std::net::SocketAddr,
    mpsc::UnboundedReceiver<(Method, String)>,
    UdpSocket,
    tokio::task::JoinHandle<()>,
) {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let addr = socket.local_addr().expect("UAS address");
    let media = UdpSocket::bind("127.0.0.1:0").await.expect("media bind");
    let media_port = media.local_addr().expect("media address").port();
    let (seen_tx, seen_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut packet = vec![0u8; 16_384];
        loop {
            let Ok((bytes, peer)) = socket.recv_from(&mut packet).await else {
                return;
            };
            let Ok(Message::Request(request)) = parse_message(&packet[..bytes]) else {
                continue;
            };
            let user = request
                .to()
                .and_then(|to| to.uri().username().map(str::to_owned))
                .unwrap_or_default();
            let _ = seen_tx.send((request.method().clone(), user));
            let response = match request.method() {
                Method::Invite => {
                    let mut response = create_response(&request, StatusCode::Ok);
                    if let Some(TypedHeader::To(to)) = response
                        .headers
                        .iter_mut()
                        .find(|header| matches!(header, TypedHeader::To(_)))
                    {
                        to.set_tag("peer-lease-uas");
                    }
                    response.headers.push(TypedHeader::Contact(
                        Contact::from_str(&format!("<sip:uas@{addr}>")).expect("Contact"),
                    ));
                    response.headers.push(TypedHeader::ContentType(
                        rvoip_sip_core::types::ContentType::sdp(),
                    ));
                    response.body = bytes::Bytes::from(format!(
                        "v=0\r\no=uas 1 1 IN IP4 127.0.0.1\r\ns=lease\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n"
                    ));
                    response
                        .headers
                        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
                    response.headers.push(TypedHeader::ContentLength(
                        rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
                    ));
                    response
                }
                Method::Bye | Method::Cancel => create_response(&request, StatusCode::Ok),
                _ => continue,
            };
            let _ = socket
                .send_to(&Message::Response(response).to_bytes(), peer)
                .await;
        }
    });
    (addr, seen_rx, media, task)
}

struct Fixture {
    orchestrator: Arc<Orchestrator>,
    operational: mpsc::Receiver<rvoip_core::operational_events::OperationalEvent>,
    uas: std::net::SocketAddr,
    seen: mpsc::UnboundedReceiver<(Method, String)>,
    _media: UdpSocket,
    uas_task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let (uas, seen, media, uas_task) = answering_uas().await;
        let coordinator = UnifiedCoordinator::new(SipConfig::local("peer-lease-uac", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let orchestrator = Orchestrator::new(CoreConfig::default());
        let operational = orchestrator
            .install_operational_event_stream(64)
            .expect("operational stream");
        orchestrator
            .register(adapter)
            .expect("register SIP adapter");
        Self {
            orchestrator,
            operational,
            uas,
            seen,
            _media: media,
            uas_task,
        }
    }

    async fn call(
        &self,
        user: &str,
        context: SipOriginateContext,
    ) -> rvoip_core::Result<ConnectionId> {
        let conversation = self
            .orchestrator
            .open_conversation(
                TenantId::new(),
                ConversationPolicy::default(),
                Default::default(),
            )
            .await
            .expect("conversation");
        let session = self
            .orchestrator
            .start_session(conversation, SessionMedium::Voice, vec![])
            .await
            .expect("session");
        tokio::time::timeout(
            Duration::from_secs(10),
            self.orchestrator.originate_connection(
                OriginateRequest::new(
                    session,
                    ParticipantId::new(),
                    format!("sip:{user}@{}", self.uas),
                    Direction::Outbound,
                    CapabilityDescriptor::default(),
                )
                .with_transport(Transport::Sip)
                .with_context(context),
            ),
        )
        .await
        .expect("originate deadline")
        .map(|handle| handle.connection.id.clone())
    }

    async fn wait_connected(&mut self, connection: &ConnectionId) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = self.operational.recv().await.expect("operational stream");
                if &event.connection_id == connection
                    && matches!(event.kind, OperationalEventKind::Connected)
                {
                    return;
                }
            }
        })
        .await
        .expect("Connected event");
    }

    async fn wait_terminal(&mut self, connection: &ConnectionId) -> OperationalEventKind {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = self.operational.recv().await.expect("operational stream");
                if &event.connection_id == connection
                    && matches!(
                        event.kind,
                        OperationalEventKind::Ended { .. } | OperationalEventKind::Failed { .. }
                    )
                {
                    return event.kind;
                }
            }
        })
        .await
        .expect("terminal event")
    }

    fn drain_seen(&mut self) -> Vec<(Method, String)> {
        let mut seen = Vec::new();
        while let Ok(request) = self.seen.try_recv() {
            seen.push(request);
        }
        seen
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_is_not_dispatched_once_its_peer_has_ended() {
    let mut fixture = Fixture::new().await;
    let peer = fixture
        .call("peer", SipOriginateContext::new())
        .await
        .expect("peer call");
    fixture.wait_connected(&peer).await;
    let lease = fixture
        .orchestrator
        .reserve_outbound_peer(&peer)
        .expect("lease on the connected peer");
    fixture
        .orchestrator
        .end_connection(peer.clone(), EndReason::Normal)
        .await
        .expect("end peer");
    fixture.wait_terminal(&peer).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    fixture.drain_seen();

    let refused = fixture
        .call(
            "callback",
            SipOriginateContext::new().with_outbound_peer(lease),
        )
        .await;
    assert!(
        matches!(refused, Err(RvoipError::OutboundPeerUnavailableBeforeWrite)),
        "got {refused:?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        fixture
            .drain_seen()
            .iter()
            .all(|(method, user)| !(method == &Method::Invite && user == "callback")),
        "no INVITE may leave for a call whose peer has gone"
    );
    fixture.uas_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_is_hung_up_when_its_peer_ends_after_dispatch() {
    let mut fixture = Fixture::new().await;
    let peer = fixture
        .call("peer", SipOriginateContext::new())
        .await
        .expect("peer call");
    fixture.wait_connected(&peer).await;
    let lease = fixture
        .orchestrator
        .reserve_outbound_peer(&peer)
        .expect("lease on the connected peer");
    let callback = fixture
        .call(
            "callback",
            SipOriginateContext::new().with_outbound_peer(lease),
        )
        .await
        .expect("callback call");
    fixture.wait_connected(&callback).await;
    fixture.drain_seen();

    fixture
        .orchestrator
        .end_connection(peer.clone(), EndReason::Normal)
        .await
        .expect("end peer");
    let terminal = fixture.wait_terminal(&callback).await;
    assert!(
        matches!(terminal, OperationalEventKind::Failed { .. }),
        "callback must fail when its peer retires, got {terminal:?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let seen = fixture.drain_seen();
    assert!(
        seen.iter()
            .any(|(method, user)| method == &Method::Bye && user == "callback"),
        "the callback must be hung up on the wire, saw {seen:?}"
    );
    fixture.uas_task.abort();
}
