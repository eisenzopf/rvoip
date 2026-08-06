//! End-to-end ICE (RFC 8445) call regression test.
//!
//! Modeled directly on `dtls_srtp_call_integration.rs`: two in-process
//! `UnifiedCoordinator`s, `Config::enable_ice = true`, place a real
//! `sip:` call. The SDP on both sides carries `a=ice-ufrag`/`a=ice-pwd`/
//! `a=candidate` lines; after the 200 OK, both sides run a real
//! `webrtc-ice` connectivity check over the same RTP port (demuxed by
//! `rvoip-rtp-core`'s shared-socket STUN bridge — that bridge and the
//! `IceAgent` itself already have their own unit/integration tests in
//! `rvoip-nat-core`/`rvoip-rtp-core`) and each side publishes
//! `Event::IceConnected` once its side resolves.
//!
//! A second test proves ICE and DTLS-SRTP coexist on the same call
//! (both ride the same shared RTP socket, demuxed by first-byte
//! classification per RFC 7983) — this is the scenario that would break
//! first if the two bridges' demux routing ever collided.

#![cfg(feature = "ice")]

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};

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

async fn run_ice_call(alice_port: u16, bob_port: u16, dtls_srtp: bool) {
    let _ = tracing_subscriber::fmt::try_init();

    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.enable_ice = true;
    let mut bob_cfg = Config::local("bob", bob_port);
    bob_cfg.enable_ice = true;

    if dtls_srtp {
        alice_cfg.offer_srtp = true;
        alice_cfg.srtp_keying = rvoip_sip::api::unified::SrtpKeyingMode::DtlsSrtp;
        bob_cfg.offer_srtp = true;
        bob_cfg.srtp_keying = rvoip_sip::api::unified::SrtpKeyingMode::DtlsSrtp;
    }

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
    .expect("alice saw no terminal event after ICE call setup");

    match answered {
        Event::CallAnswered { .. } => {}
        Event::CallFailed {
            status_code,
            reason,
            ..
        } => panic!(
            "ICE call setup failed unexpectedly: {} {}",
            status_code, reason
        ),
        _ => unreachable!(),
    }

    // The real proof: a genuine ICE connectivity check ran and resolved
    // a selected candidate pair, not just that signaling completed.
    let alice_ice = wait_for(&mut alice_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IceConnected { .. })
    })
    .await
    .expect("alice never saw IceConnected");
    let bob_ice = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IceConnected { .. })
    })
    .await
    .expect("bob never saw IceConnected");

    for event in [alice_ice, bob_ice] {
        match event {
            Event::IceConnected { selected_addr, .. } => {
                assert!(selected_addr.ip().is_loopback());
                assert_ne!(selected_addr.port(), 0);
            }
            _ => unreachable!(),
        }
    }

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ice_call_negotiates_and_connects_end_to_end() {
    run_ice_call(37271, 37281, false).await;
}

#[cfg(feature = "dtls-srtp")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ice_and_dtls_srtp_coexist_on_the_same_call() {
    run_ice_call(37291, 37301, true).await;
}

/// TEST-NET-3 (RFC 5737), so an address on no interface of any machine: an
/// `a=candidate` carrying it can only have come from the external mapping.
const EXTERNAL_MEDIA_IP: &str = "203.0.113.10";

/// `Config::media_public_addr` reaches the ICE candidates, not just the SDP
/// `c=` line.
///
/// This is the static-NAT case: the process listens on a private address and
/// the world reaches it on a public one. Advertising the public address in `c=`
/// while ICE advertises the private one is how a call connects silent, so the
/// two have to agree.
///
/// The assertion is on the offer rather than on connectivity, deliberately:
/// the mapped address is unroutable from this machine by construction, so no
/// connectivity check can succeed against it. What matters here is what goes
/// on the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_public_addr_reaches_the_ice_candidates_on_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();

    let (alice_port, bob_port) = (37311, 37321);

    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.enable_ice = true;
    alice_cfg.media_public_addr = Some(
        format!("{EXTERNAL_MEDIA_IP}:0")
            .parse()
            .expect("external media address"),
    );

    let mut bob_cfg = Config::local("bob", bob_port);
    bob_cfg.enable_ice = true;
    // Bob traces so the assertion reads Alice's offer exactly as it arrived,
    // rather than a re-serialization of it.
    bob_cfg.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    // The booleans above still leave production-safe redaction in place; the
    // verbatim packet an SDP assertion needs requires this test-only opt-in.
    let bob_cfg = bob_cfg.trace_passthrough_for_development();

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");

    let mut bob_events = bob.events().await.expect("bob events");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let _alice_session = alice
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{bob_port}"),
        )
        .send()
        .await
        .expect("alice invite.send()");

    let invite = wait_for(&mut bob_events, Duration::from_secs(8), |event| {
        matches!(
            event,
            Event::SipTrace(trace)
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with("INVITE")
        )
    })
    .await
    .expect("bob never received Alice's INVITE");

    let offer = match invite {
        Event::SipTrace(trace) => trace.raw_message,
        _ => unreachable!(),
    };

    let candidates: Vec<&str> = offer
        .lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .collect();
    assert!(
        !candidates.is_empty(),
        "the offer carried no ICE candidates:\n{offer}"
    );
    assert!(
        candidates
            .iter()
            .any(|line| line.contains(EXTERNAL_MEDIA_IP)),
        "no ICE candidate carried the configured public address {EXTERNAL_MEDIA_IP}: {candidates:?}"
    );
    assert!(
        !candidates.iter().any(|line| line.contains("127.0.0.1")),
        "the private address is still advertised alongside the public one, so SDP and ICE \
         disagree about where media arrives: {candidates:?}"
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
}
