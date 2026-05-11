//! End-to-end test for the `_with_headers` outbound-INVITE variants on
//! every public API surface.
//!
//! Each test spins up two coordinators on UDP. The *receiver* enables
//! `SipTraceConfig` so the inbound INVITE is published as
//! [`Event::SipTrace`] with the unredacted wire bytes intact. The test
//! asserts that the caller-supplied extra headers (a custom `Call-Info`
//! URI and a vendor `X-Tenant-ID` header) appear in those bytes.
//!
//! The four "appears on wire" tests prove the parameter actually threads
//! through; `pai_appears_before_extra_headers` proves the documented
//! ordering invariant; `default_make_call_omits_extra_headers` is the
//! negative sanity check.
//!
//! Per the workspace's `RUST_TEST_THREADS=1` setting, these tests run
//! serially; each pair uses a distinct port range so a slow socket
//! teardown from a prior test does not collide with the next.

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{
    CallHandler, CallHandlerDecision, CallbackPeer, Endpoint, EndpointProfile, HeaderName,
    IncomingCall, SipTraceConfig, SipTraceDirection, StreamPeer, TypedHeader,
};
use rvoip_sip_core::types::header::HeaderValue;

const PAIR_UNIFIED: (u16, u16) = (15700, 15710);
const PAIR_STREAM: (u16, u16) = (15720, 15730);
const PAIR_CALLBACK: (u16, u16) = (15740, 15750);
const PAIR_ENDPOINT: (u16, u16) = (15760, 15770);
const PAIR_ORDERING: (u16, u16) = (15780, 15790);
const PAIR_DEFAULT: (u16, u16) = (15800, 15810);

const TENANT_HEADER_NAME: &str = "X-Tenant-ID";
const TENANT_HEADER_VALUE: &str = "acme-prod";
const CALL_INFO_URI: &str = "<sip:helpdesk@example.com>;purpose=info";

fn extra_headers_fixture() -> Vec<TypedHeader> {
    let tenant = TypedHeader::Other(
        HeaderName::Other(TENANT_HEADER_NAME.to_string()),
        HeaderValue::text(TENANT_HEADER_VALUE),
    );
    let call_info = TypedHeader::Other(
        HeaderName::CallInfo,
        HeaderValue::text(CALL_INFO_URI),
    );
    vec![tenant, call_info]
}

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

async fn wait_for_inbound_invite(
    events: &mut EventReceiver,
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
                    && trace.start_line.starts_with("INVITE")
                {
                    return Some(trace);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

async fn boot_receiver(port: u16, name: &str) -> Arc<UnifiedCoordinator> {
    let bob = UnifiedCoordinator::new(receiver_config(name, port))
        .await
        .expect("receiver coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;
    bob
}

fn assert_contains_extras(raw_message: &str) {
    assert!(
        raw_message.contains(TENANT_HEADER_NAME),
        "expected `{TENANT_HEADER_NAME}` on the wire; got:\n{raw_message}"
    );
    assert!(
        raw_message.contains(TENANT_HEADER_VALUE),
        "expected tenant value `{TENANT_HEADER_VALUE}` on the wire; got:\n{raw_message}"
    );
    assert!(
        raw_message.contains(CALL_INFO_URI),
        "expected Call-Info URI on the wire; got:\n{raw_message}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unified_coordinator_extra_headers_appear_on_invite() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_UNIFIED;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    alice
        .make_call_with_headers("sip:alice@127.0.0.1", &target, extra_headers_fixture())
        .await
        .expect("make_call_with_headers");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert_contains_extras(&trace.raw_message);

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_peer_call_with_headers_appears_on_invite() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_STREAM;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let mut alice = StreamPeer::with_config(Config::local("alice", alice_port))
        .await
        .expect("alice stream peer");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _handle = alice
        .call_with_headers(&target, extra_headers_fixture())
        .await
        .expect("StreamPeer::call_with_headers");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert_contains_extras(&trace.raw_message);

    bob.terminate_current_session().await.ok();
    alice.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callback_peer_call_with_headers_appears_on_invite() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_CALLBACK;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    struct RejectAll;
    #[async_trait::async_trait]
    impl CallHandler for RejectAll {
        async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
            CallHandlerDecision::Reject {
                status: 486,
                reason: "Busy Here".into(),
            }
        }
    }

    let alice = CallbackPeer::new(RejectAll, Config::local("alice", alice_port))
        .await
        .expect("alice callback peer");
    let shutdown = alice.shutdown_handle();
    let alice_control = alice.control();
    let alice_task = tokio::spawn(async move {
        let _ = alice.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _handle = alice_control
        .call_with_headers(&target, extra_headers_fixture())
        .await
        .expect("CallbackPeerControl::call_with_headers");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert_contains_extras(&trace.raw_message);

    bob.terminate_current_session().await.ok();
    shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), alice_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn endpoint_call_with_headers_appears_on_invite() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_ENDPOINT;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = Endpoint::builder()
        .name("alice")
        .profile(EndpointProfile::Custom(Config::local("alice", alice_port)))
        .build()
        .await
        .expect("alice endpoint");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _call = alice
        .call_with_headers(&target, extra_headers_fixture())
        .await
        .expect("Endpoint::call_with_headers");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert_contains_extras(&trace.raw_message);

    bob.terminate_current_session().await.ok();
    alice.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pai_appears_before_extra_headers_on_wire() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_ORDERING;
    let pai_uri = "sip:alice@trusted.carrier.example.com";

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let mut alice_cfg = Config::local("alice", alice_port);
    alice_cfg.pai_uri = Some(pai_uri.to_string());
    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    alice
        .make_call_with_headers("sip:alice@127.0.0.1", &target, extra_headers_fixture())
        .await
        .expect("make_call_with_headers");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    let raw = &trace.raw_message;
    let pai_idx = raw
        .find("P-Asserted-Identity")
        .unwrap_or_else(|| panic!("expected P-Asserted-Identity on wire; got:\n{raw}"));
    let tenant_idx = raw
        .find(TENANT_HEADER_NAME)
        .unwrap_or_else(|| panic!("expected {TENANT_HEADER_NAME} on wire; got:\n{raw}"));
    assert!(
        pai_idx < tenant_idx,
        "P-Asserted-Identity (offset {pai_idx}) must precede caller-supplied \
         {TENANT_HEADER_NAME} (offset {tenant_idx}). Raw:\n{raw}"
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_make_call_omits_extra_headers() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_DEFAULT;

    let bob = boot_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(Config::local("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    alice
        .make_call("sip:alice@127.0.0.1", &target)
        .await
        .expect("make_call");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert!(
        !trace.raw_message.contains(TENANT_HEADER_NAME),
        "plain make_call must not include {TENANT_HEADER_NAME}; got:\n{}",
        trace.raw_message
    );
    assert!(
        !trace.raw_message.contains(CALL_INFO_URI),
        "plain make_call must not include the Call-Info fixture URI; got:\n{}",
        trace.raw_message
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}
