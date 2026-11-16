//! End-to-end DTLS-SRTP call regression test.
//!
//! Two in-process `UnifiedCoordinator`s place a real SIP call. SDP carries
//! certificate fingerprints and setup roles, while the SRTP keys come only
//! from the DTLS handshake sharing each call's RTP socket. Both endpoints must
//! report installed SRTP contexts before this test accepts the call as secure.

#![cfg(feature = "dtls-srtp")]

use std::net::UdpSocket;
use std::time::Duration;

use rvoip_sip::api::events::{Event, MediaSecurityKeying, MediaSecurityProfile};
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{
    Config, DtlsActpassAnswerRole, DtlsSetupRole, SrtpKeyingMode, UnifiedCoordinator,
};
use rvoip_sip_core::types::sdp::CryptoSuite;

fn reserve_loopback_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("reserve loopback UDP port")
        .local_addr()
        .expect("read reserved UDP port")
        .port()
}

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
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(event)) if pred(&event) => return Some(event),
            Ok(Some(_)) => {}
        }
    }
}

fn assert_dtls_srtp_installed(event: Event, endpoint: &str, expected_suite: CryptoSuite) {
    match event {
        Event::MediaSecurityNegotiated {
            keying,
            profile,
            suite,
            contexts_installed,
            ..
        } => {
            assert_eq!(keying, MediaSecurityKeying::DtlsSrtp, "{endpoint} keying");
            assert_eq!(suite, expected_suite, "{endpoint} SRTP suite");
            assert_eq!(
                profile,
                MediaSecurityProfile::UdpTlsRtpSavp,
                "{endpoint} RTP profile"
            );
            assert!(contexts_installed, "{endpoint} SRTP contexts not installed");
        }
        other => panic!("{endpoint} returned an unexpected event: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtls_srtp_call_negotiates_and_installs_contexts_on_both_endpoints() {
    run_dtls_srtp_call(
        DtlsSetupRole::Actpass,
        DtlsActpassAnswerRole::default(),
        vec![
            CryptoSuite::AesCm128HmacSha1_80,
            CryptoSuite::AesCm128HmacSha1_32,
        ],
        CryptoSuite::AesCm128HmacSha1_80,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_dtls_offer_negotiates_the_only_configured_profile() {
    run_dtls_srtp_call(
        DtlsSetupRole::Active,
        DtlsActpassAnswerRole::default(),
        vec![CryptoSuite::AesCm128HmacSha1_32],
        CryptoSuite::AesCm128HmacSha1_32,
    )
    .await;
}

/// The opt-in passive answer to an `actpass` offer: the answerer waits for
/// the offerer's ClientHello, and the call still keys both endpoints.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn passive_answer_to_actpass_offer_negotiates_when_opted_in() {
    run_dtls_srtp_call(
        DtlsSetupRole::Actpass,
        DtlsActpassAnswerRole::Passive,
        vec![CryptoSuite::AesCm128HmacSha1_80],
        CryptoSuite::AesCm128HmacSha1_80,
    )
    .await;
}

async fn run_dtls_srtp_call(
    setup_role: DtlsSetupRole,
    bob_actpass_answer: DtlsActpassAnswerRole,
    suites: Vec<CryptoSuite>,
    expected_suite: CryptoSuite,
) {
    let _ = tracing_subscriber::fmt::try_init();

    let alice_port = reserve_loopback_port();
    let mut bob_port = reserve_loopback_port();
    while bob_port == alice_port {
        bob_port = reserve_loopback_port();
    }

    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.offer_srtp = true;
    alice_cfg.srtp_required = true;
    alice_cfg.srtp_keying = SrtpKeyingMode::DtlsSrtp;
    alice_cfg.dtls_setup_role = setup_role;
    alice_cfg.srtp_offered_suites = suites.clone();

    let mut bob_cfg = Config::local("bob", bob_port);
    bob_cfg.offer_srtp = true;
    bob_cfg.srtp_required = true;
    bob_cfg.srtp_keying = SrtpKeyingMode::DtlsSrtp;
    bob_cfg.srtp_offered_suites = suites;
    bob_cfg.dtls_actpass_answer_role = bob_actpass_answer;

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
        .send()
        .await
        .expect("alice invite.send()");

    let incoming = wait_for(&mut bob_events, Duration::from_secs(8), |event| {
        matches!(event, Event::IncomingCall { .. })
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

    let answered = wait_for(&mut alice_events, Duration::from_secs(8), |event| {
        matches!(event, Event::CallAnswered { .. } | Event::CallFailed { .. })
    })
    .await
    .expect("alice saw no terminal signaling event");
    let answer_sdp = match answered {
        Event::CallAnswered { sdp, .. } => sdp.expect("the 200 OK carries the SDP answer"),
        Event::CallFailed {
            status_code,
            reason,
            ..
        } => panic!("DTLS-SRTP setup failed: {status_code} {reason}"),
        _ => unreachable!(),
    };
    // RFC 8842 §5.1: the answer picks the complement of an active/passive
    // offer, and the configured role for an actpass offer.
    let expected_answer_setup = match (setup_role, bob_actpass_answer) {
        (DtlsSetupRole::Active, _) => "a=setup:passive",
        (DtlsSetupRole::Passive, _) => "a=setup:active",
        (_, DtlsActpassAnswerRole::Passive) => "a=setup:passive",
        _ => "a=setup:active",
    };
    assert!(
        answer_sdp.contains(expected_answer_setup),
        "answer should carry {expected_answer_setup}:\n{answer_sdp}"
    );

    let alice_security = wait_for(&mut alice_events, Duration::from_secs(12), |event| {
        matches!(event, Event::MediaSecurityNegotiated { .. })
    })
    .await
    .expect("alice never installed DTLS-derived SRTP contexts");
    assert_dtls_srtp_installed(alice_security, "alice", expected_suite);

    let bob_security = wait_for(&mut bob_events, Duration::from_secs(12), |event| {
        matches!(event, Event::MediaSecurityNegotiated { .. })
    })
    .await
    .expect("bob never installed DTLS-derived SRTP contexts");
    assert_dtls_srtp_installed(bob_security, "bob", expected_suite);

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
}
