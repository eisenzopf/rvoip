//! SIP_API_DESIGN_2 §10 verification #16 —
//! `OutboundCallBuilder::with_contact_uri(...)` rewrites the Contact
//! header on the outbound INVITE.
//!
//! End-to-end: Alice issues an INVITE through a B2BUA-style rewrite,
//! a raw UDP UAS captures the outbound datagram and we assert the Contact URI
//! the builder staged is exactly what landed on the wire — proving that
//! the per-call Contact override threads from `OutboundCallBuilder`
//! through `Action::SendINVITEWithOptions` and into dialog-core's
//! initial-INVITE assembly, suppressing the default socket-derived
//! Contact that dialog-core would otherwise stamp.

use std::time::Duration;

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message, Method, StatusCode};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;

const PAIR: (u16, u16) = (15820, 15830);
const REWRITTEN_CONTACT: &str = "sip:b2bua@public.example.com:5070";

async fn capture_invite_and_reject(socket: &UdpSocket, timeout: Duration) -> String {
    tokio::time::timeout(timeout, async {
        let mut packet = vec![0u8; 16_384];
        loop {
            let (bytes, peer) = socket
                .recv_from(&mut packet)
                .await
                .expect("capture receive");
            let wire = String::from_utf8(packet[..bytes].to_vec()).expect("SIP request text");
            let Message::Request(request) =
                parse_message(&packet[..bytes]).expect("parse captured SIP request")
            else {
                continue;
            };
            if request.method() != Method::Invite {
                continue;
            }
            let response = create_response(&request, StatusCode::ServiceUnavailable);
            socket
                .send_to(&Message::Response(response).to_bytes(), peer)
                .await
                .expect("send capture response");
            return wire;
        }
    })
    .await
    .expect("capture UAS did not receive INVITE")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_call_builder_rewrites_contact_uri() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    // Assert against the actual outbound datagram. SIP traces intentionally
    // redact sensitive topology and must not be weakened for wire assertions.
    let bob = UdpSocket::bind(("127.0.0.1", bob_port))
        .await
        .expect("bob capture UAS");

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_contact_uri(REWRITTEN_CONTACT)
        .send()
        .await
        .expect("invite.send()");

    let wire = capture_invite_and_reject(&bob, Duration::from_secs(8)).await;
    let contact_values: Vec<_> = wire
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("Contact")
                .then(|| value.trim())
        })
        .collect();

    assert_eq!(
        contact_values.len(),
        1,
        "expected exactly one Contact header on the wire; got:\n{wire}"
    );
    assert!(
        contact_values[0].contains(REWRITTEN_CONTACT),
        "expected rewritten Contact `{}` on the wire; got `{}` in:\n{}",
        REWRITTEN_CONTACT,
        contact_values[0],
        wire
    );

    // Negative: dialog-core must not have also stamped its own socket-derived
    // Contact — `b2bua@public.example.com:5070` is the only Contact on the wire.
    let default_contact_marker = format!(":{}", alice_port);
    assert!(
        !contact_values[0].contains(&default_contact_marker),
        "expected dialog-core's default socket Contact (port {}) to be suppressed; got:\n{}",
        alice_port,
        wire
    );

    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}
