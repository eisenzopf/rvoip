//! Integration test for RFC 3261 §10.2.8 — REGISTER + 423 Interval Too Brief.
//!
//! An in-process raw-UDP mock registrar binds a loopback port, answers the
//! first REGISTER with 423 + `Min-Expires: 1800`, and answers the retry
//! (which the client must issue with the bumped `Expires`) with 200 OK.
//! The test asserts that:
//!
//! - Exactly two REGISTERs hit the wire (initial + retry).
//! - The retry carries `Expires: 1800`.
//! - The session-core client reaches `is_registered == true` on the
//!   registration handle's session.
//!
//! The 423 parsing + retry logic under test lives in
//! `src/adapters/dialog_adapter.rs::handle_register_response` (the `423 =>`
//! arm), with a 2-retry cap.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_session_core::api::unified::{Config, Registration};
use rvoip_session_core::StreamPeer;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_auth_core::DigestAuthenticator;
use rvoip_dialog_core::transaction::utils::response_builders::create_response;

const REGISTRAR_PORT: u16 = 35180;
const CLIENT_PORT: u16 = 35181;
const SERVER_MIN_EXPIRES: u32 = 1800;
const CLIENT_INITIAL_EXPIRES: u32 = 60;
const CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35181";
const AUTH_REGISTRAR_PORT: u16 = 35182;
const AUTH_CLIENT_PORT: u16 = 35183;
const AUTH_CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35183";

/// Extract the `Expires` header value from a request as a u32.
fn extract_expires(req: &Request) -> Option<u32> {
    req.raw_header_value(&HeaderName::Expires)
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn extract_cseq(req: &Request) -> Option<u32> {
    req.cseq().map(|cseq| cseq.sequence())
}

#[tokio::test]
async fn register_423_retry_bumps_expires_and_succeeds() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    // --- Mock registrar ------------------------------------------------------

    let registrar_addr = format!("127.0.0.1:{}", REGISTRAR_PORT);
    let sock = Arc::new(
        UdpSocket::bind(&registrar_addr)
            .await
            .expect("mock registrar bind"),
    );

    let register_count = Arc::new(AtomicU32::new(0));
    let retry_expires_seen = Arc::new(Mutex::new(None::<u32>));
    let register_headers_seen = Arc::new(Mutex::new(Vec::<(String, u32, String)>::new()));

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
    let retry_expires_task = retry_expires_seen.clone();
    let register_headers_task = register_headers_seen.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let msg = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match msg {
                Message::Request(r) if r.method() == Method::Register => r,
                _ => continue,
            };

            let count = register_count_task.fetch_add(1, Ordering::SeqCst);
            rvoip_sip_core::validation::validate_wire_request(&request)
                .expect("REGISTER should be wire-valid");
            register_headers_task.lock().await.push((
                request
                    .call_id()
                    .expect("REGISTER Call-ID")
                    .value()
                    .to_string(),
                extract_cseq(&request).expect("REGISTER CSeq"),
                request
                    .raw_header_value(&HeaderName::Contact)
                    .expect("REGISTER Contact"),
            ));

            if count == 0 {
                // First REGISTER → 423 + Min-Expires. The 423 body is empty;
                // create_response copies Via/From/To/Call-ID/CSeq and adds
                // Content-Length: 0, which is exactly what we need. We then
                // append the Min-Expires header the client's 423 arm parses.
                let mut resp = create_response(&request, StatusCode::IntervalTooBrief);
                resp.headers.push(TypedHeader::Other(
                    HeaderName::MinExpires,
                    HeaderValue::Raw(SERVER_MIN_EXPIRES.to_string().into_bytes()),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            } else {
                // Retry → 200 OK. Record the Expires the client used.
                *retry_expires_task.lock().await = extract_expires(&request);

                let mut resp = create_response(&request, StatusCode::Ok);
                // Echo Contact + Expires (RFC 3261 §10.3: registrar replies
                // with the final expiry). Copy the client's Contact verbatim.
                if let Some(contact) = request.header(&HeaderName::Contact) {
                    resp.headers.push(contact.clone());
                }
                resp.headers.push(TypedHeader::Other(
                    HeaderName::Expires,
                    HeaderValue::Raw(SERVER_MIN_EXPIRES.to_string().into_bytes()),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            }
        }
    });

    // --- Session-core-v3 client ----------------------------------------------

    // Use `Config::local` as the base — TLS / SRTP / PAI / outbound
    // proxy etc. take their default-off values automatically.
    let mut config = Config::local("alice", CLIENT_PORT);
    config.media_port_start = 40000;
    config.media_port_end = 40100;

    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register_with(
            Registration::new(
                format!("sip:127.0.0.1:{}", REGISTRAR_PORT),
                "alice",
                "password",
            )
            .contact_uri(CLIENT_CONTACT)
            .expires(CLIENT_INITIAL_EXPIRES),
        )
        .await
        .expect("register_with");

    // Wait until both REGISTERs have landed on the mock (retry = count >= 2).
    let wait_for_retry = timeout(Duration::from_secs(10), async {
        loop {
            if register_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        wait_for_retry.is_ok(),
        "mock never saw 2 REGISTERs (count={})",
        register_count.load(Ordering::SeqCst)
    );

    // Give the 200 OK time to reach the client and propagate into session state.
    sleep(Duration::from_millis(500)).await;

    // --- Assertions ----------------------------------------------------------

    assert_eq!(
        register_count.load(Ordering::SeqCst),
        2,
        "expected exactly 2 REGISTERs (initial + retry), got {}",
        register_count.load(Ordering::SeqCst)
    );

    let retry_expires = *retry_expires_seen.lock().await;
    assert_eq!(
        retry_expires,
        Some(SERVER_MIN_EXPIRES),
        "retry REGISTER should carry Expires={} (the server's Min-Expires), got {:?}",
        SERVER_MIN_EXPIRES,
        retry_expires,
    );

    let headers_seen = register_headers_seen.lock().await;
    assert_eq!(headers_seen.len(), 2, "expected two captured REGISTERs");
    assert_eq!(
        headers_seen[0].0, headers_seen[1].0,
        "REGISTER retry should reuse Call-ID"
    );
    assert!(
        headers_seen[1].1 > headers_seen[0].1,
        "REGISTER retry should increment CSeq: first={}, retry={}",
        headers_seen[0].1,
        headers_seen[1].1
    );
    assert!(
        headers_seen
            .iter()
            .all(|(_, _, contact)| contact == "<sip:alice@127.0.0.1:35181>"),
        "REGISTER Contact should be local UA contact, got {:?}",
        headers_seen.iter().map(|(_, _, c)| c).collect::<Vec<_>>()
    );

    // `is_registered` flips true on 200 OK to the retry. Currently the
    // public API doesn't also publish `Event::RegistrationSuccess` on the
    // 423-retry path; the flag is the source of truth.
    let registered = peer
        .is_registered(&handle)
        .await
        .expect("is_registered query");
    assert!(
        registered,
        "session should be marked registered after 200 OK to the retry"
    );

    registrar_handle.abort();
}

#[tokio::test]
async fn register_401_retry_reuses_call_id_and_increments_cseq() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let registrar_addr = format!("127.0.0.1:{}", AUTH_REGISTRAR_PORT);
    let sock = Arc::new(
        UdpSocket::bind(&registrar_addr)
            .await
            .expect("auth mock registrar bind"),
    );

    let register_count = Arc::new(AtomicU32::new(0));
    let register_headers_seen = Arc::new(Mutex::new(Vec::<(
        String,
        u32,
        String,
        bool,
        Option<bool>,
        Option<String>,
    )>::new()));

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
    let register_headers_task = register_headers_seen.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let msg = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match msg {
                Message::Request(r) if r.method() == Method::Register => r,
                _ => continue,
            };

            let count = register_count_task.fetch_add(1, Ordering::SeqCst);
            rvoip_sip_core::validation::validate_wire_request(&request)
                .expect("REGISTER should be wire-valid");
            let authorization = request.raw_header_value(&HeaderName::Authorization);
            let auth_validation = authorization.as_ref().map(|header| {
                let parsed = DigestAuthenticator::parse_authorization(header)
                    .expect("Authorization should parse");
                let valid = DigestAuthenticator::new("testrealm")
                    .validate_response(&parsed, "REGISTER", "password")
                    .expect("Authorization should validate");
                (valid, parsed.uri)
            });

            register_headers_task.lock().await.push((
                request
                    .call_id()
                    .expect("REGISTER Call-ID")
                    .value()
                    .to_string(),
                extract_cseq(&request).expect("REGISTER CSeq"),
                request
                    .raw_header_value(&HeaderName::Contact)
                    .expect("REGISTER Contact"),
                authorization.is_some(),
                auth_validation.as_ref().map(|(valid, _)| *valid),
                auth_validation.map(|(_, uri)| uri),
            ));

            if count == 0 {
                let mut resp = create_response(&request, StatusCode::Unauthorized);
                resp.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(
                        br#"Digest realm="testrealm", nonce="nonce123", algorithm=MD5, qop="auth""#
                            .to_vec(),
                    ),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            } else {
                let mut resp = create_response(&request, StatusCode::Ok);
                if let Some(contact) = request.header(&HeaderName::Contact) {
                    resp.headers.push(contact.clone());
                }
                resp.headers.push(TypedHeader::Other(
                    HeaderName::Expires,
                    HeaderValue::Raw("3600".as_bytes().to_vec()),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            }
        }
    });

    let mut config = Config::local("alice", AUTH_CLIENT_PORT);
    config.media_port_start = 40110;
    config.media_port_end = 40120;

    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register_with(
            Registration::new(
                format!("sip:127.0.0.1:{}", AUTH_REGISTRAR_PORT),
                "alice",
                "password",
            )
            .contact_uri(AUTH_CLIENT_CONTACT),
        )
        .await
        .expect("register_with");

    let wait_for_retry = timeout(Duration::from_secs(10), async {
        loop {
            if register_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        wait_for_retry.is_ok(),
        "mock never saw auth retry (count={})",
        register_count.load(Ordering::SeqCst)
    );

    sleep(Duration::from_millis(500)).await;

    let headers_seen = register_headers_seen.lock().await;
    assert_eq!(
        headers_seen.len(),
        2,
        "expected initial REGISTER and auth retry"
    );
    assert_eq!(
        headers_seen[0].0, headers_seen[1].0,
        "REGISTER auth retry should reuse Call-ID"
    );
    assert!(
        headers_seen[1].1 > headers_seen[0].1,
        "REGISTER auth retry should increment CSeq: first={}, retry={}",
        headers_seen[0].1,
        headers_seen[1].1
    );
    assert!(
        !headers_seen[0].3,
        "initial REGISTER should not include Authorization"
    );
    assert!(headers_seen[1].3, "auth retry should include Authorization");
    assert!(
        headers_seen
            .iter()
            .all(|(_, _, contact, _, _, _)| contact == "<sip:alice@127.0.0.1:35183>"),
        "REGISTER Contact should be local UA contact, got {:?}",
        headers_seen
            .iter()
            .map(|(_, _, c, _, _, _)| c)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        headers_seen[1].4,
        Some(true),
        "auth retry Authorization digest should validate against the registrar challenge"
    );
    assert_eq!(
        headers_seen[1].5.as_deref(),
        Some(format!("sip:127.0.0.1:{}", AUTH_REGISTRAR_PORT).as_str()),
        "auth retry Authorization uri should match REGISTER Request-URI"
    );

    assert!(
        peer.is_registered(&handle)
            .await
            .expect("is_registered query"),
        "session should be marked registered after 200 OK to auth retry"
    );

    registrar_handle.abort();
}
