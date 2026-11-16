//! Integration: `OutboundCallBuilder::with_plain_rtp(true)` makes one call
//! offer plain RTP (`RTP/AVP`) even though the originating endpoint is
//! configured for DTLS-SRTP/WebRTC.
//!
//! This is the per-call switch a B2BUA needs when one leg terminates on a plain
//! RTP peer (a SIP carrier trunk, a legacy PBX) while its other legs stay on
//! the secure profile. The plain offer must carry no ICE candidates, no DTLS
//! fingerprint and no SRTP keying, and the answer negotiation must accept the
//! plain answer despite `srtp_required = true` on the endpoint.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::SrtpKeyingMode;

fn reserve_loopback_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("reserve loopback port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn wait_for<T>(
    events: &mut rvoip_sip::api::stream_peer::EventReceiver,
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
            Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_rtp_per_call_offer_reaches_a_plain_peer_and_answers() {
    let _ = tracing_subscriber::fmt::try_init();

    let alice_port = reserve_loopback_port();
    let mut bob_port = reserve_loopback_port();
    while bob_port == alice_port {
        bob_port = reserve_loopback_port();
    }

    // Alice is a secure-voice endpoint: SRTP offered and required (SDES, so the
    // test runs with default features). Only this one call is switched to plain
    // RTP.
    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.offer_srtp = true;
    alice_cfg.srtp_required = true;
    alice_cfg.srtp_keying = SrtpKeyingMode::Sdes;

    // Bob is a plain RTP peer (no DTLS, no SRTP, no ICE).
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

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_plain_rtp(true)
        .send()
        .await
        .expect("alice invite.send()");

    let (bob_session, offer_sdp) = wait_for(&mut bob_events, Duration::from_secs(8), |event| {
        match event {
            Event::IncomingCall {
                call_id, sdp: Some(sdp), ..
            } => Some((call_id.clone(), sdp.clone())),
            _ => None,
        }
    })
    .await
    .expect("bob did not see the INVITE with SDP");

    let media_line = offer_sdp
        .lines()
        .find(|line| line.starts_with("m=audio"))
        .expect("offer has an m=audio line");
    assert!(
        media_line.contains(" RTP/AVP "),
        "a plain-RTP call must offer RTP/AVP, got `{media_line}`\n{offer_sdp}"
    );
    assert!(
        !offer_sdp.contains("a=candidate:"),
        "a plain-RTP call must not offer ICE candidates\n{offer_sdp}"
    );
    assert!(
        !offer_sdp.contains("a=fingerprint:"),
        "a plain-RTP call must not offer a DTLS fingerprint\n{offer_sdp}"
    );
    assert!(
        !offer_sdp.contains("RTP/SAVP"),
        "a plain-RTP call must not offer SRTP transport profiles\n{offer_sdp}"
    );

    bob.accept_call(&bob_session).await.expect("bob accepts");

    let answered = wait_for(&mut alice_events, Duration::from_secs(8), |event| {
        match event {
            Event::CallAnswered { call_id, .. } => Some(Ok(call_id.clone())),
            Event::CallFailed {
                status_code,
                reason,
                ..
            } => Some(Err((*status_code, reason.clone()))),
            _ => None,
        }
    })
    .await
    .expect("alice saw no terminal signaling event");
    let alice_session =
        answered.unwrap_or_else(|(status, reason)| panic!("plain-RTP call failed: {status} {reason}"));

    alice.hangup(&alice_session).await.expect("alice hangup");
}
