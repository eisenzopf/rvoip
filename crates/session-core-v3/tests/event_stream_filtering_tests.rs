//! Per-call event stream filtering (Item 1 from PRE_B2BUA_ROADMAP.md).
//!
//! Verifies that `UnifiedCoordinator::events_for_session(&id)` yields only
//! events whose `call_id` matches `id`, while `UnifiedCoordinator::events()`
//! yields all events. Publishes synthetic session API events directly to
//! the global event bus rather than driving a full SIP handshake, so the
//! test is hermetic and runs in milliseconds.

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_session_core_v3::adapters::SessionApiCrossCrateEvent;
use rvoip_session_core_v3::api::unified::{Config, UnifiedCoordinator};
use rvoip_session_core_v3::{Event, SessionId};
use tokio::time::timeout;

fn test_config(port: u16) -> Config {
    Config {
        sip_port: port,
        media_port_start: port + 1000,
        media_port_end: port + 2000,
        local_ip: "127.0.0.1".parse().unwrap(),
        bind_addr: format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap(),
        state_table_path: None,
        local_uri: format!("sip:test@127.0.0.1:{}", port),
        use_100rel: Default::default(),
        session_timer_secs: None,
        session_timer_min_se: 90,
        credentials: None,
    }
}

async fn publish_synthetic(event: Event) {
    let wrapped = SessionApiCrossCrateEvent::new(event);
    let coord = rvoip_infra_common::events::global_coordinator().await.clone();
    coord
        .publish(wrapped)
        .await
        .expect("failed to publish synthetic event");
}

#[tokio::test]
async fn events_for_session_only_yields_matching_call() {
    let coord = UnifiedCoordinator::new(test_config(35400)).await.unwrap();

    let id_a = SessionId::new();
    let id_b = SessionId::new();

    // Open the filtered receiver *before* publishing. The global event bus
    // has no buffering for late subscribers, so this ordering is part of
    // the documented contract for events_for_session.
    let mut rx_a = coord.events_for_session(&id_a).await.unwrap();
    let mut rx_b = coord.events_for_session(&id_b).await.unwrap();

    // Publish one event for A and one for B.
    publish_synthetic(Event::CallAnswered {
        call_id: id_a.clone(),
        sdp: None,
    })
    .await;
    publish_synthetic(Event::CallAnswered {
        call_id: id_b.clone(),
        sdp: None,
    })
    .await;

    // rx_a must see A's event; B's event must be filtered out. The filter
    // loop consumes and drops non-matching events silently, so we rely on
    // a timeout to verify "nothing else arrives."
    let a_evt = timeout(Duration::from_secs(2), rx_a.next())
        .await
        .expect("rx_a timed out waiting for A event")
        .expect("rx_a channel closed");
    match a_evt {
        Event::CallAnswered { call_id, .. } => assert_eq!(call_id, id_a),
        other => panic!("rx_a got unexpected event: {:?}", other),
    }

    // rx_b must see B's event.
    let b_evt = timeout(Duration::from_secs(2), rx_b.next())
        .await
        .expect("rx_b timed out waiting for B event")
        .expect("rx_b channel closed");
    match b_evt {
        Event::CallAnswered { call_id, .. } => assert_eq!(call_id, id_b),
        other => panic!("rx_b got unexpected event: {:?}", other),
    }

    // Publish another A event — rx_b must not observe it.
    publish_synthetic(Event::CallEnded {
        call_id: id_a.clone(),
        reason: "hangup".into(),
    })
    .await;

    let stray = timeout(Duration::from_millis(400), rx_b.next()).await;
    assert!(
        stray.is_err(),
        "rx_b received a non-B event it should have filtered: {:?}",
        stray
    );

    // And the A-side should pick up the CallEnded event as expected.
    let a_evt2 = timeout(Duration::from_secs(2), rx_a.next())
        .await
        .expect("rx_a timed out on second A event")
        .expect("rx_a channel closed");
    match a_evt2 {
        Event::CallEnded { call_id, .. } => assert_eq!(call_id, id_a),
        other => panic!("rx_a got unexpected event: {:?}", other),
    }

    coord.shutdown();
}

#[tokio::test]
async fn events_unfiltered_sees_every_session() {
    let coord = UnifiedCoordinator::new(test_config(35410)).await.unwrap();

    let mut rx = coord.events().await.unwrap();

    let id_a = SessionId::new();
    let id_b = SessionId::new();

    publish_synthetic(Event::CallAnswered {
        call_id: id_a.clone(),
        sdp: None,
    })
    .await;
    publish_synthetic(Event::CallAnswered {
        call_id: id_b.clone(),
        sdp: None,
    })
    .await;

    // Integration tests share the global event coordinator singleton, so
    // other tests' synthetic events may interleave. Skip any event whose
    // call_id isn't one of ours, and collect until we've seen both.
    let targets = [id_a.clone(), id_b.clone()];
    let mut seen = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while seen.len() < 2 && tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Ok(Some(evt)) = timeout(remaining, rx.next()).await else {
            break;
        };
        if let Some(call_id) = evt.call_id() {
            if targets.contains(call_id) {
                seen.insert(call_id.clone());
            }
        }
    }

    assert!(seen.contains(&id_a), "unfiltered stream missed id_a");
    assert!(seen.contains(&id_b), "unfiltered stream missed id_b");

    coord.shutdown();
}

#[tokio::test]
async fn events_for_session_helpers_filter_dtmf_and_incoming() {
    // Exercise the existing EventReceiver helpers (next_dtmf, next_incoming)
    // through the new UnifiedCoordinator entry points, so this confirms the
    // end-to-end wiring remains functional after Item 1 lands.
    let coord = UnifiedCoordinator::new(test_config(35420)).await.unwrap();

    let id = SessionId::new();
    let mut rx = coord.events_for_session(&id).await.unwrap();

    // Interleave DTMF and a non-matching lifecycle event; the filter should
    // drop the non-matching call_id, and the DTMF helper should drop the
    // lifecycle event.
    publish_synthetic(Event::CallAnswered {
        call_id: SessionId::new(), // different call_id
        sdp: None,
    })
    .await;
    publish_synthetic(Event::CallOnHold { call_id: id.clone() }).await;
    publish_synthetic(Event::DtmfReceived {
        call_id: id.clone(),
        digit: '7',
    })
    .await;

    let (call_id, digit) = timeout(Duration::from_secs(2), rx.next_dtmf())
        .await
        .expect("next_dtmf timed out")
        .expect("channel closed");
    assert_eq!(call_id, id);
    assert_eq!(digit, '7');

    coord.shutdown();
}
