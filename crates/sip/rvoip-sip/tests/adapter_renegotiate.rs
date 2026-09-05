//! Gap plan §4.2C v1 punch list — `SipAdapter::renegotiate_media`.
//!
//! Coverage:
//!
//! 1. **Empty capabilities** returns `RvoipError::UnsupportedCodec`
//!    without touching the SIP layer. The orchestrator should never
//!    drive a re-INVITE with no codec choices.
//!
//! 2. **Unknown connection** returns `RvoipError::ConnectionNotFound`
//!    (same shape as the other adapter methods — hold/resume/dtmf).
//!
//! 3. A real localhost UAS proves the adapter waits for the final response,
//!    returns the codec actually committed from the answer, and updates the
//!    live stream descriptor only after that commit.
//!
//! 4. A rejected re-INVITE returns an error, preserves the stable codec, and
//!    releases transaction ownership so a later accepted re-INVITE succeeds.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::error::RvoipError;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;

const UAS_TAG: &str = "renegotiate-uas";

struct RenegotiateUas {
    addr: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
    _media_sink: UdpSocket,
}

impl Drop for RenegotiateUas {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn sdp_response(
    request: &Request,
    addr: std::net::SocketAddr,
    media_port: u16,
    payload_type: u8,
) -> Response {
    let mut response = create_response(request, StatusCode::Ok);
    if let Some(TypedHeader::To(to)) = response
        .headers
        .iter_mut()
        .find(|header| matches!(header, TypedHeader::To(_)))
    {
        to.set_tag(UAS_TAG);
    }
    response.headers.push(TypedHeader::Contact(
        Contact::from_str(&format!("<sip:renegotiate@{addr}>")).expect("renegotiate UAS Contact"),
    ));
    response.headers.push(TypedHeader::ContentType(
        rvoip_sip_core::types::ContentType::sdp(),
    ));
    let (name, clock) = match payload_type {
        0 => ("PCMU", 8_000),
        8 => ("PCMA", 8_000),
        other => panic!("unsupported test payload type {other}"),
    };
    response.body = Bytes::from(format!(
        "v=0\r\no=renegotiate 1 1 IN IP4 127.0.0.1\r\ns=renegotiate\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP {payload_type} 101\r\na=rtpmap:{payload_type} {name}/{clock}\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n"
    ));
    response
        .headers
        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
    response.headers.push(TypedHeader::ContentLength(
        rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
    ));
    response
}

async fn boot_renegotiate_uas(reject_first_reinvite: bool) -> RenegotiateUas {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let addr = socket.local_addr().expect("UAS address");
    let media_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("media sink bind");
    let media_port = media_sink.local_addr().expect("media sink address").port();
    let task_socket = Arc::clone(&socket);
    let task = tokio::spawn(async move {
        let mut packet = vec![0u8; 65_536];
        let mut invite_count = 0usize;
        loop {
            let (bytes, peer) = task_socket
                .recv_from(&mut packet)
                .await
                .expect("UAS receive");
            let Message::Request(request) =
                parse_message(&packet[..bytes]).expect("parse SIP request")
            else {
                continue;
            };
            let response = match request.method() {
                Method::Invite => {
                    invite_count += 1;
                    if invite_count == 2 && reject_first_reinvite {
                        create_response(&request, StatusCode::NotAcceptableHere)
                    } else {
                        let payload_type = if invite_count == 1 { 0 } else { 8 };
                        sdp_response(&request, addr, media_port, payload_type)
                    }
                }
                Method::Bye | Method::Cancel => create_response(&request, StatusCode::Ok),
                Method::Ack => continue,
                _ => continue,
            };
            task_socket
                .send_to(&Message::Response(response).to_bytes(), peer)
                .await
                .expect("send SIP response");
        }
    });
    RenegotiateUas {
        addr,
        task,
        _media_sink: media_sink,
    }
}

fn pcma_capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: vec![CodecInfo {
            name: "g.711-a".into(),
            clock_rate_hz: 8_000,
            channels: 1,
            fmtp: None,
            payload_type: Some(8),
        }],
        ..Default::default()
    }
}

async fn originate_dialog(adapter: &Arc<SipAdapter>, target: std::net::SocketAddr) -> ConnectionId {
    let prepared = ConnectionAdapter::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:renegotiate@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip),
    )
    .await
    .expect("prepare outbound dialog");
    let connection_id = prepared.connection.id.clone();
    tokio::time::timeout(
        Duration::from_secs(8),
        ConnectionAdapter::activate_outbound_with_receipt(adapter.as_ref(), connection_id.clone()),
    )
    .await
    .expect("activation deadline")
    .expect("activate outbound dialog");
    connection_id
}

async fn stream_codec(adapter: &SipAdapter, connection_id: &ConnectionId) -> CodecInfo {
    ConnectionAdapter::streams(adapter, connection_id.clone())
        .await
        .expect("live streams")
        .into_iter()
        .next()
        .expect("audio stream")
        .codec()
}

fn pick_free_udp_port() -> u16 {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral");
    sock.local_addr().expect("local_addr").port()
}

async fn fresh_adapter() -> Arc<SipAdapter> {
    let sip_port = pick_free_udp_port();
    let coord = UnifiedCoordinator::new(SipConfig::local("reneg-test", sip_port))
        .await
        .expect("sip coordinator");
    SipAdapter::new(Arc::clone(&coord))
        .await
        .expect("sip adapter")
}

#[tokio::test]
async fn renegotiate_media_rejects_empty_capabilities() {
    let sip = fresh_adapter().await;
    let caps = CapabilityDescriptor::default(); // empty audio_codecs
    let err =
        <SipAdapter as ConnectionAdapter>::renegotiate_media(&*sip, ConnectionId::new(), caps)
            .await
            .unwrap_err();
    assert!(
        matches!(err, RvoipError::UnsupportedCodec(_)),
        "empty capabilities must surface UnsupportedCodec; got {err:?}"
    );
}

#[tokio::test]
async fn renegotiate_media_returns_connection_not_found_for_unknown_conn() {
    let sip = fresh_adapter().await;
    let caps = CapabilityDescriptor {
        audio_codecs: vec![CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
            payload_type: None,
        }],
        ..Default::default()
    };
    let err =
        <SipAdapter as ConnectionAdapter>::renegotiate_media(&*sip, ConnectionId::new(), caps)
            .await
            .unwrap_err();
    assert!(
        matches!(err, RvoipError::ConnectionNotFound(_)),
        "unknown ConnectionId must surface ConnectionNotFound; got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renegotiate_media_returns_only_the_codec_committed_by_the_answer() {
    let uas = boot_renegotiate_uas(false).await;
    let sip = fresh_adapter().await;
    let connection_id = originate_dialog(&sip, uas.addr).await;
    assert_eq!(stream_codec(&sip, &connection_id).await.name, "g.711-mu");

    let negotiated = tokio::time::timeout(
        Duration::from_secs(8),
        ConnectionAdapter::renegotiate_media(
            sip.as_ref(),
            connection_id.clone(),
            pcma_capabilities(),
        ),
    )
    .await
    .expect("re-INVITE deadline")
    .expect("accepted re-INVITE");
    assert_eq!(negotiated.audio.expect("audio result").name, "g.711-a");
    assert_eq!(stream_codec(&sip, &connection_id).await.name, "g.711-a");

    ConnectionAdapter::end(sip.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("end dialog");
    sip.drain().await.expect("drain adapter");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_reinvite_preserves_stable_codec_and_allows_a_later_retry() {
    let uas = boot_renegotiate_uas(true).await;
    let sip = fresh_adapter().await;
    let connection_id = originate_dialog(&sip, uas.addr).await;
    assert_eq!(stream_codec(&sip, &connection_id).await.name, "g.711-mu");

    let rejected = tokio::time::timeout(
        Duration::from_secs(8),
        ConnectionAdapter::renegotiate_media(
            sip.as_ref(),
            connection_id.clone(),
            pcma_capabilities(),
        ),
    )
    .await
    .expect("rejected re-INVITE deadline")
    .expect_err("first re-INVITE must be rejected");
    assert!(matches!(rejected, RvoipError::AdmissionRejected(_)));
    assert_eq!(
        stream_codec(&sip, &connection_id).await.name,
        "g.711-mu",
        "a rejected offer must not replace stable media"
    );

    let retried = tokio::time::timeout(
        Duration::from_secs(8),
        ConnectionAdapter::renegotiate_media(
            sip.as_ref(),
            connection_id.clone(),
            pcma_capabilities(),
        ),
    )
    .await
    .expect("retry re-INVITE deadline")
    .expect("retry accepted after rollback");
    assert_eq!(retried.audio.expect("retry audio result").name, "g.711-a");
    assert_eq!(stream_codec(&sip, &connection_id).await.name, "g.711-a");

    ConnectionAdapter::end(sip.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("end dialog");
    sip.drain().await.expect("drain adapter");
}
