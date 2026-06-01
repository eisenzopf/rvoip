//! SIP_API_DESIGN_2 §10 verification #9 (smoke slice).
//!
//! Asserts that application-staged headers reach the wire when the
//! new outbound builders are used. Each test stamps a sentinel
//! `X-Test: smoke` header via the builder's `with_raw_header` and
//! reads the resulting wire bytes off the receiver's SIP-trace
//! channel.
//!
//! Out-of-dialog methods (INVITE / MESSAGE / OPTIONS) run end-to-end
//! today. The in-dialog companions (REFER / NOTIFY / INFO / BYE) use
//! the shared `tests/support/` established-call harness to drive
//! INVITE → 200 → ACK → <method> against an auto-accepting peer.

use std::time::Duration;

use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

mod support;

use support::{
    assert_header_on_wire, boot_unified_caller, establish_call, wait_for_inbound_method,
    SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE,
};

const PAIR_INVITE: (u16, u16) = (16200, 16210);
const PAIR_MESSAGE: (u16, u16) = (16220, 16230);
const PAIR_OPTIONS: (u16, u16) = (16240, 16250);
const PAIR_BYE: (u16, u16) = (16260, 16270);
const PAIR_INFO: (u16, u16) = (16280, 16290);
const PAIR_REFER: (u16, u16) = (16300, 16310);
const PAIR_NOTIFY: (u16, u16) = (16320, 16330);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INVITE;

    let bob = boot_unified_caller(bob_port, "bob").await;
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

    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn message_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_MESSAGE;

    let bob = boot_unified_caller(bob_port, "bob").await;
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

    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    let _ = tokio::time::timeout(Duration::from_secs(12), alice_handle).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn options_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_OPTIONS;

    let bob = boot_unified_caller(bob_port, "bob").await;
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

    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    tokio::time::sleep(Duration::from_millis(150)).await;
}

// ─────────────────────────────────────────────────────────────────────
// In-dialog smoke tests — establish an INVITE dialog, then stamp
// `X-Test: smoke` via the matching builder's `with_raw_header` and
// assert the header arrives on the wire at the peer.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bye_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_BYE;
    let mut call = establish_call(alice_port, bob_port).await;

    call.alice
        .bye(&call.call_id)
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on BYE builder")
        .send()
        .await
        .expect("bye().send()");

    let trace = wait_for_inbound_method(&mut call.bob_events, "BYE", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound BYE trace");
    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    // BYE already terminated; skip the default teardown's redundant BYE.
    call.bob.shutdown().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn info_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INFO;
    let mut call = establish_call(alice_port, bob_port).await;

    call.alice
        .info(&call.call_id, "application/dtmf-relay")
        .with_body("Signal=1\r\nDuration=160\r\n")
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on INFO builder")
        .send()
        .await
        .expect("info().send()");

    let trace = wait_for_inbound_method(&mut call.bob_events, "INFO", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound INFO trace");
    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    call.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refer_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_REFER;
    let mut call = establish_call(alice_port, bob_port).await;

    call.alice
        .refer(&call.call_id, "sip:carol@127.0.0.1")
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on REFER builder")
        .send()
        .await
        .expect("refer().send()");

    let trace = wait_for_inbound_method(&mut call.bob_events, "REFER", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound REFER trace");
    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    call.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notify_builder_extras_reach_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_NOTIFY;
    let mut call = establish_call(alice_port, bob_port).await;

    call.alice
        .notify(&call.call_id, "presence")
        .with_subscription_state("active;expires=3600")
        .with_raw_header(
            rvoip_sip::HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
            SMOKE_HEADER_VALUE,
        )
        .expect("with_raw_header on NOTIFY builder")
        .send()
        .await
        .expect("notify().send()");

    let trace = wait_for_inbound_method(&mut call.bob_events, "NOTIFY", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound NOTIFY trace");
    assert_header_on_wire(&trace.raw_message, SMOKE_HEADER_NAME, SMOKE_HEADER_VALUE);

    call.teardown().await;
}
