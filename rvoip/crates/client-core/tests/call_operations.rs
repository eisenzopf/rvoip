//! Integration tests for call operations
//! 
//! Tests making calls, answering, hanging up, and call state management.

use rvoip_client_core::{
    ClientBuilder, Client, ClientError, ClientEvent,
    call::{CallState, CallDirection, CallId},
    retry_with_backoff, RetryConfig, ErrorContext,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Test making a basic outgoing call
#[tokio::test]
async fn test_make_outgoing_call() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("CallTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Make a call
    let call_id = client.make_call(
        "sip:alice@example.com".to_string(),
        "sip:bob@example.com".to_string(),
        Some("Test call".to_string()),
    ).await
    .expect("Failed to make call");

    // Verify call exists and has correct state
    let call_info = client.get_call(&call_id).await
        .expect("Failed to get call info");
    
    assert_eq!(call_info.call_id, call_id);
    assert_eq!(call_info.direction, CallDirection::Outgoing);
    assert_eq!(call_info.local_uri, "sip:alice@example.com");
    assert_eq!(call_info.remote_uri, "sip:bob@example.com");
    assert_eq!(call_info.subject, Some("Test call".to_string()));
    assert_eq!(call_info.state, CallState::Initiating);

    // Hang up the call
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    // Verify call is terminated
    let call_info = client.get_call(&call_id).await
        .expect("Failed to get call info after hangup");
    
    assert_eq!(call_info.state, CallState::Terminated);
    assert!(call_info.ended_at.is_some());

    client.stop().await.expect("Failed to stop client");
}

/// Test call with retry on network failure
#[tokio::test]
async fn test_call_with_retry() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("RetryCallTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // The make_call method already includes retry logic
    // Test that it handles retries properly
    let call_result = client.make_call(
        "sip:test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await;

    // Should succeed or fail after retries
    match call_result {
        Ok(call_id) => {
            tracing::info!("Call created with ID: {}", call_id);
            client.hangup_call(&call_id).await.ok();
        }
        Err(e) => {
            tracing::info!("Call failed after retries (expected): {}", e);
            assert_eq!(e.category(), "call");
        }
    }

    client.stop().await.expect("Failed to stop client");
}

/// Test multiple concurrent calls
#[tokio::test]
async fn test_multiple_concurrent_calls() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("MultiCallTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let mut call_ids = Vec::new();

    // Make multiple calls
    for i in 0..3 {
        let call_id = client.make_call(
            format!("sip:user{}@example.com", i),
            format!("sip:remote{}@example.com", i),
            Some(format!("Call {}", i)),
        ).await
        .expect(&format!("Failed to make call {}", i));
        
        call_ids.push(call_id);
    }

    // Verify all calls exist
    let active_calls = client.get_active_calls().await;
    assert_eq!(active_calls.len(), 3);

    // Verify stats
    let stats = client.get_client_stats().await;
    assert_eq!(stats.total_calls, 3);

    // Hang up all calls
    for call_id in call_ids {
        client.hangup_call(&call_id).await
            .expect("Failed to hang up call");
    }

    // Verify no active calls
    let active_calls = client.get_active_calls().await;
    assert_eq!(active_calls.len(), 0);

    client.stop().await.expect("Failed to stop client");
}

/// Test call state tracking
#[tokio::test]
async fn test_call_state_tracking() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("StateTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    let mut event_rx = client.subscribe_events();

    client.start().await.expect("Failed to start client");

    // Start event collector
    let event_collector = tokio::spawn(async move {
        let mut state_changes = Vec::new();
        
        while let Ok(event) = tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
            if let Ok(ClientEvent::CallStateChanged { info, .. }) = event {
                state_changes.push((info.call_id, info.new_state));
            }
        }
        
        state_changes
    });

    // Make a call
    let call_id = client.make_call(
        "sip:state_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Verify initial state
    let call_info = client.get_call(&call_id).await
        .expect("Failed to get call info");
    assert_eq!(call_info.state, CallState::Initiating);

    // Wait a bit for potential state changes
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Hang up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    // Wait for events
    tokio::time::sleep(Duration::from_millis(100)).await;

    client.stop().await.expect("Failed to stop client");

    // Check collected state changes
    let state_changes = event_collector.await.expect("Event collector panicked");
    
    // Should have at least the terminated state
    assert!(state_changes.iter().any(|(id, state)| 
        *id == call_id && *state == CallState::Terminated
    ));
}

/// Test call history and filtering
#[tokio::test]
async fn test_call_history_and_filtering() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("HistoryTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Make some calls
    let call1 = client.make_call(
        "sip:alice@example.com".to_string(),
        "sip:bob@example.com".to_string(),
        None,
    ).await.expect("Failed to make call 1");

    let call2 = client.make_call(
        "sip:alice@example.com".to_string(),
        "sip:charlie@example.com".to_string(),
        None,
    ).await.expect("Failed to make call 2");

    // Get all calls
    let all_calls = client.list_calls().await;
    assert_eq!(all_calls.len(), 2);

    // Get calls by state
    let initiating_calls = client.get_calls_by_state(CallState::Initiating).await;
    assert_eq!(initiating_calls.len(), 2);

    // Get calls by direction
    let outgoing_calls = client.get_calls_by_direction(CallDirection::Outgoing).await;
    assert_eq!(outgoing_calls.len(), 2);

    // Hang up one call
    client.hangup_call(&call1).await
        .expect("Failed to hang up call 1");

    // Check active vs history
    let active_calls = client.get_active_calls().await;
    assert_eq!(active_calls.len(), 1);
    assert_eq!(active_calls[0].call_id, call2);

    let call_history = client.get_call_history().await;
    assert_eq!(call_history.len(), 1);
    assert_eq!(call_history[0].call_id, call1);

    // Clean up
    client.hangup_call(&call2).await
        .expect("Failed to hang up call 2");

    client.stop().await.expect("Failed to stop client");
}

/// Test call metadata and detailed info
#[tokio::test]
async fn test_call_metadata() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("MetadataTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let call_id = client.make_call(
        "sip:metadata_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        Some("Metadata test call".to_string()),
    ).await
    .expect("Failed to make call");

    // Get detailed call info
    let detailed_info = client.get_call_detailed(&call_id).await
        .expect("Failed to get detailed call info");

    // Verify metadata
    assert!(detailed_info.metadata.contains_key("created_via"));
    assert_eq!(detailed_info.metadata.get("created_via"), Some(&"make_call".to_string()));
    assert!(detailed_info.metadata.contains_key("subject"));
    assert_eq!(detailed_info.metadata.get("subject"), Some(&"Metadata test call".to_string()));
    assert!(detailed_info.metadata.contains_key("session_id"));

    // Verify timestamps
    assert!(detailed_info.created_at.timestamp() > 0);
    assert!(detailed_info.connected_at.is_none()); // Not connected yet
    assert!(detailed_info.ended_at.is_none()); // Not ended yet

    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test error handling for invalid calls
#[tokio::test]
async fn test_call_error_handling() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("ErrorCallTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Try to hang up a non-existent call
    let fake_call_id = CallId::new();
    let result = client.hangup_call(&fake_call_id).await;
    
    assert!(result.is_err());
    match result {
        Err(ClientError::CallNotFound { call_id }) => {
            assert_eq!(call_id, *fake_call_id);
        }
        _ => panic!("Expected CallNotFound error"),
    }

    // Try to get info for non-existent call
    let result = client.get_call(&fake_call_id).await;
    assert!(result.is_err());

    client.stop().await.expect("Failed to stop client");
}

/// Test call operations with error context
#[tokio::test]
async fn test_call_with_error_context() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("ContextTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Make a call with context tracking
    let result = async {
        client.make_call(
            "sip:context_test@example.com".to_string(),
            "sip:remote@example.com".to_string(),
            None,
        ).await
        .context("Failed to establish test call for context demo")
    }.await;

    match result {
        Ok(call_id) => {
            tracing::info!("Call created: {}", call_id);
            
            // Clean up with context
            client.hangup_call(&call_id).await
                .context("Failed to clean up test call")?;
        }
        Err(e) => {
            tracing::info!("Call failed with context: {}", e);
        }
    }

    client.stop().await.expect("Failed to stop client");
} 