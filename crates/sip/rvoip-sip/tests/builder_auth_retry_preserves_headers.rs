//! SIP_API_DESIGN_2 §10 verification #21 — application-staged
//! extras survive 401-driven INVITE auth retry.
//!
//! Pattern reused from `register_423_retry.rs`: a raw-UDP mock UAS
//! binds a loopback port, answers the first INVITE with
//! `401 Unauthorized + WWW-Authenticate`, and the credentialed retry
//! with `200 OK`. The test asserts:
//!
//! 1. Exactly two INVITEs hit the wire (initial + retry).
//! 2. The initial INVITE carries `X-Trace: <id>` even though no
//!    Authorization is set.
//! 3. The retry INVITE carries the **same** `X-Trace: <id>` plus an
//!    `Authorization` header (the credentialed digest).
//! 4. The auth selection and all four wire legs are correlated to the call.
//! 5. The deliberately malformed 200 is ACKed and remains observable even
//!    when its post-commit BYE cannot be dispatched.
//!
//! Closes the F1 stash-preservation contract: §7.3 invariant #2
//! says auth retry re-reads the same `Arc<XxxRequestOptions>`, never
//! re-sets, so application extras stay attached across both wire
//! attempts.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::types::Credentials;
use rvoip_sip::{
    CallState, DigestAlgorithm, Event, SdesBase64Padding, SdesNegotiationFailureClass,
    SdesNegotiationStage, SipTraceConfig, SipTraceDirection,
};

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_core::types::sdp::CryptoSuite;

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const UAS_PORT: u16 = 35200;
const UAC_PORT: u16 = 35201;
const SUCCESS_UAS_PORT: u16 = 35202;
const SUCCESS_UAC_PORT: u16 = 35203;
const TRACE_HEADER_NAME: &str = "X-Trace";
const TRACE_HEADER_VALUE: &str = "trace-cafe-babe";
const MALFORMED_SDES_KEY: &str = "not+base64=inside";

async fn run_invite_extras_auth_retry(uas_port: u16, uac_port: u16, fail_bye_before_wire: bool) {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_addr = format!("127.0.0.1:{uas_port}");
    let sock = Arc::new(UdpSocket::bind(&uas_addr).await.expect("auth UAS bind"));

    let invite_count = Arc::new(AtomicU32::new(0));
    let ack_count = Arc::new(AtomicU32::new(0));
    let bye_count = Arc::new(AtomicU32::new(0));
    // For each captured INVITE, record:
    // (has_x_trace, x_trace_value, has_authorization)
    let invites_seen = Arc::new(Mutex::new(Vec::<(bool, Option<String>, bool)>::new()));

    let sock_task = sock.clone();
    let count_task = invite_count.clone();
    let ack_count_task = ack_count.clone();
    let bye_count_task = bye_count.clone();
    let captured_task = invites_seen.clone();
    let uas_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let msg = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match msg {
                Message::Request(r) if r.method() == Method::Invite => r,
                Message::Request(r) if r.method() == Method::Ack => {
                    ack_count_task.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                Message::Request(r) if r.method() == Method::Bye => {
                    bye_count_task.fetch_add(1, Ordering::SeqCst);
                    let response = create_response(&r, StatusCode::Ok);
                    let _ = sock_task
                        .send_to(&Message::Response(response).to_bytes(), from)
                        .await;
                    continue;
                }
                _ => continue,
            };

            let count = count_task.fetch_add(1, Ordering::SeqCst);

            let x_trace_val =
                request.raw_header_value(&HeaderName::Other(TRACE_HEADER_NAME.to_string()));
            let has_x_trace = x_trace_val.is_some();
            let has_authorization = request
                .raw_header_value(&HeaderName::Authorization)
                .is_some();
            captured_task
                .lock()
                .await
                .push((has_x_trace, x_trace_val, has_authorization));

            if count == 0 {
                // 401 with WWW-Authenticate.
                let mut resp = create_response(&request, StatusCode::Unauthorized);
                resp.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(
                        br#"Digest realm="testrealm", nonce="nonce-xyz", algorithm=MD5, qop="auth""#
                            .to_vec(),
                    ),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            } else {
                // Deliberately malformed AES-256 SDES answer. The UAC must
                // ACK the 2xx, expose a secret-safe diagnostic, and fail.
                let mut resp = create_response(&request, StatusCode::Ok);
                if let Some(TypedHeader::To(to)) = resp
                    .headers
                    .iter_mut()
                    .find(|header| matches!(header, TypedHeader::To(_)))
                {
                    to.set_tag("auth-sdes-uastag");
                }
                resp.headers.push(TypedHeader::Other(
                    HeaderName::Contact,
                    // ACK uses the authenticated INVITE's UDP route, while
                    // the confirmed-dialog BYE must honor this target's TCP
                    // transport parameter. The UDP-only fixture therefore
                    // injects a deterministic post-commit BYE failure.
                    HeaderValue::Raw(if fail_bye_before_wire {
                        format!("<sip:bob@127.0.0.1:{uas_port};transport=tcp>").into_bytes()
                    } else {
                        format!("<sip:bob@127.0.0.1:{uas_port};transport=udp>").into_bytes()
                    }),
                ));
                resp.body = format!(
                    "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 40000 RTP/SAVP 0\r\na=rtpmap:0 PCMU/8000\r\na=crypto:1 AES_256_CM_HMAC_SHA1_80 inline:{MALFORMED_SDES_KEY}\r\na=sendrecv\r\n"
                )
                .into_bytes()
                .into();
                resp.headers
                    .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
                resp.headers.push(TypedHeader::ContentLength(
                    rvoip_sip_core::types::ContentLength::new(resp.body.len() as u32),
                ));
                resp.headers.push(TypedHeader::ContentType(
                    rvoip_sip_core::types::ContentType::sdp(),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
                let retransmit_socket = Arc::clone(&sock_task);
                tokio::spawn(async move {
                    sleep(Duration::from_millis(100)).await;
                    let _ = retransmit_socket.send_to(&bytes, from).await;
                });
            }
        }
    });

    let mut config = Config::local("alice", uac_port);
    config.sip_trace = SipTraceConfig::enabled();
    config.offer_srtp = true;
    config.srtp_required = true;
    config.srtp_offered_suites = vec![CryptoSuite::AesCm256HmacSha1_80];
    let coord = UnifiedCoordinator::new(config)
        .await
        .expect("UAC coordinator");
    let mut events = coord.events().await.expect("UAC event stream");
    sleep(Duration::from_millis(150)).await;

    let call_id = coord
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{uas_port}"),
        )
        .with_credentials(Credentials::new("alice", "password").with_realm("testrealm"))
        .with_raw_header(
            HeaderName::Other(TRACE_HEADER_NAME.to_string()),
            TRACE_HEADER_VALUE,
        )
        .expect("X-Trace is application-controlled")
        .send()
        .await
        .expect("invite.send()");

    // Wait for exactly two INVITEs to land on the UAS.
    let observed = timeout(Duration::from_secs(8), async {
        loop {
            if invite_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "UAS never saw 2 INVITEs (count={})",
        invite_count.load(Ordering::SeqCst)
    );

    let mut auth_retry = None;
    let mut sdes_failure = None;
    let mut terminal_failure = None;
    let mut ack_after_200 = false;
    let mut trace_sequence = Vec::new();
    timeout(Duration::from_secs(8), async {
        while auth_retry.is_none()
            || sdes_failure.is_none()
            || terminal_failure.is_none()
            || trace_sequence.len() < 4
            || !ack_after_200
        {
            match events.next().await {
                Some(Event::CallAuthRetrying {
                    call_id: id,
                    status_code,
                    realm,
                    algorithm,
                    qop,
                }) if id == call_id => {
                    auth_retry = Some((status_code, realm, algorithm, qop));
                }
                Some(Event::SdesNegotiationFailed {
                    call_id: id,
                    response,
                    diagnostic,
                }) if id == call_id => {
                    assert_eq!(response.status_code, 200);
                    assert!(response
                        .sdp
                        .as_deref()
                        .is_some_and(|sdp| sdp.contains(MALFORMED_SDES_KEY)));
                    let debug = format!("{response:?} {diagnostic:?}");
                    assert!(!debug.contains(MALFORMED_SDES_KEY));
                    assert!(!debug.contains("a=crypto"));
                    sdes_failure = Some(diagnostic);
                }
                Some(Event::CallFailed {
                    call_id: id,
                    status_code,
                    reason,
                }) if id == call_id => {
                    let committed = coord.get_state(&id).await;
                    assert!(
                        committed
                            .as_ref()
                            .map_or(true, |state| !state.is_in_progress()),
                        "CallFailed must be published only after terminal state commit: {committed:?}"
                    );
                    terminal_failure = Some((status_code, reason));
                }
                Some(Event::CallEnded { call_id: id, .. }) if id == call_id => {
                    panic!("initial negotiation failure emitted contradictory CallEnded")
                }
                Some(Event::SipTrace(trace)) if trace.session_id.as_ref() == Some(&call_id) => {
                    let label = match (trace.direction, trace.start_line.as_str()) {
                        (SipTraceDirection::Outbound, line) if line.starts_with("INVITE ") => {
                            Some("INVITE")
                        }
                        (SipTraceDirection::Inbound, line) if line.starts_with("SIP/2.0 401 ") => {
                            Some("401")
                        }
                        (SipTraceDirection::Inbound, line) if line.starts_with("SIP/2.0 200 ") => {
                            Some("200")
                        }
                        (SipTraceDirection::Outbound, line)
                            if line.starts_with("ACK ")
                                && trace_sequence.last() == Some(&"200") =>
                        {
                            ack_after_200 = true;
                            None
                        }
                        _ => None,
                    };
                    if let Some(label) = label {
                        trace_sequence.push(label);
                    }
                }
                Some(_) => {}
                None => panic!("UAC event stream closed before auth observations"),
            }
        }
    })
    .await
    .expect("auth retry and trace correlation observations");
    assert_eq!(
        auth_retry,
        Some((
            401,
            "testrealm".to_string(),
            DigestAlgorithm::MD5,
            Some("auth".to_string())
        ))
    );
    let sdes_failure = sdes_failure.expect("typed SDES failure observation");
    assert_eq!(sdes_failure.stage, SdesNegotiationStage::RemoteAnswer);
    assert_eq!(
        sdes_failure.failure_class,
        SdesNegotiationFailureClass::InvalidBase64
    );
    assert_eq!(sdes_failure.tag, 1);
    assert_eq!(sdes_failure.suite, CryptoSuite::AesCm256HmacSha1_80);
    assert_eq!(sdes_failure.encoded_bytes, MALFORMED_SDES_KEY.len());
    assert_eq!(sdes_failure.padding, SdesBase64Padding::Malformed);
    assert_eq!(sdes_failure.expected_decoded_bytes, 46);
    assert_eq!(sdes_failure.actual_decoded_bytes, None);
    assert_eq!(trace_sequence, ["INVITE", "401", "INVITE", "200"]);
    assert_eq!(
        terminal_failure.as_ref().map(|failure| failure.0),
        Some(488)
    );
    assert!(terminal_failure
        .as_ref()
        .is_some_and(|failure| failure.1.contains("missing, invalid, or unusable SDP")));
    assert!(ack_after_200, "malformed INVITE 2xx must be ACKed");

    timeout(Duration::from_secs(2), async {
        while ack_count.load(Ordering::SeqCst) < 2 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("UAS receives retransmission ACKs despite terminal BYE failure");
    if fail_bye_before_wire {
        assert_eq!(
            bye_count.load(Ordering::SeqCst),
            0,
            "TCP-target BYE unexpectedly reached the UDP-only fixture"
        );
    } else {
        timeout(Duration::from_secs(2), async {
            while bye_count.load(Ordering::SeqCst) == 0 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("successful negotiation-failure BYE reached the UAS");
        assert_eq!(bye_count.load(Ordering::SeqCst), 1);
    }

    timeout(Duration::from_secs(2), async {
        loop {
            match coord.get_state(&call_id).await {
                Err(_) => break,
                Ok(CallState::Terminating) => sleep(Duration::from_millis(10)).await,
                Ok(state) => panic!("unexpected post-failure state: {state:?}"),
            }
        }
    })
    .await
    .expect("negotiation-failure BYE outcome retires the exact session");
    let contradictory_terminal = timeout(Duration::from_millis(200), async {
        loop {
            match events.next().await {
                Some(Event::CallEnded { call_id: id, .. }) if id == call_id => return true,
                Some(_) => {}
                None => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        !contradictory_terminal,
        "initial negotiation failure emitted both CallFailed and CallEnded"
    );

    // Settle.
    sleep(Duration::from_millis(300)).await;

    let captured = invites_seen.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "expected initial INVITE + auth retry, got {}",
        captured.len()
    );

    // INITIAL INVITE: X-Trace present, no Authorization.
    let (init_has_trace, init_trace, init_has_auth) = &captured[0];
    assert!(
        *init_has_trace,
        "initial INVITE must carry X-Trace; captured: {:?}",
        captured[0]
    );
    assert_eq!(
        init_trace.as_deref(),
        Some(TRACE_HEADER_VALUE),
        "initial INVITE X-Trace must echo the staged value"
    );
    assert!(
        !*init_has_auth,
        "initial INVITE must NOT carry Authorization"
    );

    // RETRY INVITE: X-Trace still present (this is what §10 #21 is about),
    // and Authorization is now stamped.
    let (retry_has_trace, retry_trace, retry_has_auth) = &captured[1];
    assert!(
        *retry_has_trace,
        "auth retry INVITE must still carry X-Trace; captured: {:?}",
        captured[1]
    );
    assert_eq!(
        retry_trace.as_deref(),
        Some(TRACE_HEADER_VALUE),
        "auth retry INVITE X-Trace must match the initial one — stash is single-source"
    );
    assert!(
        *retry_has_auth,
        "auth retry INVITE must carry Authorization (credentialed)"
    );

    uas_handle.abort();
}

/// Regression: this must complete on Tokio's default worker stack. The
/// response-to-auth-retry path must not require `RUST_MIN_STACK` or a custom
/// runtime `thread_stack_size`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_extras_survive_401_driven_auth_retry() {
    run_invite_extras_auth_retry(UAS_PORT, UAC_PORT, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn successful_negotiation_failure_bye_preserves_single_terminal_event() {
    run_invite_extras_auth_retry(SUCCESS_UAS_PORT, SUCCESS_UAC_PORT, false).await;
}
