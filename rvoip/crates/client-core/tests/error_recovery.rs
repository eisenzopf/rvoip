//! Integration tests for error recovery and retry mechanisms
//! 
//! Tests retry logic, error categorization, and recovery strategies.

use rvoip_client_core::{
    ClientBuilder, Client, ClientError,
    retry_with_backoff, RetryConfig, ErrorContext,
    RecoveryStrategies, RecoveryAction,
    with_timeout,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Test basic retry with backoff
#[tokio::test]
async fn test_retry_with_backoff_basic() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_clone = attempts.clone();
    
    let result = retry_with_backoff(
        "test_operation",
        RetryConfig::quick(),
        || async move {
            let count = attempts_clone.fetch_add(1, Ordering::SeqCst);
            if count < 2 {
                Err(ClientError::NetworkError {
                    reason: "Simulated network error".to_string()
                })
            } else {
                Ok("Success")
            }
        }
    ).await;
    
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Success");
    assert_eq!(attempts.load(Ordering::SeqCst), 3); // Failed twice, succeeded on third
}

/// Test retry with non-recoverable error
#[tokio::test]
async fn test_retry_non_recoverable() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_clone = attempts.clone();
    
    let result = retry_with_backoff(
        "test_operation",
        RetryConfig::default(),
        || async move {
            attempts_clone.fetch_add(1, Ordering::SeqCst);
            Err(ClientError::InvalidConfiguration {
                field: "test_field".to_string(),
                reason: "Invalid value".to_string()
            })
        }
    ).await;
    
    assert!(result.is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 1); // Should not retry
}

/// Test retry configuration variations
#[tokio::test]
async fn test_retry_configurations() {
    // Test quick config
    let quick_config = RetryConfig::quick();
    assert_eq!(quick_config.max_attempts, 5);
    assert!(quick_config.initial_delay < Duration::from_millis(100));
    assert!(quick_config.use_jitter);
    
    // Test slow config
    let slow_config = RetryConfig::slow();
    assert_eq!(slow_config.max_attempts, 3);
    assert!(slow_config.initial_delay >= Duration::from_secs(1));
    assert!(!slow_config.use_jitter);
    
    // Test custom config
    let custom_config = RetryConfig {
        max_attempts: 10,
        initial_delay: Duration::from_millis(200),
        max_delay: Duration::from_secs(10),
        backoff_multiplier: 1.5,
        use_jitter: true,
    };
    assert_eq!(custom_config.max_attempts, 10);
}

/// Test error context extension
#[tokio::test]
async fn test_error_context() {
    // Test basic context
    let result: Result<(), ClientError> = Err(ClientError::NetworkError {
        reason: "Connection failed".to_string()
    });
    
    let with_context = result.context("Failed to connect to server");
    assert!(with_context.is_err());
    assert!(with_context.unwrap_err().to_string().contains("Failed to connect to server"));
    
    // Test lazy context
    let result: Result<(), ClientError> = Err(ClientError::CallNotFound {
        call_id: uuid::Uuid::new_v4()
    });
    
    let with_lazy_context = result.with_context(|| {
        format!("Call lookup failed at {}", chrono::Utc::now())
    });
    assert!(with_lazy_context.is_err());
    assert!(with_lazy_context.unwrap_err().to_string().contains("Call lookup failed"));
}

/// Test timeout wrapper
#[tokio::test]
async fn test_with_timeout() {
    // Test successful operation within timeout
    let result = with_timeout(
        "quick_operation",
        Duration::from_secs(1),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok::<&str, ClientError>("Success")
        }
    ).await;
    
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Success");
    
    // Test timeout
    let result = with_timeout(
        "slow_operation",
        Duration::from_millis(100),
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<&str, ClientError>("Should timeout")
        }
    ).await;
    
    assert!(result.is_err());
    match result {
        Err(ClientError::OperationTimeout { duration_ms }) => {
            assert_eq!(duration_ms, 100);
        }
        _ => panic!("Expected OperationTimeout error"),
    }
}

/// Test recovery strategies for network errors
#[tokio::test]
async fn test_network_error_recovery() {
    // Test timeout recovery
    let error = ClientError::NetworkError {
        reason: "Connection timeout".to_string()
    };
    let recovery = RecoveryStrategies::recover_network_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::RetryWithBackoff(_))));
    
    // Test connection refused recovery
    let error = ClientError::NetworkError {
        reason: "connection refused".to_string()
    };
    let recovery = RecoveryStrategies::recover_network_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::WaitAndRetry(_))));
    
    // Test server unreachable recovery
    let error = ClientError::ServerUnreachable {
        server: "example.com".to_string()
    };
    let recovery = RecoveryStrategies::recover_network_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::TryAlternateServer)));
}

/// Test recovery strategies for registration errors
#[tokio::test]
async fn test_registration_error_recovery() {
    // Test authentication failure recovery
    let error = ClientError::RegistrationFailed {
        reason: "401 Unauthorized".to_string()
    };
    let recovery = RecoveryStrategies::recover_registration_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::UpdateCredentials)));
    
    // Test server busy recovery
    let error = ClientError::RegistrationFailed {
        reason: "503 Service Unavailable".to_string()
    };
    let recovery = RecoveryStrategies::recover_registration_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::WaitAndRetry(_))));
    
    // Test expired registration recovery
    let error = ClientError::RegistrationExpired;
    let recovery = RecoveryStrategies::recover_registration_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::Reregister)));
}

/// Test recovery strategies for media errors
#[tokio::test]
async fn test_media_error_recovery() {
    // Test codec mismatch recovery
    let error = ClientError::MediaNegotiationFailed {
        reason: "No matching codec found".to_string()
    };
    let recovery = RecoveryStrategies::recover_media_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::RenegotiateCodecs)));
    
    // Test port allocation failure recovery
    let error = ClientError::MediaNegotiationFailed {
        reason: "Failed to allocate RTP port".to_string()
    };
    let recovery = RecoveryStrategies::recover_media_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::ReallocatePorts)));
    
    // Test no compatible codecs recovery
    let error = ClientError::NoCompatibleCodecs;
    let recovery = RecoveryStrategies::recover_media_error(&error, "test_context").await;
    assert!(matches!(recovery, Some(RecoveryAction::UseDefaultCodec)));
}

/// Test error categorization
#[tokio::test]
async fn test_error_categorization() {
    // Test various error categories
    let test_cases = vec![
        (ClientError::NetworkError { reason: "test".to_string() }, "network"),
        (ClientError::RegistrationFailed { reason: "test".to_string() }, "registration"),
        (ClientError::CallNotFound { call_id: uuid::Uuid::new_v4() }, "call"),
        (ClientError::MediaError { details: "test".to_string() }, "media"),
        (ClientError::InvalidConfiguration { field: "test".to_string(), reason: "test".to_string() }, "configuration"),
        (ClientError::InternalError { message: "test".to_string() }, "system"),
    ];
    
    for (error, expected_category) in test_cases {
        assert_eq!(error.category(), expected_category);
    }
}

/// Test is_recoverable logic
#[tokio::test]
async fn test_is_recoverable() {
    // Recoverable errors
    assert!(ClientError::NetworkError { reason: "test".to_string() }.is_recoverable());
    assert!(ClientError::ConnectionTimeout.is_recoverable());
    assert!(ClientError::TransportFailed { reason: "test".to_string() }.is_recoverable());
    
    // Non-recoverable errors
    assert!(!ClientError::InvalidConfiguration { field: "test".to_string(), reason: "test".to_string() }.is_recoverable());
    assert!(!ClientError::NotImplemented { feature: "test".to_string(), reason: "test".to_string() }.is_recoverable());
    assert!(!ClientError::UnsupportedCodec { codec: "test".to_string() }.is_recoverable());
}

/// Test is_auth_error logic
#[tokio::test]
async fn test_is_auth_error() {
    assert!(ClientError::AuthenticationFailed { reason: "test".to_string() }.is_auth_error());
    assert!(ClientError::NotRegistered.is_auth_error());
    assert!(ClientError::RegistrationExpired.is_auth_error());
    
    assert!(!ClientError::NetworkError { reason: "test".to_string() }.is_auth_error());
    assert!(!ClientError::CallNotFound { call_id: uuid::Uuid::new_v4() }.is_auth_error());
}

/// Test integrated retry in client operations
#[tokio::test]
async fn test_client_retry_integration() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("RetryIntegrationTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Make a call (which includes retry logic)
    let result = client.make_call(
        "sip:retry_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await;

    // Should either succeed or fail after retries
    match result {
        Ok(call_id) => {
            tracing::info!("Call succeeded: {}", call_id);
            client.hangup_call(&call_id).await.ok();
        }
        Err(e) => {
            tracing::info!("Call failed after retries: {}", e);
            // Verify it's a call setup error
            assert!(matches!(e, ClientError::CallSetupFailed { .. }));
        }
    }

    client.stop().await.expect("Failed to stop client");
} 