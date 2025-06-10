mod common;

use std::sync::Arc;
use std::time::Duration;
use tokio::time;

use crate::common::api_test_utils::*;
use rvoip_session_core::api::*;
use rvoip_session_core::Result;

#[tokio::test]
async fn test_api_module_exports() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_api_module_exports");
        
        // Test that all API components are accessible
        
        // Types
        let session_id = SessionId::new();
        assert!(!session_id.as_str().is_empty());
        
        let test_session = CallSession {
            id: session_id.clone(),
            from: "sip:test@example.com".to_string(),
            to: "sip:target@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(std::time::Instant::now()),
        };
        assert!(test_session.is_active());
        
        // Handlers
        let auto_handler = AutoAnswerHandler::default();
        let queue_handler = QueueHandler::new(10);
        let routing_handler = RoutingHandler::new();
        
        // Builder
        let builder = SessionManagerBuilder::new()
            .with_sip_port(5070)
            .with_handler(Arc::new(auto_handler));
        
        let debug_str = format!("{:?}", builder);
        assert!(debug_str.contains("SessionManagerBuilder"));
        
        println!("Completed test_api_module_exports");
    }).await;
    
    assert!(result.is_ok(), "test_api_module_exports timed out");
}

#[tokio::test]
async fn test_complete_api_workflow() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_complete_api_workflow");
        
        let helper = ApiTypesTestHelper::new();
        
        // 1. Create a handler for incoming calls
        let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
        
        // 2. Build a session manager configuration
        let builder = SessionManagerBuilder::new()
            .with_sip_port(0) // Random port for testing
            .with_sip_bind_address("127.0.0.1")
            .with_from_uri("sip:api_test@localhost")
            .with_media_ports(20000, 30000)
            .with_handler(handler.clone());
        
        // 3. Create incoming calls
        let incoming_calls = helper.create_test_incoming_calls(3);
        
        // 4. Process calls through handler
        for call in &incoming_calls {
            let decision = handler.on_incoming_call(call.clone()).await;
            assert!(matches!(decision, CallDecision::Accept(_)));
        }
        
        // 5. Verify handler tracked all events
        assert_eq!(handler.incoming_call_count(), 3);
        
        // 6. Create and validate call sessions
        let sessions = helper.create_test_call_sessions(3);
        for session in &sessions {
            assert!(helper.validate_call_session(session).is_ok());
        }
        
        // 7. Test call state transitions
        let transitions = helper.get_valid_state_transitions();
        assert!(!transitions.is_empty());
        
        // 8. Test media info creation
        let media_info = helper.create_test_media_info("workflow_test");
        assert!(media_info.local_sdp.is_some());
        assert!(media_info.codec.is_some());
        
        println!("Completed test_complete_api_workflow");
    }).await;
    
    assert!(result.is_ok(), "test_complete_api_workflow timed out");
}

#[tokio::test]
async fn test_api_handler_integration() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_api_handler_integration");
        
        let helper = ApiTypesTestHelper::new();
        
        // Create different types of handlers
        let auto_handler = Arc::new(AutoAnswerHandler::default());
        let queue_handler = Arc::new(QueueHandler::new(5));
        let mut routing_handler = RoutingHandler::new();
        
        // Configure routing
        routing_handler.add_route("sip:100", "sip:alice@internal.com");
        routing_handler.add_route("sip:200", "sip:bob@internal.com");
        routing_handler.set_default_action(CallDecision::Reject("No route".to_string()));
        
        let routing_handler = Arc::new(routing_handler);
        
        // Create a composite handler
        let composite = CompositeHandler::new()
            .add_handler(queue_handler.clone())
            .add_handler(auto_handler.clone());
        
        // Test calls with different handlers
        let test_calls = vec![
            ("sip:100@test.com", "routing to alice"),
            ("sip:200@test.com", "routing to bob"), 
            ("sip:999@test.com", "no route"),
            ("sip:direct@test.com", "direct call"),
        ];
        
        for (to_uri, description) in test_calls {
            println!("Testing: {}", description);
            
            let call = IncomingCall {
                id: SessionId::new(),
                from: "sip:caller@test.com".to_string(),
                to: to_uri.to_string(),
                sdp: Some(helper.create_test_sdp("integration")),
                headers: std::collections::HashMap::new(),
                received_at: std::time::Instant::now(),
            };
            
            // Test auto handler
            let auto_decision = auto_handler.on_incoming_call(call.clone()).await;
            assert!(matches!(auto_decision, CallDecision::Accept(_)));
            
            // Test routing handler
            let routing_decision = routing_handler.on_incoming_call(call.clone()).await;
            match to_uri {
                "sip:100@test.com" => {
                    assert!(matches!(routing_decision, CallDecision::Forward(_)));
                }
                "sip:200@test.com" => {
                    assert!(matches!(routing_decision, CallDecision::Forward(_)));
                }
                _ => {
                    assert!(matches!(routing_decision, CallDecision::Reject(_)));
                }
            }
            
            // Test composite handler
            let composite_decision = composite.on_incoming_call(call).await;
            // Should be deferred by queue (first handler in composite)
            assert!(matches!(composite_decision, CallDecision::Defer));
        }
        
        println!("Completed test_api_handler_integration");
    }).await;
    
    assert!(result.is_ok(), "test_api_handler_integration timed out");
}

#[tokio::test]
async fn test_api_builder_and_types_integration() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_api_builder_and_types_integration");
        
        let helper = ApiTypesTestHelper::new();
        let builder_helper = ApiBuilderTestHelper::new();
        
        // Test different handler types with builders
        let handlers = vec![
            Arc::new(AutoAnswerHandler::default()) as Arc<dyn CallHandler>,
            Arc::new(QueueHandler::new(10)) as Arc<dyn CallHandler>,
            Arc::new(RoutingHandler::new()) as Arc<dyn CallHandler>,
            Arc::new(TestCallHandler::new(CallDecision::Accept(None))) as Arc<dyn CallHandler>,
        ];
        
        for (i, handler) in handlers.into_iter().enumerate() {
            // Create builder with handler
            let builder = SessionManagerBuilder::new()
                .with_sip_port(5060 + i as u16)
                .with_sip_bind_address("127.0.0.1")
                .with_from_uri(&format!("sip:test{}@example.com", i))
                .with_media_ports((10000 + i * 1000) as u16, (20000 + i * 1000) as u16)
                .with_handler(handler.clone());
            
            // Verify builder configuration
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("SessionManagerBuilder"));
            
            // Test handler with sample calls
            let call = helper.create_test_incoming_calls(1)[0].clone();
            let decision = handler.on_incoming_call(call).await;
            
            // Different handlers should return different decisions
            match i {
                0 => assert!(matches!(decision, CallDecision::Accept(_))), // AutoAnswer
                1 => assert!(matches!(decision, CallDecision::Defer)), // Queue
                2 => assert!(matches!(decision, CallDecision::Reject(_))), // Routing (no routes)
                3 => assert!(matches!(decision, CallDecision::Accept(_))), // Test handler
                _ => unreachable!(),
            }
        }
        
        println!("Completed test_api_builder_and_types_integration");
    }).await;
    
    assert!(result.is_ok(), "test_api_builder_and_types_integration timed out");
}

#[tokio::test]
async fn test_api_create_and_control_integration() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_api_create_and_control_integration");
        
        let helper = ApiTypesTestHelper::new();
        
        // Test create functions
        let incoming_call = create_incoming_call(
            "sip:caller@example.com",
            "sip:callee@example.com",
            Some(helper.create_test_sdp("integration")),
            ApiTestUtils::create_test_headers(),
        );
        
        // Validate the created call
        assert!(helper.validate_incoming_call(&incoming_call).is_ok());
        assert_eq!(incoming_call.caller(), "sip:caller@example.com");
        assert_eq!(incoming_call.called(), "sip:callee@example.com");
        
        // Test that we can call accept/reject methods (they return errors as expected)
        let accept_result = incoming_call.accept().await;
        assert!(accept_result.is_err()); // Should be todo! error
        
        let reject_result = incoming_call.reject("Integration test").await;
        assert!(reject_result.is_err()); // Should be todo! error
        
        // Test control operation state validation
        let sessions = helper.create_test_call_sessions(3);
        for session in sessions {
            println!("Testing control validation for session in state: {:?}", session.state);
            
            // Test state-based operation validation
            match session.state {
                CallState::Active => {
                    assert!(session.is_active());
                    // Active sessions should allow most control operations
                }
                CallState::OnHold => {
                    assert!(!session.is_active());
                    assert!(session.state.is_in_progress());
                    // OnHold sessions should allow resume and terminate
                }
                _ => {
                    // Other states have various restrictions
                    if session.state.is_final() {
                        assert!(!session.state.is_in_progress());
                    }
                }
            }
        }
        
        println!("Completed test_api_create_and_control_integration");
    }).await;
    
    assert!(result.is_ok(), "test_api_create_and_control_integration timed out");
}

#[tokio::test]
async fn test_api_error_handling_integration() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_api_error_handling_integration");
        
        let helper = ApiTypesTestHelper::new();
        
        // Test validation errors
        let invalid_session = CallSession {
            id: SessionId("".to_string()), // Invalid: empty ID
            from: "invalid_uri".to_string(), // Invalid: not SIP URI
            to: "sip:valid@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(std::time::Instant::now()),
        };
        
        let validation_result = helper.validate_call_session(&invalid_session);
        assert!(validation_result.is_err());
        
        // Test invalid incoming call
        let invalid_call = IncomingCall {
            id: SessionId::new(),
            from: "sip:caller@example.com".to_string(),
            to: "sip:callee@example.com".to_string(),
            sdp: Some("invalid sdp content".to_string()), // Invalid SDP
            headers: std::collections::HashMap::new(),
            received_at: std::time::Instant::now(),
        };
        
        let call_validation = helper.validate_incoming_call(&invalid_call);
        assert!(call_validation.is_err());
        
        // Test session statistics validation
        let invalid_stats = SessionStats {
            total_sessions: 10,
            active_sessions: 15, // Invalid: more active than total
            failed_sessions: 5,
            average_duration: None,
        };
        
        let stats_validation = ApiTestUtils::validate_session_stats(&invalid_stats);
        assert!(stats_validation.is_err());
        
        // Test port range validation
        let invalid_port_validation = ApiTestUtils::validate_port_range(6000, 5000); // Start > end
        assert!(invalid_port_validation.is_err());
        
        println!("Completed test_api_error_handling_integration");
    }).await;
    
    assert!(result.is_ok(), "test_api_error_handling_integration timed out");
}

#[tokio::test]
async fn test_api_performance_integration() {
    let result = time::timeout(Duration::from_secs(15), async {
        println!("Starting test_api_performance_integration");
        
        let helper = ApiTypesTestHelper::new();
        let operation_count = 500;
        
        let start = std::time::Instant::now();
        
        // Test creating many API objects quickly
        let mut session_ids = Vec::new();
        let mut call_sessions = Vec::new();
        let mut incoming_calls = Vec::new();
        let mut handlers = Vec::new();
        let mut builders = Vec::new();
        
        for i in 0..operation_count {
            // Create session ID
            session_ids.push(SessionId::new());
            
            // Create call session
            call_sessions.push(CallSession {
                id: SessionId(format!("perf_session_{}", i)),
                from: format!("sip:user{}@example.com", i),
                to: format!("sip:target{}@example.com", i),
                state: CallState::Active,
                started_at: Some(std::time::Instant::now()),
            });
            
            // Create incoming call
            incoming_calls.push(create_incoming_call(
                &format!("sip:caller{}@example.com", i),
                &format!("sip:callee{}@example.com", i),
                Some(helper.create_test_sdp(&format!("perf_{}", i))),
                ApiTestUtils::create_test_headers(),
            ));
            
            // Create handler
            handlers.push(Arc::new(TestCallHandler::new(CallDecision::Accept(None))));
            
            // Create builder
            builders.push(
                SessionManagerBuilder::new()
                    .with_sip_port(5060 + (i % 100) as u16)
                    .with_handler(handlers[i].clone())
            );
        }
        
        let duration = start.elapsed();
        println!("Created {} API objects in {:?}", operation_count * 5, duration);
        
        // Performance should be reasonable
        assert!(duration < Duration::from_secs(10), "API object creation took too long");
        
        // Verify all objects were created properly
        assert_eq!(session_ids.len(), operation_count);
        assert_eq!(call_sessions.len(), operation_count);
        assert_eq!(incoming_calls.len(), operation_count);
        assert_eq!(handlers.len(), operation_count);
        assert_eq!(builders.len(), operation_count);
        
        // Test handler processing performance
        let handler_start = std::time::Instant::now();
        
        for (i, handler) in handlers.iter().enumerate() {
            let decision = handler.on_incoming_call(incoming_calls[i].clone()).await;
            assert!(matches!(decision, CallDecision::Accept(_)));
        }
        
        let handler_duration = handler_start.elapsed();
        println!("Processed {} calls through handlers in {:?}", operation_count, handler_duration);
        
        assert!(handler_duration < Duration::from_secs(5), "Handler processing took too long");
        
        println!("Completed test_api_performance_integration");
    }).await;
    
    assert!(result.is_ok(), "test_api_performance_integration timed out");
}

#[tokio::test]
async fn test_api_concurrent_integration() {
    let result = time::timeout(Duration::from_secs(15), async {
        println!("Starting test_api_concurrent_integration");
        
        let helper = ApiTypesTestHelper::new();
        let concurrent_count = 50;
        let mut handles = Vec::new();
        
        // Create concurrent API operations
        for i in 0..concurrent_count {
            let helper_clone = ApiTypesTestHelper::new();
            
            let handle = tokio::spawn(async move {
                // Create various API objects concurrently
                let session_id = SessionId::new();
                
                let call_session = CallSession {
                    id: session_id.clone(),
                    from: format!("sip:concurrent{}@example.com", i),
                    to: format!("sip:target{}@example.com", i),
                    state: CallState::Active,
                    started_at: Some(std::time::Instant::now()),
                };
                
                let incoming_call = create_incoming_call(
                    &format!("sip:caller{}@example.com", i),
                    &format!("sip:callee{}@example.com", i),
                    Some(helper_clone.create_test_sdp(&format!("concurrent_{}", i))),
                    std::collections::HashMap::new(),
                );
                
                let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
                
                let builder = SessionManagerBuilder::new()
                    .with_sip_port(5060 + i as u16)
                    .with_handler(handler.clone());
                
                // Process a call through the handler
                let decision = handler.on_incoming_call(incoming_call.clone()).await;
                assert!(matches!(decision, CallDecision::Accept(_)));
                
                // Return data for verification
                (session_id, call_session, incoming_call, handler, builder)
            });
            handles.push(handle);
        }
        
        // Collect all results
        let mut results = Vec::new();
        for handle in handles {
            let result = handle.await.unwrap();
            results.push(result);
        }
        
        // Verify all concurrent operations completed successfully
        assert_eq!(results.len(), concurrent_count);
        
        // Verify uniqueness and correctness
        let mut unique_session_ids = std::collections::HashSet::new();
        
        for (i, (session_id, call_session, incoming_call, handler, builder)) in results.iter().enumerate() {
            // Session IDs should be unique
            assert!(unique_session_ids.insert(session_id.as_str()));
            
            // Objects should be properly constructed
            assert!(call_session.is_active());
            assert_eq!(call_session.from, format!("sip:concurrent{}@example.com", i));
            
            assert_eq!(incoming_call.caller(), &format!("sip:caller{}@example.com", i));
            assert_eq!(incoming_call.called(), &format!("sip:callee{}@example.com", i));
            
            assert_eq!(handler.incoming_call_count(), 1);
            
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("SessionManagerBuilder"));
        }
        
        println!("Completed test_api_concurrent_integration with {} operations", concurrent_count);
    }).await;
    
    assert!(result.is_ok(), "test_api_concurrent_integration timed out");
}

#[tokio::test]
async fn test_api_comprehensive_workflow() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_api_comprehensive_workflow");
        
        // This test demonstrates a complete real-world API usage scenario
        
        let helper = ApiTypesTestHelper::new();
        
        // Step 1: Create and configure a routing handler
        let mut routing_handler = RoutingHandler::new();
        routing_handler.add_route("sip:100", "sip:alice@internal.com");
        routing_handler.add_route("sip:200", "sip:bob@internal.com");
        routing_handler.add_route("sip:911", "sip:emergency@service.com");
        routing_handler.set_default_action(CallDecision::Reject("Unknown extension".to_string()));
        
        // Step 2: Create a queue for overflow calls
        let queue_handler = QueueHandler::new(5);
        
        // Step 3: Create a composite handler for complex call routing
        let composite_handler = CompositeHandler::new()
            .add_handler(Arc::new(routing_handler))
            .add_handler(Arc::new(queue_handler));
        
        // Step 4: Build session manager configuration
        let session_builder = SessionManagerBuilder::new()
            .with_sip_port(0) // Random port
            .with_sip_bind_address("127.0.0.1")
            .with_from_uri("sip:pbx@company.com")
            .with_media_ports(20000, 30000)
            .with_handler(Arc::new(composite_handler))
            .p2p_mode();
        
        // Step 5: Simulate incoming calls with different patterns
        let test_scenarios = vec![
            ("sip:100@company.com", "Internal extension to Alice"),
            ("sip:200@company.com", "Internal extension to Bob"),
            ("sip:911@company.com", "Emergency call"),
            ("sip:999@company.com", "Unknown extension"),
            ("sip:external@outside.com", "External call"),
        ];
        
        for (to_uri, description) in test_scenarios {
            println!("Processing: {}", description);
            
            let incoming_call = create_incoming_call(
                "sip:caller@external.com",
                to_uri,
                Some(helper.create_test_sdp("workflow")),
                ApiTestUtils::create_test_headers(),
            );
            
            // Validate the incoming call
            assert!(helper.validate_incoming_call(&incoming_call).is_ok());
            
            // The call would be processed by the SessionManager built from session_builder
            // For this test, we just verify the structure is correct
            assert!(!incoming_call.id.as_str().is_empty());
            assert!(ApiTestUtils::is_valid_sip_uri(&incoming_call.from));
            assert!(ApiTestUtils::is_valid_sip_uri(&incoming_call.to));
        }
        
        // Step 6: Verify configuration
        let debug_str = format!("{:?}", session_builder);
        assert!(debug_str.contains("SessionManagerBuilder"));
        assert!(debug_str.contains("p2p_mode"));
        
        // Step 7: Test session state management
        let test_session = CallSession {
            id: SessionId::new(),
            from: "sip:caller@external.com".to_string(),
            to: "sip:100@company.com".to_string(),
            state: CallState::Active,
            started_at: Some(std::time::Instant::now()),
        };
        
        assert!(test_session.is_active());
        assert!(test_session.state.is_in_progress());
        assert!(!test_session.state.is_final());
        
        // Step 8: Test media information
        let media_info = helper.create_test_media_info("comprehensive");
        assert!(media_info.local_sdp.is_some());
        assert!(media_info.remote_sdp.is_some());
        assert!(media_info.codec.is_some());
        
        println!("Completed test_api_comprehensive_workflow successfully");
    }).await;
    
    assert!(result.is_ok(), "test_api_comprehensive_workflow timed out");
} 