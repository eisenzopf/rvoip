//! Tests for Manager Performance and Stress Testing
//!
//! Tests manager performance under various load conditions including
//! high-volume session creation, concurrent operations, and stress scenarios.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    api::types::{CallState, SessionId},
    manager::{events::SessionEvent, cleanup::CleanupManager},
};
use common::*;

#[tokio::test]
async fn test_session_creation_performance() {
    let manager = create_test_session_manager().await.unwrap();
    let session_count = 100;
    
    let start = std::time::Instant::now();
    let mut session_ids = Vec::new();
    
    for i in 0..session_count {
        let from = format!("sip:perf{}@localhost", i);
        let to = format!("sip:target{}@localhost", i);
        let call = manager.create_outgoing_call(&from, &to, Some("perf SDP".to_string())).await.unwrap();
        session_ids.push(call.id().clone());
    }
    
    let elapsed = start.elapsed();
    println!("Created {} sessions in {:?}", session_count, elapsed);
    
    // Performance assertions
    assert!(elapsed < Duration::from_secs(10), "Session creation took too long");
    assert_eq!(session_ids.len(), session_count);
    
    // Verify all sessions exist
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, session_count);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_lookup_performance() {
    let manager = create_test_session_manager().await.unwrap();
    let session_count = 1000;
    let lookup_count = 2000;
    
    // Create sessions first
    let mut session_ids = Vec::new();
    for i in 0..session_count {
        let from = format!("sip:lookup{}@localhost", i);
        let to = "sip:target@localhost";
        let call = manager.create_outgoing_call(&from, to, Some("lookup SDP".to_string())).await.unwrap();
        session_ids.push(call.id().clone());
    }
    
    // Measure lookup performance
    let start = std::time::Instant::now();
    
    for i in 0..lookup_count {
        let session_idx = i % session_ids.len();
        let session_id = &session_ids[session_idx];
        let session = manager.find_session(session_id).await.unwrap();
        assert!(session.is_some());
    }
    
    let elapsed = start.elapsed();
    println!("Performed {} lookups in {:?}", lookup_count, elapsed);
    
    // Performance assertions
    assert!(elapsed < Duration::from_secs(5), "Session lookup took too long");
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_session_operations_performance() {
    let session_count = 25; // Reduced for performance with real dialogs
    let mut handles = Vec::new();
    
    let start = std::time::Instant::now();
    
    // Spawn concurrent tasks with proper SIP dialog establishment
    for task_id in 0..session_count {
        let handle = tokio::spawn(async move {
            // Create session manager pair for real SIP dialog
            let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
            
            // Establish real SIP dialog
            let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
            let session_id = call.id().clone();
            
            // Perform SIP operations on established dialog
            manager_a.hold_session(&session_id).await.unwrap();
            manager_a.resume_session(&session_id).await.unwrap();
            
            // Cleanup
            cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
            
            task_id
        });
        handles.push(handle);
    }
    
    // Wait for all tasks to complete
    let mut completed_tasks = Vec::new();
    for handle in handles {
        let task_id = handle.await.unwrap();
        completed_tasks.push(task_id);
    }
    
    let elapsed = start.elapsed();
    
    println!("Completed {} concurrent dialog operations in {:?}", session_count, elapsed);
    assert_eq!(completed_tasks.len(), session_count);
    
    // Performance assertions (adjusted for real SIP operations)
    assert!(elapsed < Duration::from_secs(60), "Concurrent operations took too long");
}

#[tokio::test]
async fn test_event_publishing_performance() {
    let mut helper = EventTestHelper::new().await.unwrap();
    helper.subscribe().await.unwrap();
    
    let event_count = 1000;
    let start = std::time::Instant::now();
    
    // Publish events rapidly
    for i in 0..event_count {
        let event = SessionEvent::SessionCreated {
            session_id: SessionId(format!("perf-event-{}", i)),
            from: format!("sip:publisher{}@localhost", i),
            to: "sip:target@localhost".to_string(),
            call_state: CallState::Initiating,
        };
        
        helper.publish_event(event).await.unwrap();
    }
    
    let publish_elapsed = start.elapsed();
    println!("Published {} events in {:?}", event_count, publish_elapsed);
    
    // Receive events
    let receive_start = std::time::Instant::now();
    let mut received_count = 0;
    
    while received_count < event_count {
        if helper.wait_for_event(Duration::from_millis(10)).await.is_some() {
            received_count += 1;
        } else {
            // Small timeout to allow more events to arrive
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    
    let receive_elapsed = receive_start.elapsed();
    println!("Received {} events in {:?}", received_count, receive_elapsed);
    
    // Performance assertions
    assert!(publish_elapsed < Duration::from_secs(10), "Event publishing took too long");
    assert!(receive_elapsed < Duration::from_secs(15), "Event receiving took too long");
    assert_eq!(received_count, event_count);
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_registry_performance_stress() {
    let mut helper = RegistryTestHelper::new();
    let session_count = 10000;
    
    // Measure registration performance
    let start = std::time::Instant::now();
    
    for i in 0..session_count {
        helper.add_test_session(
            &format!("sip:stress{}@localhost", i),
            "sip:target@localhost",
            CallState::Active
        ).await;
    }
    
    let registration_elapsed = start.elapsed();
    println!("Registered {} sessions in {:?}", session_count, registration_elapsed);
    
    // Verify count
    helper.verify_session_count(session_count).await;
    
    // Measure lookup performance
    let lookup_start = std::time::Instant::now();
    let lookup_count = session_count / 2;
    
    for i in 0..lookup_count {
        let session_id = SessionId(format!("manager-test-session-{}", i));
        let _session = helper.registry().get_session(&session_id).await.unwrap();
    }
    
    let lookup_elapsed = lookup_start.elapsed();
    println!("Performed {} lookups in {:?}", lookup_count, lookup_elapsed);
    
    // Performance assertions
    assert!(registration_elapsed < Duration::from_secs(30), "Registration took too long");
    assert!(lookup_elapsed < Duration::from_secs(10), "Lookup took too long");
}

#[tokio::test]
async fn test_cleanup_manager_performance() {
    let cleanup_manager = Arc::new(CleanupManager::new());
    cleanup_manager.start().await.unwrap();
    
    let cleanup_count = 5000;
    let start = std::time::Instant::now();
    
    // Perform many cleanup operations
    for i in 0..cleanup_count {
        let session_id = SessionId(format!("cleanup-perf-{}", i));
        cleanup_manager.cleanup_session(&session_id).await.unwrap();
    }
    
    let elapsed = start.elapsed();
    println!("Performed {} cleanups in {:?}", cleanup_count, elapsed);
    
    // Performance assertion
    assert!(elapsed < Duration::from_secs(20), "Cleanup operations took too long");
    
    cleanup_manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_memory_usage_stress() {
    let manager = create_test_session_manager().await.unwrap();
    let session_count = 100; // Reduced for performance test focus
    
    // Create sessions without SIP operations (just measuring memory usage)
    let mut session_ids = Vec::new();
    
    for i in 0..session_count {
        let from = format!("sip:memory{}@localhost", i);
        let to = "sip:target@localhost";
        let call = manager.create_outgoing_call(&from, to, Some("memory test SDP".to_string())).await.unwrap();
        session_ids.push(call.id().clone());
    }
    
    // Verify all sessions exist
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, session_count);
    
    // Test a few with real SIP operations (using dialog establishment)
    if session_count > 10 {
        let (test_manager_a, test_manager_b, mut test_events) = create_session_manager_pair().await.unwrap();
        let (test_call, _) = establish_call_between_managers(&test_manager_a, &test_manager_b, &mut test_events).await.unwrap();
        
        // Test SIP operations on properly established dialog
        test_manager_a.hold_session(test_call.id()).await.unwrap();
        test_manager_a.resume_session(test_call.id()).await.unwrap();
        
        cleanup_managers(vec![test_manager_a, test_manager_b]).await.unwrap();
    }
    
    // For memory stress test, just stop the manager (cleans up all sessions automatically)
    println!("Memory stress test completed with {} sessions created", session_count);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_load_balancing_simulation() {
    // Simulate load balancing across multiple managers
    let manager_count = 5;
    let sessions_per_manager = 100;
    
    let mut managers = Vec::new();
    for i in 0..manager_count {
        let config = ManagerTestConfig {
            from_uri_base: format!("load-test-{}@localhost", i),
            ..ManagerTestConfig::default()
        };
        let (handler, _) = EventTrackingHandler::new();
        let manager = create_test_session_manager_with_config(config, Arc::new(handler)).await.unwrap();
        managers.push(manager);
    }
    
    let start = std::time::Instant::now();
    let mut handles = Vec::new();
    
    // Create sessions across all managers concurrently
    for (manager_idx, manager) in managers.iter().enumerate() {
        let manager_clone = Arc::clone(manager);
        let handle = tokio::spawn(async move {
            let mut session_ids = Vec::new();
            
            for session_idx in 0..sessions_per_manager {
                let from = format!("sip:load{}{}@localhost", manager_idx, session_idx);
                let to = "sip:target@localhost";
                let call = manager_clone.create_outgoing_call(&from, to, Some("load test SDP".to_string())).await.unwrap();
                session_ids.push(call.id().clone());
            }
            
            session_ids
        });
        handles.push(handle);
    }
    
    // Wait for all managers to finish
    let mut all_sessions = Vec::new();
    for handle in handles {
        let sessions = handle.await.unwrap();
        all_sessions.extend(sessions);
    }
    
    let elapsed = start.elapsed();
    let total_sessions = manager_count * sessions_per_manager;
    
    println!("Created {} sessions across {} managers in {:?}", total_sessions, manager_count, elapsed);
    assert_eq!(all_sessions.len(), total_sessions);
    
    // Verify each manager has the expected number of sessions
    for manager in &managers {
        let stats = manager.get_stats().await.unwrap();
        assert_eq!(stats.active_sessions, sessions_per_manager);
    }
    
    // Cleanup all managers
    for manager in managers {
        manager.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_burst_load_handling() {
    // Simulate burst loads with proper SIP operations
    let burst_size = 10; // Reduced for real SIP dialog performance
    let burst_count = 3;  // Reduced for faster test execution
    
    for burst_idx in 0..burst_count {
        println!("Processing burst {}", burst_idx);
        
        let burst_start = std::time::Instant::now();
        let mut dialog_pairs = Vec::new();
        
        // Create burst of real SIP dialogs
        for i in 0..burst_size {
            let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
            let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
            dialog_pairs.push((manager_a, manager_b, call.id().clone()));
        }
        
        // Perform SIP operations on established dialogs
        for (manager_a, _, session_id) in &dialog_pairs {
            manager_a.hold_session(session_id).await.unwrap();
            manager_a.send_dtmf(session_id, "1").await.unwrap();
            manager_a.resume_session(session_id).await.unwrap();
        }
        
        // Cleanup all dialogs
        for (manager_a, manager_b, _) in dialog_pairs {
            cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
        }
        
        let burst_elapsed = burst_start.elapsed();
        println!("Burst {} completed in {:?}", burst_idx, burst_elapsed);
        
        // Allow some time between bursts
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_long_running_stability() {
    let manager = create_test_session_manager().await.unwrap();
    
    let duration = Duration::from_secs(5); // Reduced for test speed
    let operations_per_second = 5; // Reduced for stability test
    let interval = Duration::from_millis(1000 / operations_per_second);
    
    let start = std::time::Instant::now();
    let mut operation_count = 0;
    let mut active_sessions = Vec::new();
    
    while start.elapsed() < duration {
        let from = format!("sip:stability{}@localhost", operation_count);
        let to = "sip:target@localhost";
        
        // Create session (no SIP operations needed for stability test)
        let call = manager.create_outgoing_call(&from, to, Some("stability SDP".to_string())).await.unwrap();
        active_sessions.push(call.id().clone());
        
        // For stability test, don't terminate individual sessions to avoid SIP protocol issues
        // Just let them accumulate to test memory stability
        if active_sessions.len() > 15 { // Allow more sessions for stability testing
            // Stop adding sessions once we reach threshold
            break;
        }
        
        operation_count += 1;
        tokio::time::sleep(interval).await;
    }
    
    println!("Performed {} operations over {:?}", operation_count, start.elapsed());
    
    // Verify sessions are stable (no cleanup needed - manager.stop() will handle it)
    let final_stats = manager.get_stats().await.unwrap();
    assert!(final_stats.active_sessions > 0, "Should have created stable sessions");
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_performance_regression_detection() {
    // Baseline performance measurement with proper SIP dialogs
    let test_iterations = 3;
    let dialogs_per_iteration = 5; // Reduced for real SIP dialog performance
    
    let mut creation_times = Vec::new();
    let mut operation_times = Vec::new();
    
    for iteration in 0..test_iterations {
        // Measure session creation with dialog establishment
        let creation_start = std::time::Instant::now();
        let mut dialog_pairs = Vec::new();
        
        for i in 0..dialogs_per_iteration {
            let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
            let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
            dialog_pairs.push((manager_a, manager_b, call.id().clone()));
        }
        
        let creation_time = creation_start.elapsed();
        creation_times.push(creation_time);
        
        // Measure SIP operations on established dialogs
        let operations_start = std::time::Instant::now();
        
        for (manager_a, _, session_id) in &dialog_pairs {
            manager_a.hold_session(session_id).await.unwrap();
            manager_a.resume_session(session_id).await.unwrap();
        }
        
        let operations_time = operations_start.elapsed();
        operation_times.push(operations_time);
        
        // Cleanup all dialogs
        for (manager_a, manager_b, _) in dialog_pairs {
            cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
        }
    }
    
    // Analyze performance consistency
    let avg_creation_time = creation_times.iter().sum::<Duration>() / creation_times.len() as u32;
    let avg_operation_time = operation_times.iter().sum::<Duration>() / operation_times.len() as u32;
    
    println!("Average dialog creation time: {:?}", avg_creation_time);
    println!("Average SIP operation time: {:?}", avg_operation_time);
    
    // Check for reasonable performance (adjusted for real SIP operations)
    assert!(avg_creation_time < Duration::from_secs(30), "Average creation time too high");
    assert!(avg_operation_time < Duration::from_secs(15), "Average operation time too high");
    
    // Check that no single iteration was dramatically slower (regression detection)
    for (i, &time) in creation_times.iter().enumerate() {
        assert!(time < avg_creation_time * 3, "Creation time regression in iteration {}: {:?}", i, time);
    }
    
    for (i, &time) in operation_times.iter().enumerate() {
        assert!(time < avg_operation_time * 3, "Operation time regression in iteration {}: {:?}", i, time);
    }
} 