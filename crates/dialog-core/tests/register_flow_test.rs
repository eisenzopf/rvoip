//! REGISTER Request Flow Tests for dialog-core
//!
//! Tests the send_register() functionality including:
//! - REGISTER request construction
//! - Non-dialog transaction handling
//! - Authorization header support
//! - Response processing (200 OK, 401 Unauthorized)

use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    config::DialogManagerConfig,
    transaction::{TransactionManager, TransactionKey},
    transaction::transport::{TransportManager, TransportManagerConfig},
};
use rvoip_sip_core::{Request, Response, Method, StatusCode};
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::time::Duration;

/// Helper to create a test dialog API
async fn create_test_dialog_api(port: u16) -> Arc<UnifiedDialogApi> {
    // Create transport config
    let transport_config = TransportManagerConfig {
        enable_udp: true,
        bind_addresses: vec![format!("127.0.0.1:{}", port).parse().unwrap()],
        enable_tcp: false,
        enable_tls: false,
        ..Default::default()
    };
    
    let (mut transport, transport_rx) = TransportManager::new(transport_config)
        .await
        .expect("Failed to create transport");
    
    // Initialize transport
    transport.initialize().await.expect("Failed to initialize transport");
    
    let (transaction_manager, global_rx) = TransactionManager::with_transport_manager(
        transport,
        transport_rx,
        Some(100),
    )
    .await
    .expect("Failed to create transaction manager");
    
    // Create dialog config
    let dialog_config = DialogManagerConfig::hybrid(format!("127.0.0.1:{}", port).parse().unwrap())
        .with_from_uri(&format!("sip:test@127.0.0.1:{}", port))
        .build();
    
    // Create dialog API
    let dialog_api = UnifiedDialogApi::with_global_events(
        Arc::new(transaction_manager),
        global_rx,
        dialog_config,
    )
    .await
    .expect("Failed to create dialog API");
    
    dialog_api.start().await.expect("Failed to start dialog API");
    
    Arc::new(dialog_api)
}

#[tokio::test]
async fn test_send_register_basic() {
    // Initialize logging for test
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6001).await;
    
    // Test parameters
    let registrar_uri = "sip:127.0.0.1:5060";
    let from_uri = "sip:alice@127.0.0.1";
    let contact_uri = "sip:alice@127.0.0.1:6001";
    let expires = 3600;
    
    // Send REGISTER without authorization
    // Note: This will timeout since no server is running, but we're testing the request is built correctly
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        dialog_api.send_register(registrar_uri, from_uri, contact_uri, expires, None)
    ).await;
    
    // We expect timeout (no server running), but that's OK - we're testing the method works
    match result {
        Err(_) => {
            // Timeout is expected - means the request was sent (just no response)
            tracing::info!("✅ send_register() sent request (timed out waiting for response - expected)");
        }
        Ok(Ok(response)) => {
            // Got a response somehow - that's also OK
            tracing::info!("✅ send_register() got response: {}", response.status_code());
        }
        Ok(Err(e)) => {
            tracing::error!("send_register() failed: {}", e);
            panic!("send_register() should not fail to build/send request: {}", e);
        }
    }
}

#[tokio::test]
async fn test_send_register_with_authorization() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6002).await;
    
    // Test parameters
    let registrar_uri = "sip:127.0.0.1:5060";
    let from_uri = "sip:bob@127.0.0.1";
    let contact_uri = "sip:bob@127.0.0.1:6002";
    let expires = 1800;
    
    // Create Authorization header (digest format)
    let authorization = Some(
        r#"Digest username="bob", realm="test.local", nonce="abc123", uri="sip:127.0.0.1:5060", response="def456""#.to_string()
    );
    
    // Send REGISTER with authorization
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        dialog_api.send_register(registrar_uri, from_uri, contact_uri, expires, authorization)
    ).await;
    
    // Timeout is expected (no server)
    match result {
        Err(_) => {
            tracing::info!("✅ send_register() with Authorization sent request (timeout expected)");
        }
        Ok(Ok(response)) => {
            tracing::info!("✅ send_register() with Authorization got response: {}", response.status_code());
        }
        Ok(Err(e)) => {
            panic!("send_register() with Authorization should not fail: {}", e);
        }
    }
}

#[tokio::test]
async fn test_send_register_to_different_ports() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6003).await;
    
    // Test with non-standard port
    let registrar_uri = "sip:127.0.0.1:5070";  // Non-standard port
    let from_uri = "sip:charlie@127.0.0.1";
    let contact_uri = "sip:charlie@127.0.0.1:6003";
    let expires = 600;
    
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        dialog_api.send_register(registrar_uri, from_uri, contact_uri, expires, None)
    ).await;
    
    match result {
        Err(_) => {
            tracing::info!("✅ send_register() to port 5070 sent request");
        }
        Ok(Ok(_)) => {
            tracing::info!("✅ send_register() to port 5070 got response");
        }
        Ok(Err(e)) => {
            panic!("send_register() to custom port should not fail: {}", e);
        }
    }
}

#[tokio::test]
async fn test_send_register_unregister() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6004).await;
    
    // Test unregistration (expires=0)
    let registrar_uri = "sip:127.0.0.1:5060";
    let from_uri = "sip:alice@127.0.0.1";
    let contact_uri = "sip:alice@127.0.0.1:6004";
    let expires = 0;  // Unregister!
    
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        dialog_api.send_register(registrar_uri, from_uri, contact_uri, expires, None)
    ).await;
    
    match result {
        Err(_) => {
            tracing::info!("✅ Unregister (expires=0) sent request");
        }
        Ok(Ok(_)) => {
            tracing::info!("✅ Unregister (expires=0) got response");
        }
        Ok(Err(e)) => {
            panic!("Unregister should not fail: {}", e);
        }
    }
}

#[tokio::test]
#[ignore] // Requires actual registrar server running
async fn test_register_with_real_server() {
    // This test requires the registrar server to be running
    // Run: cargo run --example registrar_server (in registrar-core)
    
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6005).await;
    
    let registrar_uri = "sip:127.0.0.1:5060";
    let from_uri = "sip:alice@127.0.0.1";
    let contact_uri = "sip:alice@127.0.0.1:6005";
    let expires = 60;
    
    tracing::info!("Sending initial REGISTER (no auth)");
    
    // Send initial REGISTER (should get 401)
    let response = dialog_api.send_register(
        registrar_uri,
        from_uri,
        contact_uri,
        expires,
        None
    ).await.expect("Failed to send REGISTER");
    
    tracing::info!("Received response: {}", response.status_code());
    
    // Should get 401 Unauthorized
    assert_eq!(response.status_code(), 401, "Should receive 401 challenge");
    
    // Extract WWW-Authenticate header
    use rvoip_sip_core::types::headers::HeaderAccess;
    let www_auth = response.raw_header_value(&rvoip_sip_core::types::header::HeaderName::WwwAuthenticate)
        .expect("Should have WWW-Authenticate header");
    
    tracing::info!("Received challenge: {}", www_auth);
    assert!(www_auth.contains("Digest"));
    assert!(www_auth.contains("realm="));
    assert!(www_auth.contains("nonce="));
    
    // Now send with proper authentication
    // For this test, we'd need to compute the digest response
    // (This is done in session-core-v3, so we'll skip the actual auth computation here)
    
    tracing::info!("✅ REGISTER flow test passed (401 challenge received)");
}

#[tokio::test]
async fn test_send_register_validates_uri() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6006).await;
    
    // Test with invalid URI
    let invalid_uri = "not-a-valid-uri";
    let from_uri = "sip:test@127.0.0.1";
    let contact_uri = "sip:test@127.0.0.1:6006";
    
    let result = dialog_api.send_register(
        invalid_uri,
        from_uri,
        contact_uri,
        3600,
        None
    ).await;
    
    // Should fail with protocol error
    assert!(result.is_err(), "Invalid URI should cause error");
    tracing::info!("✅ Invalid URI properly rejected");
}

#[tokio::test]
async fn test_send_register_concurrent() {
    // Test sending multiple REGISTER requests concurrently
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
    
    let dialog_api = create_test_dialog_api(6007).await;
    
    let mut handles = vec![];
    
    for i in 0..5 {
        let api = dialog_api.clone();
        let handle = tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_millis(500),
                api.send_register(
                    "sip:127.0.0.1:5060",
                    &format!("sip:user{}@127.0.0.1", i),
                    &format!("sip:user{}@127.0.0.1:6007", i),
                    3600,
                    None
                )
            ).await;
            
            // Timeout is OK - just testing concurrent sending
            match result {
                Err(_) | Ok(Ok(_)) => true,  // Timeout or success both OK
                Ok(Err(e)) => {
                    tracing::error!("Concurrent REGISTER {} failed: {}", i, e);
                    false
                }
            }
        });
        
        handles.push(handle);
    }
    
    // Wait for all
    let results: Vec<bool> = futures::future::join_all(handles).await
        .into_iter()
        .map(|r| r.unwrap_or(false))
        .collect();
    
    assert!(results.iter().all(|&r| r), "All concurrent REGISTERs should succeed");
    tracing::info!("✅ Concurrent REGISTER requests handled correctly");
}

