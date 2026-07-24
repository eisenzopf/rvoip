//! Regression test: hold/resume must not fail on a signaling-only session.
//!
//! `MediaMode::SignalingOnly` sessions never register a media-core session
//! for their `media_session_id` (`MediaAdapter::create_session` returns
//! early before calling `controller.start_media`/`store_session_mapping`).
//! Before this fix, `MediaAdapter::set_media_direction` — invoked by the
//! `Action::HoldCurrentCall`/`Action::RestoreMediaFlow` state-machine
//! actions behind `coordinator.hold()`/`.resume()` — still called through to
//! the media-core controller with that unregistered id, failing with
//! "session not found" even though the SIP-level hold/resume re-INVITE
//! itself has nothing to do with local media-core state in this mode.
//!
//! `HoldCall`/`ResumeCall` only have a defined transition from the
//! `Active` state (see `state_tables/default.yaml`), so this needs a
//! fully established call — alice (signaling-only) invites bob (normal
//! media), bob accepts, then alice holds/resumes.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, MediaMode};
use rvoip_sip::UnifiedCoordinator;

/// Wait for any event matching `pred` on `events`, up to `timeout`.
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

#[tokio::test]
async fn hold_and_resume_succeed_on_signaling_only_session() {
    let _ = tracing_subscriber::fmt::try_init();

    let alice_port = 35962;
    let bob_port = 35972;

    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg = alice_cfg.with_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });
    let bob_cfg = Config::local("bob", bob_port);

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");

    let mut alice_events = alice.events().await.expect("alice events");
    let mut bob_events = bob.events().await.expect("bob events");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let session_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{}", alice_port)),
            format!("sip:bob@127.0.0.1:{}", bob_port),
        )
        .send()
        .await
        .expect("alice invite");

    let incoming = wait_for(&mut bob_events, Duration::from_secs(5), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall");
    let bob_session_id = match incoming {
        Event::IncomingCall { call_id, .. } => call_id,
        _ => unreachable!(),
    };

    bob.accept_call(&bob_session_id)
        .await
        .expect("bob accept_call");

    wait_for(&mut alice_events, Duration::from_secs(5), |ev| {
        matches!(ev, Event::CallAnswered { .. })
    })
    .await
    .expect("alice did not observe CallAnswered");

    // Give the session store a moment to land in `Active` state after
    // the ACK/media-flow completion driven by the CallAnswered event.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let hold_result = alice.hold(&session_id).await;
    assert!(
        hold_result.is_ok(),
        "hold() must succeed on a signaling-only session, got: {:?}",
        hold_result
    );

    let resume_result = alice.resume(&session_id).await;
    assert!(
        resume_result.is_ok(),
        "resume() must succeed on a signaling-only session, got: {:?}",
        resume_result
    );

    alice.hangup(&session_id).await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;
}
