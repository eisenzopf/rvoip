//! End-to-end P-Asserted-Identity (RFC 3325) regression test.
//!
//! Stands up two `UnifiedCoordinator` instances on UDP and verifies
//! that a PAI URI configured on the originating side surfaces on the
//! receiving side via `IncomingCallInfo.p_asserted_identity`. Three
//! scenarios:
//!
//! 1. `Config::pai_uri` set on the caller — every outbound INVITE
//!    carries the typed PAI header.
//! 2. `make_call_with_pai(..., Some(uri))` — per-call override.
//! 3. No PAI configured anywhere — receiving side sees `None`.
//!
//! Closes the wire-level test gap called out in
//! `crates/session-core/docs/GENERAL_PURPOSE_SIP_CLIENT_PLAN.md` for
//! Tier B item B1.

use std::sync::Arc;
use std::time::Duration;

use rvoip_session_core::api::events::Event;
use rvoip_session_core::api::stream_peer::EventReceiver;
use rvoip_session_core::api::unified::{Config, UnifiedCoordinator};

/// SIP-standard ports for the PAI integration tests. Each test fn
/// gets its own (alice, bob) pair so transport sockets from a prior
/// test don't need to fully release before the next test boots — even
/// though the workspace cargo config (`.cargo/config.toml`) sets
/// `RUST_TEST_THREADS=1` and the tests run serially.
const PAIR_CONFIG_PAI: (u16, u16) = (5060, 5070);
const PAIR_PER_CALL_PAI: (u16, u16) = (5080, 5090);
const PAIR_NO_PAI: (u16, u16) = (5100, 5110);

/// Wait for the next event matching `pred` on `events`, up to `timeout`.
async fn wait_for<F>(events: &mut EventReceiver, timeout: Duration, mut pred: F) -> Option<Event>
where
    F: FnMut(&Event) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let next = tokio::time::timeout(remaining, events.next()).await;
        match next {
            Err(_) => return None,
            Ok(None) => return None,
            Ok(Some(event)) => {
                if pred(&event) {
                    return Some(event);
                }
            }
        }
    }
}

/// Boot a pair of coordinators with disjoint UDP ports. The caller
/// receives `(alice, bob)`. Use distinct port ranges per test fn so
/// concurrent test runs don't collide.
async fn boot_pair(
    alice_port: u16,
    bob_port: u16,
    alice_pai: Option<String>,
) -> (Arc<UnifiedCoordinator>, Arc<UnifiedCoordinator>) {
    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.pai_uri = alice_pai;
    let mut bob_cfg = Config::local("bob", bob_port);
    bob_cfg.pai_uri = None;

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");

    // Let both transports finish binding before the first INVITE.
    tokio::time::sleep(Duration::from_millis(150)).await;
    (alice, bob)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_pai_uri_surfaces_on_inbound_call() {
    let _ = tracing_subscriber::fmt::try_init();

    let pai = "sip:alice@trusted.carrier.example.com".to_string();
    let (alice_port, bob_port) = PAIR_CONFIG_PAI;
    let (alice, bob) = boot_pair(alice_port, bob_port, Some(pai.clone())).await;

    let mut bob_events = bob.events().await.expect("bob events");

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _alice_session = alice
        .make_call("sip:alice@127.0.0.1", &target)
        .await
        .expect("alice make_call");

    // Bob's IncomingCall event signals that the INVITE has been parsed
    // and the call is sitting in the incoming queue. The structured
    // payload (with PAI) is fetched via get_incoming_call.
    let _incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall");

    let info = bob
        .get_incoming_call()
        .await
        .expect("bob.get_incoming_call returned None");

    assert!(
        info.p_asserted_identity.is_some(),
        "expected PAI to surface on IncomingCallInfo, got None"
    );
    let surfaced = info.p_asserted_identity.unwrap();
    assert!(
        surfaced.contains(&pai),
        "PAI on IncomingCallInfo ({:?}) does not contain originator's pai_uri ({:?})",
        surfaced,
        pai
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_call_pai_overrides_config() {
    let _ = tracing_subscriber::fmt::try_init();

    let cfg_pai = "sip:alice@cfg.example.com".to_string();
    let per_call_pai = "sip:alice@override.example.com".to_string();
    let (alice_port, bob_port) = PAIR_PER_CALL_PAI;
    let (alice, bob) = boot_pair(alice_port, bob_port, Some(cfg_pai.clone())).await;

    let mut bob_events = bob.events().await.expect("bob events");

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _alice_session = alice
        .make_call_with_pai("sip:alice@127.0.0.1", &target, Some(per_call_pai.clone()))
        .await
        .expect("alice make_call_with_pai");

    let _incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall");

    let info = bob
        .get_incoming_call()
        .await
        .expect("bob.get_incoming_call returned None");

    let surfaced = info
        .p_asserted_identity
        .expect("expected per-call PAI to surface");
    assert!(
        surfaced.contains(&per_call_pai),
        "per-call PAI ({:?}) did not override Config::pai_uri ({:?})",
        surfaced,
        cfg_pai
    );
    assert!(
        !surfaced.contains(&cfg_pai),
        "Config::pai_uri leaked through despite per-call override: {:?}",
        surfaced
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_pai_when_neither_config_nor_per_call() {
    let _ = tracing_subscriber::fmt::try_init();

    let (alice_port, bob_port) = PAIR_NO_PAI;
    let (alice, bob) = boot_pair(alice_port, bob_port, None).await;

    let mut bob_events = bob.events().await.expect("bob events");

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _alice_session = alice
        .make_call("sip:alice@127.0.0.1", &target)
        .await
        .expect("alice make_call");

    let _incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall");

    let info = bob
        .get_incoming_call()
        .await
        .expect("bob.get_incoming_call returned None");

    assert!(
        info.p_asserted_identity.is_none(),
        "expected no PAI when neither Config nor per-call PAI is set, got {:?}",
        info.p_asserted_identity
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}
