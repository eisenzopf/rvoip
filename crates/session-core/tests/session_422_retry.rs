//! Integration test for RFC 4028 §6 — INVITE + 422 Session Interval Too Small.
//!
//! An in-process raw-UDP mock UAS binds a loopback port and answers INVITEs
//! with either 422 + `Min-SE: 120` (to exercise the retry path) or 200 OK
//! (to close out the call). Two scenarios:
//!
//! 1. **Success after retry** — first INVITE gets 422 + Min-SE, the retry
//!    (which must carry `Session-Expires: 120` / `Min-SE: 120`) gets 200 OK.
//!    Assert exactly two INVITEs land and the retry carries the bumped
//!    headers.
//!
//! 2. **Two-retry cap** — the mock UAS returns 422 three times. Assert the
//!    UAC stops after the second retry (3 INVITEs total: initial + 2
//!    retries) and surfaces `CallFailed(422, "… Min-SE: 120s")`.
//!
//! The retry logic under test lives in:
//! - `src/adapters/session_event_handler.rs::handle_session_interval_too_small`
//!   (cap check + event dispatch)
//! - `src/state_machine/actions.rs::SendINVITEWithBumpedSessionExpires`
//!   (the retry INVITE)
//! - `crates/dialog-core/src/manager/transaction_integration.rs::
//!   send_invite_with_session_timer_override` (per-call timer headers)

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_session_core::api::unified::Config;
use rvoip_session_core::{Event, StreamPeer};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_dialog_core::transaction::utils::response_builders::create_response;

const UAS_MIN_SE: u32 = 120;
// Client's default Session-Expires — deliberately below the UAS's required
// floor so the first INVITE gets 422'd.
const CLIENT_SESSION_EXPIRES: u32 = 90;

/// Extract a u32 header value (Session-Expires or Min-SE) from a request.
/// `Session-Expires` carries additional `;refresher=…` params, so grab just
/// the numeric prefix.
fn extract_u32_header(req: &Request, name: &HeaderName) -> Option<u32> {
    req.raw_header_value(name).and_then(|s| {
        s.trim()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .filter(|n| !n.is_empty())
            .and_then(|n| n.parse::<u32>().ok())
    })
}

/// Build a 422 Session Interval Too Small response with `Min-SE: <secs>`.
fn build_422(request: &Request, min_se: u32) -> Vec<u8> {
    let mut resp = create_response(request, StatusCode::SessionIntervalTooSmall);
    resp.headers.push(TypedHeader::Other(
        HeaderName::MinSE,
        HeaderValue::Raw(min_se.to_string().into_bytes()),
    ));
    Message::Response(resp).to_bytes()
}

/// Build a 200 OK response with a trivial SDP answer. Adds a To-tag so the
/// dialog is properly established on the UAC side. Fixes up Content-Length
/// and Content-Type to match the body (create_response defaults to
/// Content-Length: 0 for an empty body).
fn build_200(request: &Request, uas_port: u16) -> Vec<u8> {
    let mut resp = create_response(request, StatusCode::Ok);
    if let Some(TypedHeader::To(to)) = resp
        .headers
        .iter_mut()
        .find(|h| matches!(h, TypedHeader::To(_)))
    {
        let tag = format!("uastag-{}", rand::random::<u32>());
        to.set_tag(&tag);
    }
    let sdp = format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         c=IN IP4 127.0.0.1\r\n\
         t=0 0\r\n\
         m=audio {} RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=sendrecv\r\n",
        uas_port + 2
    );
    let body_bytes = sdp.into_bytes();
    let body_len = body_bytes.len() as u32;
    resp.body = body_bytes.into();
    // Drop the Content-Length: 0 default from create_response and replace
    // with the actual body length. Also tag the body type so the UAC's SDP
    // parser picks it up for NegotiateSDPAsUAC.
    resp.headers.retain(|h| !matches!(h, TypedHeader::ContentLength(_)));
    resp.headers.push(TypedHeader::ContentLength(
        rvoip_sip_core::types::ContentLength::new(body_len),
    ));
    resp.headers.push(TypedHeader::ContentType(
        rvoip_sip_core::types::ContentType::from_type_subtype("application", "sdp"),
    ));
    Message::Response(resp).to_bytes()
}

struct MockUas {
    invite_count: Arc<AtomicU32>,
    retry_session_expires: Arc<Mutex<Option<u32>>>,
    retry_min_se: Arc<Mutex<Option<u32>>>,
    /// How many 422s to issue before succeeding with 200 OK. `u32::MAX`
    /// means "always 422" (cap-exhaustion test).
    reject_count: u32,
}

async fn run_mock_uas(sock: Arc<UdpSocket>, uas: Arc<MockUas>, uas_port: u16) {
    let mut buf = vec![0u8; 8192];
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(pair) => pair,
            Err(_) => return,
        };
        let msg = match parse_message(&buf[..n]) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let request = match msg {
            Message::Request(r) => r,
            _ => continue,
        };

        match request.method() {
            Method::Invite => {
                let count = uas.invite_count.fetch_add(1, Ordering::SeqCst);
                if count >= 1 {
                    *uas.retry_session_expires.lock().await =
                        extract_u32_header(&request, &HeaderName::SessionExpires);
                    *uas.retry_min_se.lock().await =
                        extract_u32_header(&request, &HeaderName::MinSE);
                }

                let bytes = if count < uas.reject_count {
                    build_422(&request, UAS_MIN_SE)
                } else {
                    build_200(&request, uas_port)
                };
                let _ = sock.send_to(&bytes, from).await;
            }
            Method::Bye => {
                let resp = create_response(&request, StatusCode::Ok);
                let _ = sock
                    .send_to(&Message::Response(resp).to_bytes(), from)
                    .await;
            }
            _ => {
                // ACK, CANCEL, etc. drain silently.
            }
        }
    }
}

fn client_config(client_port: u16) -> Config {
    // Build on `Config::local` so newly-added fields (TLS / SRTP /
    // PAI / outbound proxy / etc.) inherit defaults automatically.
    let mut config = Config::local("alice", client_port);
    config.media_port_start = 41000;
    config.media_port_end = 41100;
    // Set Session-Expires below UAS's Min-SE so the first INVITE gets 422'd.
    config.session_timer_secs = Some(CLIENT_SESSION_EXPIRES);
    config
}

#[tokio::test]
async fn invite_422_retry_bumps_session_expires_and_succeeds() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    // Randomized ports avoid collisions when the two tests in this file run
    // concurrently under `cargo test`.
    let uas_port = 35200 + (rand::random::<u16>() % 100);
    let client_port = uas_port + 200;

    let sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{}", uas_port))
            .await
            .expect("mock uas bind"),
    );

    let uas = Arc::new(MockUas {
        invite_count: Arc::new(AtomicU32::new(0)),
        retry_session_expires: Arc::new(Mutex::new(None)),
        retry_min_se: Arc::new(Mutex::new(None)),
        reject_count: 1, // First INVITE gets 422, retry succeeds.
    });

    let uas_handle = tokio::spawn(run_mock_uas(sock.clone(), uas.clone(), uas_port));

    let config = client_config(client_port);
    let mut peer = StreamPeer::with_config(config).await.expect("peer");

    let handle = peer
        .call(&format!("sip:bob@127.0.0.1:{}", uas_port))
        .await
        .expect("make_call");
    let call_id = handle.id().clone();

    let outcome = timeout(Duration::from_secs(10), wait_for_terminal(&mut peer, &call_id))
        .await
        .expect("call settled within 10s");

    // Small grace window so the mock observes any queued ACK before asserting
    // the exact INVITE count (ensures we don't race with in-flight retries).
    sleep(Duration::from_millis(200)).await;

    assert_eq!(
        uas.invite_count.load(Ordering::SeqCst),
        2,
        "expected exactly 2 INVITEs (initial + 422 retry)"
    );

    let retry_se = *uas.retry_session_expires.lock().await;
    assert_eq!(
        retry_se,
        Some(UAS_MIN_SE),
        "retry INVITE must carry Session-Expires={} (the UAS's Min-SE), got {:?}",
        UAS_MIN_SE,
        retry_se
    );

    let retry_min_se = *uas.retry_min_se.lock().await;
    assert_eq!(
        retry_min_se,
        Some(UAS_MIN_SE),
        "retry INVITE must carry Min-SE={} (matches floor), got {:?}",
        UAS_MIN_SE,
        retry_min_se
    );

    assert!(
        matches!(outcome, Outcome::Answered),
        "expected CallAnswered after retry, got {:?}",
        outcome
    );

    uas_handle.abort();
}

#[tokio::test]
async fn invite_422_retry_cap_surfaces_call_failed() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_port = 35400 + (rand::random::<u16>() % 100);
    let client_port = uas_port + 200;

    let sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{}", uas_port))
            .await
            .expect("mock uas bind"),
    );

    let uas = Arc::new(MockUas {
        invite_count: Arc::new(AtomicU32::new(0)),
        retry_session_expires: Arc::new(Mutex::new(None)),
        retry_min_se: Arc::new(Mutex::new(None)),
        reject_count: u32::MAX, // Always reject with 422.
    });

    let uas_handle = tokio::spawn(run_mock_uas(sock.clone(), uas.clone(), uas_port));

    let config = client_config(client_port);
    let mut peer = StreamPeer::with_config(config).await.expect("peer");

    let handle = peer
        .call(&format!("sip:bob@127.0.0.1:{}", uas_port))
        .await
        .expect("make_call");
    let call_id = handle.id().clone();

    let outcome = timeout(Duration::from_secs(10), wait_for_terminal(&mut peer, &call_id))
        .await
        .expect("call settled within 10s");

    sleep(Duration::from_millis(200)).await;

    // Expect 3 INVITEs: initial + 2 retries before the 2-retry cap trips.
    assert_eq!(
        uas.invite_count.load(Ordering::SeqCst),
        3,
        "expected 3 INVITEs (initial + 2 retries at cap), got {}",
        uas.invite_count.load(Ordering::SeqCst)
    );

    match outcome {
        Outcome::Failed { status_code, reason } => {
            assert_eq!(status_code, 422, "terminal status must be 422");
            assert!(
                reason.contains("Session Interval Too Small"),
                "reason string should mention '422 Session Interval Too Small', got: {}",
                reason
            );
            assert!(
                reason.contains(&format!("Min-SE: {}s", UAS_MIN_SE)),
                "reason string should carry the required Min-SE floor, got: {}",
                reason
            );
        }
        other => panic!("expected CallFailed after cap exhaustion, got {:?}", other),
    }

    uas_handle.abort();
}

// --- Test helpers ----------------------------------------------------------

#[derive(Debug)]
enum Outcome {
    Answered,
    Failed { status_code: u16, reason: String },
    Ended,
}

async fn wait_for_terminal(
    peer: &mut StreamPeer,
    call_id: &rvoip_session_core::api::events::CallId,
) -> Outcome {
    loop {
        let Some(event) = peer.next_event().await else {
            return Outcome::Ended;
        };
        match event {
            Event::CallAnswered { call_id: id, .. } if &id == call_id => {
                return Outcome::Answered;
            }
            Event::CallFailed { call_id: id, status_code, reason } if &id == call_id => {
                return Outcome::Failed { status_code, reason };
            }
            Event::CallEnded { call_id: id, .. } if &id == call_id => {
                return Outcome::Ended;
            }
            _ => {}
        }
    }
}
