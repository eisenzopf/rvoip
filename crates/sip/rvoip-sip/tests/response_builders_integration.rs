//! SIP_API_DESIGN_2 §10 verification #10 — UAS response builders
//! stamp Retry-After, Warning, WWW-Authenticate, and contact lists on
//! the wire.
//!
//! Scenarios: each test boots Alice as the UAC and Bob as a
//! `CallbackPeer` whose `CallHandler` exercises one response builder.
//! Alice's `SipTraceConfig` captures the inbound response and asserts
//! the application headers landed on the wire.

use std::time::{Duration, Instant};

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::respond::AuthScheme;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};

fn cfg(name: &str, port: u16) -> Config {
    let mut c = Config::local(name, port);
    c.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    c
}

async fn wait_for_inbound_response_status(
    events: &mut EventReceiver,
    status_prefix: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let target = format!("SIP/2.0 {status_prefix}");
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with(&target)
                {
                    return Some(trace.raw_message);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Scenario 1: reject_builder() with Retry-After + Warning
// ─────────────────────────────────────────────────────────────────────

struct RejectWith503;
#[async_trait::async_trait]
impl CallHandler for RejectWith503 {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call
            .reject_builder()
            .with_status(503)
            .with_reason("Service Unavailable")
            .with_retry_after(120)
            .with_warning(307, "rvoip-test", "circuit-saturated")
            .send()
            .await;
        CallHandlerDecision::Reject {
            status: 503,
            reason: "Service Unavailable".into(),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reject_builder_stamps_retry_after_and_warning_on_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let alice_port = 17900;
    let bob_port = 17901;

    let bob = CallbackPeer::new(RejectWith503, cfg("bob-r", bob_port))
        .await
        .expect("bob");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(cfg("alice-r", alice_port))
        .await
        .expect("alice");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _ = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await;

    let raw = wait_for_inbound_response_status(&mut alice_events, "503", Duration::from_secs(8))
        .await
        .expect("alice did not see an inbound 503");

    assert!(
        raw.contains("Retry-After: 120"),
        "expected Retry-After: 120 on the wire; got:\n{raw}"
    );
    assert!(
        raw.contains("circuit-saturated"),
        "expected warn text on the wire; got:\n{raw}"
    );
    assert!(
        raw.contains("Warning:") && raw.contains("307"),
        "expected Warning: 307 ... on the wire; got:\n{raw}"
    );

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

struct AcceptAll;
#[async_trait::async_trait]
impl CallHandler for AcceptAll {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        CallHandlerDecision::Accept
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_call_admission_limit_rejects_with_503_retry_after_on_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let alice_port = 17920;
    let bob_port = 17921;

    let bob_cfg = cfg("bob-admission", bob_port)
        .with_server_call_admission_limit(1)
        .with_server_overload_retry_after_secs(2);
    let bob = CallbackPeer::new(AcceptAll, bob_cfg).await.expect("bob");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(cfg("alice-admission", alice_port))
        .await
        .expect("alice");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let first = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await
        .expect("first invite");
    alice
        .session(&first)
        .wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("first call should be active");

    let _ = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await;

    let raw = wait_for_inbound_response_status(&mut alice_events, "503", Duration::from_secs(8))
        .await
        .expect("alice did not see an overload 503");

    assert!(
        raw.contains("Retry-After: 2"),
        "expected overload Retry-After: 2 on the wire; got:\n{raw}"
    );

    let _ = alice
        .session(&first)
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_call_admission_hysteresis_recovers_below_soft_limit() {
    let _ = tracing_subscriber::fmt::try_init();
    let alice_port = 17922;
    let bob_port = 17923;

    let bob_cfg = cfg("bob-hysteresis", bob_port)
        .with_server_call_admission_limit(2)
        .with_server_call_admission_soft_limit(1)
        .with_server_call_admission_pacing_delay_ms(1)
        .with_server_overload_retry_after_secs(2);
    let bob = CallbackPeer::new(AcceptAll, bob_cfg).await.expect("bob");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(cfg("alice-hysteresis", alice_port))
        .await
        .expect("alice");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let first = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await
        .expect("first invite");
    alice
        .session(&first)
        .wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("first call should be active");

    let second = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await
        .expect("second invite");
    alice
        .session(&second)
        .wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("second call should be admitted while at soft threshold");

    let _ = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await;
    let raw = wait_for_inbound_response_status(&mut alice_events, "503", Duration::from_secs(8))
        .await
        .expect("alice did not see hard-overload 503");
    assert!(
        raw.contains("Retry-After: 2"),
        "expected overload Retry-After: 2 on the wire; got:\n{raw}"
    );

    let _ = alice
        .session(&first)
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let _ = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await;
    let _ = wait_for_inbound_response_status(&mut alice_events, "503", Duration::from_secs(8))
        .await
        .expect("server should remain unavailable until below soft threshold");

    let _ = alice
        .session(&second)
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let recovered = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await
        .expect("recovered invite");
    alice
        .session(&recovered)
        .wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("server should recover once below soft threshold");
    let _ = alice
        .session(&recovered)
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await;

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_call_admission_soft_limit_paces_before_hard_limit() {
    let _ = tracing_subscriber::fmt::try_init();
    let alice_port = 17924;
    let bob_port = 17925;

    let bob_cfg = cfg("bob-soft-pace", bob_port)
        .with_server_call_admission_limit(3)
        .with_server_call_admission_soft_limit(1)
        .with_server_call_admission_pacing_delay_ms(150)
        .with_server_overload_retry_after_secs(2);
    let bob = CallbackPeer::new(AcceptAll, bob_cfg).await.expect("bob");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(cfg("alice-soft-pace", alice_port))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let first = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target.clone())
        .send()
        .await
        .expect("first invite");
    alice
        .session(&first)
        .wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("first call should be active");

    let started = Instant::now();
    let second = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await
        .expect("second invite");
    alice
        .session(&second)
        .wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("second call should be admitted after pacing");

    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "expected soft-limit pacing before admitting the second call"
    );

    let _ = alice
        .session(&first)
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await;
    let _ = alice
        .session(&second)
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

// ─────────────────────────────────────────────────────────────────────
// Scenario 2: challenge_builder() stamps 401 + WWW-Authenticate
// ─────────────────────────────────────────────────────────────────────

struct ChallengeWith401;
#[async_trait::async_trait]
impl CallHandler for ChallengeWith401 {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call
            .challenge_builder(AuthScheme::Digest)
            .with_realm("rvoip-test")
            .with_nonce("nonce-test-001")
            .with_algorithm("MD5")
            .with_qop("auth")
            .send()
            .await;
        // The builder handles the wire reply; the CallHandler decision
        // here only steers session-state bookkeeping. `Reject` is the
        // closest non-accept resolution.
        CallHandlerDecision::Reject {
            status: 401,
            reason: "Unauthorized".into(),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn challenge_builder_stamps_www_authenticate_on_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let alice_port = 17910;
    let bob_port = 17911;

    let bob = CallbackPeer::new(ChallengeWith401, cfg("bob-c", bob_port))
        .await
        .expect("bob");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(cfg("alice-c", alice_port))
        .await
        .expect("alice");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _ = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await;

    let raw = wait_for_inbound_response_status(&mut alice_events, "401", Duration::from_secs(8))
        .await
        .expect("alice did not see an inbound 401");

    assert!(
        raw.contains("WWW-Authenticate:"),
        "expected WWW-Authenticate on the wire; got:\n{raw}"
    );
    assert!(
        raw.contains("realm=\"rvoip-test\""),
        "expected realm on the wire; got:\n{raw}"
    );
    assert!(
        raw.contains("nonce=\"nonce-test-001\""),
        "expected nonce on the wire; got:\n{raw}"
    );

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}
