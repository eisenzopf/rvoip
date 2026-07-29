//! End-to-end DTLS-SRTP call regression test.
//!
//! Modeled directly on `srtp_call_integration.rs` (the SDES equivalent):
//! two in-process `UnifiedCoordinator`s, `Config::srtp_keying =
//! SrtpKeyingMode::DtlsSrtp`, place a real `sip:` call. Unlike SDES, the
//! SDP here only ever carries a certificate fingerprint
//! (`a=fingerprint`/`a=setup`) — the actual SRTP keys are derived from a
//! real DTLS 1.2 handshake run over the RTP port after the 200 OK, which
//! is the property this test exists to prove end-to-end through the
//! public API (the handshake engine itself, and the demux bridge that
//! lets it share the RTP port, already have their own unit/integration
//! tests in `rvoip-rtp-core`).
//!
//! Expected wire-level behavior:
//! 1. Alice's INVITE carries `m=audio … UDP/TLS/RTP/SAVP …` (RFC 5764 §8)
//!    plus `a=fingerprint`/`a=setup:actpass` (RFC 8842).
//! 2. Bob's `IncomingCall` event fires; Bob accepts.
//! 3. Bob's 200 OK carries his own fingerprint and a concrete
//!    `a=setup:active` (RFC 5763 §5's recommendation when the offer was
//!    actpass).
//! 4. Both sides run the DTLS handshake (Bob as client, Alice as server,
//!    per the resolved roles) over the RTP port and install matching
//!    SrtpContext pairs.
//! 5. Alice observes `CallAnswered` followed by `MediaSecurityNegotiated`
//!    with `keying == DtlsSrtp`.

#![cfg(feature = "dtls-srtp")]

use std::time::Duration;

use rvoip_sip::api::events::{Event, MediaSecurityKeying};
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, SrtpKeyingMode, UnifiedCoordinator};

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
async fn dtls_srtp_call_negotiates_and_establishes_end_to_end() {
    let _ = tracing_subscriber::fmt::try_init();

    let alice_port = 37161;
    let bob_port = 37171;

    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.offer_srtp = true;
    alice_cfg.srtp_keying = SrtpKeyingMode::DtlsSrtp;

    let mut bob_cfg = Config::local("bob", bob_port);
    bob_cfg.offer_srtp = true;
    bob_cfg.srtp_keying = SrtpKeyingMode::DtlsSrtp;

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");

    let mut alice_events = alice.events().await.expect("alice events");
    let mut bob_events = bob.events().await.expect("bob events");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _alice_session = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await
        .expect("alice invite.send()");

    let incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
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

    let answered = wait_for(&mut alice_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::CallAnswered { .. } | Event::CallFailed { .. })
    })
    .await
    .expect("alice saw no terminal event after DTLS-SRTP call setup");

    match answered {
        Event::CallAnswered { .. } => {}
        Event::CallFailed {
            status_code,
            reason,
            ..
        } => panic!(
            "DTLS-SRTP call setup failed unexpectedly: {} {}",
            status_code, reason
        ),
        _ => unreachable!(),
    }

    // The real proof: a DTLS handshake actually ran and installed real
    // SrtpContext pairs, not just that signaling completed.
    let security = wait_for(&mut alice_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::MediaSecurityNegotiated { .. })
    })
    .await
    .expect("alice never saw MediaSecurityNegotiated after DTLS-SRTP handshake");

    match security {
        Event::MediaSecurityNegotiated {
            keying,
            contexts_installed,
            ..
        } => {
            assert_eq!(keying, MediaSecurityKeying::DtlsSrtp);
            assert!(contexts_installed);
        }
        _ => unreachable!(),
    }

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
}
