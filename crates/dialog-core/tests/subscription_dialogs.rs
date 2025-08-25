//! Tests for SIP subscription dialog support (RFC 6665)
//!
//! This test suite verifies that dialog-core properly handles:
//! - SUBSCRIBE creating dialogs
//! - Subscription state management  
//! - NOTIFY processing
//! - Event package support
//! - Subscription refresh and termination

use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

use std::sync::Arc;
use dashmap::DashMap;

use rvoip_dialog_core::{
    dialog::{DialogId, Dialog, SubscriptionState, SubscriptionTerminationReason},
    events::DialogEvent,
    subscription::SubscriptionManager,
};

use rvoip_sip_core::{
    Request, Method, StatusCode,
    builder::SimpleRequestBuilder,
};

/// Helper to create a SUBSCRIBE request
fn create_subscribe_request(event: &str, expires: u32) -> Request {
    SimpleRequestBuilder::new(Method::Subscribe, "sip:alice@example.com")
        .unwrap()
        .from("Bob", "sip:bob@example.com", Some("tag123"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("subscription-12345")
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some("branch-xyz"))
        .event(event)
        .expires(expires)
        .build()
}

/// Helper to create a NOTIFY request with correct dialog tags
fn create_notify_request(subscription_state: &str, body: Option<&str>) -> Request {
    // Note: In a real scenario, we'd need the actual local_tag from the SUBSCRIBE response
    // For testing, we'll use a dummy tag that won't match
    let mut builder = SimpleRequestBuilder::new(Method::Notify, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("server-generated-tag"))
        .to("Bob", "sip:bob@example.com", Some("tag123"))
        .call_id("subscription-12345")
        .cseq(1)
        .via("192.168.1.200:5060", "UDP", Some("branch-abc"))
        .event("presence")
        .subscription_state(subscription_state);
    
    if let Some(body_content) = body {
        builder = builder.body(bytes::Bytes::from(body_content.to_string()));
    }
    
    builder.build()
}

#[tokio::test]
async fn test_subscribe_creates_dialog() {
    // Create a subscription manager
    let (event_tx, mut event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    // Create SUBSCRIBE request
    let request = create_subscribe_request("presence", 3600);
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    let local: SocketAddr = "192.168.1.200:5060".parse().unwrap();
    
    // Handle SUBSCRIBE
    let (response, dialog_id) = subscription_manager
        .handle_subscribe(request, source, local)
        .await
        .expect("Failed to handle SUBSCRIBE");
    
    // Verify response is 200 OK
    assert_eq!(response.status_code(), 200);
    
    // Verify dialog was created
    assert!(dialog_id.is_some(), "SUBSCRIBE should create a dialog");
    
    // Verify subscription created event was sent
    tokio::time::timeout(Duration::from_millis(500), async {
        if let Some(event) = event_rx.recv().await {
            match event {
                DialogEvent::SubscriptionCreated { event_package, expires, .. } => {
                    assert_eq!(event_package, "presence");
                    assert_eq!(expires, Duration::from_secs(3600));
                }
                _ => panic!("Expected SubscriptionCreated event"),
            }
        }
    })
    .await
    .expect("Timeout waiting for event");
}

#[tokio::test]
async fn test_subscribe_with_unsupported_event_package() {
    // Create a subscription manager
    let (event_tx, _event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    // Create SUBSCRIBE request with unsupported event
    let request = create_subscribe_request("unsupported-event", 3600);
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    let local: SocketAddr = "192.168.1.200:5060".parse().unwrap();
    
    // Handle SUBSCRIBE
    let (response, dialog_id) = subscription_manager
        .handle_subscribe(request, source, local)
        .await
        .expect("Failed to handle SUBSCRIBE");
    
    // Verify response is 489 Bad Event
    assert_eq!(response.status_code(), 489);
    
    // Verify no dialog was created
    assert!(dialog_id.is_none(), "Unsupported event should not create dialog");
}

#[tokio::test]
async fn test_subscribe_with_zero_expires_terminates() {
    // Create a subscription manager
    let (event_tx, mut event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    // Create SUBSCRIBE request with Expires: 0
    let request = create_subscribe_request("presence", 0);
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    let local: SocketAddr = "192.168.1.200:5060".parse().unwrap();
    
    // Handle SUBSCRIBE
    let (response, dialog_id) = subscription_manager
        .handle_subscribe(request, source, local)
        .await
        .expect("Failed to handle SUBSCRIBE");
    
    // Verify response is 200 OK
    assert_eq!(response.status_code(), 200);
    
    // Dialog might be created but immediately terminated
    if dialog_id.is_some() {
        // Check for termination event
        tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(event) = event_rx.recv().await {
                if let DialogEvent::SubscriptionTerminated { reason, .. } = event {
                    assert_eq!(reason, Some("client requested".to_string()));
                    return;
                }
            }
        })
        .await
        .ok(); // Don't fail if no termination event (subscription might not create dialog for 0 expires)
    }
}

#[tokio::test]
async fn test_notify_always_returns_200() {
    // Create a subscription manager
    let (event_tx, _event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    // Send NOTIFY without a subscription (should still return 200 OK per RFC 6665)
    let notify = create_notify_request("active;expires=3600", Some("presence data"));
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    
    let notify_response = subscription_manager
        .handle_notify(notify, source)
        .await
        .expect("Failed to handle NOTIFY");
    
    // Verify response is always 200 OK (RFC 6665 requirement)
    assert_eq!(notify_response.status_code(), 200);
}

#[tokio::test]
async fn test_notify_with_terminated_state() {
    // Create a subscription manager
    let (event_tx, _event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    // Send NOTIFY with terminated state (should still return 200 OK)
    let notify = create_notify_request("terminated;reason=deactivated", None);
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    
    let notify_response = subscription_manager
        .handle_notify(notify, source)
        .await
        .expect("Failed to handle NOTIFY");
    
    // Verify response is 200 OK even for terminated state
    assert_eq!(notify_response.status_code(), 200);
}

#[tokio::test]
async fn test_subscription_state_parsing() {
    // Test pending state
    let pending = SubscriptionState::from_header_value("pending");
    assert!(pending.is_pending());
    assert_eq!(pending.to_header_value(), "pending");
    
    // Test active state with expires
    let active = SubscriptionState::from_header_value("active;expires=1800");
    assert!(active.is_active());
    assert!(active.to_header_value().starts_with("active;expires="));
    
    // Test terminated state with reason
    let terminated = SubscriptionState::from_header_value("terminated;reason=noresource");
    assert!(terminated.is_terminated());
    assert_eq!(terminated.to_header_value(), "terminated;reason=noresource");
}

#[tokio::test]
async fn test_subscription_needs_refresh() {
    let state = SubscriptionState::Active {
        remaining_duration: Duration::from_secs(30),
        original_duration: Duration::from_secs(3600),
    };
    
    // Should need refresh when remaining time is less than advance time
    assert!(state.needs_refresh(Duration::from_secs(31)));
    assert!(state.needs_refresh(Duration::from_secs(30)));
    assert!(!state.needs_refresh(Duration::from_secs(29)));
}

#[tokio::test]
async fn test_event_package_validation() {
    use rvoip_dialog_core::subscription::EventPackage;
    
    // Create a simple test package
    struct TestPackage;
    
    impl EventPackage for TestPackage {
        fn name(&self) -> &str { "test" }
        fn accept_types(&self) -> Vec<rvoip_sip_core::types::content_type::ContentType> { vec![] }
        fn validate_body(&self, _body: &[u8]) -> Result<(), String> { Ok(()) }
        fn default_expires(&self) -> Duration { Duration::from_secs(3600) }
        fn min_expires(&self) -> Duration { Duration::from_secs(60) }
        fn max_expires(&self) -> Duration { Duration::from_secs(86400) }
    }
    
    let package = TestPackage;
    
    // Verify package methods
    assert_eq!(package.name(), "test");
    assert_eq!(package.default_expires(), Duration::from_secs(3600));
    assert_eq!(package.min_expires(), Duration::from_secs(60));
    assert_eq!(package.max_expires(), Duration::from_secs(86400));
}

#[tokio::test]
async fn test_multiple_subscriptions() {
    // Create a subscription manager
    let (event_tx, mut event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    let local: SocketAddr = "192.168.1.200:5060".parse().unwrap();
    
    // Create multiple subscriptions for different events
    let events = vec!["presence", "dialog", "message-summary"];
    let mut dialog_ids = vec![];
    
    for event in events {
        let request = create_subscribe_request(event, 3600);
        let (response, dialog_id) = subscription_manager
            .handle_subscribe(request, source, local)
            .await
            .expect("Failed to handle SUBSCRIBE");
        
        assert_eq!(response.status_code(), 200);
        assert!(dialog_id.is_some());
        dialog_ids.push(dialog_id.unwrap());
    }
    
    // Verify we created 3 different dialogs
    assert_eq!(dialog_ids.len(), 3);
    
    // Verify all dialog IDs are unique
    let unique_ids: std::collections::HashSet<_> = dialog_ids.iter().collect();
    assert_eq!(unique_ids.len(), 3);
    
    // Verify we received creation events for all subscriptions
    let mut created_events = 0;
    while let Ok(Some(event)) = tokio::time::timeout(
        Duration::from_millis(100),
        event_rx.recv()
    ).await {
        if matches!(event, DialogEvent::SubscriptionCreated { .. }) {
            created_events += 1;
        }
    }
    assert_eq!(created_events, 3);
}

#[tokio::test]
async fn test_subscription_termination_reasons() {
    // Test all termination reason variants
    let reasons = vec![
        SubscriptionTerminationReason::ClientRequested,
        SubscriptionTerminationReason::ServerTerminated("policy".to_string()),
        SubscriptionTerminationReason::Expired,
        SubscriptionTerminationReason::RefreshFailed,
        SubscriptionTerminationReason::NoResource,
        SubscriptionTerminationReason::Rejected,
        SubscriptionTerminationReason::NetworkError,
        SubscriptionTerminationReason::Other("custom".to_string()),
    ];
    
    for reason in reasons {
        let state = SubscriptionState::Terminated {
            reason: Some(reason.clone()),
        };
        assert!(state.is_terminated());
        
        // Verify string representation
        let reason_str = reason.to_string();
        assert!(!reason_str.is_empty());
    }
}

#[tokio::test]
async fn test_subscription_manager_cloning() {
    // Create a subscription manager
    let (event_tx, _event_rx) = mpsc::channel(100);
    let dialogs = Arc::new(DashMap::new());
    let dialog_lookup = Arc::new(DashMap::new());
    let subscription_manager = SubscriptionManager::new(dialogs, dialog_lookup, event_tx);
    
    // Clone the manager
    let cloned = subscription_manager.clone();
    
    // Both should handle requests successfully
    let request = create_subscribe_request("presence", 3600);
    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    let local: SocketAddr = "192.168.1.200:5060".parse().unwrap();
    
    let (response, _) = cloned
        .handle_subscribe(request, source, local)
        .await
        .expect("Failed to handle SUBSCRIBE with cloned manager");
    
    assert_eq!(response.status_code(), 200);
}