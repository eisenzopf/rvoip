//! `SipAdapter::hold` / `resume` against a real localhost UAS.
//!
//! 1. The result is the peer's answer: a rejected hold or resume re-INVITE is
//!    an error, and the call can be held and resumed afterwards.
//! 2. A committed resume restarts the inbound playout. A source that paused
//!    its RTP clock during the hold resumes with timestamps that continue from
//!    before the hold; its audio must play right away rather than be dropped
//!    as late behind a playout clock that ran on through the hold.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::error::RvoipError;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::stream::MediaFrame;
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::{PlayoutConfig, SipAdapter};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

const UAS_TAG: &str = "hold-uas";

struct HoldUas {
    addr: std::net::SocketAddr,
    media: Arc<UdpSocket>,
    /// Our RTP address, from the INVITE offer.
    local_rtp: Arc<Mutex<Option<std::net::SocketAddr>>>,
    reject_next_reinvite: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HoldUas {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn offered_rtp_addr(body: &str) -> Option<std::net::SocketAddr> {
    let ip = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("c=IN IP4 "))?;
    let port = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("m=audio "))?
        .split_whitespace()
        .next()?;
    format!("{ip}:{port}").parse().ok()
}

fn sdp_response(
    request: &Request,
    addr: std::net::SocketAddr,
    media_port: u16,
    direction: &str,
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
        Contact::from_str(&format!("<sip:hold@{addr}>")).expect("hold UAS Contact"),
    ));
    response.headers.push(TypedHeader::ContentType(
        rvoip_sip_core::types::ContentType::sdp(),
    ));
    let version = request.cseq().map(|cseq| cseq.sequence()).unwrap_or(1);
    response.body = Bytes::from(format!(
        "v=0\r\no=hold 1 {version} IN IP4 127.0.0.1\r\ns=hold\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na={direction}\r\n"
    ));
    response
        .headers
        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
    response.headers.push(TypedHeader::ContentLength(
        rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
    ));
    response
}

async fn boot_hold_uas() -> HoldUas {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let addr = socket.local_addr().expect("UAS address");
    let media = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("UAS media bind"),
    );
    let media_port = media.local_addr().expect("UAS media address").port();
    let local_rtp = Arc::new(Mutex::new(None));
    let reject_next_reinvite = Arc::new(AtomicBool::new(false));
    let task_socket = Arc::clone(&socket);
    let task_local_rtp = Arc::clone(&local_rtp);
    let task_reject = Arc::clone(&reject_next_reinvite);
    let task = tokio::spawn(async move {
        let mut packet = vec![0u8; 65_536];
        let mut invites = 0usize;
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
                    invites += 1;
                    let body = String::from_utf8_lossy(&request.body).into_owned();
                    if invites == 1 {
                        *task_local_rtp.lock().unwrap() = offered_rtp_addr(&body);
                    }
                    if invites > 1 && task_reject.swap(false, Ordering::AcqRel) {
                        create_response(&request, StatusCode::NotAcceptableHere)
                    } else {
                        // Answer a hold (sendonly) offer with recvonly.
                        let direction = if body.contains("a=sendonly") {
                            "recvonly"
                        } else {
                            "sendrecv"
                        };
                        sdp_response(&request, addr, media_port, direction)
                    }
                }
                Method::Bye | Method::Cancel => create_response(&request, StatusCode::Ok),
                _ => continue,
            };
            task_socket
                .send_to(&Message::Response(response).to_bytes(), peer)
                .await
                .expect("send SIP response");
        }
    });
    HoldUas {
        addr,
        media,
        local_rtp,
        reject_next_reinvite,
        task,
    }
}

fn pick_free_udp_port() -> u16 {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral");
    sock.local_addr().expect("local_addr").port()
}

async fn fresh_adapter(playout: Option<PlayoutConfig>) -> Arc<SipAdapter> {
    let mut config = SipConfig::local("hold-test", pick_free_udp_port());
    config.playout = playout;
    let coord = UnifiedCoordinator::new(config)
        .await
        .expect("sip coordinator");
    SipAdapter::new(Arc::clone(&coord))
        .await
        .expect("sip adapter")
}

async fn originate_dialog(adapter: &Arc<SipAdapter>, target: std::net::SocketAddr) -> ConnectionId {
    let prepared = ConnectionAdapter::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:hold@{target}"),
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

async fn control(
    adapter: &Arc<SipAdapter>,
    connection_id: &ConnectionId,
    hold: bool,
) -> std::result::Result<(), RvoipError> {
    let call = async {
        if hold {
            ConnectionAdapter::hold(adapter.as_ref(), connection_id.clone()).await
        } else {
            ConnectionAdapter::resume(adapter.as_ref(), connection_id.clone()).await
        }
    };
    tokio::time::timeout(Duration::from_secs(8), call)
        .await
        .expect("hold/resume deadline")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hold_and_resume_return_the_peers_answer() {
    let uas = boot_hold_uas().await;
    let sip = fresh_adapter(None).await;
    let connection_id = originate_dialog(&sip, uas.addr).await;

    uas.reject_next_reinvite.store(true, Ordering::Release);
    let rejected = control(&sip, &connection_id, true).await;
    assert!(
        matches!(rejected, Err(RvoipError::AdmissionRejected(_))),
        "a hold the peer rejected must not report success: {rejected:?}"
    );
    // The call is still active and can be held and resumed.
    control(&sip, &connection_id, true)
        .await
        .expect("accepted hold");
    uas.reject_next_reinvite.store(true, Ordering::Release);
    let rejected = control(&sip, &connection_id, false).await;
    assert!(
        matches!(rejected, Err(RvoipError::AdmissionRejected(_))),
        "a resume the peer rejected must not report success: {rejected:?}"
    );
    control(&sip, &connection_id, false)
        .await
        .expect("accepted resume");
    // Already active: success without another re-INVITE.
    control(&sip, &connection_id, false)
        .await
        .expect("idempotent resume");

    ConnectionAdapter::end(sip.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("end dialog");
    sip.drain().await.expect("drain adapter");
}

/// Send PCMU packets of one level at the 20 ms cadence.
async fn send_rtp(
    media: &UdpSocket,
    destination: std::net::SocketAddr,
    first_sequence: u16,
    first_timestamp: u32,
    packets: u16,
    level: u8,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    for index in 0..packets {
        interval.tick().await;
        let sequence = first_sequence.wrapping_add(index);
        let timestamp = first_timestamp.wrapping_add(u32::from(index) * 160);
        let mut packet = vec![0x80, 0x00];
        packet.extend_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&0x4844_4c44u32.to_be_bytes());
        packet.extend(std::iter::repeat_n(level, 160));
        media.send_to(&packet, destination).await.expect("send RTP");
    }
}

async fn wait_for_level(frames: &mut mpsc::UnboundedReceiver<MediaFrame>, level: u8) -> bool {
    tokio::time::timeout(Duration::from_millis(1_500), async {
        while let Some(frame) = frames.recv().await {
            if !frame.payload.is_empty() && frame.payload.iter().all(|byte| *byte == level) {
                return;
            }
        }
        std::future::pending::<()>().await;
    })
    .await
    .is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_restarts_inbound_playout_for_a_source_that_paused_its_clock() {
    let uas = boot_hold_uas().await;
    let sip = fresh_adapter(Some(PlayoutConfig::default())).await;
    let connection_id = originate_dialog(&sip, uas.addr).await;
    let local_rtp = uas.local_rtp.lock().unwrap().expect("offered RTP address");
    let stream = ConnectionAdapter::streams(sip.as_ref(), connection_id.clone())
        .await
        .expect("live streams")
        .into_iter()
        .next()
        .expect("audio stream");
    let mut inbound = stream.try_frames_in().expect("inbound frames");
    // Keep the inbound channel drained so the pump never parks on it.
    let (frames_tx, mut frames) = mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        while let Some(frame) = inbound.recv().await {
            if frames_tx.send(frame).is_err() {
                break;
            }
        }
    });

    // PCMU 0x20 / 0x30 decode and re-encode to themselves.
    send_rtp(&uas.media, local_rtp, 1_000, 80_000, 25, 0x20).await;
    assert!(
        wait_for_level(&mut frames, 0x20).await,
        "audio before the hold"
    );

    control(&sip, &connection_id, true)
        .await
        .expect("accepted hold");
    // The peer sends nothing while held, and its RTP clock stands still.
    tokio::time::sleep(Duration::from_secs(2)).await;
    control(&sip, &connection_id, false)
        .await
        .expect("accepted resume");
    while frames.try_recv().is_ok() {}

    let resumed = tokio::spawn({
        let media = Arc::clone(&uas.media);
        async move { send_rtp(&media, local_rtp, 1_025, 80_000 + 25 * 160, 75, 0x30).await }
    });
    assert!(
        wait_for_level(&mut frames, 0x30).await,
        "resumed audio from a source with a paused clock must play promptly"
    );
    resumed.await.expect("resumed RTP sender");
    reader.abort();

    ConnectionAdapter::end(sip.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("end dialog");
    sip.drain().await.expect("drain adapter");
}
