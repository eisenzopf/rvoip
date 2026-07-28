//! SIP_API_DESIGN_2 §10 verification #13 — Config × builder merge.
//!
//! Asserts the §6.1 precedence table is honored on the wire:
//!
//! - **Path 1 (pure Config)**: `Config.pai_uri` set, no builder
//!   override → the wire INVITE carries `P-Asserted-Identity` from
//!   Config.
//! - **Path 2 (pure builder)**: empty `Config.pai_uri`, builder calls
//!   `.with_pai(...)` → the wire INVITE carries the builder-supplied
//!   value.
//! - **Path 3 (mixed)**: `Config.pai_uri` set, builder calls
//!   `.without_pai()` → the wire INVITE carries NO PAI (suppression
//!   wins over the Config default).
//!
//! The three branches exist to prove §6.1's merge precedence does not
//! drift as the builder/legacy paths consolidate. Each scenario uses a
//! disjoint port pair to avoid socket-rebind flakiness.

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};

const CONFIG_PAI_VALUE: &str = "sip:config-pai@trusted.carrier.example";
const BUILDER_PAI_VALUE: &str = "sip:builder-pai@trusted.carrier.example";

const PAIR_CONFIG: (u16, u16) = (17500, 17501);
const PAIR_BUILDER: (u16, u16) = (17502, 17503);
const PAIR_SUPPRESS: (u16, u16) = (17504, 17505);

fn cfg(name: &str, port: u16, pai: Option<&str>) -> Config {
    let mut c = Config::local(name, port);
    c.pai_uri = pai.map(String::from);
    c.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    // This test intentionally treats the local trace as a packet-capture
    // oracle. The boolean compatibility fields alone remain production-safe;
    // verbatim values require this explicit development-only opt-in.
    c.trace_passthrough_for_development()
}

async fn boot_receiver(port: u16, name: &str) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(cfg(name, port, None))
        .await
        .expect("receiver");
    tokio::time::sleep(Duration::from_millis(150)).await;
    coord
}

async fn next_inbound_invite_trace(
    events: &mut EventReceiver,
    timeout: Duration,
) -> Option<String> {
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
async fn path1_config_only_pai_lands_on_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_CONFIG;

    let bob = boot_receiver(bob_port, "bob-cfg").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(cfg("alice-cfg", alice_port, Some(CONFIG_PAI_VALUE)))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await
        .expect("invite.send()");

    let raw = next_inbound_invite_trace(&mut bob_events, Duration::from_secs(5))
        .await
        .expect("inbound INVITE trace");

    assert!(
        raw.contains("P-Asserted-Identity:"),
        "Path 1: Config PAI must appear on wire; trace:\n{raw}"
    );
    assert!(
        raw.contains(CONFIG_PAI_VALUE),
        "Path 1: PAI value must come from Config; trace:\n{raw}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn path2_builder_pai_overrides_empty_config() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_BUILDER;

    let bob = boot_receiver(bob_port, "bob-bld").await;
    let mut bob_events = bob.events().await.expect("bob events");

    // No Config.pai_uri set — builder is the sole source.
    let alice = UnifiedCoordinator::new(cfg("alice-bld", alice_port, None))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_pai(BUILDER_PAI_VALUE)
        .send()
        .await
        .expect("invite.send()");

    let raw = next_inbound_invite_trace(&mut bob_events, Duration::from_secs(5))
        .await
        .expect("inbound INVITE trace");

    assert!(
        raw.contains("P-Asserted-Identity:"),
        "Path 2: builder PAI must appear on wire; trace:\n{raw}"
    );
    assert!(
        raw.contains(BUILDER_PAI_VALUE),
        "Path 2: PAI value must come from the builder; trace:\n{raw}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn path3_builder_without_pai_suppresses_config_default() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_SUPPRESS;

    let bob = boot_receiver(bob_port, "bob-sup").await;
    let mut bob_events = bob.events().await.expect("bob events");

    // Config has a PAI — but the builder explicitly suppresses it.
    let alice = UnifiedCoordinator::new(cfg("alice-sup", alice_port, Some(CONFIG_PAI_VALUE)))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .without_pai()
        .send()
        .await
        .expect("invite.send()");

    let raw = next_inbound_invite_trace(&mut bob_events, Duration::from_secs(5))
        .await
        .expect("inbound INVITE trace");

    assert!(
        !raw.contains("P-Asserted-Identity:"),
        "Path 3: without_pai() must suppress Config default; trace:\n{raw}"
    );
}
