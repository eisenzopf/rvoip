//! SIP_API_DESIGN_2 §10 verification #25 — reliable provisional
//! (RFC 3262) advertising.
//!
//! The §10 #25 spec describes two contracts:
//!
//! 1. `OutboundCallBuilder::with_supported_100rel(true)` advertises
//!    reliability on the outbound INVITE — the wire INVITE carries
//!    `Supported: 100rel`.
//! 2. (B2BUA bridging) upstream 18x with `Require: 100rel` mirrored to
//!    the downstream 1xx with `with_require_100rel(true)`.
//!
//! Today the wire-side check is what's exercised end-to-end. The
//! downstream-PRACK bridging contract (#2) needs an established
//! upstream + downstream call pair plus PRACK round-trip captures —
//! that's the broader `prack_integration.rs` territory.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};

const PAIR: (u16, u16) = (18200, 18201);

fn cfg(name: &str, port: u16) -> Config {
    let mut c = Config::local(name, port);
    c.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    c
}

async fn next_inbound_invite(events: &mut EventReceiver, timeout: Duration) -> Option<String> {
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
                    return Some(trace.raw_message);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_call_builder_advertises_100rel_when_requested() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    let bob = UnifiedCoordinator::new(cfg("bob-100rel", bob_port))
        .await
        .expect("bob");
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(cfg("alice-100rel", alice_port))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_supported_100rel(true)
        .send()
        .await
        .expect("invite.send()");

    let raw = next_inbound_invite(&mut bob_events, Duration::from_secs(5))
        .await
        .expect("inbound INVITE trace");

    // RFC 3262 §4 — Supported: 100rel must ride on the outbound INVITE
    // when the caller opted in. We tolerate the value being part of a
    // list (`Supported: 100rel, timer`) — the assertion is the token
    // appears somewhere on a Supported: line.
    let supported_line = raw
        .lines()
        .find(|l| l.starts_with("Supported:"))
        .expect("INVITE with with_supported_100rel(true) must include a Supported header");
    assert!(
        supported_line.contains("100rel"),
        "Supported header must contain `100rel` token; got `{supported_line}`"
    );
}
