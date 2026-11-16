//! Integration: the public SIP-negotiated media format of a call.
//!
//! Two in-process coordinators place a real call. Once the answer commits,
//! `UnifiedCoordinator::negotiated_media`,
//! `SessionHandle::negotiated_media` and
//! `SessionHandle::wait_for_negotiated_media` report the codec, payload type,
//! clock rate and channel count both legs agreed, and the two legs report
//! the same format.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

fn reserve_loopback_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("reserve loopback port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn wait_for<T>(
    events: &mut EventReceiver,
    timeout: Duration,
    mut predicate: impl FnMut(&Event) -> Option<T>,
) -> Option<T> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Ok(Some(event)) => {
                if let Some(value) = predicate(&event) {
                    return Some(value);
                }
            }
            Ok(None) | Err(_) => return None,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn negotiated_media_reports_the_committed_format_on_both_legs() {
    let _ = tracing_subscriber::fmt::try_init();

    let alice_port = reserve_loopback_port();
    let mut bob_port = reserve_loopback_port();
    while bob_port == alice_port {
        bob_port = reserve_loopback_port();
    }

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(Config::local("bob", bob_port))
        .await
        .expect("bob coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    let mut bob_events = bob.events().await.expect("bob events");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let alice_call = alice
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{bob_port}"),
        )
        .send()
        .await
        .expect("alice invite.send()");
    let alice_handle = alice.session(&alice_call);

    let bob_call = wait_for(&mut bob_events, Duration::from_secs(8), |event| {
        match event {
            Event::IncomingCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        }
    })
    .await
    .expect("bob did not see the INVITE");
    bob.accept_call(&bob_call).await.expect("bob accepts");

    wait_for(&mut alice_events, Duration::from_secs(8), |event| match event {
        Event::CallAnswered { call_id, .. } if call_id == &alice_call => Some(()),
        Event::CallFailed {
            status_code,
            reason,
            ..
        } => panic!("call failed: {status_code} {reason}"),
        _ => None,
    })
    .await
    .expect("alice saw no answer");

    let waited = alice_handle
        .wait_for_negotiated_media(Some(Duration::from_secs(5)))
        .await
        .expect("alice negotiated media");
    assert!(!waited.codec.is_empty(), "codec name is reported");
    assert!(waited.payload_type < 128, "payload type is an RTP payload type");
    assert!(waited.clock_rate > 0, "clock rate is reported");
    assert!(waited.channels >= 1, "channel count is reported");

    let from_handle = alice_handle
        .negotiated_media()
        .await
        .expect("handle lookup")
        .expect("alice handle reports the negotiated media");
    let from_coordinator = alice
        .negotiated_media(&alice_call)
        .await
        .expect("coordinator lookup")
        .expect("alice coordinator reports the negotiated media");
    assert_eq!(from_handle, waited);
    assert_eq!(from_coordinator, waited);

    // The answerer committed the same selection.
    let bob_media = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(media) = bob.negotiated_media(&bob_call).await.expect("bob lookup") {
                return media;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("bob reports the negotiated media");
    assert!(
        bob_media.codec.eq_ignore_ascii_case(&waited.codec),
        "legs disagree on the codec: {bob_media:?} vs {waited:?}"
    );
    assert_eq!(
        (bob_media.payload_type, bob_media.clock_rate, bob_media.channels),
        (waited.payload_type, waited.clock_rate, waited.channels)
    );

    alice.hangup(&alice_call).await.expect("alice hangup");
}
