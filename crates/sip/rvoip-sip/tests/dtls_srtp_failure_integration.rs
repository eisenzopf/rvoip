//! A failed DTLS-SRTP handshake on a call's RTP (media) component ends the
//! call: `Event::MediaSecurityFailed`, then teardown carrying
//! `Reason: SIP;cause=488;text="DTLS-SRTP handshake failed"` (RFC 3326).
//!
//! The `test-hooks` option `dtls_present_unadvertised_rtp_certificate_for_testing`
//! makes one endpoint present a certificate other than the one its SDP
//! fingerprint names on the RTP handshake, so its peer's fingerprint check
//! fails. The three cases follow the call's state when the check fails:
//! answered (BYE), outbound not yet answered (CANCEL after early media) and
//! inbound not yet answered (488 after early media).

#![cfg(all(feature = "dtls-srtp", feature = "test-hooks"))]

use std::time::Duration;

use rvoip_sip::api::unified::{Config, SrtpKeyingMode};
use rvoip_sip::{
    Event, EventReceiver, MediaSecurityFailure, MediaSecurityKeying, SipTraceDirection, StreamPeer,
};

const REASON: &str = "SIP;cause=488;text=\"DTLS-SRTP handshake failed\"";

fn config(name: &str, sip_port: u16, media: (u16, u16), impostor: bool) -> Config {
    let mut config = Config::local(name, sip_port)
        .with_media_ports(media.0, media.1)
        .trace_passthrough_for_development();
    config.sip_trace.enabled = true;
    config.offer_srtp = true;
    config.srtp_required = true;
    config.srtp_keying = SrtpKeyingMode::DtlsSrtp;
    config.dtls_present_unadvertised_rtp_certificate_for_testing = impostor;
    config
}

fn reason_headers(raw: &str) -> Vec<String> {
    raw.lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("Reason"))
        // RFC 3326 allows whitespace around `;`; compare the parameters.
        .map(|(_, value)| {
            value
                .split(';')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(";")
        })
        .collect()
}

/// Collect events until `done` returns true, failing after `within`.
async fn collect_until(
    events: &mut EventReceiver,
    within: Duration,
    mut done: impl FnMut(&Event) -> bool,
) -> Vec<Event> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(event)) => {
                let stop = done(&event);
                seen.push(event);
                if stop {
                    return seen;
                }
            }
            Ok(None) => panic!("event stream closed"),
            Err(_) => panic!(
                "timed out; saw {:?}",
                seen.iter()
                    .filter(|event| !matches!(event, Event::SipTrace(_)))
                    .collect::<Vec<_>>()
            ),
        }
    }
}

fn position(events: &[Event], pred: impl Fn(&Event) -> bool) -> usize {
    events
        .iter()
        .position(pred)
        .unwrap_or_else(|| panic!("event missing from {} events", events.len()))
}

fn assert_failure_event(events: &[Event]) -> usize {
    let index = position(events, |event| {
        matches!(event, Event::MediaSecurityFailed { .. })
    });
    match &events[index] {
        Event::MediaSecurityFailed { keying, reason, .. } => {
            assert_eq!(*keying, MediaSecurityKeying::DtlsSrtp);
            assert_eq!(*reason, MediaSecurityFailure::FingerprintMismatch);
        }
        _ => unreachable!(),
    }
    index
}

/// Outbound trace whose start line begins with `start`, carrying the
/// expected Reason header.
fn assert_outbound_with_reason(events: &[Event], start: &str) -> usize {
    let index = position(events, |event| {
        matches!(event, Event::SipTrace(trace)
            if trace.direction == SipTraceDirection::Outbound
                && trace.start_line.starts_with(start))
    });
    let Event::SipTrace(trace) = &events[index] else {
        unreachable!()
    };
    assert_eq!(
        reason_headers(&trace.raw_message),
        vec![REASON.to_string()],
        "{}",
        trace.raw_message
    );
    index
}

async fn wait_until_released(peer: &StreamPeer) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !peer.coordinator().list_sessions().await.is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the failed call's session was not released"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answered_call_whose_dtls_fingerprint_fails_is_ended_with_bye_and_reason() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut bob = StreamPeer::with_config(config("bob", 37_401, (37_430, 37_449), true))
        .await
        .unwrap();
    let alice = StreamPeer::with_config(config("alice", 37_400, (37_410, 37_429), false))
        .await
        .unwrap();
    let mut alice_events = alice.coordinator().events().await.unwrap();
    let mut bob_events = bob.coordinator().events().await.unwrap();

    let bob_task = tokio::spawn(async move {
        let incoming = bob.wait_for_incoming().await.unwrap();
        let handle = incoming.accept().await.unwrap();
        (bob, handle)
    });
    alice
        .invite("sip:bob@127.0.0.1:37401".to_string())
        .send()
        .await
        .unwrap();

    // Alice sees Bob's unadvertised certificate on the media handshake.
    let events = collect_until(&mut alice_events, Duration::from_secs(15), |event| {
        matches!(event, Event::CallEnded { .. } | Event::CallFailed { .. })
    })
    .await;
    let failed = assert_failure_event(&events);
    let bye = assert_outbound_with_reason(&events, "BYE ");
    let ended = position(&events, |event| matches!(event, Event::CallEnded { .. }));
    assert!(failed < bye && bye < ended, "event, BYE, then CallEnded");
    match &events[ended] {
        Event::CallEnded { reason, .. } => assert_eq!(reason, "DTLS-SRTP handshake failed"),
        _ => unreachable!(),
    }
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::CallFailed { .. })),
        "one teardown only"
    );

    // Bob receives the BYE with its Reason and ends normally.
    let bob_seen = collect_until(&mut bob_events, Duration::from_secs(5), |event| {
        matches!(event, Event::CallEnded { .. })
    })
    .await;
    let inbound_bye = position(&bob_seen, |event| {
        matches!(event, Event::SipTrace(trace)
            if trace.direction == SipTraceDirection::Inbound && trace.start_line.starts_with("BYE "))
    });
    let Event::SipTrace(trace) = &bob_seen[inbound_bye] else {
        unreachable!()
    };
    assert_eq!(reason_headers(&trace.raw_message), vec![REASON.to_string()]);
    assert!(!bob_seen
        .iter()
        .any(|event| matches!(event, Event::MediaSecurityFailed { .. })));

    wait_until_released(&alice).await;
    let (bob, _handle) = bob_task.await.unwrap();
    wait_until_released(&bob).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unanswered_outbound_call_whose_early_media_dtls_fails_is_cancelled_with_reason() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut bob = StreamPeer::with_config(config("bob", 37_451, (37_480, 37_499), true))
        .await
        .unwrap();
    let alice = StreamPeer::with_config(config("alice", 37_450, (37_460, 37_479), false))
        .await
        .unwrap();
    let mut alice_events = alice.coordinator().events().await.unwrap();

    let bob_task = tokio::spawn(async move {
        let incoming = bob.wait_for_incoming().await.unwrap();
        incoming.send_early_media(None).await.unwrap();
        // Never answers; Alice's CANCEL ends the call.
        tokio::time::sleep(Duration::from_secs(8)).await;
        (bob, incoming)
    });
    alice
        .invite("sip:bob@127.0.0.1:37451".to_string())
        .send()
        .await
        .unwrap();

    let events = collect_until(&mut alice_events, Duration::from_secs(15), |event| {
        matches!(
            event,
            Event::CallCancelled { .. } | Event::CallFailed { .. } | Event::CallEnded { .. }
        )
    })
    .await;
    let failed = assert_failure_event(&events);
    let cancel = assert_outbound_with_reason(&events, "CANCEL ");
    assert!(failed < cancel);
    assert!(
        !events.iter().any(|event| matches!(event, Event::SipTrace(trace)
            if trace.direction == SipTraceDirection::Outbound && trace.start_line.starts_with("BYE "))),
        "an unanswered call is cancelled, not BYE'd"
    );
    wait_until_released(&alice).await;
    bob_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unanswered_inbound_call_whose_early_media_dtls_fails_is_rejected_488_with_reason() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut bob = StreamPeer::with_config(config("bob", 37_501, (37_530, 37_549), false))
        .await
        .unwrap();
    let alice = StreamPeer::with_config(config("alice", 37_500, (37_510, 37_529), true))
        .await
        .unwrap();
    let mut alice_events = alice.coordinator().events().await.unwrap();
    let mut bob_events = bob.coordinator().events().await.unwrap();

    let bob_task = tokio::spawn(async move {
        let incoming = bob.wait_for_incoming().await.unwrap();
        incoming.send_early_media(None).await.unwrap();
        tokio::time::sleep(Duration::from_secs(8)).await;
        (bob, incoming)
    });
    alice
        .invite("sip:bob@127.0.0.1:37501".to_string())
        .send()
        .await
        .unwrap();

    // Bob sees Alice's unadvertised certificate before answering.
    let bob_seen = collect_until(&mut bob_events, Duration::from_secs(15), |event| {
        matches!(event, Event::SipTrace(trace)
            if trace.direction == SipTraceDirection::Outbound
                && trace.start_line.starts_with("SIP/2.0 488"))
    })
    .await;
    let failed = assert_failure_event(&bob_seen);
    let rejected = assert_outbound_with_reason(&bob_seen, "SIP/2.0 488");
    assert!(failed < rejected);

    let alice_seen = collect_until(&mut alice_events, Duration::from_secs(5), |event| {
        matches!(event, Event::CallFailed { .. })
    })
    .await;
    match alice_seen.last() {
        Some(Event::CallFailed { status_code, .. }) => assert_eq!(*status_code, 488),
        other => panic!("{other:?}"),
    }
    wait_until_released(&alice).await;
    bob_task.abort();
}
