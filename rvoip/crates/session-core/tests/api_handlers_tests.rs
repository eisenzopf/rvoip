mod common;

use std::sync::Arc;
use std::time::Duration;
use tokio::time;

use crate::common::api_test_utils::*;
use rvoip_session_core::api::handlers::*;
use rvoip_session_core::api::types::*;
use rvoip_session_core::Result;

#[tokio::test]
async fn test_auto_answer_handler() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_auto_answer_handler");
        
        let handler = AutoAnswerHandler::default();
        let helper = ApiTypesTestHelper::new();
        
        // Create test incoming calls
        let calls = helper.create_test_incoming_calls(5);
        
        for call in calls {
            let decision = handler.on_incoming_call(call.clone()).await;
            assert!(matches!(decision, CallDecision::Accept(_)));
        }
        
        // Test call ended event
        let test_session = helper.create_test_call_sessions(1)[0].clone();
        handler.on_call_ended(test_session, "Test reason").await;
        
        println!("Completed test_auto_answer_handler");
    }).await;
    
    assert!(result.is_ok(), "test_auto_answer_handler timed out");
}

#[tokio::test]
async fn test_queue_handler() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_queue_handler");
        
        let max_queue_size = 3;
        let handler = QueueHandler::new(max_queue_size);
        let helper = ApiTypesTestHelper::new();
        
        // Test initial state
        assert_eq!(handler.queue_size(), 0);
        assert!(handler.dequeue().is_none());
        
        // Create test incoming calls
        let calls = helper.create_test_incoming_calls(5);
        
        // Queue up to max size
        for i in 0..max_queue_size {
            let decision = handler.on_incoming_call(calls[i].clone()).await;
            assert!(matches!(decision, CallDecision::Defer));
            assert_eq!(handler.queue_size(), i + 1);
        }
        
        // Should reject when queue is full
        let reject_decision = handler.on_incoming_call(calls[3].clone()).await;
        assert!(matches!(reject_decision, CallDecision::Reject(_)));
        assert_eq!(handler.queue_size(), max_queue_size);
        
        // Test dequeue
        for i in 0..max_queue_size {
            let dequeued = handler.dequeue();
            assert!(dequeued.is_some());
            assert_eq!(handler.queue_size(), max_queue_size - i - 1);
        }
        
        // Queue should be empty now
        assert_eq!(handler.queue_size(), 0);
        assert!(handler.dequeue().is_none());
        
        println!("Completed test_queue_handler");
    }).await;
    
    assert!(result.is_ok(), "test_queue_handler timed out");
}

#[tokio::test]
async fn test_routing_handler() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_routing_handler");
        
        let mut handler = RoutingHandler::new();
        let helper = ApiTypesTestHelper::new();
        
        // Configure routing rules
        handler.add_route("sip:100", "sip:alice@internal.com");
        handler.add_route("sip:200", "sip:bob@internal.com");
        handler.add_route("sip:911", "sip:emergency@service.com");
        
        // Set default action
        handler.set_default_action(CallDecision::Reject("No route found".to_string()));
        
        // Test routing decisions
        let test_cases = vec![
            ("sip:100@example.com", "sip:alice@internal.com"),
            ("sip:200@example.com", "sip:bob@internal.com"),
            ("sip:911@example.com", "sip:emergency@service.com"),
        ];
        
        for (incoming_uri, expected_target) in test_cases {
            let incoming_call = IncomingCall {
                id: SessionId::new(),
                from: "sip:caller@example.com".to_string(),
                to: incoming_uri.to_string(),
                sdp: Some(helper.create_test_sdp("routing")),
                headers: std::collections::HashMap::new(),
                received_at: std::time::Instant::now(),
            };
            
            let decision = handler.on_incoming_call(incoming_call).await;
            match decision {
                CallDecision::Forward(target) => {
                    assert_eq!(target, expected_target);
                }
                _ => panic!("Expected Forward decision for {}", incoming_uri),
            }
        }
        
        // Test default action for unmatched route
        let unmatched_call = IncomingCall {
            id: SessionId::new(),
            from: "sip:caller@example.com".to_string(),
            to: "sip:999@example.com".to_string(),
            sdp: None,
            headers: std::collections::HashMap::new(),
            received_at: std::time::Instant::now(),
        };
        
        let default_decision = handler.on_incoming_call(unmatched_call).await;
        assert!(matches!(default_decision, CallDecision::Reject(_)));
        
        println!("Completed test_routing_handler");
    }).await;
    
    assert!(result.is_ok(), "test_routing_handler timed out");
}

#[tokio::test]
async fn test_composite_handler() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_composite_handler");
        
        let helper = ApiTypesTestHelper::new();
        
        // Create component handlers
        let queue_handler = Arc::new(QueueHandler::new(2));
        let auto_handler = Arc::new(AutoAnswerHandler::default());
        
        // Create composite handler
        let composite = CompositeHandler::new()
            .add_handler(queue_handler.clone())
            .add_handler(auto_handler.clone());
        
        // Test that first handler (queue) processes calls
        let calls = helper.create_test_incoming_calls(3);
        
        // First two calls should be deferred by queue
        let decision1 = composite.on_incoming_call(calls[0].clone()).await;
        let decision2 = composite.on_incoming_call(calls[1].clone()).await;
        assert!(matches!(decision1, CallDecision::Defer));
        assert!(matches!(decision2, CallDecision::Defer));
        
        // Third call should be rejected by queue (full)
        let decision3 = composite.on_incoming_call(calls[2].clone()).await;
        assert!(matches!(decision3, CallDecision::Reject(_)));
        
        // Test call ended notification to all handlers
        let test_session = helper.create_test_call_sessions(1)[0].clone();
        composite.on_call_ended(test_session, "Composite test").await;
        
        println!("Completed test_composite_handler");
    }).await;
    
    assert!(result.is_ok(), "test_composite_handler timed out");
}

#[tokio::test]
async fn test_test_call_handler() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_test_call_handler");
        
        let handler = TestCallHandler::new(CallDecision::Accept(None));
        let helper = ApiTypesTestHelper::new();
        
        // Test initial state
        assert_eq!(handler.incoming_call_count(), 0);
        assert_eq!(handler.ended_call_count(), 0);
        
        // Test incoming calls
        let calls = helper.create_test_incoming_calls(3);
        for call in &calls {
            let decision = handler.on_incoming_call(call.clone()).await;
            assert!(matches!(decision, CallDecision::Accept(_)));
        }
        
        assert_eq!(handler.incoming_call_count(), 3);
        let recorded_calls = handler.get_incoming_calls();
        assert_eq!(recorded_calls.len(), 3);
        
        // Test call ended events
        let sessions = helper.create_test_call_sessions(2);
        handler.on_call_ended(sessions[0].clone(), "Test reason 1").await;
        handler.on_call_ended(sessions[1].clone(), "Test reason 2").await;
        
        assert_eq!(handler.ended_call_count(), 2);
        let ended_calls = handler.get_ended_calls();
        assert_eq!(ended_calls.len(), 2);
        assert_eq!(ended_calls[0].1, "Test reason 1");
        assert_eq!(ended_calls[1].1, "Test reason 2");
        
        // Test clear events
        handler.clear_events();
        assert_eq!(handler.incoming_call_count(), 0);
        assert_eq!(handler.ended_call_count(), 0);
        
        println!("Completed test_test_call_handler");
    }).await;
    
    assert!(result.is_ok(), "test_test_call_handler timed out");
}

#[tokio::test]
async fn test_handler_with_different_decisions() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_handler_with_different_decisions");
        
        let helper = ApiTypesTestHelper::new();
        let decisions = helper.get_all_call_decisions();
        
        for decision in decisions {
            let handler = TestCallHandler::new(decision.clone());
            let call = helper.create_test_incoming_calls(1)[0].clone();
            
            let result_decision = handler.on_incoming_call(call).await;
            
            match (&decision, &result_decision) {
                (CallDecision::Accept(_), CallDecision::Accept(_)) => {},
                (CallDecision::Defer, CallDecision::Defer) => {},
                (CallDecision::Reject(reason1), CallDecision::Reject(reason2)) => {
                    assert_eq!(reason1, reason2);
                },
                (CallDecision::Forward(target1), CallDecision::Forward(target2)) => {
                    assert_eq!(target1, target2);
                },
                _ => panic!("Decision mismatch: expected {:?}, got {:?}", decision, result_decision),
            }
        }
        
        println!("Completed test_handler_with_different_decisions");
    }).await;
    
    assert!(result.is_ok(), "test_handler_with_different_decisions timed out");
}

#[tokio::test]
async fn test_queue_handler_with_notifications() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_queue_handler_with_notifications");
        
        let handler = QueueHandler::new(3);
        let helper = ApiTypesTestHelper::new();
        
        // Set up notification channel
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        handler.set_notify_channel(sender);
        
        // Queue some calls
        let calls = helper.create_test_incoming_calls(2);
        
        for call in &calls {
            let decision = handler.on_incoming_call(call.clone()).await;
            assert!(matches!(decision, CallDecision::Defer));
        }
        
        // Check notifications
        for expected_call in calls {
            let notified_call = receiver.recv().await.unwrap();
            assert_eq!(notified_call.id, expected_call.id);
            assert_eq!(notified_call.from, expected_call.from);
            assert_eq!(notified_call.to, expected_call.to);
        }
        
        println!("Completed test_queue_handler_with_notifications");
    }).await;
    
    assert!(result.is_ok(), "test_queue_handler_with_notifications timed out");
}

#[tokio::test]
async fn test_handlers_with_edge_cases() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_handlers_with_edge_cases");
        
        let helper = ApiTypesTestHelper::new();
        
        // Test with empty call data
        let empty_call = IncomingCall {
            id: SessionId("".to_string()),
            from: "".to_string(),
            to: "".to_string(),
            sdp: None,
            headers: std::collections::HashMap::new(),
            received_at: std::time::Instant::now(),
        };
        
        let auto_handler = AutoAnswerHandler::default();
        let decision = auto_handler.on_incoming_call(empty_call.clone()).await;
        assert!(matches!(decision, CallDecision::Accept(_)));
        
        // Test with unicode data
        let unicode_call = IncomingCall {
            id: SessionId("unicode_🦀_call".to_string()),
            from: "sip:caller🦀@example.com".to_string(),
            to: "sip:target🚀@example.com".to_string(),
            sdp: Some("unicode sdp 🔥".to_string()),
            headers: std::collections::HashMap::new(),
            received_at: std::time::Instant::now(),
        };
        
        let queue_handler = QueueHandler::new(5);
        let unicode_decision = queue_handler.on_incoming_call(unicode_call).await;
        assert!(matches!(unicode_decision, CallDecision::Defer));
        
        // Test routing with unicode
        let mut routing_handler = RoutingHandler::new();
        routing_handler.add_route("sip:🦀", "sip:crab@ocean.com");
        
        let crab_call = IncomingCall {
            id: SessionId::new(),
            from: "sip:caller@example.com".to_string(),
            to: "sip:🦀@example.com".to_string(),
            sdp: None,
            headers: std::collections::HashMap::new(),
            received_at: std::time::Instant::now(),
        };
        
        let routing_decision = routing_handler.on_incoming_call(crab_call).await;
        assert!(matches!(routing_decision, CallDecision::Forward(_)));
        
        println!("Completed test_handlers_with_edge_cases");
    }).await;
    
    assert!(result.is_ok(), "test_handlers_with_edge_cases timed out");
}

#[tokio::test]
async fn test_handlers_performance() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_handlers_performance");
        
        let helper = ApiTypesTestHelper::new();
        let call_count = 1000;
        
        // Test AutoAnswerHandler performance
        let auto_handler = AutoAnswerHandler::default();
        let start = std::time::Instant::now();
        
        for i in 0..call_count {
            let call = IncomingCall {
                id: SessionId(format!("perf_call_{}", i)),
                from: format!("sip:caller{}@example.com", i),
                to: format!("sip:target{}@example.com", i),
                sdp: Some(helper.create_test_sdp(&format!("perf_{}", i))),
                headers: std::collections::HashMap::new(),
                received_at: std::time::Instant::now(),
            };
            
            let decision = auto_handler.on_incoming_call(call).await;
            assert!(matches!(decision, CallDecision::Accept(_)));
        }
        
        let auto_duration = start.elapsed();
        println!("AutoAnswerHandler processed {} calls in {:?}", call_count, auto_duration);
        
        // Test QueueHandler performance
        let queue_handler = QueueHandler::new(call_count);
        let queue_start = std::time::Instant::now();
        
        for i in 0..call_count {
            let call = IncomingCall {
                id: SessionId(format!("queue_call_{}", i)),
                from: format!("sip:caller{}@example.com", i),
                to: format!("sip:target{}@example.com", i),
                sdp: None,
                headers: std::collections::HashMap::new(),
                received_at: std::time::Instant::now(),
            };
            
            let decision = queue_handler.on_incoming_call(call).await;
            assert!(matches!(decision, CallDecision::Defer));
        }
        
        let queue_duration = queue_start.elapsed();
        println!("QueueHandler processed {} calls in {:?}", call_count, queue_duration);
        
        // Performance assertions
        assert!(auto_duration < Duration::from_secs(5), "AutoAnswerHandler took too long");
        assert!(queue_duration < Duration::from_secs(5), "QueueHandler took too long");
        
        println!("Completed test_handlers_performance");
    }).await;
    
    assert!(result.is_ok(), "test_handlers_performance timed out");
}

#[tokio::test]
async fn test_concurrent_handler_operations() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_concurrent_handler_operations");
        
        let helper = ApiTypesTestHelper::new();
        let handler = Arc::new(QueueHandler::new(100));
        let concurrent_count = 50;
        let mut handles = Vec::new();
        
        // Spawn concurrent handler operations
        for i in 0..concurrent_count {
            let handler_clone = handler.clone();
            let call = IncomingCall {
                id: SessionId(format!("concurrent_call_{}", i)),
                from: format!("sip:caller{}@example.com", i),
                to: format!("sip:target{}@example.com", i),
                sdp: Some(helper.create_test_sdp(&format!("concurrent_{}", i))),
                headers: std::collections::HashMap::new(),
                received_at: std::time::Instant::now(),
            };
            
            let handle = tokio::spawn(async move {
                handler_clone.on_incoming_call(call).await
            });
            handles.push(handle);
        }
        
        // Collect all results
        let mut decisions = Vec::new();
        for handle in handles {
            let decision = handle.await.unwrap();
            decisions.push(decision);
        }
        
        // All operations should have completed successfully
        assert_eq!(decisions.len(), concurrent_count);
        for decision in decisions {
            assert!(matches!(decision, CallDecision::Defer));
        }
        
        // Check that all calls were queued
        assert_eq!(handler.queue_size(), concurrent_count);
        
        println!("Completed test_concurrent_handler_operations");
    }).await;
    
    assert!(result.is_ok(), "test_concurrent_handler_operations timed out");
} 