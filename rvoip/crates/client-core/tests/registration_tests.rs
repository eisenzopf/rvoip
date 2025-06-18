//! Integration tests for SIP registration functionality
//! 
//! Tests registration, unregistration, refresh, and error handling.

use rvoip_client_core::{
    ClientBuilder, Client, ClientError, ClientEvent,
    registration::{RegistrationConfig, RegistrationStatus},
};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Mock SIP server address for testing
/// In real tests, this would be a test SIP server
const TEST_SERVER: &str = "127.0.0.1:15090";

/// Test basic registration flow
#[tokio::test]
#[ignore = "Requires a SIP server to test against"]
async fn test_basic_registration() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug,rvoip_session_core=info")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("RegistrationTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Create registration config
    let config = RegistrationConfig {
        server_uri: format!("sip:{}", TEST_SERVER),
        from_uri: "sip:test_user@example.com".to_string(),
        contact_uri: format!("sip:test_user@{}", client.get_client_stats().await.local_sip_addr),
        expires: 3600,
        username: Some("test_user".to_string()),
        password: Some("test_password".to_string()),
        realm: Some("example.com".to_string()),
    };

    // Register
    let reg_id = client.register(config.clone()).await
        .expect("Failed to register");

    // Verify registration
    let reg_info = client.get_registration(reg_id).await
        .expect("Failed to get registration info");
    
    assert_eq!(reg_info.status, RegistrationStatus::Active);
    assert_eq!(reg_info.from_uri, config.from_uri);
    assert_eq!(reg_info.server_uri, config.server_uri);

    // Unregister
    client.unregister(reg_id).await
        .expect("Failed to unregister");

    // Verify unregistration
    let reg_info = client.get_registration(reg_id).await
        .expect("Failed to get registration info after unregister");
    
    assert_eq!(reg_info.status, RegistrationStatus::Cancelled);

    client.stop().await.expect("Failed to stop client");
}

/// Test registration with retry on failure
#[tokio::test]
async fn test_registration_with_retry() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("RetryTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Try to register with an invalid server (should fail but retry)
    let config = RegistrationConfig {
        server_uri: "sip:invalid.server:99999".to_string(),
        from_uri: "sip:retry_user@example.com".to_string(),
        contact_uri: format!("sip:retry_user@{}", client.get_client_stats().await.local_sip_addr),
        expires: 3600,
        username: None,
        password: None,
        realm: None,
    };

    // This should fail after retries
    let result = client.register(config).await;
    
    assert!(result.is_err());
    
    // Check that the error is categorized correctly
    if let Err(e) = result {
        assert!(e.is_recoverable() || matches!(e, ClientError::NetworkError { .. }));
        tracing::info!("Expected error with retry: {}", e);
    }

    client.stop().await.expect("Failed to stop client");
}

/// Test multiple registrations
#[tokio::test]
#[ignore = "Requires a SIP server to test against"]
async fn test_multiple_registrations() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("MultiRegTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let mut reg_ids = Vec::new();

    // Register multiple users
    for i in 0..3 {
        let config = RegistrationConfig {
            server_uri: format!("sip:{}", TEST_SERVER),
            from_uri: format!("sip:user{}@example.com", i),
            contact_uri: format!("sip:user{}@{}", i, client.get_client_stats().await.local_sip_addr),
            expires: 3600,
            username: Some(format!("user{}", i)),
            password: Some(format!("password{}", i)),
            realm: Some("example.com".to_string()),
        };

        let reg_id = client.register(config).await
            .expect(&format!("Failed to register user{}", i));
        
        reg_ids.push(reg_id);
    }

    // Verify all registrations
    let all_regs = client.get_all_registrations().await;
    assert_eq!(all_regs.len(), 3);

    // Unregister all
    for reg_id in reg_ids {
        client.unregister(reg_id).await
            .expect("Failed to unregister");
    }

    // Verify all unregistered
    let all_regs = client.get_all_registrations().await;
    assert_eq!(all_regs.len(), 0);

    client.stop().await.expect("Failed to stop client");
}

/// Test registration refresh
#[tokio::test]
#[ignore = "Requires a SIP server to test against"]
async fn test_registration_refresh() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("RefreshTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let config = RegistrationConfig {
        server_uri: format!("sip:{}", TEST_SERVER),
        from_uri: "sip:refresh_user@example.com".to_string(),
        contact_uri: format!("sip:refresh_user@{}", client.get_client_stats().await.local_sip_addr),
        expires: 60, // Short expiry for testing
        username: Some("refresh_user".to_string()),
        password: Some("refresh_password".to_string()),
        realm: Some("example.com".to_string()),
    };

    let reg_id = client.register(config).await
        .expect("Failed to register");

    // Wait a bit
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Refresh the registration
    client.refresh_registration(reg_id).await
        .expect("Failed to refresh registration");

    // Verify refresh time is updated
    let reg_info = client.get_registration(reg_id).await
        .expect("Failed to get registration info");
    
    assert!(reg_info.refresh_time.is_some());
    assert_eq!(reg_info.status, RegistrationStatus::Active);

    client.unregister(reg_id).await
        .expect("Failed to unregister");

    client.stop().await.expect("Failed to stop client");
}

/// Test registration event notifications
#[tokio::test]
#[ignore = "Requires a SIP server to test against"]
async fn test_registration_events() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("EventRegTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    let mut event_rx = client.subscribe_events();

    client.start().await.expect("Failed to start client");

    let config = RegistrationConfig {
        server_uri: format!("sip:{}", TEST_SERVER),
        from_uri: "sip:event_user@example.com".to_string(),
        contact_uri: format!("sip:event_user@{}", client.get_client_stats().await.local_sip_addr),
        expires: 3600,
        username: Some("event_user".to_string()),
        password: Some("event_password".to_string()),
        realm: Some("example.com".to_string()),
    };

    // Start event collection task
    let event_task = tokio::spawn(async move {
        let mut events = Vec::new();
        
        while let Ok(event) = tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
            if let Ok(event) = event {
                tracing::info!("Received event: {:?}", event);
                
                if let ClientEvent::RegistrationStatusChanged { info, .. } = &event {
                    events.push(info.status.clone());
                }
            }
        }
        
        events
    });

    // Register
    let reg_id = client.register(config).await
        .expect("Failed to register");

    // Wait for events
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Unregister
    client.unregister(reg_id).await
        .expect("Failed to unregister");

    // Wait for final events
    tokio::time::sleep(Duration::from_secs(1)).await;

    client.stop().await.expect("Failed to stop client");

    // Check collected events
    let events = event_task.await.expect("Event task panicked");
    
    // Should have received at least Active status
    assert!(events.contains(&RegistrationStatus::Active));
}

/// Test registration convenience methods
#[tokio::test]
#[ignore = "Requires a SIP server to test against"]
async fn test_registration_convenience_methods() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("ConvenienceTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let server_addr: std::net::SocketAddr = TEST_SERVER.parse().unwrap();

    // Use convenience method
    client.register_simple(
        "sip:simple_user@example.com",
        &server_addr,
        Duration::from_secs(3600)
    ).await
    .expect("Failed to register with simple method");

    // Verify registration exists
    let all_regs = client.get_all_registrations().await;
    assert_eq!(all_regs.len(), 1);
    assert_eq!(all_regs[0].from_uri, "sip:simple_user@example.com");

    // Unregister with convenience method
    client.unregister_simple(
        "sip:simple_user@example.com",
        &server_addr
    ).await
    .expect("Failed to unregister with simple method");

    // Verify unregistered
    let all_regs = client.get_all_registrations().await;
    assert_eq!(all_regs.len(), 0);

    client.stop().await.expect("Failed to stop client");
}

/// Test registration error categorization
#[tokio::test]
async fn test_registration_error_categorization() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("ErrorCatTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Test various error scenarios
    
    // 1. Network error (unreachable server)
    let config = RegistrationConfig {
        server_uri: "sip:1.2.3.4:99999".to_string(),
        from_uri: "sip:test@example.com".to_string(),
        contact_uri: "sip:test@localhost".to_string(),
        expires: 3600,
        username: None,
        password: None,
        realm: None,
    };

    match client.register(config).await {
        Err(e) => {
            assert!(e.is_recoverable());
            assert_eq!(e.category(), "network");
            tracing::info!("Network error (expected): {}", e);
        }
        Ok(_) => panic!("Should have failed with network error"),
    }

    client.stop().await.expect("Failed to stop client");
} 