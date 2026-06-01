//! SIP_API_DESIGN_2 §10 verification #16 —
//! `OutboundCallBuilder::with_contact_uri(...)` rewrites the Contact
//! header on the outbound INVITE.
//!
//! End-to-end: Alice issues an INVITE through a B2BUA-style rewrite,
//! Bob captures the inbound wire trace and we assert the Contact URI
//! the builder staged is exactly what landed on the wire — proving that
//! the per-call Contact override threads from `OutboundCallBuilder`
//! through `Action::SendINVITEWithOptions` and into dialog-core's
//! initial-INVITE assembly, suppressing the default socket-derived
//! Contact that dialog-core would otherwise stamp.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTrace, SipTraceConfig, SipTraceDirection};

const PAIR: (u16, u16) = (15820, 15830);
const REWRITTEN_CONTACT: &str = "sip:b2bua@public.example.com:5070";

fn receiver_config(name: &str, port: u16) -> Config {
    let mut cfg = Config::local(name, port);
    cfg.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    cfg
}

async fn wait_for_inbound_invite(
    events: &mut EventReceiver,
    timeout: Duration,
) -> Option<SipTrace> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with("INVITE")
                {
                    return Some(trace);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_call_builder_rewrites_contact_uri() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    let bob = UnifiedCoordinator::new(receiver_config("bob", bob_port))
        .await
        .expect("bob coordinator");
    let mut bob_events = bob.events().await.expect("bob events");
    tokio::time::sleep(Duration::from_millis(150)).await;

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

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert!(
        trace.raw_message.contains(REWRITTEN_CONTACT),
        "expected rewritten Contact `{}` on the wire; got:\n{}",
        REWRITTEN_CONTACT,
        trace.raw_message
    );

    // Negative: dialog-core must not have also stamped its own socket-derived
    // Contact — `b2bua@public.example.com:5070` is the only Contact on the wire.
    let default_contact_marker = format!(":{}", alice_port);
    assert!(
        !trace
            .raw_message
            .lines()
            .filter(|line| line.starts_with("Contact:") || line.starts_with("Contact "))
            .any(|line| line.contains(&default_contact_marker)),
        "expected dialog-core's default socket Contact (port {}) to be suppressed; got:\n{}",
        alice_port,
        trace.raw_message
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}
