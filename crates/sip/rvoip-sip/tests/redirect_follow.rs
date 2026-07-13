//! Integration test for RFC 3261 §8.1.3.4 — 3xx redirect follow on the UAC.
//!
//! Two in-process raw-UDP mock UASes bind loopback ports. The first
//! (`redirector`) answers the initial INVITE with `302 Moved Temporarily` +
//! a `Contact:` header pointing at the second mock (`acceptor`). We then
//! assert that the StreamPeer re-issues an INVITE against the acceptor's
//! port — the defining invariant of redirect follow.
//!
//! The full 180/200/ACK/BYE exchange is *not* completed: building a
//! full-featured mock UAS is out of scope; what matters for T2.1 is the
//! retry-with-new-Contact behavior itself. The acceptor simply counts the
//! INVITE and hangs up with 486 to bring the session to a clean terminal
//! state so the test can exit.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::unified::Config;
use rvoip_sip::{CallHandlerDecision, CallbackPeer, StreamPeer};

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::types::headers::HeaderValue;

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const REDIRECTOR_PORT: u16 = 35200;
const ACCEPTOR_PORT: u16 = 35201;
const CLIENT_PORT: u16 = 35202;
const UAS_REDIRECT_PORT: u16 = 35203;

#[derive(Debug)]
struct RedirectProxyInvite {
    request_uri: String,
    has_route: bool,
    has_authorization: bool,
}

#[test]
fn redirect_keeps_structural_proxy_but_drops_origin_authorization() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("redirect test runtime");
    runtime.block_on(async move {
        tokio::spawn(async move {
            let proxy_port = 35800 + (rand::random::<u16>() % 100);
            let client_port = proxy_port + 200;
            let socket = Arc::new(
                UdpSocket::bind(format!("127.0.0.1:{proxy_port}"))
                    .await
                    .expect("redirect proxy bind"),
            );
            let seen = Arc::new(Mutex::new(Vec::<RedirectProxyInvite>::new()));
            let socket_task = Arc::clone(&socket);
            let seen_task = Arc::clone(&seen);
            let server = tokio::spawn(async move {
                let mut buffer = vec![0_u8; 8192];
                loop {
                    let (length, peer) = match socket_task.recv_from(&mut buffer).await {
                        Ok(received) => received,
                        Err(_) => return,
                    };
                    let request = match parse_message(&buffer[..length]) {
                        Ok(Message::Request(request)) if request.method() == Method::Invite => {
                            request
                        }
                        _ => continue,
                    };
                    let attempt = {
                        let mut captured = seen_task.lock().await;
                        let attempt = captured.len();
                        captured.push(RedirectProxyInvite {
                            request_uri: request.uri().to_string(),
                            has_route: request.raw_header_value(&HeaderName::Route).is_some(),
                            has_authorization: request
                                .raw_header_value(&HeaderName::Authorization)
                                .is_some(),
                        });
                        attempt
                    };
                    let response = if attempt == 0 {
                        let mut response = create_response(&request, StatusCode::MovedTemporarily);
                        response.headers.push(TypedHeader::Other(
                            HeaderName::Contact,
                            HeaderValue::Raw(b"<sip:bob@redirected-origin.invalid>".to_vec()),
                        ));
                        response
                    } else {
                        create_response(&request, StatusCode::BusyHere)
                    };
                    let _ = socket_task
                        .send_to(&Message::Response(response).to_bytes(), peer)
                        .await;
                }
            });

            let mut config = Config::local("alice", client_port);
            config.media_port_start = 40400;
            config.media_port_end = 40500;
            let peer = StreamPeer::with_config(config).await.expect("peer");
            peer.invite("sip:bob@original-origin.invalid")
                .with_outbound_proxy(format!("sip:127.0.0.1:{proxy_port};lr"))
                .with_precomputed_authorization("Bearer redirect-origin-secret")
                .send()
                .await
                .expect("initial redirected INVITE");

            timeout(Duration::from_secs(8), async {
                while seen.lock().await.len() < 2 {
                    sleep(Duration::from_millis(40)).await;
                }
            })
            .await
            .expect("redirected INVITE traversed configured proxy");

            let captured = seen.lock().await;
            assert_eq!(captured.len(), 2);
            assert_eq!(captured[0].request_uri, "sip:bob@original-origin.invalid");
            assert_eq!(captured[1].request_uri, "sip:bob@redirected-origin.invalid");
            assert!(captured.iter().all(|invite| invite.has_route));
            assert!(captured[0].has_authorization);
            assert!(
                !captured[1].has_authorization,
                "origin authorization must not cross a redirect boundary"
            );

            server.abort();
        })
        .await
        .expect("redirect scenario task");
    })
}

#[tokio::test]
async fn uac_follows_302_and_reissues_invite_to_new_contact() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    // --- Mock redirector (port A): answer first INVITE with 302 → Contact: sip:bob@127.0.0.1:B ---
    let redirector_sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{}", REDIRECTOR_PORT))
            .await
            .expect("redirector bind"),
    );
    let redirector_invites = Arc::new(AtomicU32::new(0));
    let redirector_sock_task = redirector_sock.clone();
    let redirector_invites_task = redirector_invites.clone();
    let redirector_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match redirector_sock_task.recv_from(&mut buf).await {
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
                    redirector_invites_task.fetch_add(1, Ordering::SeqCst);
                    let mut resp = create_response(&request, StatusCode::MovedTemporarily);
                    // Contact: <sip:bob@127.0.0.1:ACCEPTOR_PORT>
                    let contact_val = format!("<sip:bob@127.0.0.1:{}>", ACCEPTOR_PORT);
                    resp.headers.push(TypedHeader::Other(
                        HeaderName::Contact,
                        HeaderValue::Raw(contact_val.into_bytes()),
                    ));
                    let bytes = Message::Response(resp).to_bytes();
                    let _ = redirector_sock_task.send_to(&bytes, from).await;
                }
                Method::Ack => {
                    // RFC 3261 §17.1.1.3 — UAC ACKs the 3xx. Nothing more to do.
                }
                _ => {}
            }
        }
    });

    // --- Mock acceptor (port B): count the re-issued INVITE, reject with 486 ---
    let acceptor_sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{}", ACCEPTOR_PORT))
            .await
            .expect("acceptor bind"),
    );
    let acceptor_invites = Arc::new(AtomicU32::new(0));
    let acceptor_sock_task = acceptor_sock.clone();
    let acceptor_invites_task = acceptor_invites.clone();
    let acceptor_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match acceptor_sock_task.recv_from(&mut buf).await {
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
            if request.method() == Method::Invite {
                acceptor_invites_task.fetch_add(1, Ordering::SeqCst);
                // 486 Busy Here just to close out the transaction cleanly.
                let resp = create_response(&request, StatusCode::BusyHere);
                let bytes = Message::Response(resp).to_bytes();
                let _ = acceptor_sock_task.send_to(&bytes, from).await;
            }
        }
    });

    // --- Session-core-v3 client calls the redirector ---
    let mut config = Config::local("alice", CLIENT_PORT);
    config.media_port_start = 40200;
    config.media_port_end = 40300;

    let peer = StreamPeer::with_config(config).await.expect("peer");
    let _call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{}", REDIRECTOR_PORT))
        .send()
        .await
        .expect("invite.send()");

    // Wait for (a) the redirector to see at least one INVITE, and (b) the
    // acceptor to see at least one INVITE as a consequence of the 302.
    let wait = timeout(Duration::from_secs(10), async {
        loop {
            if redirector_invites.load(Ordering::SeqCst) >= 1
                && acceptor_invites.load(Ordering::SeqCst) >= 1
            {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    let r = redirector_invites.load(Ordering::SeqCst);
    let a = acceptor_invites.load(Ordering::SeqCst);

    assert!(
        wait.is_ok(),
        "redirect follow did not happen within timeout \
         (redirector INVITEs = {}, acceptor INVITEs = {})",
        r,
        a
    );

    assert!(
        r >= 1,
        "redirector should have received at least one INVITE, got {}",
        r
    );
    assert!(
        a >= 1,
        "UAC should have followed the 302 and sent an INVITE to the acceptor, got {}",
        a
    );

    redirector_handle.abort();
    acceptor_handle.abort();
}

#[tokio::test]
async fn uas_redirect_decision_sends_302_with_contact() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let redirect_target = "sip:voicemail@127.0.0.1:35299";
    let peer = CallbackPeer::builder(Config::local("redirector", UAS_REDIRECT_PORT))
        .on_incoming(move |_call| async move {
            CallHandlerDecision::Redirect(redirect_target.to_string())
        })
        .build()
        .await
        .expect("redirect peer");
    let stop = peer.shutdown_handle();
    let run_task = tokio::spawn(async move { peer.run().await });

    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("uac bind");
    let source_addr = socket.local_addr().expect("uac addr");
    let target_uri = format!("sip:redirector@127.0.0.1:{UAS_REDIRECT_PORT}");
    let request = SimpleRequestBuilder::new(Method::Invite, &target_uri)
        .unwrap()
        .from("Caller", "sip:caller@example.test", Some("caller-tag"))
        .to("Redirector", &target_uri, None)
        .call_id("rvoip-sip-uas-redirect-call-id")
        .cseq(1)
        .via(
            &source_addr.to_string(),
            "UDP",
            Some("z9hG4bK-rvoip-sip-uas-redirect"),
        )
        .max_forwards(70)
        .contact(&format!("sip:caller@{}", source_addr), Some("Caller"))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    socket
        .send_to(
            &Message::Request(request).to_bytes(),
            format!("127.0.0.1:{UAS_REDIRECT_PORT}"),
        )
        .await
        .expect("send INVITE");

    let mut buf = [0u8; 8192];
    let response = timeout(Duration::from_secs(5), async {
        loop {
            let (len, _) = socket.recv_from(&mut buf).await.expect("recv response");
            let message = parse_message(&buf[..len]).expect("parse response");
            let Message::Response(response) = message else {
                continue;
            };
            if response.status_code() == 302 {
                return response;
            }
        }
    })
    .await
    .expect("timed out waiting for 302");

    assert_eq!(response.status_code(), 302);
    let contact = response
        .raw_header_value(&HeaderName::Contact)
        .expect("302 Contact");
    assert!(
        contact.contains(redirect_target),
        "302 Contact should contain redirect target, got {contact}"
    );

    stop.shutdown();
    timeout(Duration::from_secs(2), run_task)
        .await
        .expect("peer shutdown timeout")
        .expect("peer task panicked")
        .expect("peer run failed");
}
