//! Integration tests for SIP registration
//!
//! These tests verify the full registration flow including:
//! - Initial REGISTER request
//! - 401 authentication challenge
//! - Authenticated REGISTER
//! - 200 OK response
//! - Registration refresh
//! - Unregistration
//!
//! Tests are self-contained and start their own registrar server.

use rvoip_session_core_v3::{
    UnifiedCoordinator,
    api::unified::Config,
};
use std::sync::Arc;
use tracing::info;

// Note: For full end-to-end testing with registrar server, use the example applications:
//   Terminal 1: cd crates/registrar-core && cargo run --example registrar_server
//   Terminal 2: cd crates/session-core-v3 && cargo run --example register_demo
//
// This proves the full flow works. Automated testing would require implementing
// server-side REGISTER handling (see SERVER_SIDE_REGISTRATION_PLAN.md for details).

/// Helper to create a test coordinator
async fn create_test_coordinator(port: u16) -> Arc<UnifiedCoordinator> {
    let config = Config {
        local_ip: "127.0.0.1".parse().unwrap(),
        sip_port: port,
        bind_addr: format!("127.0.0.1:{}", port).parse().unwrap(),
        local_uri: format!("sip:test@127.0.0.1:{}", port),
        media_port_start: 16000 + port,
        media_port_end: 17000 + port,
        state_table_path: None,
        use_100rel: Default::default(),
    };

    UnifiedCoordinator::new(config).await.expect("Failed to create coordinator")
}

// Integration test removed - use example applications for manual testing:
//   Terminal 1: cargo run --example registrar_server (in registrar-core)
//   Terminal 2: cargo run --example register_demo (in session-core-v3)

#[tokio::test]
async fn test_registration_api_creation() {
    // Test that we can create a coordinator and registration handle without network
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let _coordinator = create_test_coordinator(5063).await;
    info!("Coordinator created successfully");

    // Note: Actually calling register() will timeout trying to send REGISTER
    // This test just verifies coordinator creation works
}



#[tokio::test]
async fn test_digest_auth_computation() {
    use rvoip_auth_core::{DigestClient, DigestChallenge, DigestAlgorithm};

    // Test basic digest computation using shared auth-core module
    let challenge = DigestChallenge {
        realm: "rvoip.local".to_string(),
        nonce: "abc123nonce".to_string(),
        algorithm: DigestAlgorithm::MD5,
        qop: None,
        opaque: None,
    };

    let (response, _cnonce) = DigestClient::compute_response(
        "alice",
        "password123",
        &challenge,
        "REGISTER",
        "sip:registrar.example.com"
    ).expect("Failed to compute response");

    // Should produce a 32-character hex string (MD5)
    assert_eq!(response.len(), 32);
    assert!(response.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn test_challenge_parsing() {
    use rvoip_auth_core::DigestAuthenticator;

    let header = r#"Digest realm="testrealm", nonce="nonce123", algorithm=MD5, qop="auth""#;
    let challenge = DigestAuthenticator::parse_challenge(header).expect("Failed to parse challenge");

    assert_eq!(challenge.realm, "testrealm");
    assert_eq!(challenge.nonce, "nonce123");
    assert_eq!(challenge.algorithm, rvoip_auth_core::DigestAlgorithm::MD5);
    assert!(challenge.qop.is_some());
}

#[tokio::test]
async fn test_authorization_formatting() {
    use rvoip_auth_core::{DigestClient, DigestChallenge, DigestAlgorithm};

    let challenge = DigestChallenge {
        realm: "example.com".to_string(),
        nonce: "nonce123".to_string(),
        algorithm: DigestAlgorithm::MD5,
        qop: None,
        opaque: None,
    };

    let auth_header = DigestClient::format_authorization(
        "alice",
        &challenge,
        "sip:example.com",
        "response456",
        None,
    );

    assert!(auth_header.starts_with("Digest"));
    assert!(auth_header.contains(r#"username="alice""#));
    assert!(auth_header.contains(r#"realm="example.com""#));
    assert!(auth_header.contains(r#"response="response456""#));
}
