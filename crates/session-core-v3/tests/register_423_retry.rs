//! Integration test for RFC 3261 §10.2.8 — REGISTER + 423 Interval Too Brief.
//!
//! An in-process raw-UDP mock registrar binds a loopback port, answers the
//! first REGISTER with 423 + `Min-Expires: 1800`, and answers the retry
//! (which the client must issue with the bumped `Expires`) with 200 OK.
//! The test asserts that:
//!
//! - Exactly two REGISTERs hit the wire (initial + retry).
//! - The retry carries `Expires: 1800`.
//! - The session-core-v3 client reaches `is_registered == true` on the
//!   registration handle's session.
//!
//! The 423 parsing + retry logic under test lives in
//! `src/adapters/dialog_adapter.rs::handle_register_response` (the `423 =>`
//! arm), with a 2-retry cap.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_session_core_v3::api::unified::{Config, Registration};
use rvoip_session_core_v3::StreamPeer;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_dialog_core::transaction::utils::response_builders::create_response;

const REGISTRAR_PORT: u16 = 35180;
const CLIENT_PORT: u16 = 35181;
const SERVER_MIN_EXPIRES: u32 = 1800;
const CLIENT_INITIAL_EXPIRES: u32 = 60;

/// Extract the `Expires` header value from a request as a u32.
fn extract_expires(req: &Request) -> Option<u32> {
    req.raw_header_value(&HeaderName::Expires)
        .and_then(|s| s.trim().parse::<u32>().ok())
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

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
    let retry_expires_task = retry_expires_seen.clone();
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

    let config = Config {
        local_ip: "127.0.0.1".parse().unwrap(),
        sip_port: CLIENT_PORT,
        bind_addr: format!("127.0.0.1:{}", CLIENT_PORT).parse().unwrap(),
        local_uri: format!("sip:alice@127.0.0.1:{}", CLIENT_PORT),
        media_port_start: 40000,
        media_port_end: 40100,
        state_table_path: None,
        use_100rel: Default::default(),
        session_timer_secs: None,
        session_timer_min_se: 90,
        credentials: None,
    };

    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register_with(
            Registration::new(
                format!("sip:127.0.0.1:{}", REGISTRAR_PORT),
                "alice",
                "password",
            )
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
