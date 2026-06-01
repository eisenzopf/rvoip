//! SIP_API_DESIGN_2 R2 — application-staged extras survive 401-driven
//! in-dialog REFER auth retry.
//!
//! Mirror of `bye_auth_retry.rs` but for REFER, demonstrating that the
//! per-method `SendRequestWithAuth` dispatch routes correctly when
//! `pending_auth_method = "REFER"` arrives on the wire CSeq.
//!
//! The test asserts:
//! 1. Exactly two REFERs hit the wire (initial + retry).
//! 2. Both REFERs carry the application-staged `X-Trace` header.
//! 3. The retry REFER carries `Authorization:`; the initial does not.
//! 4. Both REFERs carry the same `Refer-To:` (method-shaped header
//!    survives the retry via the stash).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::types::Credentials;
use rvoip_sip::CallState;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const UAS_PORT: u16 = 35260;
const UAC_PORT: u16 = 35261;
const TRACE_HEADER_NAME: &str = "X-Trace";
const TRACE_HEADER_VALUE: &str = "trace-refer-cafe";
const REFER_TARGET: &str = "sip:transfer-target@127.0.0.1:35262";

/// Per-REFER capture: (has_x_trace, x_trace_value, has_authorization, refer_to).
type ReferCapture = (bool, Option<String>, bool, Option<String>);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refer_extras_survive_401_driven_auth_retry() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_addr = format!("127.0.0.1:{UAS_PORT}");
    let sock = Arc::new(UdpSocket::bind(&uas_addr).await.expect("UAS bind"));

    let refer_count = Arc::new(AtomicU32::new(0));
    let refers_seen: Arc<Mutex<Vec<ReferCapture>>> = Arc::new(Mutex::new(Vec::new()));

    let sock_task = sock.clone();
    let count_task = refer_count.clone();
    let captured_task = refers_seen.clone();
    let uas_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let parsed = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match parsed {
                Message::Request(r) => r,
                _ => continue,
            };

            match request.method() {
                Method::Invite => {
                    let mut resp = create_response(&request, StatusCode::Ok);
                    for hdr in resp.headers.iter_mut() {
                        if let TypedHeader::To(to) = hdr {
                            if to.tag().is_none() {
                                to.set_tag("uas-refer-tag");
                            }
                            break;
                        }
                    }
                    resp.headers.push(TypedHeader::Other(
                        HeaderName::Contact,
                        HeaderValue::Raw(format!("<sip:bob@127.0.0.1:{UAS_PORT}>").into_bytes()),
                    ));
                    let _ = sock_task
                        .send_to(&Message::Response(resp).to_bytes(), from)
                        .await;
                }
                Method::Ack => {}
                Method::Refer => {
                    let count = count_task.fetch_add(1, Ordering::SeqCst);
                    let x_trace_val =
                        request.raw_header_value(&HeaderName::Other(TRACE_HEADER_NAME.to_string()));
                    let has_x_trace = x_trace_val.is_some();
                    let has_authorization = request
                        .raw_header_value(&HeaderName::Authorization)
                        .is_some();
                    let refer_to = request.raw_header_value(&HeaderName::ReferTo);
                    captured_task.lock().await.push((
                        has_x_trace,
                        x_trace_val,
                        has_authorization,
                        refer_to,
                    ));

                    if count == 0 {
                        let mut resp = create_response(&request, StatusCode::Unauthorized);
                        resp.headers.push(TypedHeader::Other(
                            HeaderName::WwwAuthenticate,
                            HeaderValue::Raw(
                                br#"Digest realm="testrealm", nonce="refer-nonce-1", algorithm=MD5, qop="auth""#
                                    .to_vec(),
                            ),
                        ));
                        let _ = sock_task
                            .send_to(&Message::Response(resp).to_bytes(), from)
                            .await;
                    } else {
                        let resp = create_response(&request, StatusCode::Accepted);
                        let _ = sock_task
                            .send_to(&Message::Response(resp).to_bytes(), from)
                            .await;
                    }
                }
                Method::Bye => {
                    let resp = create_response(&request, StatusCode::Ok);
                    let _ = sock_task
                        .send_to(&Message::Response(resp).to_bytes(), from)
                        .await;
                }
                _ => {
                    let resp = create_response(&request, StatusCode::Ok);
                    let _ = sock_task
                        .send_to(&Message::Response(resp).to_bytes(), from)
                        .await;
                }
            }
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let call_id = coord
        .invite(
            Some(format!("sip:alice@127.0.0.1:{UAC_PORT}")),
            format!("sip:bob@127.0.0.1:{UAS_PORT}"),
        )
        .with_credentials(Credentials::new("alice", "password").with_realm("testrealm"))
        .send()
        .await
        .expect("invite.send()");

    let active = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(state) = coord.get_state(&call_id).await {
                if state == CallState::Active {
                    return true;
                }
            }
            sleep(Duration::from_millis(40)).await;
        }
    })
    .await;
    assert!(matches!(active, Ok(true)), "call never reached Active");

    coord
        .refer(&call_id, REFER_TARGET)
        .with_raw_header(
            HeaderName::Other(TRACE_HEADER_NAME.to_string()),
            TRACE_HEADER_VALUE,
        )
        .expect("X-Trace staging")
        .send()
        .await
        .expect("refer.send()");

    let observed = timeout(Duration::from_secs(8), async {
        loop {
            if refer_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "UAS never saw 2 REFERs (count={})",
        refer_count.load(Ordering::SeqCst)
    );

    sleep(Duration::from_millis(200)).await;

    let captured = refers_seen.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "expected initial REFER + auth retry, got {captured:?}"
    );

    let (init_has_trace, init_trace, init_has_auth, init_refer_to) = &captured[0];
    assert!(*init_has_trace, "initial REFER must carry X-Trace");
    assert_eq!(init_trace.as_deref(), Some(TRACE_HEADER_VALUE));
    assert!(
        !*init_has_auth,
        "initial REFER must NOT carry Authorization"
    );
    assert!(
        init_refer_to
            .as_deref()
            .unwrap_or("")
            .contains(REFER_TARGET),
        "initial REFER must carry Refer-To: {REFER_TARGET}; got {:?}",
        init_refer_to
    );

    let (retry_has_trace, retry_trace, retry_has_auth, retry_refer_to) = &captured[1];
    assert!(
        *retry_has_trace,
        "auth retry REFER must still carry X-Trace"
    );
    assert_eq!(
        retry_trace.as_deref(),
        Some(TRACE_HEADER_VALUE),
        "retry REFER X-Trace must match the initial"
    );
    assert!(*retry_has_auth, "auth retry REFER must carry Authorization");
    assert!(
        retry_refer_to
            .as_deref()
            .unwrap_or("")
            .contains(REFER_TARGET),
        "auth retry REFER must still carry Refer-To (method-shaped header survives stash)"
    );

    uas_handle.abort();
}
