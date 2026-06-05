//! v0.x MP1 — Orchestrator subscription-table API surface.
//!
//! These tests exercise the routing-table primitives in isolation; MP2
//! adds the UCTP coordinator hook that calls `add_subscription` on
//! inbound `stream.subscribe`, and MP3 adds the media-path fanout that
//! consults `subscribers_for`.

use rvoip_core::config::Config;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::orchestrator::Orchestrator;

fn ids() -> (
    SessionId,
    ConnectionId,
    ConnectionId,
    ConnectionId,
    StreamId,
) {
    (
        SessionId::new(),
        ConnectionId::new(),
        ConnectionId::new(),
        ConnectionId::new(),
        StreamId::new(),
    )
}

#[tokio::test]
async fn add_subscription_then_subscribers_for_returns_subscriber() {
    let orch = Orchestrator::new(Config::default());
    let (sid, sub_a, pub_a, _sub_b, strm) = ids();

    orch.add_subscription(sid.clone(), sub_a.clone(), pub_a.clone(), strm.clone());

    let subs = orch.subscribers_for(&sid, &pub_a, &strm);
    assert_eq!(subs, vec![sub_a]);
}

#[tokio::test]
async fn add_subscription_is_idempotent() {
    let orch = Orchestrator::new(Config::default());
    let (sid, sub, publisher, _, strm) = ids();

    orch.add_subscription(sid.clone(), sub.clone(), publisher.clone(), strm.clone());
    orch.add_subscription(sid.clone(), sub.clone(), publisher.clone(), strm.clone());
    orch.add_subscription(sid.clone(), sub.clone(), publisher.clone(), strm.clone());

    let subs = orch.subscribers_for(&sid, &publisher, &strm);
    assert_eq!(subs.len(), 1, "duplicate adds must collapse to one row");
}

#[tokio::test]
async fn multiple_subscribers_fan_out() {
    let orch = Orchestrator::new(Config::default());
    let (sid, sub_a, publisher, sub_b, strm) = ids();
    let sub_c = ConnectionId::new();

    orch.add_subscription(sid.clone(), sub_a.clone(), publisher.clone(), strm.clone());
    orch.add_subscription(sid.clone(), sub_b.clone(), publisher.clone(), strm.clone());
    orch.add_subscription(sid.clone(), sub_c.clone(), publisher.clone(), strm.clone());

    let mut subs = orch.subscribers_for(&sid, &publisher, &strm);
    subs.sort_by_key(|c| c.to_string());
    let mut expected = vec![sub_a, sub_b, sub_c];
    expected.sort_by_key(|c| c.to_string());
    assert_eq!(subs, expected);
}

#[tokio::test]
async fn remove_subscription_is_idempotent() {
    let orch = Orchestrator::new(Config::default());
    let (sid, sub, publisher, _, strm) = ids();

    orch.add_subscription(sid.clone(), sub.clone(), publisher.clone(), strm.clone());

    assert!(orch.remove_subscription(&sid, &sub, &publisher, &strm));
    // Second remove is a no-op.
    assert!(!orch.remove_subscription(&sid, &sub, &publisher, &strm));
    // Remove of a never-subscribed id is also a no-op.
    let ghost = ConnectionId::new();
    assert!(!orch.remove_subscription(&sid, &ghost, &publisher, &strm));

    assert!(orch.subscribers_for(&sid, &publisher, &strm).is_empty());
}

#[tokio::test]
async fn subscribers_for_unknown_session_returns_empty() {
    let orch = Orchestrator::new(Config::default());
    let (sid, _, publisher, _, strm) = ids();
    assert!(orch.subscribers_for(&sid, &publisher, &strm).is_empty());
}

#[tokio::test]
async fn drop_session_subscriptions_clears_all_rows() {
    let orch = Orchestrator::new(Config::default());
    let (sid, sub_a, pub_a, sub_b, strm) = ids();
    let strm2 = StreamId::new();

    orch.add_subscription(sid.clone(), sub_a.clone(), pub_a.clone(), strm.clone());
    orch.add_subscription(sid.clone(), sub_b.clone(), pub_a.clone(), strm2.clone());
    orch.drop_session_subscriptions(&sid);

    assert!(orch.subscribers_for(&sid, &pub_a, &strm).is_empty());
    assert!(orch.subscribers_for(&sid, &pub_a, &strm2).is_empty());
}

#[tokio::test]
async fn drop_session_subscriptions_also_clears_publisher_registry() {
    // A2 regression: `drop_session_subscriptions` must mirror the wipe
    // into the publisher registry so a recycled SessionId can't resolve
    // `from_participant` lookups against rows belonging to the previous
    // tenant.
    use rvoip_core::subscriptions::PublisherEntry;

    let orch = Orchestrator::new(Config::default());
    let sid = SessionId::new();
    let publisher_connid = ConnectionId::new();
    let registry = orch.publisher_registry();

    registry.register(
        sid.clone(),
        "strm_audio".to_string(),
        PublisherEntry {
            connection: publisher_connid.clone(),
            participant: "alice".to_string(),
            kind: "audio".to_string(),
            codec: None,
        },
    );
    registry.register(
        sid.clone(),
        "strm_video".to_string(),
        PublisherEntry {
            connection: publisher_connid,
            participant: "alice".to_string(),
            kind: "video".to_string(),
            codec: None,
        },
    );
    assert!(registry.entry(&sid, "strm_audio").is_some());
    assert_eq!(registry.streams_for_participant(&sid, "alice").len(), 2);

    orch.drop_session_subscriptions(&sid);

    assert!(
        registry.entry(&sid, "strm_audio").is_none(),
        "publisher rows for the dropped session must be gone"
    );
    assert!(
        registry.entry(&sid, "strm_video").is_none(),
        "all of the session's publisher rows must be gone, not just the first"
    );
    assert!(
        registry.streams_for_participant(&sid, "alice").is_empty(),
        "the participant index must drop along with the primary table"
    );
}

#[tokio::test]
async fn drop_session_subscriptions_is_idempotent_with_uninitialized_publisher_registry() {
    // If the orchestrator was never used for multi-party, the publisher
    // registry's OnceLock never initializes. `drop_session_subscriptions`
    // must not pay to create it just to drop nothing.
    let orch = Orchestrator::new(Config::default());
    let sid = SessionId::new();
    // Should be a no-op; must not panic, must not force registry init.
    orch.drop_session_subscriptions(&sid);
}

#[tokio::test]
async fn subscriptions_are_isolated_per_session() {
    let orch = Orchestrator::new(Config::default());
    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    let publisher = ConnectionId::new();
    let strm = StreamId::new();
    let sub_a = ConnectionId::new();
    let sub_b = ConnectionId::new();

    orch.add_subscription(
        sid_a.clone(),
        sub_a.clone(),
        publisher.clone(),
        strm.clone(),
    );
    orch.add_subscription(
        sid_b.clone(),
        sub_b.clone(),
        publisher.clone(),
        strm.clone(),
    );

    assert_eq!(
        orch.subscribers_for(&sid_a, &publisher, &strm),
        vec![sub_a.clone()]
    );
    assert_eq!(
        orch.subscribers_for(&sid_b, &publisher, &strm),
        vec![sub_b.clone()]
    );
}
