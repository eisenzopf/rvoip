//! An answered SIP call publishes its committed audio selection on the core
//! operational event stream as `OperationalEventKind::MediaCommitted`.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::OriginateRequest;
use rvoip_core::operational_events::OperationalEventKind;
use rvoip_core::{
    CapabilityDescriptor, CommittedAudioMedia, Config as CoreConfig, ConversationPolicy, Direction,
    Orchestrator, ParticipantId, SessionMedium, TenantId, Transport,
};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;

/// Answers every INVITE with `200 OK` selecting PCMU on plain RTP/AVP.
async fn answering_uas() -> (std::net::SocketAddr, UdpSocket, tokio::task::JoinHandle<()>) {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let addr = socket.local_addr().expect("UAS address");
    // A separate RTP sink keeps media out of the SIP parser.
    let media = UdpSocket::bind("127.0.0.1:0").await.expect("media bind");
    let media_port = media.local_addr().expect("media address").port();
    let task = tokio::spawn(async move {
        let mut packet = vec![0u8; 16_384];
        loop {
            let Ok((bytes, peer)) = socket.recv_from(&mut packet).await else {
                return;
            };
            let Ok(Message::Request(request)) = parse_message(&packet[..bytes]) else {
                continue;
            };
            let response = match request.method() {
                Method::Invite => {
                    let mut response = create_response(&request, StatusCode::Ok);
                    if let Some(TypedHeader::To(to)) = response
                        .headers
                        .iter_mut()
                        .find(|header| matches!(header, TypedHeader::To(_)))
                    {
                        to.set_tag("committed-media-uas");
                    }
                    response.headers.push(TypedHeader::Contact(
                        Contact::from_str(&format!("<sip:uas@{addr}>")).expect("Contact"),
                    ));
                    response.headers.push(TypedHeader::ContentType(
                        rvoip_sip_core::types::ContentType::sdp(),
                    ));
                    response.body = bytes::Bytes::from(format!(
                        "v=0\r\no=uas 1 1 IN IP4 127.0.0.1\r\ns=committed\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n"
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
    (addr, media, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answered_call_publishes_committed_audio_media() {
    let (uas_addr, _media, uas_task) = answering_uas().await;
    let coordinator = UnifiedCoordinator::new(SipConfig::local("committed-media-uac", 0))
        .await
        .expect("coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut operational = orchestrator
        .install_operational_event_stream(64)
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
    let handle = tokio::time::timeout(
        Duration::from_secs(10),
        orchestrator.originate_connection(
            OriginateRequest::new(
                session,
                ParticipantId::new(),
                format!("sip:callee@{uas_addr}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        ),
    )
    .await
    .expect("originate deadline")
    .expect("originate");
    let connection_id = handle.connection.id.clone();

    let (connected_seen, media) = tokio::time::timeout(Duration::from_secs(10), async {
        let mut connected_seen = false;
        loop {
            let event = operational.recv().await.expect("operational stream open");
            if event.connection_id != connection_id {
                continue;
            }
            match event.kind {
                OperationalEventKind::Connected => connected_seen = true,
                OperationalEventKind::MediaCommitted { media } => {
                    return (connected_seen, media);
                }
                OperationalEventKind::Failed { .. } | OperationalEventKind::Ended { .. } => {
                    panic!("answered call ended before committing media")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("MediaCommitted event");

    assert!(connected_seen, "MediaCommitted follows Connected");
    assert!(media.valid());
    assert!(media.revision > 0);
    assert_eq!(
        media,
        CommittedAudioMedia::new(media.revision, "PCMU", 0, 8000, 1).with_media_profile("RTP/AVP")
    );

    let _ = orchestrator
        .end_connection(connection_id, rvoip_core::adapter::EndReason::Normal)
        .await;
    uas_task.abort();
}
