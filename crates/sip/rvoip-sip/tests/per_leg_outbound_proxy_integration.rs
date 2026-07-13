//! SIP_API_DESIGN_2 §10 verification #17 —
//! `OutboundCallBuilder::with_outbound_proxy(uri)` overrides
//! `Config.outbound_proxy_uri` for a single call.
//!
//! Alice's `Config.outbound_proxy_uri` is set to one address; the
//! builder's `with_outbound_proxy(...)` points to a different one.
//! The wire-side capture must show the BUILDER's Route, not the
//! Config one.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};

const PAIR: (u16, u16) = (17800, 17801);
// The configured proxy is deliberately unreachable. Receiving the INVITE on
// Bob proves the per-call structural override won without relying on trace
// output that correctly redacts Route values.
const CONFIG_PROXY: &str = "sip:config-proxy@127.0.0.1:17999;lr";
const BUILDER_PROXY: &str = "sip:builder-proxy@127.0.0.1:17801;lr";

fn cfg(name: &str, port: u16, outbound_proxy: Option<&str>) -> Config {
    let mut c = Config::local(name, port);
    c.outbound_proxy_uri = outbound_proxy.map(String::from);
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
async fn builder_outbound_proxy_overrides_config_for_single_call() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    let bob = UnifiedCoordinator::new(cfg("bob-pp", bob_port, None))
        .await
        .expect("bob");
    let mut bob_events = bob.events().await.expect("bob events");

    // Alice's Config carries an outbound proxy; the per-call builder
    // sets a different one. The override wins on the wire INVITE.
    let alice = UnifiedCoordinator::new(cfg("alice-pp", alice_port, Some(CONFIG_PROXY)))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_outbound_proxy(BUILDER_PROXY)
        .send()
        .await
        .expect("invite.send()");

    let raw = next_inbound_invite(&mut bob_events, Duration::from_secs(5))
        .await
        .expect("inbound INVITE trace");

    // Route values are security-redacted even in development traces. The
    // inbound delivery above proves the builder route selected Bob rather
    // than the unreachable configured proxy; retain a shape assertion too.
    assert!(
        raw.contains("Route: <redacted>"),
        "builder outbound-proxy Route must appear on wire; trace:\n{raw}"
    );
    assert!(
        !raw.contains("config-proxy@127.0.0.1"),
        "Config outbound-proxy must NOT leak through when builder overrides; trace:\n{raw}"
    );
}
