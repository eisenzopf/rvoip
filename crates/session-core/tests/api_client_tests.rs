//! Tests for the SipClient trait and related functionality
//!
//! These tests verify:
//! - SipClient trait implementation
//! - Registration handling
//! - OPTIONS, MESSAGE, SUBSCRIBE operations
//! - Error handling and edge cases

use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_sip_client_not_enabled_by_default() {
    // Create coordinator without enabling SIP client
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15060) // Use test port
        .build()
        .await
        .unwrap();

    // Try to use SipClient methods - should fail
    let result = coordinator.register(
        "sip:127.0.0.1:5060",  // Use localhost instead of example.com
        "sip:alice@127.0.0.1",
        "sip:alice@127.0.0.1:5060",
        3600,
    ).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::NotSupported { feature, reason } => {
            assert_eq!(feature, "SIP client operations");
            assert!(reason.contains("enable_sip_client"));
        }
        _ => panic!("Expected NotSupported error"),
    }
}

#[tokio::test]
async fn test_sip_client_enabled() {
    // Create coordinator with SIP client enabled
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15061) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    // Now when we send a REGISTER to localhost it will fail to connect but won't fail DNS resolution
    let result = coordinator.register(
        "sip:127.0.0.1:5060",  // Use localhost
        "sip:alice@127.0.0.1",
        "sip:alice@127.0.0.1:15061",
        3600,
    ).await;

    // Since there's no actual SIP server running, we expect a connection/protocol error
    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::Other(_) => {
            // Expected - connection failed or timeout
        }
        SessionError::ProtocolError { .. } => {
            // Also acceptable - server responded with error
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_register_with_zero_expires() {
    // Test de-registration (expires=0)
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15062) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    let result = coordinator.register(
        "sip:127.0.0.1:5060",  // Use localhost
        "sip:alice@127.0.0.1",
        "sip:alice@127.0.0.1:15062",
        0, // De-register
    ).await;

    // Since there's no actual SIP server running, we expect a connection/protocol error
    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::Other(_) => {
            // Expected - connection failed or timeout
        }
        SessionError::ProtocolError { .. } => {
            // Also acceptable
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_register_invalid_uri() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15063) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    // Test with invalid registrar URI
    let result = coordinator.register(
        "not-a-valid-uri",
        "sip:alice@example.com",
        "sip:alice@192.168.1.100:5060",
        3600,
    ).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::InvalidUri(msg) => {
            assert!(msg.contains("Invalid registrar URI"));
        }
        _ => panic!("Expected InvalidUri error"),
    }
}

#[tokio::test]
async fn test_send_options_not_implemented() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15064) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    let result = coordinator.send_options("sip:127.0.0.1:5060").await;

    // Since there's no actual SIP server running, we expect a connection/protocol error
    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::Other(_) => {
            // Expected - connection failed or timeout
        }
        SessionError::NetworkError(_) => {
            // Also acceptable - network error
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_send_message_not_implemented() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15065) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    let result = coordinator.send_message(
        "sip:bob@127.0.0.1:5060",
        "Hello!",
        Some("text/plain"),
    ).await;

    // Since there's no actual SIP server running, we expect a connection/protocol error
    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::Other(_) => {
            // Expected - connection failed or timeout
        }
        SessionError::NetworkError(_) => {
            // Also acceptable - network error
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_subscribe_not_implemented() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15066) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    let result = coordinator.subscribe(
        "sip:alice@example.com",
        "presence",
        3600,
    ).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::NotImplemented { feature } => {
            assert_eq!(feature, "SUBSCRIBE requests");
        }
        _ => panic!("Expected NotImplemented error"),
    }
}

#[tokio::test]
async fn test_send_raw_request_not_implemented() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15067) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    // Create a dummy request
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    let request = SimpleRequestBuilder::options("sip:test@127.0.0.1:5060")
        .unwrap()
        .build();

    let result = coordinator.send_raw_request(
        request,
        Duration::from_secs(5),
    ).await;

    // Since there's no actual SIP server running, we expect a connection/protocol error
    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::Other(_) => {
            // Expected - connection failed or timeout
        }
        SessionError::NetworkError(_) => {
            // Also acceptable - network error
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_registration_handle_fields() {
    let handle = RegistrationHandle {
        transaction_id: "test-123".to_string(),
        expires: 3600,
        contact_uri: "sip:alice@192.168.1.100:5060".to_string(),
        registrar_uri: "sip:registrar.example.com".to_string(),
    };

    assert_eq!(handle.transaction_id, "test-123");
    assert_eq!(handle.expires, 3600);
    assert_eq!(handle.contact_uri, "sip:alice@192.168.1.100:5060");
    assert_eq!(handle.registrar_uri, "sip:registrar.example.com");

    // Test clone
    let cloned = handle.clone();
    assert_eq!(cloned.transaction_id, handle.transaction_id);
}

#[tokio::test]
async fn test_sip_response_fields() {
    use std::collections::HashMap;

    let mut headers = HashMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    headers.insert("Content-Length".to_string(), "13".to_string());

    let response = SipResponse {
        status_code: 200,
        reason_phrase: "OK".to_string(),
        headers: headers.clone(),
        body: Some("Hello, World!".to_string()),
    };

    assert_eq!(response.status_code, 200);
    assert_eq!(response.reason_phrase, "OK");
    assert_eq!(response.headers.len(), 2);
    assert_eq!(response.headers.get("Content-Type"), Some(&"text/plain".to_string()));
    assert_eq!(response.body, Some("Hello, World!".to_string()));

    // Test clone
    let cloned = response.clone();
    assert_eq!(cloned.status_code, response.status_code);
    assert_eq!(cloned.headers.len(), response.headers.len());
}

#[tokio::test]
async fn test_subscription_handle_fields() {
    use std::time::Instant;

    let now = Instant::now();
    let handle = SubscriptionHandle {
        dialog_id: "dlg-123".to_string(),
        event_type: "presence".to_string(),
        expires_at: now + Duration::from_secs(3600),
    };

    assert_eq!(handle.dialog_id, "dlg-123");
    assert_eq!(handle.event_type, "presence");
    assert!(handle.expires_at > now);

    // Test clone
    let cloned = handle.clone();
    assert_eq!(cloned.dialog_id, handle.dialog_id);
    assert_eq!(cloned.event_type, handle.event_type);
}

#[tokio::test]
async fn test_multiple_registrations() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15068) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    // Register multiple endpoints using localhost addresses
    let reg1 = coordinator.register(
        "sip:127.0.0.1:5061",
        "sip:alice@127.0.0.1",
        "sip:alice@127.0.0.1:15068",
        3600,
    ).await;

    let reg2 = coordinator.register(
        "sip:127.0.0.1:5062",
        "sip:bob@127.0.0.1",
        "sip:bob@127.0.0.1:15068",
        7200,
    ).await;

    // Both should fail with connection errors since no SIP servers are running
    assert!(reg1.is_err());
    assert!(reg2.is_err());
    
    // Verify we get expected error types
    match reg1.unwrap_err() {
        SessionError::Other(_) | SessionError::NetworkError(_) => {
            // Expected
        }
        other => panic!("Unexpected error type for reg1: {:?}", other),
    }
    
    match reg2.unwrap_err() {
        SessionError::Other(_) | SessionError::NetworkError(_) => {
            // Expected
        }
        other => panic!("Unexpected error type for reg2: {:?}", other),
    }
}

#[tokio::test]
async fn test_register_with_ipv6() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15069) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    // Test with IPv6 localhost addresses
    let result = coordinator.register(
        "sip:[::1]:5060",
        "sip:alice@[::1]",
        "sip:alice@[::1]:15069",
        3600,
    ).await;

    // Since there's no actual SIP server running, we expect a connection/protocol error
    assert!(result.is_err());
    match result.unwrap_err() {
        SessionError::Other(_) => {
            // Expected - connection failed or timeout
        }
        SessionError::NetworkError(_) => {
            // Also acceptable - network error
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_coordinator_with_sip_client_lifecycle() {
    // Test that SIP client functionality survives coordinator lifecycle
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15070) // Use test port
        .enable_sip_client()
        .build()
        .await
        .unwrap();

    // Start the coordinator
    SessionControl::start(&coordinator).await.unwrap();

    // Use SIP client
    let reg = coordinator.register(
        "sip:127.0.0.1:5060",
        "sip:alice@127.0.0.1",
        "sip:alice@127.0.0.1:15070",
        3600,
    ).await;

    // Expect connection error since no SIP server is running
    assert!(reg.is_err());
    match reg.unwrap_err() {
        SessionError::Other(_) | SessionError::NetworkError(_) => {
            // Expected
        }
        other => panic!("Unexpected error type: {:?}", other),
    }

    // Stop the coordinator
    SessionControl::stop(&coordinator).await.unwrap();
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_sip_client_with_session_operations() {
        // Test that SIP client and session operations can coexist
        let coordinator = SessionManagerBuilder::new()
            .with_sip_port(15071) // Use test port
            .enable_sip_client()
            .build()
            .await
            .unwrap();

        SessionControl::start(&coordinator).await.unwrap();

        // Use SIP client
        let reg = coordinator.register(
            "sip:127.0.0.1:5060",
            "sip:alice@127.0.0.1",
            "sip:alice@127.0.0.1:15071",
            3600,
        ).await;

        // Expect connection error since no SIP server is running
        assert!(reg.is_err());
        match reg.unwrap_err() {
            SessionError::Other(_) | SessionError::NetworkError(_) => {
                // Expected - this verifies SIP client is enabled and attempting to send
            }
            other => panic!("Unexpected error type: {:?}", other),
        }

        // Also create a session to verify both features work together
        // Since we're testing locally, use a different port to avoid conflicts
        let session_result = SessionControl::create_outgoing_call(
            &coordinator,
            "sip:alice@127.0.0.1:15071",
            "sip:bob@127.0.0.1:15072",
            None,
        ).await;

        // This should succeed in creating the session
        assert!(session_result.is_ok());
        let session = session_result.unwrap();
        
        // Verify session was created
        assert!(!session.id().to_string().is_empty());
        
        // Both SIP client and session operations work independently
        // No need to terminate the session as it may already be in a terminated state
        
        SessionControl::stop(&coordinator).await.unwrap();
    }
} 