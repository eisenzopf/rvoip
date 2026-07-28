//! End-to-end test for the `_with_headers` outbound-INVITE variants on
//! every public API surface.
//!
//! Each test spins up two coordinators on UDP. The *receiver* explicitly
//! enables the development-only trace passthrough so the inbound INVITE is
//! published as [`Event::SipTrace`] with the unredacted packet bytes intact.
//! The test asserts that the caller-supplied extra headers (a custom
//! `Call-Info` URI and a vendor `X-Tenant-ID` header) appear in those bytes;
//! the latter is serialized with the builder's canonical `X-Tenant-Id`
//! spelling because SIP header identity is case-insensitive.
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
use rvoip_sip::api::headers::SipRequestOptions;
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

const TENANT_HEADER_INPUT_NAME: &str = "X-Tenant-ID";
const TENANT_HEADER_WIRE_NAME: &str = "X-Tenant-Id";
const TENANT_HEADER_VALUE: &str = "acme-prod";
const CALL_INFO_URI: &str = "<sip:helpdesk@example.com>;purpose=info";

fn extra_headers_fixture() -> Vec<TypedHeader> {
    let tenant = TypedHeader::Other(
        HeaderName::Other(TENANT_HEADER_INPUT_NAME.to_string()),
        HeaderValue::text(TENANT_HEADER_VALUE),
    );
    let call_info = TypedHeader::Other(HeaderName::CallInfo, HeaderValue::text(CALL_INFO_URI));
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
    // Treat this loopback-only integration trace as a packet-capture oracle.
    // The compatibility booleans alone intentionally retain production-safe
    // redaction; verbatim header values require this explicit opt-in.
    cfg.trace_passthrough_for_development()
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
                    assert_verbatim_packet(&trace);
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

fn wire_headers(raw_message: &str) -> impl Iterator<Item = (&str, &str)> {
    raw_message
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.trim_end_matches('\r').split_once(':'))
        .map(|(name, value)| (name, value.trim_start()))
}

fn find_wire_header<'a>(
    raw_message: &'a str,
    expected_name: &str,
) -> Option<(usize, &'a str, &'a str)> {
    wire_headers(raw_message)
        .enumerate()
        .find_map(|(index, (name, value))| {
            name.eq_ignore_ascii_case(expected_name)
                .then_some((index, name, value))
        })
}

fn assert_verbatim_packet(trace: &rvoip_sip::SipTrace) {
    assert!(!trace.redacted, "packet-capture trace must be verbatim");
    assert!(
        !trace.truncated,
        "packet-capture trace must not be truncated"
    );
}

fn assert_contains_extras(raw_message: &str) {
    let (_, tenant_name, tenant_value) = find_wire_header(raw_message, TENANT_HEADER_INPUT_NAME)
        .unwrap_or_else(|| panic!("expected tenant header on the wire; got:\n{raw_message}"));
    assert_eq!(
        tenant_name, TENANT_HEADER_WIRE_NAME,
        "custom header must use the builder's canonical wire spelling"
    );
    assert_eq!(tenant_value, TENANT_HEADER_VALUE);

    let (_, call_info_name, call_info_value) = find_wire_header(raw_message, "Call-Info")
        .unwrap_or_else(|| panic!("expected Call-Info on the wire; got:\n{raw_message}"));
    assert_eq!(call_info_name, "Call-Info");
    assert_eq!(call_info_value, CALL_INFO_URI);
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
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_headers(extra_headers_fixture())
        .expect("staging extra headers")
        .send()
        .await
        .expect("invite.send()");

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

    let alice = StreamPeer::with_config(Config::local("alice", alice_port))
        .await
        .expect("alice stream peer");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let _call_id = alice
        .invite(target)
        .with_headers(extra_headers_fixture())
        .expect("staging extra headers")
        .send()
        .await
        .expect("StreamPeer invite.send()");

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
    let _call_id = alice_control
        .invite(target)
        .with_headers(extra_headers_fixture())
        .expect("staging extra headers")
        .send()
        .await
        .expect("CallbackPeerControl invite.send()");

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
    let _call_id = alice
        .invite(&target)
        .expect("Endpoint invite")
        .with_headers(extra_headers_fixture())
        .expect("staging extra headers")
        .send()
        .await
        .expect("Endpoint invite.send()");

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
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_headers(extra_headers_fixture())
        .expect("staging extra headers")
        .send()
        .await
        .expect("invite.send()");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    let raw = &trace.raw_message;
    let (pai_idx, pai_name, pai_value) = find_wire_header(raw, "P-Asserted-Identity")
        .unwrap_or_else(|| panic!("expected P-Asserted-Identity on wire; got:\n{raw}"));
    assert_eq!(pai_name, "P-Asserted-Identity");
    assert_eq!(pai_value, format!("<{pai_uri}>"));

    let (tenant_idx, tenant_name, tenant_value) = find_wire_header(raw, TENANT_HEADER_INPUT_NAME)
        .unwrap_or_else(|| panic!("expected tenant header on wire; got:\n{raw}"));
    assert_eq!(tenant_name, TENANT_HEADER_WIRE_NAME);
    assert_eq!(tenant_value, TENANT_HEADER_VALUE);
    assert!(
        pai_idx < tenant_idx,
        "P-Asserted-Identity (header index {pai_idx}) must precede caller-supplied \
         {TENANT_HEADER_WIRE_NAME} (header index {tenant_idx}). Raw:\n{raw}"
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
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await
        .expect("invite send");

    let trace = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");

    assert!(
        find_wire_header(&trace.raw_message, TENANT_HEADER_INPUT_NAME).is_none(),
        "plain invite must not include the tenant header; got:\n{}",
        trace.raw_message
    );
    assert!(
        find_wire_header(&trace.raw_message, "Call-Info").is_none(),
        "plain invite must not include Call-Info; got:\n{}",
        trace.raw_message
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}
