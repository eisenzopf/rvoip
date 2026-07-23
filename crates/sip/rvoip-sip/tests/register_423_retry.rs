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

use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::unified::{Config, RegistrationStatus};
use rvoip_sip::StreamPeer;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_auth_core::DigestAuthenticator;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const REGISTRAR_PORT: u16 = 35180;
const CLIENT_PORT: u16 = 35181;
const SERVER_MIN_EXPIRES: u32 = 1800;
const CLIENT_INITIAL_EXPIRES: u32 = 60;
const CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35181";
const AUTH_REGISTRAR_PORT: u16 = 35182;
const AUTH_CLIENT_PORT: u16 = 35183;
const AUTH_CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35183";
const INFO_REGISTRAR_PORT: u16 = 35184;
const INFO_CLIENT_PORT: u16 = 35185;
const INFO_CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35185";
const FAIL_REGISTRAR_PORT: u16 = 35186;
const FAIL_CLIENT_PORT: u16 = 35187;
const FAIL_CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35187";
const PROXY_AUTH_REGISTRAR_PORT: u16 = 35188;
const PROXY_AUTH_CLIENT_PORT: u16 = 35189;
const PROXY_AUTH_CLIENT_CONTACT: &str = "sip:alice@127.0.0.1:35189";

/// Extract the `Expires` header value from a request as a u32.
fn extract_expires(req: &Request) -> Option<u32> {
    req.raw_header_value(&HeaderName::Expires)
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn extract_cseq(req: &Request) -> Option<u32> {
    req.cseq().map(|cseq| cseq.sequence())
}

fn response_with_contact_and_expires(request: &Request, expires: u32) -> Message {
    let mut resp = create_response(request, StatusCode::Ok);
    if let Some(contact) = request.header(&HeaderName::Contact) {
        resp.headers.push(contact.clone());
    }
    resp.headers.push(TypedHeader::Other(
        HeaderName::Expires,
        HeaderValue::Raw(expires.to_string().into_bytes()),
    ));
    Message::Response(resp)
}

fn response_with_contact_param_expires(
    request: &Request,
    contact_expires: Option<u32>,
    header_expires: Option<u32>,
    service_route: Option<&str>,
    gruu: Option<(&str, &str)>,
) -> Message {
    let mut resp = create_response(request, StatusCode::Ok);
    if let Some(TypedHeader::Contact(contact)) = request.header(&HeaderName::Contact) {
        let mut contact = contact.clone();
        if let Some(expires) = contact_expires {
            contact.set_expires(expires);
        }
        if let Some((pub_gruu, temp_gruu)) = gruu {
            for address in contact.addresses_mut() {
                rvoip_sip_core::types::outbound::set_gruu_contact_params(
                    address,
                    &rvoip_sip_core::types::outbound::GruuContactParams {
                        pub_gruu: Some(pub_gruu.to_string()),
                        temp_gruu: Some(temp_gruu.to_string()),
                    },
                );
            }
        }
        resp.headers.push(TypedHeader::Contact(contact));
    } else if let Some(contact) = request.header(&HeaderName::Contact) {
        resp.headers.push(contact.clone());
    }
    if let Some(expires) = header_expires {
        resp.headers.push(TypedHeader::Other(
            HeaderName::Expires,
            HeaderValue::Raw(expires.to_string().into_bytes()),
        ));
    }
    if let Some(route) = service_route {
        resp.headers.push(TypedHeader::ServiceRoute(
            route.parse().expect("test Service-Route should parse"),
        ));
    }
    Message::Response(resp)
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
    let register_headers_seen = Arc::new(Mutex::new(
        Vec::<(String, u32, String, Option<String>)>::new(),
    ));

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
                request.raw_header_value(&HeaderName::Other("X-Register-423-Canary".to_string())),
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

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", REGISTRAR_PORT),
            "alice",
            "password",
        )
        .with_contact_uri(CLIENT_CONTACT)
        .with_expires(CLIENT_INITIAL_EXPIRES)
        .with_raw_header(
            HeaderName::Other("X-Register-423-Canary".to_string()),
            "retained-across-423",
        )
        .expect("custom 423 retry header")
        .send()
        .await
        .expect("register");

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
            .all(|(_, _, contact, _)| contact == "<sip:alice@127.0.0.1:35181>"),
        "REGISTER Contact should be local UA contact, got {:?}",
        headers_seen
            .iter()
            .map(|(_, _, contact, _)| contact)
            .collect::<Vec<_>>()
    );
    assert!(
        headers_seen
            .iter()
            .all(|(_, _, _, canary)| canary.as_deref() == Some("retained-across-423")),
        "423 retry must retain the initial REGISTER custom-header snapshot"
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

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", AUTH_REGISTRAR_PORT),
            "alice",
            "password",
        )
        .with_contact_uri(AUTH_CLIENT_CONTACT)
        .send()
        .await
        .expect("register");

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

#[tokio::test]
async fn register_407_retry_uses_proxy_authorization() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let registrar_addr = format!("127.0.0.1:{}", PROXY_AUTH_REGISTRAR_PORT);
    let sock = Arc::new(
        UdpSocket::bind(&registrar_addr)
            .await
            .expect("proxy auth mock registrar bind"),
    );

    let register_count = Arc::new(AtomicU32::new(0));
    let register_headers_seen =
        Arc::new(Mutex::new(
            Vec::<(bool, bool, Option<bool>, Option<String>)>::new(),
        ));

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
            let proxy_authorization = request.raw_header_value(&HeaderName::ProxyAuthorization);
            let proxy_auth_validation = proxy_authorization.as_ref().map(|header| {
                let parsed = DigestAuthenticator::parse_authorization(header)
                    .expect("Proxy-Authorization should parse");
                let valid = DigestAuthenticator::new("testrealm")
                    .validate_response(&parsed, "REGISTER", "password")
                    .expect("Proxy-Authorization should validate");
                (valid, parsed.uri)
            });

            register_headers_task.lock().await.push((
                authorization.is_some(),
                proxy_authorization.is_some(),
                proxy_auth_validation.as_ref().map(|(valid, _)| *valid),
                proxy_auth_validation.map(|(_, uri)| uri),
            ));

            if count == 0 {
                let mut resp = create_response(&request, StatusCode::ProxyAuthenticationRequired);
                resp.headers.push(TypedHeader::Other(
                    HeaderName::ProxyAuthenticate,
                    HeaderValue::Raw(
                        br#"Digest realm="testrealm", nonce="proxy-nonce", algorithm=MD5, qop="auth""#
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

    let mut config = Config::local("alice", PROXY_AUTH_CLIENT_PORT);
    config.media_port_start = 40130;
    config.media_port_end = 40140;

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", PROXY_AUTH_REGISTRAR_PORT),
            "alice",
            "password",
        )
        .with_contact_uri(PROXY_AUTH_CLIENT_CONTACT)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(10), async {
        loop {
            if register_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("mock never saw proxy auth retry");

    sleep(Duration::from_millis(500)).await;

    let headers_seen = register_headers_seen.lock().await;
    assert_eq!(headers_seen.len(), 2);
    assert!(
        !headers_seen[0].0 && !headers_seen[0].1,
        "initial REGISTER should not include auth headers"
    );
    assert!(
        !headers_seen[1].0,
        "407 REGISTER retry must not include Authorization"
    );
    assert!(
        headers_seen[1].1,
        "407 REGISTER retry must include Proxy-Authorization"
    );
    assert_eq!(headers_seen[1].2, Some(true));
    assert_eq!(
        headers_seen[1].3.as_deref(),
        Some(format!("sip:127.0.0.1:{}", PROXY_AUTH_REGISTRAR_PORT).as_str())
    );
    assert!(
        peer.is_registered(&handle)
            .await
            .expect("is_registered query"),
        "session should be marked registered after 200 OK to proxy auth retry"
    );

    registrar_handle.abort();
}

#[tokio::test]
async fn manual_refresh_auth_and_stale_retry_preserve_one_request_snapshot() {
    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("manual refresh registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let seen = Arc::new(Mutex::new(Vec::<(
        String,
        u32,
        u32,
        Option<String>,
        bool,
        Option<String>,
    )>::new()));

    let sock_task = Arc::clone(&sock);
    let seen_task = Arc::clone(&seen);
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }

            let authorization = request.raw_header_value(&HeaderName::Authorization);
            let auth_nonce = authorization.as_ref().map(|header| {
                DigestAuthenticator::parse_authorization(header)
                    .expect("refresh Authorization should parse")
                    .nonce
            });
            let custom = request
                .raw_header_value(&HeaderName::Other("X-Register-Refresh-Canary".to_string()));
            let request_index = {
                let mut captured = seen_task.lock().await;
                captured.push((
                    request
                        .call_id()
                        .expect("REGISTER Call-ID")
                        .value()
                        .to_string(),
                    extract_cseq(&request).expect("REGISTER CSeq"),
                    extract_expires(&request).expect("REGISTER Expires"),
                    custom,
                    authorization.is_some(),
                    auth_nonce,
                ));
                captured.len() - 1
            };

            let response = match request_index {
                0 => response_with_contact_and_expires(&request, 300),
                1 => {
                    let mut response = create_response(&request, StatusCode::Unauthorized);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::WwwAuthenticate,
                        HeaderValue::Raw(
                            br#"Digest realm="refresh-realm", nonce="refresh-nonce-1", algorithm=MD5, qop="auth""#
                                .to_vec(),
                        ),
                    ));
                    Message::Response(response)
                }
                2 => {
                    let mut response = create_response(&request, StatusCode::Unauthorized);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::WwwAuthenticate,
                        HeaderValue::Raw(
                            br#"Digest realm="refresh-realm", nonce="refresh-nonce-2", algorithm=MD5, qop="auth", stale=true"#
                                .to_vec(),
                        ),
                    ));
                    Message::Response(response)
                }
                _ => response_with_contact_and_expires(&request, 180),
            };
            let _ = sock_task.send_to(&response.to_bytes(), from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40300;
    config.media_port_end = 40310;
    config.registration_auto_refresh = false;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{registrar_port}"),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40300")
        .with_expires(300)
        .send()
        .await
        .expect("initial register");

    peer.control()
        .coordinator()
        .refresh(&handle)
        .with_expires(180)
        .with_raw_header(
            HeaderName::Other("X-Register-Refresh-Canary".to_string()),
            "retained-across-refresh-auth",
        )
        .expect("custom refresh header")
        .send()
        .await
        .expect("challenged manual refresh");

    timeout(Duration::from_secs(5), async {
        loop {
            if seen.lock().await.len() >= 4 {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("initial plus challenged/stale refresh attempts");

    let captured = seen.lock().await;
    assert_eq!(captured.len(), 4, "no duplicate REGISTER attempts");
    assert!(
        captured.windows(2).all(|pair| pair[0].0 == pair[1].0),
        "initial and every refresh retry must reuse one Call-ID"
    );
    assert!(
        captured.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "every logical REGISTER retry must advance CSeq: {:?}",
        captured.iter().map(|entry| entry.1).collect::<Vec<_>>()
    );
    assert_eq!(captured[0].2, 300);
    assert!(captured[1..].iter().all(|entry| entry.2 == 180));
    assert!(captured[0].3.is_none());
    assert!(captured[1..]
        .iter()
        .all(|entry| { entry.3.as_deref() == Some("retained-across-refresh-auth") }));
    assert!(!captured[1].4, "first refresh should be unchallenged");
    assert_eq!(captured[2].5.as_deref(), Some("refresh-nonce-1"));
    assert_eq!(captured[3].5.as_deref(), Some("refresh-nonce-2"));
    drop(captured);

    assert!(
        peer.is_registered(&handle)
            .await
            .expect("registered after manual refresh"),
        "successful stale recovery must keep the registration active"
    );
    peer.control()
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .expect("shutdown");
    registrar_handle.abort();
}

#[tokio::test]
async fn challenged_unregister_retries_proxy_auth_and_stale_with_expires_zero() {
    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("challenged unregister registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let seen = Arc::new(Mutex::new(Vec::<(
        String,
        u32,
        u32,
        bool,
        bool,
        Option<String>,
    )>::new()));

    let sock_task = Arc::clone(&sock);
    let seen_task = Arc::clone(&seen);
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }

            let authorization = request.raw_header_value(&HeaderName::Authorization);
            let proxy_authorization = request.raw_header_value(&HeaderName::ProxyAuthorization);
            let proxy_nonce = proxy_authorization.as_ref().map(|header| {
                DigestAuthenticator::parse_authorization(header)
                    .expect("unregister Proxy-Authorization should parse")
                    .nonce
            });
            let request_index = {
                let mut captured = seen_task.lock().await;
                captured.push((
                    request
                        .call_id()
                        .expect("REGISTER Call-ID")
                        .value()
                        .to_string(),
                    extract_cseq(&request).expect("REGISTER CSeq"),
                    extract_expires(&request).expect("REGISTER Expires"),
                    authorization.is_some(),
                    proxy_authorization.is_some(),
                    proxy_nonce,
                ));
                captured.len() - 1
            };

            let response = match request_index {
                0 => response_with_contact_and_expires(&request, 300),
                1 => {
                    let mut response =
                        create_response(&request, StatusCode::ProxyAuthenticationRequired);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::ProxyAuthenticate,
                        HeaderValue::Raw(
                            br#"Digest realm="unregister-realm", nonce="unregister-nonce-1", algorithm=MD5, qop="auth""#
                                .to_vec(),
                        ),
                    ));
                    Message::Response(response)
                }
                2 => {
                    let mut response =
                        create_response(&request, StatusCode::ProxyAuthenticationRequired);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::ProxyAuthenticate,
                        HeaderValue::Raw(
                            br#"Digest realm="unregister-realm", nonce="unregister-nonce-2", algorithm=MD5, qop="auth", stale=true"#
                                .to_vec(),
                        ),
                    ));
                    Message::Response(response)
                }
                _ => response_with_contact_and_expires(&request, 0),
            };
            let _ = sock_task.send_to(&response.to_bytes(), from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40320;
    config.media_port_end = 40330;
    config.registration_auto_refresh = false;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{registrar_port}"),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40320")
        .with_expires(300)
        .send()
        .await
        .expect("initial register");

    peer.control()
        .coordinator()
        .unregister_and_wait(&handle, Some(Duration::from_secs(5)))
        .await
        .expect("challenged unregister");

    timeout(Duration::from_secs(5), async {
        loop {
            if seen.lock().await.len() >= 4 {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("initial plus challenged/stale unregister attempts");

    let captured = seen.lock().await;
    assert_eq!(captured.len(), 4, "no duplicate unregister attempts");
    assert!(captured.windows(2).all(|pair| pair[0].0 == pair[1].0));
    assert!(captured.windows(2).all(|pair| pair[1].1 > pair[0].1));
    assert_eq!(captured[0].2, 300);
    assert!(
        captured[1..].iter().all(|entry| entry.2 == 0),
        "every challenged unregister retry must retain Expires: 0"
    );
    assert!(captured.iter().all(|entry| !entry.3));
    assert!(!captured[1].4);
    assert_eq!(captured[2].5.as_deref(), Some("unregister-nonce-1"));
    assert_eq!(captured[3].5.as_deref(), Some("unregister-nonce-2"));
    drop(captured);

    assert!(
        !peer
            .is_registered(&handle)
            .await
            .expect("unregistered state"),
        "successful challenged unregister must remove the active binding"
    );
    peer.control()
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .expect("shutdown");
    registrar_handle.abort();
}

#[tokio::test]
async fn registration_info_tracks_success_refresh_shape_and_unregister_wait() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let registrar_addr = format!("127.0.0.1:{}", INFO_REGISTRAR_PORT);
    let sock = Arc::new(
        UdpSocket::bind(&registrar_addr)
            .await
            .expect("info mock registrar bind"),
    );
    let register_count = Arc::new(AtomicU32::new(0));
    let unregister_seen = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
    let unregister_seen_task = unregister_seen.clone();
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

            rvoip_sip_core::validation::validate_wire_request(&request)
                .expect("REGISTER should be wire-valid");
            register_count_task.fetch_add(1, Ordering::SeqCst);

            let is_unregister = extract_expires(&request) == Some(0)
                || request
                    .raw_header_value(&HeaderName::Contact)
                    .map(|contact| contact.contains("expires=0"))
                    .unwrap_or(false);
            if is_unregister {
                unregister_seen_task.fetch_add(1, Ordering::SeqCst);
            }

            let expires = if is_unregister { 0 } else { 300 };
            let bytes = response_with_contact_and_expires(&request, expires).to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", INFO_CLIENT_PORT);
    config.media_port_start = 40130;
    config.media_port_end = 40140;

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", INFO_REGISTRAR_PORT),
            "alice",
            "password",
        )
        .with_contact_uri(INFO_CLIENT_CONTACT)
        .with_expires(300)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(5), async {
        loop {
            if peer
                .is_registered(&handle)
                .await
                .expect("is_registered query")
            {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration should become registered");

    let info = peer
        .control()
        .coordinator()
        .registration_info(&handle)
        .await
        .expect("registration_info");
    assert_eq!(info.status, RegistrationStatus::Registered);
    assert_eq!(
        info.registrar.as_deref(),
        Some(format!("sip:127.0.0.1:{}", INFO_REGISTRAR_PORT).as_str())
    );
    assert_eq!(info.contact.as_deref(), Some(INFO_CLIENT_CONTACT));
    assert_eq!(info.expires_secs, Some(300));
    assert_eq!(info.retry_count, 0);
    assert!(info.last_failure.is_none());
    let next_refresh = info
        .next_refresh_in
        .expect("registered metadata should include refresh timing");
    assert!(
        next_refresh <= Duration::from_secs(300),
        "refresh metadata should not exceed registration expiry: {:?}",
        next_refresh
    );

    peer.control()
        .coordinator()
        .unregister_and_wait(&handle, Some(Duration::from_secs(5)))
        .await
        .expect("unregister_and_wait");

    timeout(Duration::from_secs(5), async {
        loop {
            if unregister_seen.load(Ordering::SeqCst) > 0 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("mock registrar should see Expires: 0 REGISTER");

    let info = peer
        .control()
        .coordinator()
        .registration_info(&handle)
        .await
        .expect("registration_info after unregister");
    assert_eq!(info.status, RegistrationStatus::Unregistered);
    assert_eq!(info.retry_count, 0);
    assert!(info.last_failure.is_none());

    assert!(
        register_count.load(Ordering::SeqCst) >= 2,
        "expected initial REGISTER plus unregister REGISTER"
    );

    registrar_handle.abort();
}

#[tokio::test]
async fn registration_info_tracks_auth_failure_metadata() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let registrar_addr = format!("127.0.0.1:{}", FAIL_REGISTRAR_PORT);
    let sock = Arc::new(
        UdpSocket::bind(&registrar_addr)
            .await
            .expect("failure mock registrar bind"),
    );
    let register_count = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
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

            register_count_task.fetch_add(1, Ordering::SeqCst);
            let mut resp = create_response(&request, StatusCode::Unauthorized);
            resp.headers.push(TypedHeader::Other(
                HeaderName::WwwAuthenticate,
                HeaderValue::Raw(
                    br#"Digest realm="testrealm", nonce="nonce-fail", algorithm=MD5, qop="auth""#
                        .to_vec(),
                ),
            ));
            let bytes = Message::Response(resp).to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", FAIL_CLIENT_PORT);
    config.media_port_start = 40150;
    config.media_port_end = 40160;

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", FAIL_REGISTRAR_PORT),
            "alice",
            "wrong-password",
        )
        .with_contact_uri(FAIL_CLIENT_CONTACT)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(5), async {
        loop {
            if register_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("mock registrar should see initial REGISTER and auth retry");

    timeout(Duration::from_secs(5), async {
        loop {
            let info = peer
                .control()
                .coordinator()
                .registration_info(&handle)
                .await
                .expect("registration_info");
            if info.status == RegistrationStatus::Failed {
                assert_eq!(info.retry_count, 1);
                assert!(
                    info.last_failure
                        .as_deref()
                        .unwrap_or_default()
                        .contains("1 retry"),
                    "failure metadata should describe retry count, got {:?}",
                    info.last_failure
                );
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration_info should report failed status after repeated 401");

    assert!(
        !peer
            .is_registered(&handle)
            .await
            .expect("is_registered query"),
        "failed registration should not be marked registered"
    );

    registrar_handle.abort();
}

#[tokio::test]
async fn registration_info_uses_contact_expires_and_exposes_route_and_gruu() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("metadata mock registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let register_count = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }
            register_count_task.fetch_add(1, Ordering::SeqCst);
            let bytes = response_with_contact_param_expires(
                &request,
                Some(120),
                Some(300),
                Some("<sip:service.example.com;lr>"),
                Some((
                    "sip:alice-pub@example.com;gr",
                    "sip:alice-temp@example.com;gr",
                )),
            )
            .to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40170;
    config.media_port_end = 40180;
    config.registration_refresh_jitter_percent = 0;

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", registrar_port),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40170")
        .with_expires(300)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(5), async {
        loop {
            if peer
                .is_registered(&handle)
                .await
                .expect("is_registered query")
            {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration should succeed");

    let info = peer
        .control()
        .coordinator()
        .registration_info(&handle)
        .await
        .expect("registration_info");
    assert_eq!(info.status, RegistrationStatus::Registered);
    assert_eq!(info.accepted_expires_secs, Some(120));
    assert_eq!(info.expires_secs, Some(120));
    assert!(info.registered_at.is_some());
    assert!(info.next_refresh_at.is_some());
    assert!(info.next_refresh_in.expect("next refresh") <= Duration::from_secs(102));
    assert_eq!(
        info.service_route,
        Some(vec!["sip:service.example.com;lr".to_string()])
    );
    assert_eq!(
        info.pub_gruu.as_deref(),
        Some("sip:alice-pub@example.com;gr")
    );
    assert_eq!(
        info.temp_gruu.as_deref(),
        Some("sip:alice-temp@example.com;gr")
    );

    peer.control()
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .expect("shutdown");
    registrar_handle.abort();
}

#[tokio::test]
async fn registration_accepted_expiry_falls_back_to_header_then_request() {
    async fn run_case(header_expires: Option<u32>, requested: u32) -> u32 {
        let sock = Arc::new(
            UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("expiry mock registrar bind"),
        );
        let registrar_port = sock.local_addr().expect("registrar addr").port();

        let sock_task = sock.clone();
        let registrar_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                let (n, from) = match sock_task.recv_from(&mut buf).await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                    continue;
                };
                if request.method() != Method::Register {
                    continue;
                }
                let bytes =
                    response_with_contact_param_expires(&request, None, header_expires, None, None)
                        .to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            }
        });

        let mut config = Config::local("alice", 0);
        config.media_port_start = 40190;
        config.media_port_end = 40200;
        config.registration_auto_refresh = false;
        let peer = StreamPeer::with_config(config).await.expect("peer");
        let handle = peer
            .register(
                format!("sip:127.0.0.1:{}", registrar_port),
                "alice",
                "password",
            )
            .with_contact_uri("sip:alice@127.0.0.1:40190")
            .with_expires(requested)
            .send()
            .await
            .expect("register");

        timeout(Duration::from_secs(5), async {
            loop {
                if peer
                    .is_registered(&handle)
                    .await
                    .expect("is_registered query")
                {
                    return;
                }
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("registration should succeed");

        let accepted = peer
            .control()
            .coordinator()
            .registration_info(&handle)
            .await
            .expect("registration_info")
            .accepted_expires_secs
            .expect("accepted expires");
        peer.control()
            .coordinator()
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .expect("shutdown");
        registrar_handle.abort();
        accepted
    }

    assert_eq!(run_case(Some(180), 300).await, 180);
    assert_eq!(run_case(None, 240).await, 240);
}

#[tokio::test]
async fn automatic_registration_refresh_reuses_call_id_and_increments_cseq() {
    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("refresh mock registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let seen = Arc::new(Mutex::new(Vec::<(String, u32, u32)>::new()));

    let sock_task = sock.clone();
    let seen_task = seen.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }
            seen_task.lock().await.push((
                request.call_id().expect("Call-ID").value().to_string(),
                extract_cseq(&request).expect("CSeq"),
                extract_expires(&request).expect("Expires"),
            ));
            let bytes =
                response_with_contact_param_expires(&request, None, None, None, None).to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40210;
    config.media_port_end = 40220;
    config.registration_refresh_jitter_percent = 0;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", registrar_port),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40210")
        .with_expires(2)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(5), async {
        loop {
            if seen.lock().await.len() >= 2 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("automatic refresh should send a second REGISTER");

    let seen = seen.lock().await;
    assert_eq!(seen[0].0, seen[1].0, "refresh should reuse Call-ID");
    assert!(seen[1].1 > seen[0].1, "refresh should increment CSeq");
    assert_eq!(seen[0].2, 2);
    assert_eq!(seen[1].2, 2);
    drop(seen);

    peer.control()
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .expect("shutdown");
    let _ = handle;
    registrar_handle.abort();
}

#[tokio::test]
async fn manual_refresh_serializes_with_due_automatic_refresh_and_advances_cseq_once() {
    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("concurrent refresh mock registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let seen = Arc::new(Mutex::new(Vec::<(String, u32, u32)>::new()));
    let manual_seen = Arc::new(tokio::sync::Notify::new());
    let (release_manual, release_manual_rx) = tokio::sync::oneshot::channel::<()>();

    let sock_task = sock.clone();
    let seen_task = seen.clone();
    let manual_seen_task = manual_seen.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        let mut release_manual_rx = Some(release_manual_rx);
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }
            let observation = (
                request.call_id().expect("Call-ID").value().to_string(),
                extract_cseq(&request).expect("CSeq"),
                extract_expires(&request).expect("Expires"),
            );
            let request_number = {
                let mut seen = seen_task.lock().await;
                // The deliberately delayed response crosses Timer E, so the
                // UDP client transaction retransmits the same REGISTER. This
                // test is about refresh ownership: count each Call-ID/CSeq
                // transaction once and continue to reject any stale refresh,
                // which necessarily owns a new CSeq.
                if !seen
                    .iter()
                    .any(|item| item.0 == observation.0 && item.1 == observation.1)
                {
                    seen.push(observation);
                }
                seen.len()
            };

            if request_number == 2 {
                manual_seen_task.notify_one();
                if let Some(release) = release_manual_rx.take() {
                    let _ = release.await;
                }
            }

            let accepted_expires = if request_number == 1 { 2 } else { 30 };
            let bytes = response_with_contact_and_expires(&request, accepted_expires).to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40221;
    config.media_port_end = 40229;
    config.registration_refresh_jitter_percent = 0;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{registrar_port}"),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40221")
        .with_expires(2)
        .send()
        .await
        .expect("initial registration");

    timeout(Duration::from_secs(5), async {
        loop {
            if peer.is_registered(&handle).await.expect("registered query") {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("initial registration should succeed");

    let coordinator = Arc::clone(peer.control().coordinator());
    let manual_handle = handle.clone();
    let manual = tokio::spawn(async move {
        coordinator
            .refresh(&manual_handle)
            .with_expires(30)
            .send()
            .await
    });
    timeout(Duration::from_secs(2), manual_seen.notified())
        .await
        .expect("manual refresh reached registrar");

    // The original two-second binding makes its automatic refresh due after
    // one second. Keep the manual response open past that point: the automatic
    // owner must wait behind the exact session lane and must then be replaced
    // by the manual success's new 30-second schedule, not emit a stale CSeq.
    sleep(Duration::from_millis(1500)).await;
    assert_eq!(seen.lock().await.len(), 2);
    release_manual.send(()).expect("release manual response");
    manual
        .await
        .expect("join manual refresh")
        .expect("manual refresh succeeds");
    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        seen.lock().await.len(),
        2,
        "due automatic refresh must not survive the committed manual replacement"
    );

    peer.control()
        .coordinator()
        .refresh(&handle)
        .with_expires(30)
        .send()
        .await
        .expect("next manual refresh");
    let seen = seen.lock().await.clone();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].0, seen[1].0);
    assert_eq!(seen[1].0, seen[2].0);
    assert_eq!(seen[1].1, seen[0].1 + 1);
    assert_eq!(seen[2].1, seen[1].1 + 1);
    assert_eq!([seen[0].2, seen[1].2, seen[2].2], [2, 30, 30]);

    peer.control()
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown concurrent refresh coordinator");
    registrar_handle.abort();
}

#[tokio::test]
async fn unregister_aborts_pending_automatic_refresh() {
    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("unregister refresh mock registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let register_count = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let register_count_task = register_count.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }
            register_count_task.fetch_add(1, Ordering::SeqCst);
            let expires = extract_expires(&request).unwrap_or(2);
            let bytes = response_with_contact_and_expires(&request, expires).to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40230;
    config.media_port_end = 40240;
    config.registration_refresh_jitter_percent = 0;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", registrar_port),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40230")
        .with_expires(2)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(5), async {
        loop {
            if peer
                .is_registered(&handle)
                .await
                .expect("is_registered query")
            {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration should succeed");

    peer.control()
        .coordinator()
        .unregister_and_wait(&handle, Some(Duration::from_secs(5)))
        .await
        .expect("unregister");
    sleep(Duration::from_millis(1500)).await;

    assert_eq!(
        register_count.load(Ordering::SeqCst),
        2,
        "expected initial REGISTER plus unregister, with no later refresh"
    );

    registrar_handle.abort();
}

#[tokio::test]
async fn stream_peer_shutdown_gracefully_unregisters_active_registration() {
    let sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("shutdown mock registrar bind"),
    );
    let registrar_port = sock.local_addr().expect("registrar addr").port();
    let unregister_seen = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let unregister_seen_task = unregister_seen.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }
            let expires = extract_expires(&request).unwrap_or(300);
            if expires == 0 {
                unregister_seen_task.fetch_add(1, Ordering::SeqCst);
            }
            let bytes = response_with_contact_and_expires(&request, expires).to_bytes();
            let _ = sock_task.send_to(&bytes, from).await;
        }
    });

    let mut config = Config::local("alice", 0);
    config.media_port_start = 40250;
    config.media_port_end = 40260;
    config.registration_refresh_jitter_percent = 0;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{}", registrar_port),
            "alice",
            "password",
        )
        .with_contact_uri("sip:alice@127.0.0.1:40250")
        .with_expires(300)
        .send()
        .await
        .expect("register");

    timeout(Duration::from_secs(5), async {
        loop {
            if peer
                .is_registered(&handle)
                .await
                .expect("is_registered query")
            {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration should succeed");

    peer.shutdown().await.expect("stream peer shutdown");
    assert_eq!(unregister_seen.load(Ordering::SeqCst), 1);
    registrar_handle.abort();
}

#[tokio::test]
async fn register_uses_outbound_proxy_as_destination_and_route_header() {
    let proxy_sock = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("proxy mock bind"),
    );
    let proxy_port = proxy_sock.local_addr().expect("proxy addr").port();
    let received = Arc::new(Mutex::new(None::<(String, Option<String>)>));

    let proxy_sock_task = proxy_sock.clone();
    let received_task = received.clone();
    let proxy_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match proxy_sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }
            *received_task.lock().await = Some((
                request.uri().to_string(),
                request.raw_header_value(&HeaderName::Route),
            ));
            let bytes = response_with_contact_and_expires(&request, 300).to_bytes();
            let _ = proxy_sock_task.send_to(&bytes, from).await;
        }
    });

    let registrar_uri = "sip:registrar.example.com:5060";
    let outbound_proxy_uri = format!("sip:127.0.0.1:{};lr", proxy_port);
    let mut config = Config::local("alice", 0);
    config.media_port_start = 40270;
    config.media_port_end = 40280;
    config.registration_auto_refresh = false;
    config.outbound_proxy_uri = Some(outbound_proxy_uri.clone());
    let peer = StreamPeer::with_config(config).await.expect("peer");
    let handle = peer
        .register(registrar_uri, "alice", "password")
        .with_contact_uri("sip:alice@127.0.0.1:40270")
        .with_expires(300)
        .send()
        .await
        .expect("register through proxy");

    timeout(Duration::from_secs(5), async {
        loop {
            if peer
                .is_registered(&handle)
                .await
                .expect("is_registered query")
            {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration should succeed through proxy");

    let received = received.lock().await.clone().expect("proxy saw REGISTER");
    assert_eq!(received.0, registrar_uri);
    let route = received.1.expect("REGISTER should include Route header");
    assert!(
        route.contains(&format!("127.0.0.1:{}", proxy_port)) && route.contains("lr"),
        "unexpected Route header: {}",
        route
    );

    peer.control()
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .expect("shutdown");
    proxy_handle.abort();
}
