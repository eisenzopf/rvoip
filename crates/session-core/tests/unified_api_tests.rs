//! Tests for the unified API
//!
//! These tests demonstrate the unified coordinator API usage

use rvoip_session_core::api::unified::{Config, SipContactMode, SipTlsMode, UnifiedCoordinator};
use rvoip_session_core::state_table::types::SessionId;
use rvoip_session_core::types::CallState;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::{ContentLength, HeaderName, TypedHeader};
use rvoip_sip_core::{parse_message, Message, Method};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Create a test configuration with unique ports.
///
/// Builds on `Config::local` so newly-added fields (TLS paths, SRTP,
/// PAI, outbound proxy, etc.) inherit their default-off values
/// automatically — older inline-literal initializers had to enumerate
/// every field.
fn test_config(base_port: u16) -> Config {
    let mut config = Config::local("test", base_port);
    config.media_port_start = base_port + 1000;
    config.media_port_end = base_port + 2000;
    config
}

#[tokio::test]
async fn test_create_coordinator() {
    let coordinator = UnifiedCoordinator::new(test_config(15200)).await;
    assert!(coordinator.is_ok());
}

#[tokio::test]
async fn tls_client_only_config_does_not_require_endpoint_certificates() {
    let mut config = test_config(15229);
    config.sip_tls_mode = SipTlsMode::ClientOnly;
    config.local_uri = "sips:test@127.0.0.1".to_string();
    config.contact_uri = Some("sips:test@127.0.0.1:15229;transport=tls".to_string());

    let coordinator = UnifiedCoordinator::new(config).await;
    assert!(
        coordinator.is_ok(),
        "client-only SIP TLS must not require tls_cert_path/tls_key_path: {:?}",
        coordinator.err()
    );
}

#[test]
fn reachable_tls_client_only_requires_explicit_contact_uri() {
    let mut config = test_config(15233);
    config.sip_tls_mode = SipTlsMode::ClientOnly;
    config.local_uri = "sips:test@127.0.0.1".to_string();

    let err = config
        .validate()
        .expect_err("ClientOnly reachable Contact needs explicit Contact");
    assert!(
        err.to_string().contains("contact_uri"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn rfc5626_registered_flow_requires_outbound_and_instance() {
    let mut config = test_config(15234);
    config.sip_tls_mode = SipTlsMode::ClientOnly;
    config.sip_contact_mode = SipContactMode::RegisteredFlowRfc5626;

    let err = config
        .validate()
        .expect_err("RFC5626 mode must require sip_outbound_enabled");
    assert!(
        err.to_string().contains("sip_outbound_enabled"),
        "unexpected validation error: {err}"
    );

    config.sip_outbound_enabled = true;
    let err = config
        .validate()
        .expect_err("RFC5626 mode must require sip_instance");
    assert!(
        err.to_string().contains("sip_instance"),
        "unexpected validation error: {err}"
    );

    config.sip_instance = Some("urn:uuid:00000000-0000-4000-8000-000000000001".to_string());
    config
        .validate()
        .expect("RFC5626 registered-flow should be valid with outbound params");
}

#[test]
fn symmetric_registered_flow_allows_tls_client_only_without_listener_certificates() {
    let config = test_config(15235)
        .tls_registered_flow_symmetric("urn:uuid:00000000-0000-4000-8000-000000000002");

    config
        .validate()
        .expect("symmetric registered-flow should not require listener cert/key");
}

#[test]
fn rfc5626_registered_flow_helper_sets_outbound_params() {
    let config = test_config(15238)
        .tls_registered_flow_rfc5626("urn:uuid:00000000-0000-4000-8000-000000000003");

    assert_eq!(config.sip_tls_mode, SipTlsMode::ClientOnly);
    assert_eq!(
        config.sip_contact_mode,
        SipContactMode::RegisteredFlowRfc5626
    );
    assert!(config.sip_outbound_enabled);
    assert_eq!(
        config.sip_instance.as_deref(),
        Some("urn:uuid:00000000-0000-4000-8000-000000000003")
    );
    config
        .validate()
        .expect("RFC5626 helper should produce a valid config shape");
}

#[test]
fn reachable_tls_helper_sets_listener_requirements() {
    let config = test_config(15236).tls_reachable_contact(
        "127.0.0.1:15237".parse().unwrap(),
        "/tmp/rvoip-test-cert.pem",
        "/tmp/rvoip-test-key.pem",
    );

    assert_eq!(config.sip_tls_mode, SipTlsMode::ClientAndServer);
    assert_eq!(config.sip_contact_mode, SipContactMode::ReachableContact);
    assert_eq!(config.tls_bind_addr.unwrap().port(), 15237);
    config
        .validate()
        .expect("reachable TLS helper should produce a valid config shape");
}

#[test]
fn advertised_signaling_addresses_are_distinct_from_bind_addresses() {
    let mut config = test_config(15239);
    config.bind_addr = "0.0.0.0:15239".parse().unwrap();
    config.sip_advertised_addr = Some("192.0.2.10:15239".parse().unwrap());
    config.tls_bind_addr = Some("0.0.0.0:15240".parse().unwrap());
    config.tls_advertised_addr = Some("192.0.2.10:15240".parse().unwrap());

    assert_eq!(config.bind_addr.to_string(), "0.0.0.0:15239");
    assert_eq!(
        config.sip_advertised_addr.unwrap().to_string(),
        "192.0.2.10:15239"
    );
    assert_eq!(config.tls_bind_addr.unwrap().to_string(), "0.0.0.0:15240");
    assert_eq!(
        config.tls_advertised_addr.unwrap().to_string(),
        "192.0.2.10:15240"
    );
}

#[test]
fn reachable_tls_helper_defaults_advertised_tls_addr_when_bind_is_specific() {
    let config = test_config(15241).tls_reachable_contact(
        "192.0.2.10:15242".parse().unwrap(),
        "/tmp/rvoip-test-cert.pem",
        "/tmp/rvoip-test-key.pem",
    );

    assert_eq!(
        config.tls_advertised_addr.unwrap().to_string(),
        "192.0.2.10:15242"
    );
}

#[tokio::test]
async fn tls_listener_modes_require_endpoint_certificates() {
    let mut config = test_config(15230);
    config.sip_tls_mode = SipTlsMode::ServerOnly;

    let coordinator = UnifiedCoordinator::new(config).await;
    assert!(
        coordinator.is_err(),
        "server-side SIP TLS listener mode must require tls_cert_path/tls_key_path"
    );
}

#[tokio::test]
async fn tls_client_and_server_mode_requires_endpoint_certificates() {
    let mut config = test_config(15231);
    config.sip_tls_mode = SipTlsMode::ClientAndServer;
    config.tls_bind_addr = Some("127.0.0.1:15232".parse().unwrap());

    let coordinator = UnifiedCoordinator::new(config).await;
    assert!(
        coordinator.is_err(),
        "client-and-server SIP TLS mode must require tls_cert_path/tls_key_path"
    );
}

#[tokio::test]
async fn inbound_options_gets_dialog_core_200_without_session_state() {
    let port = 15228;
    let coordinator = UnifiedCoordinator::new(test_config(port)).await.unwrap();

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source_addr = socket.local_addr().unwrap();
    let target_uri = format!("sip:test@127.0.0.1:{port}");
    let request = SimpleRequestBuilder::new(Method::Options, &target_uri)
        .unwrap()
        .from("Asterisk", "sip:asterisk@example.com", Some("ast-tag"))
        .to("Endpoint", &target_uri, None)
        .call_id("session-core-options-call-id")
        .cseq(1)
        .via(
            &source_addr.to_string(),
            "UDP",
            Some("z9hG4bK-session-core-options"),
        )
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    socket
        .send_to(
            &Message::Request(request).to_bytes(),
            format!("127.0.0.1:{port}"),
        )
        .await
        .unwrap();

    let mut buf = [0u8; 4096];
    let (len, _) = timeout(Duration::from_secs(1), socket.recv_from(&mut buf))
        .await
        .expect("timed out waiting for OPTIONS response")
        .unwrap();

    let message = parse_message(&buf[..len]).unwrap();
    let response = match message {
        Message::Response(response) => response,
        other => panic!("expected OPTIONS response, got {other:?}"),
    };

    assert_eq!(response.status_code(), 200);
    assert!(response.header(&HeaderName::Allow).is_some());
    assert!(response.header(&HeaderName::ContentLength).is_some());
    rvoip_sip_core::validation::validate_wire_response(&response).unwrap();
    assert!(
        coordinator.list_sessions().await.is_empty(),
        "OPTIONS qualify must not create session-core state"
    );
}

#[tokio::test]
async fn test_make_call() {
    let coordinator = UnifiedCoordinator::new(test_config(15201)).await.unwrap();

    let session_id = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15202")
        .await;

    assert!(session_id.is_ok());
    let session_id = session_id.unwrap();

    // Check state
    let state = coordinator.get_state(&session_id).await;
    assert!(state.is_ok());
    // Should be Initiating
    assert_eq!(state.unwrap(), CallState::Initiating);
}

#[tokio::test]
async fn test_hold_resume() {
    let coordinator = UnifiedCoordinator::new(test_config(15203)).await.unwrap();

    let session_id = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15204")
        .await
        .unwrap();

    // Hold
    let hold_result = coordinator.hold(&session_id).await;
    assert!(hold_result.is_ok());

    // Resume
    let resume_result = coordinator.resume(&session_id).await;
    assert!(resume_result.is_ok());
}

#[tokio::test]
async fn test_conference_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15205)).await.unwrap();

    // Create first call
    let session1 = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15206")
        .await
        .unwrap();

    // Create conference from first call
    let conf_result = coordinator
        .create_conference(&session1, "Board Meeting")
        .await;
    assert!(conf_result.is_ok());

    // Create second call
    let session2 = coordinator
        .make_call("sip:alice@localhost", "sip:charlie@localhost:15207")
        .await
        .unwrap();

    // Add to conference
    let add_result = coordinator.add_to_conference(&session1, &session2).await;
    assert!(add_result.is_ok());

    // Check if in conference
    let in_conf1 = coordinator.is_in_conference(&session1).await;
    assert!(in_conf1.is_ok());
}

#[tokio::test]
async fn test_dtmf_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15214)).await.unwrap();

    let session_id = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15215")
        .await
        .unwrap();

    // Send DTMF digits
    for digit in "1234567890*#".chars() {
        let result = coordinator.send_dtmf(&session_id, digit).await;
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_recording_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15216)).await.unwrap();

    let session_id = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15217")
        .await
        .unwrap();

    // Start recording
    let start_result = coordinator.start_recording(&session_id).await;
    assert!(start_result.is_ok());

    // Stop recording
    let stop_result = coordinator.stop_recording(&session_id).await;
    assert!(stop_result.is_ok());
}

#[tokio::test]
async fn test_session_queries() {
    let coordinator = UnifiedCoordinator::new(test_config(15218)).await.unwrap();

    // List sessions (should be empty)
    let sessions = coordinator.list_sessions().await;
    assert_eq!(sessions.len(), 0);

    // Make a call
    let session_id = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15219")
        .await
        .unwrap();

    // List sessions (should have one)
    let sessions = coordinator.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, session_id);

    // Get session info
    let info = coordinator.get_session_info(&session_id).await;
    assert!(info.is_ok());
    let info = info.unwrap();
    assert_eq!(info.from, "sip:alice@localhost");
    assert_eq!(info.to, "sip:bob@localhost:15219");
}

#[tokio::test]
async fn test_event_subscription() {
    let coordinator = UnifiedCoordinator::new(test_config(15220)).await.unwrap();

    let session_id = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15221")
        .await
        .unwrap();

    // Subscribe to events
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);
    coordinator
        .subscribe(session_id.clone(), move |event| {
            let _ = tx.try_send(event);
        })
        .await;

    // Hangup to generate an event
    let _ = coordinator.hangup(&session_id).await;

    // Should receive event (with timeout)
    let _event = timeout(Duration::from_millis(100), rx.recv()).await;
    // Event system is async, may or may not receive immediately

    // Unsubscribe
    coordinator.unsubscribe(&session_id).await;
}

#[tokio::test]
async fn test_accept_reject_calls() {
    let coordinator = UnifiedCoordinator::new(test_config(15222)).await.unwrap();

    // These will fail without actual incoming calls, but test the API
    let fake_session_id = SessionId::new();

    // Accept
    let accept_result = coordinator.accept_call(&fake_session_id).await;
    assert!(accept_result.is_err()); // No such session

    // Reject
    let reject_result = coordinator.reject_call(&fake_session_id, 486, "Busy").await;
    assert!(reject_result.is_err()); // No such session
}

#[tokio::test]
async fn test_multiple_calls() {
    let coordinator = UnifiedCoordinator::new(test_config(15223)).await.unwrap();

    // Make multiple calls
    let session1 = coordinator
        .make_call("sip:alice@localhost", "sip:bob@localhost:15224")
        .await
        .unwrap();

    let session2 = coordinator
        .make_call("sip:alice@localhost", "sip:charlie@localhost:15225")
        .await
        .unwrap();

    let session3 = coordinator
        .make_call("sip:alice@localhost", "sip:david@localhost:15226")
        .await
        .unwrap();

    // List all sessions
    let sessions = coordinator.list_sessions().await;
    assert_eq!(sessions.len(), 3);

    // Hangup all
    assert!(coordinator.hangup(&session1).await.is_ok());
    assert!(coordinator.hangup(&session2).await.is_ok());
    assert!(coordinator.hangup(&session3).await.is_ok());
}
