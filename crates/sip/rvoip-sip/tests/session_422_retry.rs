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
//! 3. **422 interleaved with 401/407 authentication** — a policy UAS that,
//!    like FreeSWITCH, rejects any INVITE whose `Session-Expires` is below
//!    its Min-SE and challenges any INVITE without credentials. RFC 4028
//!    §7.4 requires every later attempt to carry the largest Min-SE received
//!    in a 422, so the authenticated retry must not fall back to the
//!    configured 90 s interval (which costs a second 422 round trip and burns
//!    the 422 retry cap). Covered in both orders (422 → 407, 407 → 422), for
//!    401 as well as 407, and with a stale-nonce re-challenge that used to
//!    exhaust the cap.
//!
//! The retry logic under test lives in:
//! - `src/adapters/session_event_handler.rs::handle_session_interval_too_small`
//!   (cap check + event dispatch)
//! - `src/state_machine/actions.rs::SendINVITEWithBumpedSessionExpires`
//!   (the retry INVITE)
//! - `crates/dialog-core/src/manager/transaction_integration.rs::
//!   send_invite_with_session_timer_override` (per-call timer headers)

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::unified::Config;
use rvoip_sip::types::Credentials;
use rvoip_sip::{Event, StreamPeer};

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

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
fn build_200(request: &Request, uas_rtp_port: u16) -> Vec<u8> {
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
        uas_rtp_port
    );
    let body_bytes = sdp.into_bytes();
    let body_len = body_bytes.len() as u32;
    resp.body = body_bytes.into();
    // Drop the Content-Length: 0 default from create_response and replace
    // with the actual body length. Also tag the body type so the UAC's SDP
    // parser picks it up for NegotiateSDPAsUAC.
    resp.headers
        .retain(|h| !matches!(h, TypedHeader::ContentLength(_)));
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

async fn run_mock_uas(sock: Arc<UdpSocket>, uas: Arc<MockUas>, uas_rtp_port: u16) {
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
                    build_200(&request, uas_rtp_port)
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

fn client_config() -> Config {
    // Build on `Config::local` so newly-added fields (TLS / SRTP /
    // PAI / outbound proxy / etc.) inherit defaults automatically. Port 0
    // keeps the listener allocation atomic: the SIP transport binds its own
    // OS-assigned port instead of racing another test after a probe socket is
    // released.
    let mut config = Config::local("alice", 0);
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

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("mock uas bind"));
    let uas_port = sock.local_addr().expect("mock uas address").port();
    // Keep the OS-assigned RTP destination bound for the lifetime of the
    // call. This avoids both fixed-port collisions and the bind/release race
    // inherent in probing for an available port.
    let _uas_rtp_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("mock uas RTP bind");
    let uas_rtp_port = _uas_rtp_sink
        .local_addr()
        .expect("mock uas RTP address")
        .port();

    let uas = Arc::new(MockUas {
        invite_count: Arc::new(AtomicU32::new(0)),
        retry_session_expires: Arc::new(Mutex::new(None)),
        retry_min_se: Arc::new(Mutex::new(None)),
        reject_count: 1, // First INVITE gets 422, retry succeeds.
    });

    let uas_handle = tokio::spawn(run_mock_uas(sock.clone(), uas.clone(), uas_rtp_port));

    let config = client_config();
    let mut peer = StreamPeer::with_config(config).await.expect("peer");

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{}", uas_port))
        .send()
        .await
        .expect("invite.send()");

    let outcome = timeout(
        Duration::from_secs(10),
        wait_for_terminal(&mut peer, &call_id),
    )
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

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("mock uas bind"));
    let uas_port = sock.local_addr().expect("mock uas address").port();
    let _uas_rtp_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("mock uas RTP bind");
    let uas_rtp_port = _uas_rtp_sink
        .local_addr()
        .expect("mock uas RTP address")
        .port();

    let uas = Arc::new(MockUas {
        invite_count: Arc::new(AtomicU32::new(0)),
        retry_session_expires: Arc::new(Mutex::new(None)),
        retry_min_se: Arc::new(Mutex::new(None)),
        reject_count: u32::MAX, // Always reject with 422.
    });

    let uas_handle = tokio::spawn(run_mock_uas(sock.clone(), uas.clone(), uas_rtp_port));

    let config = client_config();
    let mut peer = StreamPeer::with_config(config).await.expect("peer");

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{}", uas_port))
        .send()
        .await
        .expect("invite.send()");

    let outcome = timeout(
        Duration::from_secs(10),
        wait_for_terminal(&mut peer, &call_id),
    )
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
        Outcome::Failed {
            status_code,
            reason,
        } => {
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

// --- 422 interleaved with 401/407 authentication ----------------------------

/// Which challenge the policy UAS issues to an unauthenticated INVITE.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Challenge {
    /// 407 + `Proxy-Authenticate`; credentials come back in
    /// `Proxy-Authorization`.
    Proxy,
    /// 401 + `WWW-Authenticate`; credentials come back in `Authorization`.
    Origin,
}

impl Challenge {
    fn credential_header(self) -> HeaderName {
        match self {
            Self::Proxy => HeaderName::ProxyAuthorization,
            Self::Origin => HeaderName::Authorization,
        }
    }
}

/// A UAS that decides each INVITE on policy rather than on attempt number,
/// the way a real PBX (FreeSWITCH with `minimum-session-expires=120`) does:
/// any INVITE whose `Session-Expires` is below the floor gets 422, any
/// INVITE without acceptable credentials gets challenged, everything else
/// is answered.
struct PolicyUas {
    challenge: Challenge,
    /// Check authentication before the session timer (407 → 422 order)
    /// instead of after it (422 → 407 order, FreeSWITCH's behaviour).
    auth_first: bool,
    /// Treat the first nonce as expired: credentials computed from it get a
    /// `stale=true` re-challenge carrying a fresh nonce (RFC 7616 §3.3), as
    /// when a nonce times out mid-setup.
    stale_once: bool,
    invites: Mutex<Vec<SeenInvite>>,
}

#[derive(Clone, Debug)]
struct SeenInvite {
    cseq: Option<String>,
    session_expires: Option<u32>,
    min_se: Option<u32>,
    credentials: Option<String>,
}

const FIRST_NONCE: &str = "minse-nonce-1";
const FRESH_NONCE: &str = "minse-nonce-2";

fn build_challenge(request: &Request, challenge: Challenge, nonce: &str, stale: bool) -> Vec<u8> {
    let (status, header) = match challenge {
        Challenge::Proxy => (
            StatusCode::ProxyAuthenticationRequired,
            HeaderName::ProxyAuthenticate,
        ),
        Challenge::Origin => (StatusCode::Unauthorized, HeaderName::WwwAuthenticate),
    };
    let mut resp = create_response(request, status);
    let stale = if stale { ", stale=true" } else { "" };
    resp.headers.push(TypedHeader::Other(
        header,
        HeaderValue::Raw(
            format!(r#"Digest realm="pbx", nonce="{nonce}", algorithm=MD5, qop="auth"{stale}"#)
                .into_bytes(),
        ),
    ));
    Message::Response(resp).to_bytes()
}

async fn run_policy_uas(sock: Arc<UdpSocket>, uas: Arc<PolicyUas>, uas_rtp_port: u16) {
    let mut buf = vec![0u8; 8192];
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(pair) => pair,
            Err(_) => return,
        };
        let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
            continue;
        };
        match request.method() {
            Method::Invite => {
                let seen = SeenInvite {
                    cseq: request.raw_header_value(&HeaderName::CSeq),
                    session_expires: extract_u32_header(&request, &HeaderName::SessionExpires),
                    min_se: extract_u32_header(&request, &HeaderName::MinSE),
                    credentials: request.raw_header_value(&uas.challenge.credential_header()),
                };
                uas.invites.lock().await.push(seen.clone());

                let timer_too_small = seen.session_expires.is_some_and(|se| se < UAS_MIN_SE);
                let auth_reply = match seen.credentials.as_deref() {
                    None => Some(build_challenge(&request, uas.challenge, FIRST_NONCE, false)),
                    Some(credentials) if uas.stale_once && credentials.contains(FIRST_NONCE) => {
                        Some(build_challenge(&request, uas.challenge, FRESH_NONCE, true))
                    }
                    Some(_) => None,
                };
                let timer_reply = timer_too_small.then(|| build_422(&request, UAS_MIN_SE));
                let reply = if uas.auth_first {
                    auth_reply.or(timer_reply)
                } else {
                    timer_reply.or(auth_reply)
                }
                .unwrap_or_else(|| build_200(&request, uas_rtp_port));
                let _ = sock.send_to(&reply, from).await;
            }
            Method::Bye => {
                let resp = create_response(&request, StatusCode::Ok);
                let _ = sock
                    .send_to(&Message::Response(resp).to_bytes(), from)
                    .await;
            }
            _ => {}
        }
    }
}

/// Place one call against a [`PolicyUas`] and return its outcome plus every
/// INVITE the UAS saw, in order.
async fn call_policy_uas(
    challenge: Challenge,
    auth_first: bool,
    stale_once: bool,
) -> (Outcome, Vec<SeenInvite>) {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("mock uas bind"));
    let uas_port = sock.local_addr().expect("mock uas address").port();
    let uas_rtp_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("mock uas RTP bind");
    let uas_rtp_port = uas_rtp_sink
        .local_addr()
        .expect("mock uas RTP address")
        .port();

    let uas = Arc::new(PolicyUas {
        challenge,
        auth_first,
        stale_once,
        invites: Mutex::new(Vec::new()),
    });
    let uas_handle = tokio::spawn(run_policy_uas(sock.clone(), uas.clone(), uas_rtp_port));

    let mut peer = StreamPeer::with_config(client_config())
        .await
        .expect("peer");
    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{}", uas_port))
        .with_credentials(Credentials::new("alice", "secret"))
        .send()
        .await
        .expect("invite.send()");

    let outcome = timeout(
        Duration::from_secs(10),
        wait_for_terminal(&mut peer, &call_id),
    )
    .await
    .expect("call settled within 10s");
    // Let any in-flight retry land before the INVITE count is asserted.
    sleep(Duration::from_millis(200)).await;
    uas_handle.abort();

    let invites = uas.invites.lock().await.clone();
    (outcome, invites)
}

/// The authenticated INVITE that follows a 422 must keep the learned floor:
/// `Session-Expires` at least the 422's Min-SE and `Min-SE` equal to it.
fn assert_carries_learned_min_se(invite: &SeenInvite, label: &str) {
    assert!(
        invite.session_expires.is_some_and(|se| se >= UAS_MIN_SE),
        "{label} must carry Session-Expires >= {UAS_MIN_SE} (RFC 4028 §7.4), got {invite:?}"
    );
    assert_eq!(
        invite.min_se,
        Some(UAS_MIN_SE),
        "{label} must carry Min-SE = {UAS_MIN_SE} (RFC 4028 §7.4), got {invite:?}"
    );
}

fn assert_distinct_ascending_cseq(invites: &[SeenInvite]) {
    let numbers: Vec<u32> = invites
        .iter()
        .map(|invite| {
            invite
                .cseq
                .as_deref()
                .and_then(|cseq| cseq.split_whitespace().next())
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("INVITE without a numeric CSeq: {invite:?}"))
        })
        .collect();
    assert!(
        numbers.windows(2).all(|pair| pair[0] < pair[1]),
        "each retry must take a new, larger CSeq: {numbers:?}"
    );
}

/// FreeSWITCH order: 422 first, then the proxy challenge. The authenticated
/// INVITE must keep Session-Expires/Min-SE at the 422's floor, so the call
/// completes in three INVITEs instead of drawing a second 422.
#[tokio::test]
async fn invite_422_then_407_auth_retry_keeps_learned_min_se() {
    let (outcome, invites) = call_policy_uas(Challenge::Proxy, false, false).await;

    assert_eq!(
        invites.len(),
        3,
        "expected INVITE(SE {CLIENT_SESSION_EXPIRES}) → 422, INVITE(SE {UAS_MIN_SE}) → 407, \
         authenticated INVITE → 200; saw {invites:#?}"
    );
    assert_eq!(invites[0].session_expires, Some(CLIENT_SESSION_EXPIRES));
    assert!(invites[0].credentials.is_none());
    assert_carries_learned_min_se(&invites[1], "422 retry");
    assert!(invites[1].credentials.is_none());
    assert!(
        invites[2].credentials.is_some(),
        "third INVITE must carry Proxy-Authorization: {:?}",
        invites[2]
    );
    assert_carries_learned_min_se(&invites[2], "authenticated INVITE after 422");
    assert_distinct_ascending_cseq(&invites);
    assert!(
        matches!(outcome, Outcome::Answered),
        "expected CallAnswered, got {outcome:?}"
    );
}

/// Same as above for an origin (401 / WWW-Authenticate) challenge.
#[tokio::test]
async fn invite_422_then_401_auth_retry_keeps_learned_min_se() {
    let (outcome, invites) = call_policy_uas(Challenge::Origin, false, false).await;

    assert_eq!(
        invites.len(),
        3,
        "expected 422 → 401 → 200 in three INVITEs; saw {invites:#?}"
    );
    assert_carries_learned_min_se(&invites[1], "422 retry");
    assert!(
        invites[2].credentials.is_some(),
        "third INVITE must carry Authorization: {:?}",
        invites[2]
    );
    assert_carries_learned_min_se(&invites[2], "authenticated INVITE after 422");
    assert_distinct_ascending_cseq(&invites);
    assert!(
        matches!(outcome, Outcome::Answered),
        "expected CallAnswered, got {outcome:?}"
    );
}

/// Reverse order: 407 first, then 422. The 422 retry must keep the proxy
/// credentials and raise the timer to the floor.
#[tokio::test]
async fn invite_407_then_422_retry_keeps_credentials_and_min_se() {
    let (outcome, invites) = call_policy_uas(Challenge::Proxy, true, false).await;

    assert_eq!(
        invites.len(),
        3,
        "expected 407 → 422 → 200 in three INVITEs; saw {invites:#?}"
    );
    assert!(invites[0].credentials.is_none());
    assert_eq!(invites[1].session_expires, Some(CLIENT_SESSION_EXPIRES));
    assert!(invites[1].credentials.is_some());
    assert!(
        invites[2].credentials.is_some(),
        "422 retry must keep Proxy-Authorization: {:?}",
        invites[2]
    );
    assert_carries_learned_min_se(&invites[2], "422 retry after 407");
    assert_distinct_ascending_cseq(&invites);
    assert!(
        matches!(outcome, Outcome::Answered),
        "expected CallAnswered, got {outcome:?}"
    );
}

/// 422 → 407 → stale 407 → 200. Before the fix each authenticated retry fell
/// back to the configured interval and drew another 422, so a single
/// stale-nonce re-challenge exhausted the 422 retry cap (2) and failed the
/// call. Auth retries must neither lose the floor nor consume that budget.
#[tokio::test]
async fn invite_422_then_stale_407_rechallenge_does_not_exhaust_422_cap() {
    let (outcome, invites) = call_policy_uas(Challenge::Proxy, false, true).await;

    assert!(
        matches!(outcome, Outcome::Answered),
        "a stale-nonce re-challenge after a 422 must not fail the call, got {outcome:?}; \
         INVITEs: {invites:#?}"
    );
    assert_eq!(
        invites.len(),
        4,
        "expected 422 → 407 → 407(stale) → 200 in four INVITEs; saw {invites:#?}"
    );
    for (index, invite) in invites.iter().enumerate().skip(1) {
        assert_carries_learned_min_se(invite, &format!("INVITE #{}", index + 1));
    }
    assert!(invites[2]
        .credentials
        .as_deref()
        .is_some_and(|credentials| credentials.contains(FIRST_NONCE)));
    assert!(invites[3]
        .credentials
        .as_deref()
        .is_some_and(|credentials| credentials.contains(FRESH_NONCE)));
    assert_distinct_ascending_cseq(&invites);
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
    call_id: &rvoip_sip::api::events::CallId,
) -> Outcome {
    loop {
        let Some(event) = peer.next_event().await else {
            return Outcome::Ended;
        };
        match event {
            Event::CallAnswered { call_id: id, .. } if &id == call_id => {
                return Outcome::Answered;
            }
            Event::CallFailed {
                call_id: id,
                status_code,
                reason,
            } if &id == call_id => {
                return Outcome::Failed {
                    status_code,
                    reason,
                };
            }
            Event::CallEnded { call_id: id, .. } if &id == call_id => {
                return Outcome::Ended;
            }
            _ => {}
        }
    }
}
