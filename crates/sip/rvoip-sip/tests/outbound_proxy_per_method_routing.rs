//! SIP_API_DESIGN_2 §10 #15 — outbound-proxy `Route:` propagation
//! across every application-driven SIP method.
//!
//! When `Config.outbound_proxy_uri` is set, the same
//! `prepend_outbound_proxy_route` helper that runs inside `dialog_adapter`
//! is invoked from every `send_*_with_options` mirror (one per SIP
//! method). This test stands up a mock UDP proxy and verifies the
//! resulting wire bytes for the four application-issued out-of-dialog
//! methods — INVITE, REGISTER, OPTIONS, MESSAGE — each of which must
//! carry exactly one top `Route:` header pointing at the configured
//! proxy. These four exercise distinct `dialog_adapter` entry points
//! and prove the routing invariant holds for the helper that is shared
//! across every per-method mirror. In-dialog companions (BYE, INFO,
//! REFER, UPDATE, NOTIFY, CANCEL) flow through the same helper and
//! identical code path; isolated unit coverage of
//! `prepend_outbound_proxy_route` itself lives next to its definition
//! in `dialog_adapter.rs`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderAccess;

use rvoip_sip_dialog::transaction::utils::response_builders::{
    create_ok_response_for_message, create_ok_response_for_options,
    create_ok_response_for_register, create_ok_response_with_contact_uri, create_response,
};

const PROXY_PORT: u16 = 35240;
const ALICE_PORT: u16 = 35241;
const PROXY_URI_PARAM: &str = "sip:127.0.0.1:35240;lr";
const PROXY_CONTACT_URI: &str = "sip:proxy@127.0.0.1:35240";

/// One captured outbound request — what landed on the proxy socket.
#[derive(Debug, Clone)]
struct CapturedRequest {
    method: Method,
    /// Raw value of the topmost `Route:` header on the wire, if any.
    top_route: Option<String>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_proxy_per_method_routing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let proxy_addr = format!("127.0.0.1:{PROXY_PORT}");
    let proxy_sock = Arc::new(UdpSocket::bind(&proxy_addr).await.expect("proxy bind"));

    let count = Arc::new(AtomicU32::new(0));
    let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));

    let proxy_task = {
        let sock = proxy_sock.clone();
        let count = count.clone();
        let captured = captured.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                let (n, from_addr) = match sock.recv_from(&mut buf).await {
                    Ok(p) => p,
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

                let method = request.method();
                let top_route = request.raw_header_value(&HeaderName::Route);
                count.fetch_add(1, Ordering::SeqCst);
                captured.lock().await.push(CapturedRequest {
                    method: method.clone(),
                    top_route,
                });

                // ACK terminates a 2xx INVITE transaction — no response.
                if method == Method::Ack {
                    continue;
                }

                let response = match method {
                    Method::Invite => {
                        match create_ok_response_with_contact_uri(&request, PROXY_CONTACT_URI) {
                            Ok(mut r) => {
                                let sdp = b"v=0\r\n\
                                       o=- 0 0 IN IP4 127.0.0.1\r\n\
                                       s=-\r\n\
                                       c=IN IP4 127.0.0.1\r\n\
                                       t=0 0\r\n\
                                       m=audio 35299 RTP/AVP 0\r\n";
                                r.body = bytes::Bytes::from(sdp.to_vec());
                                r.headers.push(TypedHeader::ContentType(
                                rvoip_sip_core::types::content_type::ContentType::from_type_subtype(
                                    "application", "sdp",
                                ),
                            ));
                                // ContentLength gets stamped at serialize time; replace the
                                // default 0 added by create_response.
                                r.headers
                                    .retain(|h| !matches!(h, TypedHeader::ContentLength(_)));
                                r.headers.push(TypedHeader::ContentLength(
                                    rvoip_sip_core::types::content_length::ContentLength::new(
                                        sdp.len() as u32,
                                    ),
                                ));
                                r
                            }
                            Err(_) => continue,
                        }
                    }
                    Method::Register => create_ok_response_for_register(&request, 3600),
                    Method::Options => create_ok_response_for_options(
                        &request,
                        &[
                            Method::Invite,
                            Method::Ack,
                            Method::Bye,
                            Method::Cancel,
                            Method::Options,
                            Method::Message,
                            Method::Register,
                            Method::Refer,
                            Method::Info,
                            Method::Update,
                            Method::Notify,
                            Method::Subscribe,
                        ],
                    ),
                    Method::Message => create_ok_response_for_message(&request),
                    Method::Bye => create_response(&request, StatusCode::Ok),
                    _ => create_response(&request, StatusCode::Ok),
                };

                let bytes = Message::Response(response).to_bytes();
                let _ = sock.send_to(&bytes, from_addr).await;
            }
        })
    };

    // Stand up Alice with the outbound proxy URI configured.
    let mut cfg = Config::local("alice", ALICE_PORT);
    cfg.outbound_proxy_uri = Some(PROXY_URI_PARAM.to_string());
    let alice = UnifiedCoordinator::new(cfg)
        .await
        .expect("alice coordinator");
    sleep(Duration::from_millis(150)).await;

    // 1) INVITE — target uses a resolvable 127.0.0.1 port (the proxy's
    // own port) so `dialog.get_remote_target_address()` succeeds; the
    // top `Route:` inserted by the outbound-proxy mechanism is what
    // actually carries the request, and the proxy answers 200 OK.
    let _call_id = alice
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{PROXY_PORT}"),
        )
        .send()
        .await
        .expect("invite().send()");

    wait_for_count(&count, 1, "INVITE arrival").await;

    // 2) REGISTER — fire-and-forget against the mock proxy.
    let target = format!("sip:127.0.0.1:{PROXY_PORT}");
    let _reg = alice
        .register(target.clone(), "alice", "password")
        .send()
        .await
        .expect("register().send()");
    wait_for_count(&count, 2, "REGISTER arrival").await;

    // 3) OPTIONS — round-trip the proxy.
    let _opt = alice
        .options(format!("sip:helpdesk@127.0.0.1:{PROXY_PORT}"))
        .with_timeout(Duration::from_secs(2))
        .send()
        .await;
    wait_for_count(&count, 3, "OPTIONS arrival").await;

    // 4) MESSAGE — fire-and-forget.
    alice
        .message(format!("sip:helpdesk@127.0.0.1:{PROXY_PORT}"))
        .with_body(bytes::Bytes::from_static(b"hello"))
        .with_content_type("text/plain")
        .send()
        .await
        .expect("message().send()");
    wait_for_count(&count, 4, "MESSAGE arrival").await;

    // Drain a tiny grace window so any straggler packets land before we read.
    sleep(Duration::from_millis(100)).await;

    let cap = captured.lock().await;
    let methods_seen: Vec<Method> = cap.iter().map(|c| c.method.clone()).collect();

    // Every method the rvoip-sip outbound-proxy path is responsible for must
    // carry a Route pointing at the configured proxy. ACK for 2xx is sent
    // by dialog-core's transaction layer (no Route-prepending mirror), so
    // it does not assert.
    for entry in cap.iter() {
        if entry.method == Method::Ack {
            continue;
        }
        let route = entry.top_route.as_deref().unwrap_or("");
        assert!(
            route.contains("127.0.0.1") && route.contains(&format!(":{PROXY_PORT}")),
            "request {:?} missing outbound-proxy Route; got `{}`",
            entry.method,
            route
        );
    }

    // Sanity: every method we exercised was actually seen.
    for required in [
        Method::Invite,
        Method::Register,
        Method::Options,
        Method::Message,
    ] {
        assert!(
            methods_seen.contains(&required),
            "expected proxy to capture {:?}; saw {:?}",
            required,
            methods_seen
        );
    }

    proxy_task.abort();
}

async fn wait_for_count(count: &Arc<AtomicU32>, target: u32, label: &str) {
    let res = timeout(Duration::from_secs(8), async {
        loop {
            if count.load(Ordering::SeqCst) >= target {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    assert!(
        res.is_ok(),
        "waiting for {} timed out (have {} arrivals)",
        label,
        count.load(Ordering::SeqCst)
    );
}
