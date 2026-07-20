//! Generator-side SIP compliance assertions.
//!
//! These tests build representative SIP requests and responses through the
//! public [`UnifiedCoordinator`](rvoip_sip::UnifiedCoordinator) and
//! [`StreamPeer`](rvoip_sip::StreamPeer) entry points, then hand the on-wire
//! bytes back to
//! [`rvoip_sip_core::validation::validate_generated_request`] and
//! [`rvoip_sip_core::validation::validate_generated_response`] to confirm
//! `rvoip-sip` always emits messages that the parser independently accepts as
//! RFC 3261-compliant.
//!
//! Gated behind the `generated-validation` Cargo feature so the validation
//! tables are only compiled when this regression file runs:
//!
//! ```sh
//! cargo test -p rvoip-sip --features generated-validation \
//!     --test generated_sip_compliance
//! ```

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::types::Credentials;
use rvoip_sip::{CallState, Event, StreamPeer};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_core::types::{
    HeaderName, Message, Method, Request, Response, StatusCode, TypedHeader,
};
use rvoip_sip_core::validation::{validate_generated_request, validate_generated_response};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

fn random_port(base: u16) -> u16 {
    base + (rand::random::<u16>() % 1000)
}

fn config(name: &str, port: u16) -> Config {
    let mut config = Config::local(name, port);
    config.media_port_start = 43000 + (port % 1000);
    config.media_port_end = config.media_port_start + 50;
    config
}

fn response_bytes(response: Response) -> Vec<u8> {
    validate_generated_response(&response).unwrap();
    Message::Response(response).to_bytes()
}

fn ok_response(request: &Request) -> Vec<u8> {
    response_bytes(create_response(request, StatusCode::Ok))
}

fn invite_proxy_auth_response(request: &Request, nonce: &str) -> Vec<u8> {
    let mut response = create_response(request, StatusCode::ProxyAuthenticationRequired);
    response.headers.push(TypedHeader::Other(
        HeaderName::ProxyAuthenticate,
        HeaderValue::Raw(
            format!(r#"Digest realm="testrealm", nonce="{nonce}", algorithm=MD5, qop="auth""#)
                .into_bytes(),
        ),
    ));
    response_bytes(response)
}

fn invite_proxy_auth_response_with_stale(request: &Request, nonce: &str, stale: bool) -> Vec<u8> {
    let mut response = create_response(request, StatusCode::ProxyAuthenticationRequired);
    let mut challenge =
        format!(r#"Digest realm="testrealm", nonce="{nonce}", algorithm=MD5, qop="auth""#);
    if stale {
        challenge.push_str(", stale=true");
    }
    response.headers.push(TypedHeader::Other(
        HeaderName::ProxyAuthenticate,
        HeaderValue::Raw(challenge.into_bytes()),
    ));
    response_bytes(response)
}

async fn wait_for_call_failed(
    events: &mut EventReceiver,
    call_id: &rvoip_sip::CallId,
) -> (u16, String) {
    timeout(Duration::from_secs(8), async {
        loop {
            match events.next().await {
                Some(Event::CallFailed {
                    call_id: id,
                    status_code,
                    reason,
                }) if id == *call_id => return (status_code, reason),
                Some(_) => continue,
                None => panic!("event stream closed before CallFailed"),
            }
        }
    })
    .await
    .expect("timed out waiting for CallFailed")
}

async fn wait_for_count(count: &AtomicU32, expected: u32, label: &str) {
    timeout(Duration::from_secs(10), async {
        loop {
            if count.load(Ordering::SeqCst) >= expected {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for {label}; count={}",
            count.load(Ordering::SeqCst)
        )
    });
}

async fn recv_request(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> Option<(Request, std::net::SocketAddr)> {
    let (n, from) = socket.recv_from(buf).await.ok()?;
    let message = parse_message(&buf[..n]).ok()?;
    let Message::Request(request) = message else {
        return None;
    };
    validate_generated_request(&request).expect("captured generated request should be valid");
    Some((request, from))
}

#[tokio::test]
async fn generated_sip_compliance_register_auth_retry_and_unregister_are_generated_valid() {
    let registrar_port = random_port(36000);
    let client_port = registrar_port + 1200;
    let contact = format!("sip:alice@127.0.0.1:{client_port}");
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{registrar_port}"))
            .await
            .expect("mock registrar bind"),
    );
    let count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_count = count.clone();
    let task_captured = captured.clone();
    let registrar = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }

            let index = task_count.fetch_add(1, Ordering::SeqCst);
            task_captured.lock().await.push(request.clone());

            let bytes = if index == 0 {
                let mut response = create_response(&request, StatusCode::Unauthorized);
                response.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(
                        br#"Digest realm="testrealm", nonce="nonce123", algorithm=MD5, qop="auth""#
                            .to_vec(),
                    ),
                ));
                response_bytes(response)
            } else {
                ok_response(&request)
            };
            let _ = task_socket.send_to(&bytes, from).await;
        }
    });

    let coordinator = UnifiedCoordinator::new(config("alice", client_port))
        .await
        .expect("coordinator");
    let handle = coordinator
        .register(
            format!("sip:127.0.0.1:{registrar_port}"),
            "alice",
            "password",
        )
        .with_contact_uri(&contact)
        .send()
        .await
        .expect("register");

    wait_for_count(&count, 2, "initial REGISTER + auth retry").await;
    timeout(Duration::from_secs(5), async {
        loop {
            if coordinator.is_registered(&handle).await.unwrap_or(false) {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("registration did not become active");
    assert_eq!(
        coordinator.get_state(&handle.session_id).await.unwrap(),
        CallState::Registered
    );

    coordinator.unregister(&handle).await.expect("unregister");
    wait_for_count(&count, 3, "unregister REGISTER").await;

    let captured = captured.lock().await;
    assert_eq!(captured.len(), 3);
    let call_id = captured[0].call_id().unwrap().value().to_string();
    assert!(captured
        .iter()
        .all(|r| r.call_id().unwrap().value() == call_id));
    assert!(captured[1].cseq().unwrap().seq > captured[0].cseq().unwrap().seq);
    assert!(captured[2].cseq().unwrap().seq > captured[1].cseq().unwrap().seq);
    assert_eq!(
        captured[0]
            .raw_header_value(&HeaderName::Contact)
            .as_deref(),
        Some(format!("<{contact}>").as_str())
    );
    assert_eq!(
        captured[2]
            .raw_header_value(&HeaderName::Expires)
            .as_deref(),
        Some("0")
    );
    assert!(captured[1].header(&HeaderName::Authorization).is_some());

    registrar.abort();
}

#[tokio::test]
async fn generated_sip_compliance_register_423_retry_is_generated_valid() {
    let registrar_port = random_port(37200);
    let client_port = registrar_port + 1200;
    let contact = format!("sip:alice@127.0.0.1:{client_port}");
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{registrar_port}"))
            .await
            .expect("mock registrar bind"),
    );
    let count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_count = count.clone();
    let task_captured = captured.clone();
    let registrar = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };
            if request.method() != Method::Register {
                continue;
            }

            let index = task_count.fetch_add(1, Ordering::SeqCst);
            task_captured.lock().await.push(request.clone());

            let bytes = if index == 0 {
                let mut response = create_response(&request, StatusCode::IntervalTooBrief);
                response.headers.push(TypedHeader::Other(
                    HeaderName::MinExpires,
                    HeaderValue::Raw(b"1800".to_vec()),
                ));
                response_bytes(response)
            } else {
                ok_response(&request)
            };
            let _ = task_socket.send_to(&bytes, from).await;
        }
    });

    let peer = StreamPeer::with_config(config("alice", client_port))
        .await
        .expect("peer");
    let handle = peer
        .register(
            format!("sip:127.0.0.1:{registrar_port}"),
            "alice",
            "password",
        )
        .with_contact_uri(&contact)
        .with_expires(60)
        .send()
        .await
        .expect("register");

    wait_for_count(&count, 2, "423 retry").await;
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(
        captured[0].call_id().unwrap().value(),
        captured[1].call_id().unwrap().value()
    );
    assert!(captured[1].cseq().unwrap().seq > captured[0].cseq().unwrap().seq);
    assert_eq!(
        captured[1]
            .raw_header_value(&HeaderName::Expires)
            .as_deref(),
        Some("1800")
    );
    assert_eq!(
        captured[0]
            .raw_header_value(&HeaderName::Contact)
            .as_deref(),
        Some(format!("<{contact}>").as_str())
    );
    drop(captured);
    assert!(peer.is_registered(&handle).await.unwrap_or(false));

    registrar.abort();
}

#[tokio::test]
async fn generated_sip_compliance_invite_401_retry_is_generated_valid() {
    let uas_port = random_port(38400);
    let client_port = uas_port + 1200;
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{uas_port}"))
            .await
            .expect("mock uas bind"),
    );
    let count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_count = count.clone();
    let task_captured = captured.clone();
    let uas = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };
            if request.method() != Method::Invite {
                continue;
            }

            let index = task_count.fetch_add(1, Ordering::SeqCst);
            task_captured.lock().await.push(request.clone());
            let bytes = if index == 0 {
                let mut response = create_response(&request, StatusCode::Unauthorized);
                response.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(
                        br#"Digest realm="testrealm", nonce="invite-nonce", algorithm=MD5, qop="auth""#
                            .to_vec(),
                    ),
                ));
                response_bytes(response)
            } else {
                response_bytes(create_response(&request, StatusCode::BusyHere))
            };
            let _ = task_socket.send_to(&bytes, from).await;
        }
    });

    let peer = StreamPeer::with_config(config("alice", client_port))
        .await
        .expect("peer");
    let _ = peer
        .control()
        .invite(format!("sip:bob@127.0.0.1:{uas_port}"))
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await;

    wait_for_count(&count, 2, "INVITE auth retry").await;
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(
        captured[0].call_id().unwrap().value(),
        captured[1].call_id().unwrap().value()
    );
    assert!(captured[1].cseq().unwrap().seq > captured[0].cseq().unwrap().seq);
    assert!(captured[1].header(&HeaderName::Authorization).is_some());

    uas.abort();
}

#[tokio::test]
async fn generated_sip_compliance_invite_407_retry_uses_proxy_authorization() {
    let uas_port = random_port(38600);
    let client_port = uas_port + 1200;
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{uas_port}"))
            .await
            .expect("mock uas bind"),
    );
    let invite_count = Arc::new(AtomicU32::new(0));
    let ack_count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_invite_count = invite_count.clone();
    let task_ack_count = ack_count.clone();
    let task_captured = captured.clone();
    let uas = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };

            match request.method() {
                Method::Invite => {
                    let index = task_invite_count.fetch_add(1, Ordering::SeqCst);
                    task_captured.lock().await.push(request.clone());
                    let bytes = if index == 0 {
                        invite_proxy_auth_response(&request, "proxy-nonce")
                    } else {
                        response_bytes(create_response(&request, StatusCode::BusyHere))
                    };
                    let _ = task_socket.send_to(&bytes, from).await;
                }
                Method::Ack => {
                    task_ack_count.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    });

    let peer = StreamPeer::with_config(config("alice", client_port))
        .await
        .expect("peer");
    let _ = peer
        .control()
        .invite(format!("sip:bob@127.0.0.1:{uas_port}"))
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await;

    wait_for_count(&invite_count, 2, "INVITE proxy-auth retry").await;
    wait_for_count(&ack_count, 1, "ACK for first 407").await;
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(
        captured[0].call_id().unwrap().value(),
        captured[1].call_id().unwrap().value()
    );
    assert!(captured[1].cseq().unwrap().seq > captured[0].cseq().unwrap().seq);
    assert!(captured[1].body().len() > 0, "retry must preserve SDP body");
    assert!(
        captured[1]
            .header(&HeaderName::ProxyAuthorization)
            .is_some(),
        "407 retry must use Proxy-Authorization"
    );
    assert!(
        captured[1].header(&HeaderName::Authorization).is_none(),
        "407 retry must not use WWW Authorization"
    );

    uas.abort();
}

#[tokio::test]
async fn generated_sip_compliance_invite_407_without_credentials_fails_fast() {
    let uas_port = random_port(38800);
    let client_port = uas_port + 1200;
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{uas_port}"))
            .await
            .expect("mock uas bind"),
    );
    let invite_count = Arc::new(AtomicU32::new(0));

    let task_socket = socket.clone();
    let task_invite_count = invite_count.clone();
    let uas = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };
            if request.method() != Method::Invite {
                continue;
            }
            task_invite_count.fetch_add(1, Ordering::SeqCst);
            let bytes = invite_proxy_auth_response(&request, "missing-creds-nonce");
            let _ = task_socket.send_to(&bytes, from).await;
        }
    });

    let peer = StreamPeer::with_config(config("alice", client_port))
        .await
        .expect("peer");
    let mut events = peer.control().subscribe_events().await.expect("events");
    let call_id = peer
        .control()
        .invite(format!("sip:bob@127.0.0.1:{uas_port}"))
        .send()
        .await
        .expect("invite send");

    let (status, reason) = wait_for_call_failed(&mut events, &call_id).await;
    assert_eq!(status, 407);
    assert_eq!(
        reason, "INVITE authentication failed (class=missing-credentials)",
        "failure reason must expose only the stable diagnostic class"
    );
    sleep(Duration::from_millis(250)).await;
    assert_eq!(
        invite_count.load(Ordering::SeqCst),
        1,
        "missing credentials must not retry INVITE"
    );

    uas.abort();
}

#[tokio::test]
async fn generated_sip_compliance_invite_407_second_challenge_fails_fast() {
    let uas_port = random_port(39000);
    let client_port = uas_port + 1200;
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{uas_port}"))
            .await
            .expect("mock uas bind"),
    );
    let invite_count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_invite_count = invite_count.clone();
    let task_captured = captured.clone();
    let uas = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };
            if request.method() != Method::Invite {
                continue;
            }
            let index = task_invite_count.fetch_add(1, Ordering::SeqCst);
            task_captured.lock().await.push(request.clone());
            let bytes = invite_proxy_auth_response(&request, &format!("retry-nonce-{index}"));
            let _ = task_socket.send_to(&bytes, from).await;
        }
    });

    let mut cfg = config("alice", client_port);
    cfg.credentials = Some(Credentials::new("alice", "password"));
    let peer = StreamPeer::with_config(cfg).await.expect("peer");
    let mut events = peer.control().subscribe_events().await.expect("events");
    let call_id = peer
        .control()
        .invite(format!("sip:bob@127.0.0.1:{uas_port}"))
        .send()
        .await
        .expect("invite send");

    let (status, reason) = wait_for_call_failed(&mut events, &call_id).await;
    assert_eq!(status, 407);
    assert_eq!(
        reason, "INVITE authentication failed (class=retry-limit)",
        "failure reason must expose only the stable diagnostic class"
    );
    wait_for_count(&invite_count, 2, "initial INVITE plus one auth retry").await;
    sleep(Duration::from_millis(250)).await;
    assert_eq!(
        invite_count.load(Ordering::SeqCst),
        2,
        "retry cap must prevent a third INVITE"
    );
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    assert!(captured[1]
        .header(&HeaderName::ProxyAuthorization)
        .is_some());

    uas.abort();
}

#[tokio::test]
async fn generated_sip_compliance_invite_407_stale_challenge_retries_with_fresh_nonce() {
    let uas_port = random_port(39200);
    let client_port = uas_port + 1200;
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{uas_port}"))
            .await
            .expect("mock uas bind"),
    );
    let invite_count = Arc::new(AtomicU32::new(0));
    let ack_count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_invite_count = invite_count.clone();
    let task_ack_count = ack_count.clone();
    let task_captured = captured.clone();
    let uas = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };

            match request.method() {
                Method::Invite => {
                    let index = task_invite_count.fetch_add(1, Ordering::SeqCst);
                    task_captured.lock().await.push(request.clone());
                    let bytes = match index {
                        0 => invite_proxy_auth_response_with_stale(&request, "stale-old", false),
                        1 => invite_proxy_auth_response_with_stale(&request, "stale-fresh", true),
                        _ => response_bytes(create_response(&request, StatusCode::BusyHere)),
                    };
                    let _ = task_socket.send_to(&bytes, from).await;
                }
                Method::Ack => {
                    task_ack_count.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    });

    let mut cfg = config("alice", client_port);
    cfg.credentials = Some(Credentials::new("alice", "password"));
    let peer = StreamPeer::with_config(cfg).await.expect("peer");
    let _ = peer
        .control()
        .invite(format!("sip:bob@127.0.0.1:{uas_port}"))
        .send()
        .await;

    wait_for_count(&invite_count, 3, "INVITE stale auth recovery").await;
    wait_for_count(&ack_count, 2, "ACKs for both 407 challenges").await;
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 3);
    assert!(captured[1]
        .raw_header_value(&HeaderName::ProxyAuthorization)
        .expect("first retry Proxy-Authorization")
        .contains(r#"nonce="stale-old""#));
    let second_retry = captured[2]
        .raw_header_value(&HeaderName::ProxyAuthorization)
        .expect("stale retry Proxy-Authorization");
    assert!(
        second_retry.contains(r#"nonce="stale-fresh""#),
        "stale retry must use fresh nonce: {second_retry}"
    );
    assert!(
        second_retry.contains("nc=00000001"),
        "fresh nonce must reset nonce-count: {second_retry}"
    );
    assert!(captured[2].cseq().unwrap().seq > captured[1].cseq().unwrap().seq);

    uas.abort();
}

#[tokio::test]
async fn generated_sip_compliance_invite_422_retry_is_generated_valid() {
    let uas_port = random_port(39600);
    let client_port = uas_port + 1200;
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{uas_port}"))
            .await
            .expect("mock uas bind"),
    );
    let count = Arc::new(AtomicU32::new(0));
    let captured = Arc::new(Mutex::new(Vec::<Request>::new()));

    let task_socket = socket.clone();
    let task_count = count.clone();
    let task_captured = captured.clone();
    let uas = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let Some((request, from)) = recv_request(&task_socket, &mut buf).await else {
                continue;
            };
            if request.method() != Method::Invite {
                continue;
            }

            let index = task_count.fetch_add(1, Ordering::SeqCst);
            task_captured.lock().await.push(request.clone());
            let bytes = if index == 0 {
                let mut response = create_response(&request, StatusCode::SessionIntervalTooSmall);
                response.headers.push(TypedHeader::Other(
                    HeaderName::MinSE,
                    HeaderValue::Raw(b"120".to_vec()),
                ));
                response_bytes(response)
            } else {
                response_bytes(create_response(&request, StatusCode::BusyHere))
            };
            let _ = task_socket.send_to(&bytes, from).await;
        }
    });

    let mut cfg = config("alice", client_port);
    cfg.session_timer_secs = Some(90);
    let peer = StreamPeer::with_config(cfg).await.expect("peer");
    let _ = peer
        .control()
        .invite(format!("sip:bob@127.0.0.1:{uas_port}"))
        .send()
        .await;

    wait_for_count(&count, 2, "INVITE 422 retry").await;
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(
        captured[0].call_id().unwrap().value(),
        captured[1].call_id().unwrap().value()
    );
    assert!(captured[1].cseq().unwrap().seq > captured[0].cseq().unwrap().seq);
    assert!(captured[1].header(&HeaderName::SessionExpires).is_some());
    assert_eq!(
        captured[1].raw_header_value(&HeaderName::MinSE).as_deref(),
        Some("120")
    );

    uas.abort();
}

#[tokio::test]
async fn generated_sip_compliance_inbound_options_response_is_generated_valid_and_creates_no_session(
) {
    let port = random_port(40800);
    let coordinator = UnifiedCoordinator::new(config("test", port)).await.unwrap();

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source_addr = socket.local_addr().unwrap();
    let target_uri = format!("sip:test@127.0.0.1:{port}");
    let request = SimpleRequestBuilder::new(Method::Options, &target_uri)
        .unwrap()
        .from("Asterisk", "sip:asterisk@example.com", Some("ast-tag"))
        .to("Endpoint", &target_uri, None)
        .call_id("rvoip-sip-generated-options")
        .cseq(1)
        .via(
            &source_addr.to_string(),
            "UDP",
            Some("z9hG4bK-session-generated-options"),
        )
        .max_forwards(70)
        .build();
    validate_generated_request(&request).unwrap();

    socket
        .send_to(
            &Message::Request(request).to_bytes(),
            format!("127.0.0.1:{port}"),
        )
        .await
        .unwrap();

    let mut buf = [0u8; 4096];
    let (len, _) = timeout(Duration::from_secs(1), socket.recv_from(&mut buf))
        .await
        .expect("timed out waiting for OPTIONS response")
        .unwrap();
    let Message::Response(response) = parse_message(&buf[..len]).unwrap() else {
        panic!("expected OPTIONS response");
    };
    validate_generated_response(&response).unwrap();
    assert!(response.header(&HeaderName::Allow).is_some());
    assert!(
        coordinator.list_sessions().await.is_empty(),
        "OPTIONS qualify must not create rvoip-sip state"
    );
}
