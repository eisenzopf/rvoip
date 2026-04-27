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
use tokio::time::{sleep, timeout};

use rvoip_session_core::api::unified::Config;
use rvoip_session_core::StreamPeer;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderValue;

use rvoip_dialog_core::transaction::utils::response_builders::create_response;

const REDIRECTOR_PORT: u16 = 35200;
const ACCEPTOR_PORT: u16 = 35201;
const CLIENT_PORT: u16 = 35202;

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

    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let _handle = peer
        .call(&format!("sip:bob@127.0.0.1:{}", REDIRECTOR_PORT))
        .await
        .expect("call");

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
