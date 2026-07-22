//! Regression test for the missing SDP offer on `Event::IncomingCall`.
//!
//! Stands up two `UnifiedCoordinator` instances on UDP, sends a normal
//! INVITE from alice to bob, and asserts that bob observes
//! `Event::IncomingCall { sdp: Some(_), .. }` — not `None`.
//!
//! Before the fix, `SessionCrossCrateEventHandler` captured
//! `session.remote_sdp` before `process_event()` populated it, and
//! published the public event using that stale (always-`None`) snapshot,
//! even though the SDP offer was already available and correctly parsed
//! a few lines earlier in the same handler. See the two call sites in
//! `crates/sip/rvoip-sip/src/adapters/session_event_handler.rs`:
//! `handle_incoming_call_parts` and the legacy `handle_incoming_call`
//! (event_str) path.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

const ALICE_PORT: u16 = 5130;
const BOB_PORT: u16 = 5140;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incoming_call_event_carries_the_offered_sdp() {
    let _ = tracing_subscriber::fmt::try_init();

    let alice_cfg = Config::local("alice", ALICE_PORT);
    let bob_cfg = Config::local("bob", BOB_PORT);

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");

    // Let both transports finish binding before the first INVITE.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut bob_events = bob.events().await.expect("bob events");

    let target = format!("sip:bob@127.0.0.1:{}", BOB_PORT);
    let _alice_session = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await
        .expect("alice invite.send()");

    let incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall");

    match incoming {
        Event::IncomingCall { sdp, .. } => {
            assert!(
                sdp.is_some(),
                "Event::IncomingCall.sdp was None; the offer from alice's \
                 INVITE was not preserved onto the public event"
            );
            let sdp = sdp.unwrap();
            assert!(
                sdp.contains("m=audio"),
                "Event::IncomingCall.sdp did not contain an audio media \
                 line, got: {sdp:?}"
            );
        }
        other => panic!("expected Event::IncomingCall, got {other:?}"),
    }

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}
