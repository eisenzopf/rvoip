//! SIP_API_DESIGN_2 §10 verification #19 — third-party REGISTER
//! (PBX/SBC pattern). The PBX registers on behalf of an extension by
//! rewriting `From`, `Contact`, and stamping `P-Asserted-Identity`.
//!
//! Pattern reused from `register_423_retry.rs`: a raw-UDP mock
//! registrar captures the REGISTER bytes and the test asserts the
//! three application-controlled fields landed correctly.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const REGISTRAR_PORT: u16 = 35220;
const CLIENT_PORT: u16 = 35221;
// Plain URI form expected by dialog-core's RegisterBuilder; the
// SimpleRequestBuilder wraps it in `<>` on the wire.
const PROXY_CONTACT_URI: &str = "sip:proxy@127.0.0.1:35221";
const PROXY_CONTACT_WIRE: &str = "<sip:proxy@127.0.0.1:35221>";
const PROXY_PAI: &str = "<sip:trunk@trusted.carrier.example>";
const BEHALF_AOR: &str = "sip:behalf@enterprise.example";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn third_party_register_rewrites_from_contact_and_pai_on_wire() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let registrar_addr = format!("127.0.0.1:{REGISTRAR_PORT}");
    let sock = Arc::new(
        UdpSocket::bind(&registrar_addr)
            .await
            .expect("mock registrar bind"),
    );

    let register_count = Arc::new(AtomicU32::new(0));
    // Capture (from, contact, pai) for each REGISTER.
    let captured = Arc::new(Mutex::new(Vec::<(String, String, Option<String>)>::new()));

    let sock_task = sock.clone();
    let count_task = register_count.clone();
    let captured_task = captured.clone();
    let registrar_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from_addr) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
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
            count_task.fetch_add(1, Ordering::SeqCst);


            let from_value = request
                .from()
                .map(|f| f.uri.to_string())
                .unwrap_or_default();
            let contact_value = request
                .raw_header_value(&HeaderName::Contact)
                .unwrap_or_default();
            // sip-core recognizes P-Asserted-Identity as a typed
            // header (`HeaderName::PAssertedIdentity`), not Other(_).
            let pai_value = request.raw_header_value(&HeaderName::PAssertedIdentity);
            captured_task
                .lock()
                .await
                .push((from_value, contact_value, pai_value));

            // Reply 200 OK so the client exits the registration loop.
            let mut resp = create_response(&request, StatusCode::Ok);
            if let Some(contact) = request.header(&HeaderName::Contact) {
                resp.headers.push(contact.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(b"3600".to_vec()),
            ));
            let bytes = Message::Response(resp).to_bytes();
            let _ = sock_task.send_to(&bytes, from_addr).await;
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", CLIENT_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let _handle = coord
        .register(
            format!("sip:127.0.0.1:{REGISTRAR_PORT}"),
            "alice",
            "password",
        )
        .with_from_uri(BEHALF_AOR)
        .with_contact_uri(PROXY_CONTACT_URI)
        .with_raw_header(
            HeaderName::Other("P-Asserted-Identity".to_string()),
            PROXY_PAI,
        )
        .expect("PAI is application-controlled")
        .send()
        .await
        .expect("register.send()");

    let observed = timeout(Duration::from_secs(5), async {
        loop {
            if register_count.load(Ordering::SeqCst) >= 1 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "registrar never saw the third-party REGISTER (count={})",
        register_count.load(Ordering::SeqCst)
    );
    sleep(Duration::from_millis(200)).await;

    let cap = captured.lock().await;
    assert!(!cap.is_empty(), "no REGISTERs captured");
    let (from_uri, contact, pai) = &cap[0];

    assert_eq!(
        from_uri, BEHALF_AOR,
        "REGISTER From URI must be the behalf-of AOR, not the local UA URI"
    );
    assert_eq!(
        contact, PROXY_CONTACT_WIRE,
        "REGISTER Contact must be the proxy override"
    );
    assert_eq!(
        pai.as_deref(),
        Some(PROXY_PAI),
        "REGISTER must carry the raw P-Asserted-Identity stamped by the builder"
    );

    registrar_handle.abort();
}
