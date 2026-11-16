//! A SIP call that fails with a final non-2xx response reaches the core
//! operational event stream as `OperationalFailureReason::SipResponse`,
//! carrying the numeric status and the analytics-block flag only.

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::OriginateRequest;
use rvoip_core::operational_events::{OperationalEventKind, OperationalFailureReason};
use rvoip_core::{
    CapabilityDescriptor, Config as CoreConfig, ConversationPolicy, Direction, Orchestrator,
    ParticipantId, RvoipError, SessionMedium, TenantId, Transport,
};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;

const PEER_ONLY_TEXT: &str = "peer-only-diagnostic";

/// Rings, then rejects every INVITE with `status` and reason `phrase`.
async fn rejecting_uas(
    status: u16,
    phrase: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let addr = socket.local_addr().expect("UAS address");
    let task = tokio::spawn(async move {
        let mut packet = vec![0u8; 16_384];
        loop {
            let Ok((bytes, peer)) = socket.recv_from(&mut packet).await else {
                return;
            };
            let Ok(Message::Request(request)) = parse_message(&packet[..bytes]) else {
                continue;
            };
            if request.method() != Method::Invite {
                continue;
            }
            let tag = |response: &mut Response| {
                if let Some(TypedHeader::To(to)) = response
                    .headers
                    .iter_mut()
                    .find(|header| matches!(header, TypedHeader::To(_)))
                {
                    to.set_tag("rejecting-uas");
                }
            };
            let mut ringing = create_response(&request, StatusCode::Ringing);
            tag(&mut ringing);
            let _ = socket
                .send_to(&Message::Response(ringing).to_bytes(), peer)
                .await;
            // Let the originate call return before the call fails, so the
            // terminal is published on the operational stream.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut final_response = create_response(
                &request,
                StatusCode::from_u16(status).expect("valid status"),
            );
            final_response.reason = Some(phrase.to_string());
            tag(&mut final_response);
            let _ = socket
                .send_to(&Message::Response(final_response).to_bytes(), peer)
                .await;
        }
    });
    (addr, task)
}

async fn failure_reason_for(status: u16, phrase: &'static str) -> OperationalFailureReason {
    let (uas_addr, uas_task) = rejecting_uas(status, phrase).await;
    let coordinator = UnifiedCoordinator::new(SipConfig::local("operational-failure-uac", 0))
        .await
        .expect("coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut operational = orchestrator
        .install_operational_event_stream(16)
        .expect("operational stream");
    orchestrator
        .register(adapter)
        .expect("register SIP adapter");
    let conversation = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            Default::default(),
        )
        .await
        .expect("conversation");
    let session = orchestrator
        .start_session(conversation, SessionMedium::Voice, vec![])
        .await
        .expect("session");
    let prepared = orchestrator
        .prepare_outbound_connection(
            OriginateRequest::new(
                session,
                ParticipantId::new(),
                format!("sip:callee@{uas_addr}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepare outbound");
    let connection_id = prepared.connection_id().clone();
    // The rejection may land while activation is still in flight; the
    // terminal is published on the operational stream either way.
    let commit = tokio::spawn(prepared.commit());
    let reason = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = operational.recv().await.expect("operational stream open");
            if event.connection_id != connection_id {
                continue;
            }
            assert!(
                !format!("{event:?}").contains(PEER_ONLY_TEXT),
                "peer text must not reach the operational stream"
            );
            match event.kind {
                OperationalEventKind::Failed { reason } => return reason,
                OperationalEventKind::Ended { .. } | OperationalEventKind::Connected => {
                    panic!("rejected call must fail, got {:?}", event.kind)
                }
                _ => {}
            }
        }
    })
    .await
    .expect("operational Failed event");
    // The terminal was published before activation failed, so the caller
    // is told not to report a second failure.
    let committed = commit.await.expect("commit task");
    assert!(
        matches!(
            committed,
            Err(RvoipError::OutboundTerminalPublishedDuringActivation)
        ),
        "commit must report the already-published terminal, got {committed:?}"
    );
    uas_task.abort();
    reason
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_rejection_publishes_final_sip_status() {
    assert_eq!(
        failure_reason_for(486, "Busy Here peer-only-diagnostic").await,
        OperationalFailureReason::SipResponse {
            status_code: 486,
            network_blocked: false,
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decline_is_not_reported_as_network_blocked() {
    assert_eq!(
        failure_reason_for(603, "Decline").await,
        OperationalFailureReason::SipResponse {
            status_code: 603,
            network_blocked: false,
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn network_blocked_rejection_sets_the_block_flag() {
    assert_eq!(
        failure_reason_for(603, "Network Blocked").await,
        OperationalFailureReason::SipResponse {
            status_code: 603,
            network_blocked: true,
        }
    );
}
