//! Tests for Bridge Performance and Concurrency
//!
//! Performance tests, benchmarks, and concurrency tests for bridge functionality.
//! These tests ensure bridges can handle high loads and concurrent operations.

mod common;

use std::time::Duration;
use tokio::time::Instant;
use rvoip_session_core::{
    api::types::SessionId,
    bridge::SessionBridge,
};
use common::*;

#[tokio::test]
async fn test_bridge_add_session_performance() {
    let mut perf_test = BridgePerformanceTest::new("perf-add-test", 100);
    
    let duration = perf_test.run_add_session_benchmark().await;
    
    println!("Added 100 sessions in {:?}", duration);
    
    // Performance assertion - should complete within reasonable time
    assert!(duration < Duration::from_secs(1), "Adding 100 sessions took too long: {:?}", duration);
    
    // Verify final state
    assert_eq!(perf_test.bridge().session_count(), 100);
}

#[tokio::test]
async fn test_bridge_remove_session_performance() {
    let mut perf_test = BridgePerformanceTest::new("perf-remove-test", 100);
    
    // First add all sessions
    let _add_duration = perf_test.run_add_session_benchmark().await;
    
    // Then time the removal
    let remove_duration = perf_test.run_remove_session_benchmark().await;
    
    println!("Removed 100 sessions in {:?}", remove_duration);
    
    // Performance assertion
    assert!(remove_duration < Duration::from_secs(1), "Removing 100 sessions took too long: {:?}", remove_duration);
    
    // Verify final state
    perf_test.verify_final_state();
}

#[tokio::test]
async fn test_bridge_large_session_count_performance() {
    let session_count = 1000;
    let mut bridge = create_test_bridge("large-bridge-test");
    let session_ids = create_test_session_ids(session_count);
    
    let start = Instant::now();
    
    // Add all sessions
    for session_id in &session_ids {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    
    let add_duration = start.elapsed();
    println!("Added {} sessions in {:?}", session_count, add_duration);
    
    // Start bridge
    let start_time = Instant::now();
    assert!(bridge.start().is_ok());
    let start_duration = start_time.elapsed();
    println!("Started bridge with {} sessions in {:?}", session_count, start_duration);
    
    // Verify state
    verify_bridge_state(&bridge, true, session_count);
    
    // Remove all sessions
    let remove_start = Instant::now();
    for session_id in &session_ids {
        assert!(bridge.remove_session(session_id).is_ok());
    }
    let remove_duration = remove_start.elapsed();
    println!("Removed {} sessions in {:?}", session_count, remove_duration);
    
    verify_bridge_state(&bridge, true, 0);
    
    // Performance assertions
    assert!(add_duration < Duration::from_secs(5), "Adding {} sessions took too long", session_count);
    assert!(remove_duration < Duration::from_secs(5), "Removing {} sessions took too long", session_count);
    assert!(start_duration < Duration::from_millis(100), "Starting bridge took too long");
}

#[tokio::test]
async fn test_bridge_concurrent_operations() {
    let concurrency_test = BridgeConcurrencyTest::new(5);
    let operations_per_bridge = 50;
    
    let start = Instant::now();
    let results = concurrency_test.run_concurrent_operations(operations_per_bridge).await;
    let total_duration = start.elapsed();
    
    println!("Completed concurrent operations on 5 bridges in {:?}", total_duration);
    
    // Verify all operations succeeded
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "Bridge {} operations failed: {:?}", i, result);
    }
    
    println!("✓ All {} bridges completed {} operations successfully", results.len(), operations_per_bridge);
    
    // Performance assertion - all concurrent operations should complete reasonably quickly
    assert!(total_duration < Duration::from_secs(10), "Concurrent operations took too long: {:?}", total_duration);
}

#[tokio::test]
async fn test_bridge_rapid_start_stop_cycles() {
    let mut bridge = create_test_bridge("rapid-cycle-test");
    let session_ids = create_test_session_ids(10);
    
    // Add some sessions
    for session_id in &session_ids {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    
    let cycles = 100;
    let start = Instant::now();
    
    // Rapid start/stop cycles
    for i in 0..cycles {
        assert!(bridge.start().is_ok(), "Failed to start bridge on cycle {}", i);
        assert!(bridge.is_active(), "Bridge not active on cycle {}", i);
        
        assert!(bridge.stop().is_ok(), "Failed to stop bridge on cycle {}", i);
        assert!(!bridge.is_active(), "Bridge still active on cycle {}", i);
    }
    
    let duration = start.elapsed();
    println!("Completed {} start/stop cycles in {:?}", cycles, duration);
    
    // Performance assertion
    assert!(duration < Duration::from_secs(2), "Rapid cycles took too long: {:?}", duration);
    
    // Verify final state
    verify_bridge_state(&bridge, false, 10);
}

#[tokio::test]
async fn test_bridge_mixed_concurrent_operations() {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    
    let bridge = Arc::new(Mutex::new(create_test_bridge("mixed-ops-test")));
    let session_ids = Arc::new(create_test_session_ids(200));
    
    let mut handles = Vec::new();
    
    // Spawn tasks for different types of operations
    
    // Task 1: Add sessions
    {
        let bridge_clone = bridge.clone();
        let sessions_clone = session_ids.clone();
        let handle = tokio::spawn(async move {
            for session_id in sessions_clone.iter().take(100) {
                let mut bridge_guard = bridge_clone.lock().await;
                bridge_guard.add_session(session_id.clone()).expect("Failed to add session");
            }
        });
        handles.push(handle);
    }
    
    // Task 2: Start/stop bridge
    {
        let bridge_clone = bridge.clone();
        let handle = tokio::spawn(async move {
            for _ in 0..20 {
                {
                    let mut bridge_guard = bridge_clone.lock().await;
                    bridge_guard.start().expect("Failed to start bridge");
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
                {
                    let mut bridge_guard = bridge_clone.lock().await;
                    bridge_guard.stop().expect("Failed to stop bridge");
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        handles.push(handle);
    }
    
    // Task 3: Add more sessions
    {
        let bridge_clone = bridge.clone();
        let sessions_clone = session_ids.clone();
        let handle = tokio::spawn(async move {
            for session_id in sessions_clone.iter().skip(100).take(100) {
                let mut bridge_guard = bridge_clone.lock().await;
                bridge_guard.add_session(session_id.clone()).expect("Failed to add session");
                tokio::time::sleep(Duration::from_micros(100)).await; // Small delay
            }
        });
        handles.push(handle);
    }
    
    let start = Instant::now();
    
    // Wait for all tasks to complete
    for handle in handles {
        handle.await.expect("Task failed");
    }
    
    let duration = start.elapsed();
    println!("Mixed concurrent operations completed in {:?}", duration);
    
    // Verify final state
    let bridge_guard = bridge.lock().await;
    assert_eq!(bridge_guard.session_count(), 200);
    println!("✓ Bridge has correct final session count: {}", bridge_guard.session_count());
    
    // Performance assertion
    assert!(duration < Duration::from_secs(5), "Mixed operations took too long: {:?}", duration);
}

#[tokio::test]
async fn test_bridge_memory_efficiency() {
    // Test that bridges don't consume excessive memory with many sessions
    let session_count = 5000;
    let mut bridge = create_test_bridge("memory-test");
    
    // Measure memory-like behavior by tracking operation performance as size increases
    let mut durations = Vec::new();
    
    let session_ids = create_test_session_ids(session_count);
    
    // Add sessions in batches and measure time per batch
    for chunk in session_ids.chunks(1000) {
        let start = Instant::now();
        for session_id in chunk {
            assert!(bridge.add_session(session_id.clone()).is_ok());
        }
        let duration = start.elapsed();
        durations.push(duration);
        println!("Added batch of 1000 sessions (total: {}) in {:?}", bridge.session_count(), duration);
    }
    
    // Verify performance doesn't degrade significantly (no major memory leaks/inefficiencies)
    let first_batch_time = durations[0];
    let last_batch_time = durations[durations.len() - 1];
    
    // Last batch should be at most 3x slower than first (allowing for some degradation)
    assert!(
        last_batch_time <= first_batch_time * 3,
        "Performance degraded too much: first batch {:?}, last batch {:?}",
        first_batch_time,
        last_batch_time
    );
    
    verify_bridge_state(&bridge, false, session_count);
    println!("✓ Bridge memory efficiency test passed with {} sessions", session_count);
}

#[tokio::test]
async fn test_bridge_stress_test() {
    // Comprehensive stress test combining multiple scenarios
    let bridge_count = 10;
    let sessions_per_bridge = 100;
    let concurrent_operations = 50;
    
    println!("Starting bridge stress test with {} bridges, {} sessions each", bridge_count, sessions_per_bridge);
    
    let start = Instant::now();
    
    // Create multiple concurrent tests
    let mut stress_handles = Vec::new();
    
    for bridge_idx in 0..bridge_count {
        let handle = tokio::spawn(async move {
            let mut manager = BridgeSessionManager::new(&format!("stress-bridge-{}", bridge_idx));
            let session_ids = create_test_session_ids(sessions_per_bridge);
            
            // Add sessions
            for session_id in &session_ids {
                manager.add_session(session_id.clone()).expect("Failed to add session");
            }
            
            // Start bridge
            manager.start_bridge().expect("Failed to start bridge");
            
            // Perform rapid operations
            for i in 0..concurrent_operations {
                if i % 10 == 0 {
                    // Occasionally stop and restart
                    manager.stop_bridge().expect("Failed to stop bridge");
                    manager.start_bridge().expect("Failed to restart bridge");
                }
                
                // Add and remove temporary session
                let temp_session = SessionId(format!("temp-{}-{}", bridge_idx, i));
                manager.add_session(temp_session.clone()).expect("Failed to add temp session");
                manager.remove_session(&temp_session).expect("Failed to remove temp session");
            }
            
            // Verify consistency
            manager.verify_consistency();
            
            // Final state check
            assert_eq!(manager.bridge().session_count(), sessions_per_bridge);
            assert!(manager.bridge().is_active());
            
            bridge_idx
        });
        stress_handles.push(handle);
    }
    
    // Wait for all stress tests to complete
    let mut completed_bridges = Vec::new();
    for handle in stress_handles {
        let bridge_idx = handle.await.expect("Stress test task failed");
        completed_bridges.push(bridge_idx);
    }
    
    let total_duration = start.elapsed();
    
    println!("Stress test completed in {:?}", total_duration);
    println!("Successfully tested {} bridges with {} operations each", completed_bridges.len(), concurrent_operations);
    
    // Verify all bridges completed
    assert_eq!(completed_bridges.len(), bridge_count);
    
    // Performance assertion for stress test
    assert!(total_duration < Duration::from_secs(30), "Stress test took too long: {:?}", total_duration);
}

#[tokio::test]
async fn test_bridge_performance_regression() {
    // Baseline performance test to catch regressions
    let operations = [10, 50, 100, 500, 1000];
    let mut baseline_established = false;
    let mut baseline_time_per_op = Duration::from_nanos(0);
    
    for &op_count in &operations {
        let mut bridge = create_test_bridge(&format!("regression-test-{}", op_count));
        let session_ids = create_test_session_ids(op_count);
        
        let start = Instant::now();
        
        // Add sessions
        for session_id in &session_ids {
            assert!(bridge.add_session(session_id.clone()).is_ok());
        }
        
        // Start bridge
        assert!(bridge.start().is_ok());
        
        // Remove sessions  
        for session_id in &session_ids {
            assert!(bridge.remove_session(session_id).is_ok());
        }
        
        let duration = start.elapsed();
        let time_per_op = duration / op_count as u32;
        
        println!("Operations: {}, Total time: {:?}, Time per op: {:?}", op_count, duration, time_per_op);
        
        if !baseline_established && op_count == 100 {
            baseline_time_per_op = time_per_op;
            baseline_established = true;
            println!("Established baseline: {:?} per operation", baseline_time_per_op);
        }
        
        // For larger operation counts, time per op shouldn't be significantly worse than baseline
        if baseline_established && op_count > 100 {
            let max_acceptable = baseline_time_per_op * 5; // Allow 5x degradation for large datasets to account for system variability
            assert!(
                time_per_op <= max_acceptable,
                "Performance regression detected: {} ops/sec is {:?} per op (baseline: {:?}, max acceptable: {:?})",
                op_count,
                time_per_op,
                baseline_time_per_op,
                max_acceptable
            );
        }
        
        verify_bridge_state(&bridge, true, 0);
    }
    
    println!("✓ Performance regression test passed for all operation counts");
} 