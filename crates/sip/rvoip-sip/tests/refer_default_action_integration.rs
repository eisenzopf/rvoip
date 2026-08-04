//! `Config::refer_default_action` decides who answers an inbound REFER.
//!
//! Under the historical policy rvoip-sip sends `202 Accepted` for the
//! application when it does not decide within a fixed delay, which turns "not
//! decided yet" into consent. These tests pin both policies: the accepting one
//! still answers on its own (unchanged for consumers that configure nothing),
//! and the decision-required one never does.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::ReferDefaultAction;
use serial_test::serial;

/// Comfortably longer than the historical 500 ms auto-accept window, so a
/// decision taken after it proves the REFER was still the application's to
/// make.
const PAST_LEGACY_WINDOW: Duration = Duration::from_millis(1_500);

/// Media ports live in a band no other integration test in this crate uses,
/// and every peer gets its own slice so two coordinators in one test never
/// contend for the same RTP port.
fn config(name: &str, port: u16, media_base: u16) -> Config {
    let mut config = Config::local(name, port);
    config.media_port_start = media_base;
    config.media_port_end = media_base + 19;
    config
}

struct Peers {
    transferee: std::sync::Arc<UnifiedCoordinator>,
    transferee_events: EventReceiver,
    transferee_call: rvoip_sip::api::handle::CallId,
    /// Kept alive for the duration of the test; dropping it tears the call down.
    _transferor: std::sync::Arc<UnifiedCoordinator>,
}

/// Establish a call from the peer under test to a plain peer, then have the
/// plain peer REFER it elsewhere. The peer under test is the REFER receiver,
/// which is where `refer_default_action` applies.
async fn establish_call_and_send_refer(
    transferee_port: u16,
    transferor_port: u16,
    media_base: u16,
    policy: ReferDefaultAction,
) -> Peers {
    let transferee = UnifiedCoordinator::new(
        config("transferee", transferee_port, media_base).with_refer_default_action(policy),
    )
    .await
    .expect("transferee coordinator");
    let transferor =
        UnifiedCoordinator::new(config("transferor", transferor_port, media_base + 20))
            .await
            .expect("transferor coordinator");
    tokio::time::sleep(Duration::from_millis(250)).await;

    let mut transferee_events = transferee.events().await.expect("transferee events");
    let mut transferor_events = transferor.events().await.expect("transferor events");

    let call = transferee
        .invite(
            Some(format!("sip:transferee@127.0.0.1:{transferee_port}")),
            format!("sip:transferor@127.0.0.1:{transferor_port}"),
        )
        .send()
        .await
        .expect("outbound INVITE");

    // Answer on the transferor side and wait until both ends agree the call is
    // up, so the REFER lands in an established dialog.
    let inbound = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match transferor_events.next().await {
                Some(Event::IncomingCall { call_id, .. }) => return call_id,
                Some(_) => continue,
                None => panic!("transferor event stream closed before IncomingCall"),
            }
        }
    })
    .await
    .expect("inbound call on the transferor");

    transferor
        .accept_call(&inbound)
        .await
        .expect("accept inbound call");

    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match transferee_events.next().await {
                Some(Event::CallAnswered { .. }) => return,
                Some(_) => continue,
                None => panic!("transferee event stream closed before CallAnswered"),
            }
        }
    })
    .await
    .expect("call answered");

    transferor
        .session(&inbound)
        .transfer_blind("sip:charlie@127.0.0.1:19999")
        .await
        .expect("send REFER");

    Peers {
        transferee,
        transferee_events,
        transferee_call: call,
        _transferor: transferor,
    }
}

/// Drain the transferee's events until one of the two REFER outcomes lands.
async fn next_refer_outcome(events: &mut EventReceiver, within: Duration) -> Option<Event> {
    tokio::time::timeout(within, async {
        loop {
            match events.next().await {
                Some(event @ Event::ReferReceived { .. }) => return Some(event),
                Some(event @ Event::ReferDefaultActionApplied { .. }) => return Some(event),
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .unwrap_or(None)
}

/// Consumers that configure nothing keep the historical behaviour: the REFER is
/// accepted for them shortly after it arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn default_policy_still_accepts_an_undecided_refer() {
    assert_eq!(
        Config::local("defaults", 5060).refer_default_action,
        ReferDefaultAction::AcceptAfter(Duration::from_millis(500)),
        "an unconfigured consumer must keep the historical auto-accept"
    );

    let mut peers =
        establish_call_and_send_refer(17_620, 17_621, 41_200, ReferDefaultAction::default()).await;

    let received = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    assert!(
        matches!(received, Some(Event::ReferReceived { .. })),
        "the application still sees the REFER first, got {received:?}"
    );

    let applied = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    match applied {
        Some(Event::ReferDefaultActionApplied {
            status_code,
            accepted,
            ..
        }) => {
            assert!(accepted, "the default policy accepts");
            assert_eq!(status_code, 202);
        }
        other => panic!("expected the default action to answer the REFER, got {other:?}"),
    }

    let _ = peers.transferee.hangup(&peers.transferee_call).await;
}

/// The accept delay is honoured rather than hard-coded: a policy configured
/// well past the historical 500 ms does not answer at 500 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn configured_accept_delay_replaces_the_hard_coded_window() {
    let mut peers = establish_call_and_send_refer(
        17_626,
        17_627,
        41_240,
        ReferDefaultAction::AcceptAfter(Duration::from_secs(3)),
    )
    .await;

    let received = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    assert!(
        matches!(received, Some(Event::ReferReceived { .. })),
        "expected ReferReceived, got {received:?}"
    );

    let early = next_refer_outcome(&mut peers.transferee_events, PAST_LEGACY_WINDOW).await;
    assert!(
        early.is_none(),
        "a configured 3s delay must not answer inside the old 500ms window, got {early:?}"
    );

    let applied = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    match applied {
        Some(Event::ReferDefaultActionApplied { accepted, .. }) => {
            assert!(accepted, "the accepting policy still accepts, just later");
        }
        other => panic!("expected the configured delay to elapse and accept, got {other:?}"),
    }

    let _ = peers.transferee.hangup(&peers.transferee_call).await;
}

/// With an explicit decision required, the REFER stays the application's to
/// answer well past the window that used to decide it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn required_decision_leaves_the_refer_pending_past_the_legacy_window() {
    let mut peers = establish_call_and_send_refer(
        17_622,
        17_623,
        41_280,
        ReferDefaultAction::require_application_decision(Duration::from_secs(30)),
    )
    .await;

    let received = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    let call_id = match received {
        Some(Event::ReferReceived { call_id, .. }) => call_id,
        other => panic!("expected ReferReceived, got {other:?}"),
    };

    // Nothing may answer for the application inside the old race window.
    tokio::time::sleep(PAST_LEGACY_WINDOW).await;
    let intruder =
        next_refer_outcome(&mut peers.transferee_events, Duration::from_millis(200)).await;
    assert!(
        intruder.is_none(),
        "no REFER answer may be sent for the application, got {intruder:?}"
    );

    // The decision is still ours to make, which is only true if the REFER
    // transaction is still pending. Before this policy existed the same call
    // failed here, because the auto-accept had already closed it.
    peers
        .transferee
        .reject_refer(&call_id, 603, "Decline")
        .await
        .expect("the application must still own an undecided REFER");

    let _ = peers.transferee.hangup(&peers.transferee_call).await;
}

/// The decision window is bounded: an application that never answers gets an
/// explicit rejection rather than an open transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn required_decision_rejects_explicitly_once_its_timeout_expires() {
    let mut peers = establish_call_and_send_refer(
        17_624,
        17_625,
        41_320,
        ReferDefaultAction::require_application_decision(Duration::from_secs(1)),
    )
    .await;

    let received = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    assert!(
        matches!(received, Some(Event::ReferReceived { .. })),
        "expected ReferReceived, got {received:?}"
    );

    let applied = next_refer_outcome(&mut peers.transferee_events, Duration::from_secs(8)).await;
    match applied {
        Some(Event::ReferDefaultActionApplied {
            status_code,
            accepted,
            ..
        }) => {
            assert!(
                !accepted,
                "an expired decision window rejects, never accepts"
            );
            assert_eq!(status_code, 603);
        }
        other => panic!("expected an explicit rejection on timeout, got {other:?}"),
    }

    let _ = peers.transferee.hangup(&peers.transferee_call).await;
}
