//! Tests for SessionEventProcessor Operations
//!
//! Tests the session event system functionality including event publishing,
//! subscribing, filtering, and integration with the infra-common event system.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    api::types::{CallState, SessionId},
    manager::events::{SessionEvent, SessionEventProcessor},
};
use common::*;

#[tokio::test]
async fn test_event_processor_creation() {
    let mut helper = EventTestHelper::new().await.unwrap();
    
    // Verify processor is running
    assert!(helper.processor().is_running().await);
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_publishing_basic() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let session_id = SessionId("test-session-1".to_string());
    let event = SessionEvent::SessionCreated {
        session_id: session_id.clone(),
        from: "sip:alice@localhost".to_string(),
        to: "sip:bob@localhost".to_string(),
        call_state: CallState::Initiating,
    };
    
    // Publish event
    helper.publish_event(event.clone()).await.unwrap();
    
    // Wait for and verify event
    let received_event = helper.wait_for_event(Duration::from_secs(1)).await;
    assert!(received_event.is_some());
    
    if let Some(SessionEvent::SessionCreated { session_id: received_id, from, to, call_state }) = received_event {
        assert_eq!(received_id, session_id);
        assert_eq!(from, "sip:alice@localhost");
        assert_eq!(to, "sip:bob@localhost");
        assert_eq!(call_state, CallState::Initiating);
    } else {
        panic!("Received wrong event type");
    }
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_publishing_multiple_types() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let session_id = SessionId("multi-event-session".to_string());
    
    let events = vec![
        SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: "sip:alice@localhost".to_string(),
            to: "sip:bob@localhost".to_string(),
            call_state: CallState::Initiating,
        },
        SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::Initiating,
            new_state: CallState::Active,
        },
        SessionEvent::MediaEvent {
            session_id: session_id.clone(),
            event: "media_connected".to_string(),
        },
        SessionEvent::SessionTerminated {
            session_id: session_id.clone(),
            reason: "Normal termination".to_string(),
        },
    ];
    
    // Publish all events
    for event in &events {
        helper.publish_event(event.clone()).await.unwrap();
    }
    
    // Receive and verify all events
    for expected_event in &events {
        let received_event = helper.wait_for_event(Duration::from_secs(1)).await;
        assert!(received_event.is_some());
        
        match (&received_event.unwrap(), expected_event) {
            (SessionEvent::SessionCreated { session_id: r_id, .. }, SessionEvent::SessionCreated { session_id: e_id, .. }) => {
                assert_eq!(r_id, e_id);
            }
            (SessionEvent::StateChanged { session_id: r_id, .. }, SessionEvent::StateChanged { session_id: e_id, .. }) => {
                assert_eq!(r_id, e_id);
            }
            (SessionEvent::MediaEvent { session_id: r_id, .. }, SessionEvent::MediaEvent { session_id: e_id, .. }) => {
                assert_eq!(r_id, e_id);
            }
            (SessionEvent::SessionTerminated { session_id: r_id, .. }, SessionEvent::SessionTerminated { session_id: e_id, .. }) => {
                assert_eq!(r_id, e_id);
            }
            _ => panic!("Event type mismatch"),
        }
    }
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_publishing_without_subscribers() {
    let helper = EventTestHelper::new().await.unwrap();
    // Note: No subscription
    
    let event = SessionEvent::SessionCreated {
        session_id: SessionId("no-sub-test".to_string()),
        from: "sip:alice@localhost".to_string(),
        to: "sip:bob@localhost".to_string(),
        call_state: CallState::Initiating,
    };
    
    // Should not fail to publish even without subscribers
    let result = helper.publish_event(event).await;
    assert!(result.is_ok());
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_subscription_and_unsubscription() {
    let mut helper = EventTestHelper::new().await.unwrap();
    
    // Subscribe
    helper.subscribe().await.unwrap();
    
    let event = SessionEvent::SessionCreated {
        session_id: SessionId("sub-test".to_string()),
        from: "sip:alice@localhost".to_string(),
        to: "sip:bob@localhost".to_string(),
        call_state: CallState::Initiating,
    };
    
    // Publish and receive event
    helper.publish_event(event.clone()).await.unwrap();
    let received = helper.wait_for_event(Duration::from_secs(1)).await;
    assert!(received.is_some());
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_filtering() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let session1_id = SessionId("session-1".to_string());
    let session2_id = SessionId("session-2".to_string());
    
    // Publish events for different sessions
    helper.publish_event(SessionEvent::SessionCreated {
        session_id: session1_id.clone(),
        from: "sip:alice@localhost".to_string(),
        to: "sip:bob@localhost".to_string(),
        call_state: CallState::Initiating,
    }).await.unwrap();
    
    helper.publish_event(SessionEvent::SessionCreated {
        session_id: session2_id.clone(),
        from: "sip:charlie@localhost".to_string(),
        to: "sip:david@localhost".to_string(),
        call_state: CallState::Initiating,
    }).await.unwrap();
    
    // Wait for specific session event
    let session1_event = helper.wait_for_specific_event(
        Duration::from_secs(2),
        |event| match event {
            SessionEvent::SessionCreated { session_id, .. } => session_id == &session1_id,
            _ => false,
        }
    ).await;
    
    assert!(session1_event.is_some());
    if let Some(SessionEvent::SessionCreated { session_id, .. }) = session1_event {
        assert_eq!(session_id, session1_id);
    }
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_processor_start_stop() {
    let processor = Arc::new(SessionEventProcessor::new());
    
    // Initially not running
    assert!(!processor.is_running().await);
    
    // Start processor
    processor.start().await.unwrap();
    assert!(processor.is_running().await);
    
    // Stop processor
    processor.stop().await.unwrap();
    assert!(!processor.is_running().await);
    
    // Should be able to start again
    processor.start().await.unwrap();
    assert!(processor.is_running().await);
    
    processor.stop().await.unwrap();
}

#[tokio::test]
async fn test_event_processor_multiple_subscribers() {
    let processor = Arc::new(SessionEventProcessor::new());
    processor.start().await.unwrap();
    
    // Create multiple subscribers
    let mut subscriber1 = processor.subscribe().await.unwrap();
    let mut subscriber2 = processor.subscribe().await.unwrap();
    let mut subscriber3 = processor.subscribe().await.unwrap();
    
    let event = SessionEvent::SessionCreated {
        session_id: SessionId("multi-sub-test".to_string()),
        from: "sip:alice@localhost".to_string(),
        to: "sip:bob@localhost".to_string(),
        call_state: CallState::Initiating,
    };
    
    // Publish event
    processor.publish_event(event).await.unwrap();
    
    // All subscribers should receive the event
    let event1 = wait_for_session_event(&mut subscriber1, Duration::from_secs(1)).await;
    let event2 = wait_for_session_event(&mut subscriber2, Duration::from_secs(1)).await;
    let event3 = wait_for_session_event(&mut subscriber3, Duration::from_secs(1)).await;
    
    assert!(event1.is_some());
    assert!(event2.is_some());
    assert!(event3.is_some());
    
    processor.stop().await.unwrap();
}

#[tokio::test]
async fn test_event_ordering() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let session_id = SessionId("ordering-test".to_string());
    
    // Publish events in sequence
    let events = vec![
        SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: "sip:alice@localhost".to_string(),
            to: "sip:bob@localhost".to_string(),
            call_state: CallState::Initiating,
        },
        SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::Initiating,
            new_state: CallState::Ringing,
        },
        SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::Ringing,
            new_state: CallState::Active,
        },
        SessionEvent::SessionTerminated {
            session_id: session_id.clone(),
            reason: "Normal completion".to_string(),
        },
    ];
    
    // Publish all events quickly
    for event in &events {
        helper.publish_event(event.clone()).await.unwrap();
    }
    
    // Verify events are received in order
    for (i, expected_event) in events.iter().enumerate() {
        let received_event = helper.wait_for_event(Duration::from_secs(1)).await;
        assert!(received_event.is_some(), "Failed to receive event {}", i);
        
        match (&received_event.unwrap(), expected_event) {
            (SessionEvent::SessionCreated { .. }, SessionEvent::SessionCreated { .. }) => {},
            (SessionEvent::StateChanged { new_state: r_state, .. }, SessionEvent::StateChanged { new_state: e_state, .. }) => {
                assert_eq!(r_state, e_state);
            },
            (SessionEvent::SessionTerminated { .. }, SessionEvent::SessionTerminated { .. }) => {},
            _ => panic!("Event order mismatch at position {}", i),
        }
    }
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_error_handling() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    // Test error event
    let error_event = SessionEvent::Error {
        session_id: Some(SessionId("error-test".to_string())),
        error: "Test error condition".to_string(),
    };
    
    helper.publish_event(error_event).await.unwrap();
    
    let received_event = helper.wait_for_event(Duration::from_secs(1)).await;
    assert!(received_event.is_some());
    
    if let Some(SessionEvent::Error { session_id, error }) = received_event {
        assert!(session_id.is_some());
        assert_eq!(error, "Test error condition");
    } else {
        panic!("Expected error event");
    }
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_event_performance() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let event_count = 1000;
    let start = std::time::Instant::now();
    
    // Publish many events
    for i in 0..event_count {
        let event = SessionEvent::SessionCreated {
            session_id: SessionId(format!("perf-session-{}", i)),
            from: format!("sip:caller{}@localhost", i),
            to: "sip:target@localhost".to_string(),
            call_state: CallState::Initiating,
        };
        
        helper.publish_event(event).await.unwrap();
    }
    
    let publish_time = start.elapsed();
    println!("Published {} events in {:?}", event_count, publish_time);
    
    // Receive all events
    let receive_start = std::time::Instant::now();
    let mut received_count = 0;
    
    while received_count < event_count {
        if helper.wait_for_event(Duration::from_millis(100)).await.is_some() {
            received_count += 1;
        } else {
            break; // Timeout
        }
    }
    
    let receive_time = receive_start.elapsed();
    println!("Received {} events in {:?}", received_count, receive_time);
    
    assert_eq!(received_count, event_count);
    assert!(publish_time < Duration::from_secs(5), "Publishing took too long");
    assert!(receive_time < Duration::from_secs(10), "Receiving took too long");
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_event_publishing() {
    let processor = Arc::new(SessionEventProcessor::new());
    processor.start().await.unwrap();
    
    let mut subscriber = processor.subscribe().await.unwrap();
    
    let concurrent_publishers = 10;
    let events_per_publisher = 10;
    let mut handles = Vec::new();
    
    // Spawn concurrent publishers
    for publisher_id in 0..concurrent_publishers {
        let processor_clone = Arc::clone(&processor);
        let handle = tokio::spawn(async move {
            for event_id in 0..events_per_publisher {
                let event = SessionEvent::SessionCreated {
                    session_id: SessionId(format!("pub{}-event{}", publisher_id, event_id)),
                    from: format!("sip:pub{}@localhost", publisher_id),
                    to: "sip:target@localhost".to_string(),
                    call_state: CallState::Initiating,
                };
                
                processor_clone.publish_event(event).await.unwrap();
            }
        });
        handles.push(handle);
    }
    
    // Wait for all publishers to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Receive all events
    let total_events = concurrent_publishers * events_per_publisher;
    let mut received_count = 0;
    
    while received_count < total_events {
        if wait_for_session_event(&mut subscriber, Duration::from_millis(100)).await.is_some() {
            received_count += 1;
        } else {
            break;
        }
    }
    
    assert_eq!(received_count, total_events);
    
    processor.stop().await.unwrap();
}

#[tokio::test]
async fn test_event_processor_restart() {
    let processor = Arc::new(SessionEventProcessor::new());
    
    // Start, publish, stop
    processor.start().await.unwrap();
    let mut subscriber1 = processor.subscribe().await.unwrap();
    
    let event1 = SessionEvent::SessionCreated {
        session_id: SessionId("restart-test-1".to_string()),
        from: "sip:alice@localhost".to_string(),
        to: "sip:bob@localhost".to_string(),
        call_state: CallState::Initiating,
    };
    
    processor.publish_event(event1).await.unwrap();
    let received1 = wait_for_session_event(&mut subscriber1, Duration::from_secs(1)).await;
    assert!(received1.is_some());
    
    processor.stop().await.unwrap();
    
    // Restart and verify it works again
    processor.start().await.unwrap();
    let mut subscriber2 = processor.subscribe().await.unwrap();
    
    let event2 = SessionEvent::SessionCreated {
        session_id: SessionId("restart-test-2".to_string()),
        from: "sip:charlie@localhost".to_string(),
        to: "sip:david@localhost".to_string(),
        call_state: CallState::Initiating,
    };
    
    processor.publish_event(event2).await.unwrap();
    let received2 = wait_for_session_event(&mut subscriber2, Duration::from_secs(1)).await;
    assert!(received2.is_some());
    
    processor.stop().await.unwrap();
}

#[tokio::test]
async fn test_event_types_comprehensive() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let session_id = SessionId("comprehensive-test".to_string());
    
    // Test all event types
    let events = vec![
        SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: "sip:alice@localhost".to_string(),
            to: "sip:bob@localhost".to_string(),
            call_state: CallState::Initiating,
        },
        SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::Initiating,
            new_state: CallState::Active,
        },
        SessionEvent::MediaEvent {
            session_id: session_id.clone(),
            event: "codec_negotiated".to_string(),
        },
        SessionEvent::Error {
            session_id: Some(session_id.clone()),
            error: "Media negotiation failed".to_string(),
        },
        SessionEvent::SessionTerminated {
            session_id: session_id.clone(),
            reason: "Error recovery".to_string(),
        },
    ];
    
    // Publish and verify each event type
    for event in events {
        helper.publish_event(event.clone()).await.unwrap();
        
        let received = helper.wait_for_event(Duration::from_secs(1)).await;
        assert!(received.is_some());
        
        // Verify event type matches
        match (&received.unwrap(), &event) {
            (SessionEvent::SessionCreated { .. }, SessionEvent::SessionCreated { .. }) => {},
            (SessionEvent::StateChanged { .. }, SessionEvent::StateChanged { .. }) => {},
            (SessionEvent::MediaEvent { .. }, SessionEvent::MediaEvent { .. }) => {},
            (SessionEvent::Error { .. }, SessionEvent::Error { .. }) => {},
            (SessionEvent::SessionTerminated { .. }, SessionEvent::SessionTerminated { .. }) => {},
            _ => panic!("Event type mismatch"),
        }
    }
    
    helper.cleanup().await.unwrap();
} 