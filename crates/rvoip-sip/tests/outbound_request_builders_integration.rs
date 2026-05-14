//! SIP_API_DESIGN_2 §10 verification #9 (smoke slice).
//!
//! Asserts that application-staged headers reach the wire when the
//! new outbound builders are used. Each test stamps a sentinel
//! `X-Test: smoke` header via the builder's `with_raw_header` and
//! reads the resulting wire bytes off the receiver's SIP-trace
//! channel.
//!
//! Out-of-dialog methods (INVITE / MESSAGE / OPTIONS) run end-to-end
//! today. The in-dialog companions (REFER / NOTIFY / INFO / BYE)
//! exist as `#[ignore]`d skeletons — they need an established-call
//! harness (INVITE → 200 → ACK → BYE; INVITE → 200 → ACK → INFO; …)
//! that lands with the full §10 verification suite (PR 11).

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::{SipTraceConfig, SipTraceDirection};

const SMOKE_HEADER_NAME: &str = "X-Test";
const SMOKE_HEADER_VALUE: &str = "smoke";

const PAIR_INVITE: (u16, u16) = (16200, 16210);
const PAIR_MESSAGE: (u16, u16) = (16220, 16230);
const PAIR_OPTIONS: (u16, u16) = (16240, 16250);

fn receiver_config(name: &str, port: u16) -> Config {
    let mut cfg = Config::local(name, port);
    cfg.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    cfg
}

async fn boot_receiver(port: u16, name: &str) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(receiver_config(name, port))
        .await
        .expect("receiver coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;
    coord
}

async fn wait_for_inbound_method(
    events: &mut EventReceiver,
    method_prefix: &str,
    timeout: Duration,
) -> Option<rvoip_sip::SipTrace> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with(method_prefix)
                {
                    return Some(trace);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

fn assert_smoke_header_on_wire(raw_message: &str) {
    assert!(
        raw_message.contains(SMOKE_HEADER_NAME),
        "expected `{SMOKE_HEADER_NAME}` on the wire; got:\n{raw_message}"
    );
    assert!(
        raw_message.contains(SMOKE_HEADER_VALUE),
        "expected smoke value `{SMOKE_HEADER_VALUE}` on the wire; got:\n{raw_message}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INVITE;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _call_id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on INVITE builder")
        .send()
        .await
        .expect("invite().send()");

    let trace = wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert_smoke_header_on_wire(&trace.raw_message);

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn message_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_MESSAGE;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    // Spawn `send` concurrently with `wait_for_inbound_method` —
    // bob's default handler 405s the MESSAGE (no registrar surface),
    // but we only need the wire bytes to assert against.
    let alice_handle = tokio::spawn({
        let alice = alice.clone();
        let target = target.clone();
        async move {
            let _ = alice
                .message(target)
                .with_body("hello")
                .with_from_uri("sip:alice@127.0.0.1")
                .with_raw_header(
                    rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
                    SMOKE_HEADER_VALUE,
                )
                .expect("with_raw_header on MESSAGE builder")
                .send()
                .await;
        }
    });

    let trace = wait_for_inbound_method(&mut bob_events, "MESSAGE", Duration::from_secs(8))
        .await
        .expect("bob did not see inbound MESSAGE trace");

    assert_smoke_header_on_wire(&trace.raw_message);

    let _ = tokio::time::timeout(Duration::from_secs(12), alice_handle).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn options_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_OPTIONS;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _ = alice
        .options(target)
        .with_from_uri("sip:alice@127.0.0.1")
        .with_timeout(Duration::from_secs(2))
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on OPTIONS builder")
        .send()
        .await;

    let trace = wait_for_inbound_method(&mut bob_events, "OPTIONS", Duration::from_secs(8))
        .await
        .expect("bob did not see inbound OPTIONS trace");

    assert_smoke_header_on_wire(&trace.raw_message);

    tokio::time::sleep(Duration::from_millis(150)).await;
}

// ─────────────────────────────────────────────────────────────────────
// In-dialog smoke tests — establish an INVITE dialog, then stamp
// `X-Test: smoke` via the matching builder's `with_raw_header` and
// assert the header arrives on the wire at the peer.
//
// Bob auto-accepts via a `CallHandler` impl so a real dialog forms.
// Alice waits for `CallAnswered`, then sends the mid-dialog request.
// Bob's coordinator publishes the inbound trace, which the test
// consumes off the event channel and asserts on.
// ─────────────────────────────────────────────────────────────────────

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle};
use rvoip_sip::api::incoming::IncomingCall;

const PAIR_BYE: (u16, u16) = (16260, 16270);
const PAIR_INFO: (u16, u16) = (16280, 16290);
const PAIR_REFER: (u16, u16) = (16300, 16310);
const PAIR_NOTIFY: (u16, u16) = (16320, 16330);

struct AutoAccept;
#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

async fn boot_callback_receiver(
    port: u16,
    name: &str,
) -> (Arc<UnifiedCoordinator>, tokio::task::JoinHandle<()>, ShutdownHandle) {
    let bob = CallbackPeer::new(AutoAccept, receiver_config(name, port))
        .await
        .expect("callback peer");
    let coord = bob.coordinator().clone();
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    (coord, task, shutdown)
}

async fn wait_for_call_answered(
    events: &mut EventReceiver,
    target_call_id: &rvoip_sip::api::events::CallId,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return false,
            Ok(Some(Event::CallAnswered { call_id, .. })) if &call_id == target_call_id => {
                return true;
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bye_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_BYE;

    let (bob, bob_task, bob_shutdown) = boot_callback_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let call_id = alice
        .invite(Some(format!("sip:alice@127.0.0.1:{}", alice_port)), target)
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    // Drain any leftover bob inbound INVITE trace so the BYE assertion
    // doesn't match the prior request.
    let _ =
        wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    alice
        .bye(&call_id)
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on BYE builder")
        .send()
        .await
        .expect("bye().send()");

    let trace = wait_for_inbound_method(&mut bob_events, "BYE", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound BYE trace");
    assert_smoke_header_on_wire(&trace.raw_message);

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn info_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INFO;

    let (bob, bob_task, bob_shutdown) = boot_callback_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let call_id = alice
        .invite(Some(format!("sip:alice@127.0.0.1:{}", alice_port)), target)
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    let _ =
        wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    alice
        .info(&call_id, "application/dtmf-relay")
        .with_body("Signal=1\r\nDuration=160\r\n")
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on INFO builder")
        .send()
        .await
        .expect("info().send()");

    let trace = wait_for_inbound_method(&mut bob_events, "INFO", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound INFO trace");
    assert_smoke_header_on_wire(&trace.raw_message);

    let _ = alice.bye(&call_id).send().await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refer_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_REFER;

    let (bob, bob_task, bob_shutdown) = boot_callback_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let call_id = alice
        .invite(Some(format!("sip:alice@127.0.0.1:{}", alice_port)), target)
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    let _ =
        wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    alice
        .refer(&call_id, "sip:carol@127.0.0.1")
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on REFER builder")
        .send()
        .await
        .expect("refer().send()");

    let trace = wait_for_inbound_method(&mut bob_events, "REFER", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound REFER trace");
    assert_smoke_header_on_wire(&trace.raw_message);

    let _ = alice.bye(&call_id).send().await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notify_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_NOTIFY;

    let (bob, bob_task, bob_shutdown) = boot_callback_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let call_id = alice
        .invite(Some(format!("sip:alice@127.0.0.1:{}", alice_port)), target)
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    let _ =
        wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    alice
        .notify(&call_id, "presence")
        .with_subscription_state("active;expires=3600")
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on NOTIFY builder")
        .send()
        .await
        .expect("notify().send()");

    let trace = wait_for_inbound_method(&mut bob_events, "NOTIFY", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound NOTIFY trace");
    assert_smoke_header_on_wire(&trace.raw_message);

    let _ = alice.bye(&call_id).send().await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}
