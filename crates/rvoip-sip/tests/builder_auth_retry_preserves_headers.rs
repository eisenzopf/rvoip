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

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const UAS_PORT: u16 = 35200;
const UAC_PORT: u16 = 35201;
const TRACE_HEADER_NAME: &str = "X-Trace";
const TRACE_HEADER_VALUE: &str = "trace-cafe-babe";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_extras_survive_401_driven_auth_retry() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_addr = format!("127.0.0.1:{UAS_PORT}");
    let sock = Arc::new(
        UdpSocket::bind(&uas_addr)
            .await
            .expect("auth UAS bind"),
    );

    let invite_count = Arc::new(AtomicU32::new(0));
    // For each captured INVITE, record:
    // (has_x_trace, x_trace_value, has_authorization)
    let invites_seen = Arc::new(Mutex::new(Vec::<(bool, Option<String>, bool)>::new()));

    let sock_task = sock.clone();
    let count_task = invite_count.clone();
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
                    // ACK to the 401 — fire-and-forget; the auth retry follows.
                    continue;
                }
                _ => continue,
            };

            let count = count_task.fetch_add(1, Ordering::SeqCst);

            let x_trace_val = request.raw_header_value(&HeaderName::Other(
                TRACE_HEADER_NAME.to_string(),
            ));
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
                // 200 OK on the credentialed retry. Echo the To header
                // and stamp a To-tag so the dialog completes cleanly.
                let mut resp = create_response(&request, StatusCode::Ok);
                if let Some(contact) = request.header(&HeaderName::Contact) {
                    resp.headers.push(contact.clone());
                }
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            }
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let _call_id = coord
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{UAS_PORT}"),
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
