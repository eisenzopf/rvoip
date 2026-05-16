//! SIP_API_DESIGN_2 R2 — application-staged extras survive 401-driven
//! in-dialog BYE auth retry.
//!
//! Pattern mirrors `builder_auth_retry_preserves_headers.rs` (the INVITE
//! case): a raw-UDP UAS binds a loopback port, the UAC establishes a
//! dialog via INVITE → 200 OK → ACK, then sends a BYE. The UAS replies
//! `401 Unauthorized + WWW-Authenticate` to the first BYE and `200 OK`
//! to the credentialed retry. The test asserts:
//!
//! 1. Exactly two BYEs hit the wire (initial + retry).
//! 2. Both BYEs carry the application-staged `X-Trace: <id>` extra.
//! 3. The retry BYE carries an `Authorization:` header; the initial
//!    does not.
//!
//! Closes the R2 contract for in-dialog auth retry: the new
//! `Action::SendRequestWithAuth` reads
//! `session.pending_auth_method = "BYE"` (extracted by dialog-core's
//! event_hub from the response `CSeq:`), pulls the matching
//! `pending_bye_options` stash, computes a digest, and dispatches via
//! `DialogAdapter::send_bye_with_auth`.

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

const UAS_PORT: u16 = 35240;
const UAC_PORT: u16 = 35241;
const TRACE_HEADER_NAME: &str = "X-Trace";
const TRACE_HEADER_VALUE: &str = "trace-bye-cafe";

/// Per-BYE capture: (has_x_trace, x_trace_value, has_authorization).
type ByeCapture = (bool, Option<String>, bool);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bye_extras_survive_401_driven_auth_retry() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_addr = format!("127.0.0.1:{UAS_PORT}");
    let sock = Arc::new(UdpSocket::bind(&uas_addr).await.expect("UAS bind"));

    let bye_count = Arc::new(AtomicU32::new(0));
    let byes_seen: Arc<Mutex<Vec<ByeCapture>>> = Arc::new(Mutex::new(Vec::new()));

    let sock_task = sock.clone();
    let count_task = bye_count.clone();
    let captured_task = byes_seen.clone();
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
                    // 200 OK with To-tag and Contact for the established dialog.
                    let mut resp = create_response(&request, StatusCode::Ok);
                    // Stamp a To-tag — needed for the dialog to settle.
                    for hdr in resp.headers.iter_mut() {
                        if let TypedHeader::To(to) = hdr {
                            if to.tag().is_none() {
                                to.set_tag("uas-bye-tag");
                            }
                            break;
                        }
                    }
                    // Echo a Contact so ACK / BYE route back to us.
                    resp.headers.push(TypedHeader::Other(
                        HeaderName::Contact,
                        HeaderValue::Raw(
                            format!("<sip:bob@127.0.0.1:{UAS_PORT}>").into_bytes(),
                        ),
                    ));
                    let bytes = Message::Response(resp).to_bytes();
                    let _ = sock_task.send_to(&bytes, from).await;
                }
                Method::Ack => {
                    // ACK to the 200 OK — fire-and-forget; nothing to send.
                }
                Method::Bye => {
                    let count = count_task.fetch_add(1, Ordering::SeqCst);
                    let x_trace_val = request
                        .raw_header_value(&HeaderName::Other(TRACE_HEADER_NAME.to_string()));
                    let has_x_trace = x_trace_val.is_some();
                    let has_authorization = request
                        .raw_header_value(&HeaderName::Authorization)
                        .is_some();
                    captured_task
                        .lock()
                        .await
                        .push((has_x_trace, x_trace_val, has_authorization));

                    if count == 0 {
                        // 401 with WWW-Authenticate on the first BYE.
                        let mut resp = create_response(&request, StatusCode::Unauthorized);
                        resp.headers.push(TypedHeader::Other(
                            HeaderName::WwwAuthenticate,
                            HeaderValue::Raw(
                                br#"Digest realm="testrealm", nonce="bye-nonce-1", algorithm=MD5, qop="auth""#
                                    .to_vec(),
                            ),
                        ));
                        let bytes = Message::Response(resp).to_bytes();
                        let _ = sock_task.send_to(&bytes, from).await;
                    } else {
                        // 200 OK on the credentialed retry.
                        let resp = create_response(&request, StatusCode::Ok);
                        let bytes = Message::Response(resp).to_bytes();
                        let _ = sock_task.send_to(&bytes, from).await;
                    }
                }
                _ => {
                    // Other methods (CANCEL, etc.) — generic 200 OK.
                    let resp = create_response(&request, StatusCode::Ok);
                    let _ = sock_task.send_to(&Message::Response(resp).to_bytes(), from).await;
                }
            }
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    // Step 1: place INVITE — must establish a dialog so the BYE goes in-dialog.
    let call_id = coord
        .invite(
            Some(format!("sip:alice@127.0.0.1:{UAC_PORT}")),
            format!("sip:bob@127.0.0.1:{UAS_PORT}"),
        )
        .with_credentials(Credentials::new("alice", "password").with_realm("testrealm"))
        .send()
        .await
        .expect("invite.send()");

    // Wait until the call reaches Active so the BYE is dispatched
    // in-dialog (the YAML row `Active + SendOutboundBye` fires).
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

    // Step 2: BYE with application extras. The credentials staged on
    // the original INVITE are still on the session.
    coord
        .bye(&call_id)
        .with_raw_header(
            HeaderName::Other(TRACE_HEADER_NAME.to_string()),
            TRACE_HEADER_VALUE,
        )
        .expect("X-Trace staging")
        .send()
        .await
        .expect("bye.send()");

    // Wait for both BYEs (initial + retry).
    let observed = timeout(Duration::from_secs(8), async {
        loop {
            if bye_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "UAS never saw 2 BYEs (count={})",
        bye_count.load(Ordering::SeqCst)
    );

    sleep(Duration::from_millis(200)).await;

    let captured = byes_seen.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "expected initial BYE + auth retry, got {captured:?}"
    );

    // INITIAL BYE: X-Trace present, no Authorization.
    let (init_has_trace, init_trace, init_has_auth) = &captured[0];
    assert!(
        *init_has_trace,
        "initial BYE must carry X-Trace; captured: {:?}",
        captured[0]
    );
    assert_eq!(
        init_trace.as_deref(),
        Some(TRACE_HEADER_VALUE),
        "initial BYE X-Trace must echo the staged value"
    );
    assert!(!*init_has_auth, "initial BYE must NOT carry Authorization");

    // RETRY BYE: X-Trace still present, Authorization now stamped.
    let (retry_has_trace, retry_trace, retry_has_auth) = &captured[1];
    assert!(
        *retry_has_trace,
        "auth retry BYE must still carry X-Trace; captured: {:?}",
        captured[1]
    );
    assert_eq!(
        retry_trace.as_deref(),
        Some(TRACE_HEADER_VALUE),
        "auth retry BYE X-Trace must match the initial — stash is single-source"
    );
    assert!(
        *retry_has_auth,
        "auth retry BYE must carry Authorization (credentialed)"
    );

    uas_handle.abort();
}
