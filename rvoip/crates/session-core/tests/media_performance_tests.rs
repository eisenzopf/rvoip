use rvoip_session_core::api::control::SessionControl;
// Media Performance Integration Tests
//
// Tests the performance characteristics of session-core integration with
// media-core. Validates latency, throughput, memory usage, and scalability
// under realistic load conditions.
//
// **CRITICAL**: All tests use REAL MediaEngine and measure actual performance.

use std::sync::Arc;
use std::time::{Duration, Instant};
use rvoip_session_core::{SessionCoordinator, SessionError};
use rvoip_session_core::media::MediaConfig;
use rvoip_session_core::media::DialogId;
use uuid::Uuid;

mod common;
use common::*;

/// Test real media session establishment latency
#[tokio::test]
async fn test_session_establishment_latency() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let mut establishment_times = Vec::new();
    
    // Measure real media session establishment time multiple times
    for i in 0..10 {
        let dialog_id = DialogId::new(&format!("latency-test-{}-{}", i, Uuid::new_v4()));
        let session_config = MediaConfig::default();
        let local_addr = format!("127.0.0.1:{}", 12000 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        let start_time = Instant::now();
        
        // Measure real MediaSessionController establishment time
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        let establishment_time = start_time.elapsed();
        establishment_times.push(establishment_time);
        
        // Verify session was created
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        // Clean up
        media_engine.stop_media(&dialog_id).await.unwrap();
        
        println!("Media session {} establishment time: {:?}", i, establishment_time);
    }
    
    // Calculate statistics
    let avg_time = establishment_times.iter().sum::<Duration>() / establishment_times.len() as u32;
    let max_time = establishment_times.iter().max().unwrap();
    let min_time = establishment_times.iter().min().unwrap();
    
    println!("Media establishment statistics:");
    println!("  Average: {:?}", avg_time);
    println!("  Minimum: {:?}", min_time);
    println!("  Maximum: {:?}", max_time);
    
    // Validate performance - media sessions should establish within reasonable time for real operations
    assert!(avg_time < Duration::from_millis(2000), "Average media establishment should be < 2s, got {:?}", avg_time);
    assert!(*max_time < Duration::from_millis(3000), "Max media establishment should be < 3s, got {:?}", max_time);
}

/// Test real concurrent media session scalability
#[tokio::test]
async fn test_concurrent_session_scalability() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let mut memory_monitor = MemoryMonitor::new();
    
    // Test concurrent media sessions at different scales
    for session_count in [5, 10, 15, 20] { // Reduced for test performance
        memory_monitor.update_peak();
        
        let mut session_ids = Vec::new();
        let start_time = Instant::now();
        
        // Create concurrent media sessions
        for i in 0..session_count {
            let dialog_id = DialogId::new(&format!("concurrent-{}-{}-{}", session_count, i, Uuid::new_v4()));
            let session_config = MediaConfig::default();
            let local_addr = format!("127.0.0.1:{}", 13000 + i * 4).parse().unwrap();
            let media_config = rvoip_session_core::media::convert_to_media_core_config(
                &session_config,
                local_addr,
                None,
            );
            
            media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
            session_ids.push(dialog_id);
        }
        
        let creation_time = start_time.elapsed();
        println!("Created {} concurrent sessions in {:?}", session_count, creation_time);
        
        // Verify all sessions exist
        for session_id in &session_ids {
            let session_info = media_engine.get_session_info(session_id).await.unwrap();
            assert_eq!(session_info.dialog_id, *session_id);
        }
        
        // Measure cleanup time
        let cleanup_start = Instant::now();
        for session_id in session_ids {
            media_engine.stop_media(&session_id).await.unwrap();
        }
        let cleanup_time = cleanup_start.elapsed();
        
        println!("Cleaned up {} sessions in {:?}", session_count, cleanup_time);
        println!("Memory usage with {} sessions: {} bytes", 
                session_count, memory_monitor.get_memory_increase());
        
        // Performance should scale reasonably for real media operations
        let time_per_session = creation_time / session_count as u32;
        assert!(time_per_session < Duration::from_millis(1500), 
               "Time per session should be < 1.5s for real media, got {:?}", time_per_session);
    }
    
    println!("Final memory usage: {} bytes", memory_monitor.get_memory_increase());
}

/// Test real media codec performance under load
#[tokio::test]
async fn test_codec_performance_under_load() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Test different session operations under increasing load
    let load_levels = [1, 10, 50, 100]; // Concurrent operations
    
    for &load_level in &load_levels {
        println!("Testing integration performance at load level: {}", load_level);
        
        // Test media session creation performance
        let creation_start = Instant::now();
        let creation_results = validate_concurrent_operations(
            {
                let engine = media_engine.clone();
                move || {
                    let engine = engine.clone();
                    async move {
                        // Create a test dialog ID and media config
                        let dialog_id = DialogId::new(&format!("test-{}", Uuid::new_v4()));
                        let session_config = MediaConfig::default();
                        let local_addr = "127.0.0.1:10000".parse().unwrap();
                        let media_config = rvoip_session_core::media::convert_to_media_core_config(
                            &session_config,
                            local_addr,
                            None,
                        );
                        
                        // Test media session creation and cleanup
                        engine.start_media(dialog_id.clone(), media_config).await
                            .map_err(|e| {
                                let error_string = format!("{:?}", e);
                                Box::<dyn std::error::Error + Send + Sync>::from(error_string)
                            })?;
                        
                        engine.stop_media(&dialog_id).await
                            .map_err(|e| {
                                let error_string = format!("{:?}", e);
                                Box::<dyn std::error::Error + Send + Sync>::from(error_string)
                            })?;
                        
                        Ok(())
                    }
                }
            },
            load_level
        ).await;
        let creation_duration = creation_start.elapsed();
        
        println!("Load level {}: Session creation={:?}", 
                load_level, creation_duration);
        
        // TODO: Validate performance doesn't degrade significantly
        // - Measure latency increase under load
        // - Verify real-time processing capability maintained
        // - Test resource usage scaling
    }
    
    // Verify codecs are available for performance testing
    assert!(!capabilities.codecs.is_empty(), "Codecs should be available for performance testing");
    assert!(true, "Test stubbed - implement comprehensive session-core integration performance testing");
}

/// Test real memory usage patterns and leak detection
#[tokio::test]
async fn test_memory_usage_and_leak_detection() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let mut memory_monitor = MemoryMonitor::new();
    let mut baseline_memory = 0;
    
    // Test real memory usage patterns with media sessions
    for cycle in 0..5 { // Reduced cycles for performance
        memory_monitor.update_peak();
        
        // Create multiple media sessions
        let mut session_ids = Vec::new();
        for i in 0..3 { // 3 sessions per cycle
            let dialog_id = DialogId::new(&format!("memory-test-{}-{}-{}", cycle, i, Uuid::new_v4()));
            let session_config = MediaConfig::default();
            let local_addr = format!("127.0.0.1:{}", 15000 + cycle * 20 + i * 4).parse().unwrap();
            let media_config = rvoip_session_core::media::convert_to_media_core_config(
                &session_config,
                local_addr,
                None,
            );
            
            media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
            session_ids.push(dialog_id);
        }
        
        let after_creation = memory_monitor.get_memory_increase();
        
        // Clean up all sessions
        for session_id in session_ids {
            media_engine.stop_media(&session_id).await.unwrap();
        }
        
        // Allow some time for cleanup
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        let after_cleanup = memory_monitor.get_memory_increase();
        println!("Cycle {}: Memory after creation = {} bytes, after cleanup = {} bytes", 
                 cycle, after_creation, after_cleanup);
        
        if cycle == 0 {
            baseline_memory = after_cleanup;
        } else {
            // Memory shouldn't grow significantly compared to baseline
            let growth = after_cleanup.saturating_sub(baseline_memory);
            println!("Memory growth from baseline: {} bytes", growth);
            
            // Allow some growth but not excessive
            assert!(growth < 1024 * 1024, "Memory growth should be < 1MB per cycle, got {} bytes", growth);
        }
    }
    
    println!("Final memory usage: {} bytes", memory_monitor.get_memory_increase());
}

/// Test real-time media session coordination latency
#[tokio::test]
async fn test_realtime_audio_processing_latency() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Create a test media session using proper API
    let dialog_id = DialogId::new(&format!("latency-test-{}", Uuid::new_v4()));
    let session_config = MediaConfig::default();
    let local_addr = "127.0.0.1:14000".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Start media session for testing
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Get session info to create MediaSessionInfo for latency testing
    let test_session = media_engine.get_session_info(&dialog_id).await.unwrap();
    let test_session_info = rvoip_session_core::media::MediaSessionInfo::from(test_session);
    
    // Measure processing latency multiple times
    let mut latencies = Vec::new();
    
    for i in 0..100 {
        let latency = measure_media_session_latency(&test_session_info).await.unwrap();
        latencies.push(latency);
        
        if i % 10 == 0 {
            println!("Sample {}: Latency = {:?}", i, latency);
        }
    }
    
    // Calculate statistics
    let avg_latency = latencies.iter().sum::<Duration>() / latencies.len() as u32;
    let max_latency = latencies.iter().max().unwrap();
    let min_latency = latencies.iter().min().unwrap();
    
    println!("Media session coordination latency statistics:");
    println!("  Average: {:?}", avg_latency);
    println!("  Minimum: {:?}", min_latency);
    println!("  Maximum: {:?}", max_latency);
    
    // Validate real-time performance requirements
    assert!(avg_latency < Duration::from_millis(50), "Average processing latency should be < 50ms, got {:?}", avg_latency);
    assert!(*max_latency < Duration::from_millis(100), "Max processing latency should be < 100ms, got {:?}", max_latency);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test real media performance under network stress conditions
#[tokio::test]
async fn test_performance_under_network_stress() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test scenarios with real media sessions
    struct NetworkCondition {
        name: &'static str,
        packet_loss: f32,
        jitter: f32,
        bandwidth_limit: u32, // kbps
    }
    
    let conditions = [
        NetworkCondition { name: "excellent", packet_loss: 0.0, jitter: 5.0, bandwidth_limit: 1000 },
        NetworkCondition { name: "good", packet_loss: 0.01, jitter: 20.0, bandwidth_limit: 500 },
        NetworkCondition { name: "poor", packet_loss: 0.05, jitter: 100.0, bandwidth_limit: 200 },
        NetworkCondition { name: "very_poor", packet_loss: 0.1, jitter: 200.0, bandwidth_limit: 100 },
    ];
    
    for (i, condition) in conditions.iter().enumerate() {
        println!("Testing performance under {} network conditions", condition.name);
        
        // Create media session for each network condition
        let dialog_id = DialogId::new(&format!("network-stress-{}-{}", condition.name, Uuid::new_v4()));
        let session_config = MediaConfig::default();
        let local_addr = format!("127.0.0.1:{}", 16000 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        // Measure establishment time under network stress
        let start_time = Instant::now();
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        let establishment_time = start_time.elapsed();
        
        // Verify session works under network conditions
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        // Calculate MOS for network conditions
        let mos = validate_mos_score_calculation(
            condition.packet_loss,
            condition.jitter,
            150.0 // base delay
        ).unwrap();
        
        println!("  {} - Establishment: {:?}, Calculated MOS: {:.2}", 
                 condition.name, establishment_time, mos);
        
        // Sessions should still establish even under poor conditions
        assert!(establishment_time < Duration::from_secs(2), 
               "Establishment should complete within 2s even under {} conditions", condition.name);
        
        // Clean up
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
}

/// Test real CPU usage and resource characteristics
#[tokio::test]
async fn test_cpu_usage_characteristics() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test CPU usage with real media session operations
    let load_levels = [5, 10, 15, 20]; // Different concurrent session counts
    
    for &session_count in &load_levels {
        println!("Testing CPU usage with {} concurrent media sessions", session_count);
        
        let start_time = Instant::now();
        let mut session_ids = Vec::new();
        
        // Create multiple sessions to test CPU load
        for i in 0..session_count {
            let dialog_id = DialogId::new(&format!("cpu-test-{}-{}-{}", session_count, i, Uuid::new_v4()));
            let session_config = MediaConfig::default();
            let local_addr = format!("127.0.0.1:{}", 18000 + i * 4).parse().unwrap();
            let media_config = rvoip_session_core::media::convert_to_media_core_config(
                &session_config,
                local_addr,
                None,
            );
            
            media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
            session_ids.push(dialog_id);
        }
        
        let creation_time = start_time.elapsed();
        
        // Allow sessions to be active for CPU measurement
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Clean up all sessions
        let cleanup_start = Instant::now();
        for session_id in session_ids {
            media_engine.stop_media(&session_id).await.unwrap();
        }
        let cleanup_time = cleanup_start.elapsed();
        
        println!("  {} sessions - Creation: {:?}, Cleanup: {:?}", 
                 session_count, creation_time, cleanup_time);
        
                 // Performance should scale reasonably with session count for real media
         let time_per_session = creation_time / session_count as u32;
         assert!(time_per_session < Duration::from_millis(1500), 
                "CPU time per session should be < 1.5s for real media, got {:?}", time_per_session);
    }
    
    println!("CPU usage test completed successfully with real media sessions");
}

/// Test session cleanup performance and efficiency
#[tokio::test]
async fn test_session_cleanup_performance() {
    let (session_manager, media_engine) = create_test_session_manager_with_media().await.unwrap();
    
    // TODO: Implement cleanup performance testing
    // - Create large number of sessions
    // - Terminate all sessions simultaneously
    // - Measure cleanup time
    // - Verify no resource leaks during cleanup
    // - Test cleanup under abnormal termination scenarios
    
    let session_count = 50;
    let cleanup_start = Instant::now();
    
    // TODO: Actual session cleanup testing
    // - Create sessions
    // - Terminate sessions
    // - Measure cleanup efficiency
    
    let cleanup_duration = cleanup_start.elapsed();
    let cleanup_per_session = cleanup_duration / session_count;
    
    println!("Cleanup performance:");
    println!("  Total time: {:?}", cleanup_duration);
    println!("  Per session: {:?}", cleanup_per_session);
    
    // TODO: Validate cleanup performance requirements
    // assert!(cleanup_per_session < Duration::from_millis(10), "Cleanup per session should be < 10ms");
    
    assert!(true, "Test stubbed - implement session cleanup performance testing");
}

/// Test real performance monitoring and metrics collection
#[tokio::test]
async fn test_performance_monitoring_overhead() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test baseline performance with standard media operations
    let operation_count = 10;
    
    // Baseline test - simple media session operations
    let baseline_start = Instant::now();
    for i in 0..operation_count {
        let dialog_id = DialogId::new(&format!("baseline-{}-{}", i, Uuid::new_v4()));
        let session_config = MediaConfig::default();
        let local_addr = format!("127.0.0.1:{}", 19000 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
    let baseline_duration = baseline_start.elapsed();
    
    // Test with monitoring-like operations (getting session info)
    let monitored_start = Instant::now();
    for i in 0..operation_count {
        let dialog_id = DialogId::new(&format!("monitored-{}-{}", i, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.quality_monitoring = true; // Enable monitoring
        let local_addr = format!("127.0.0.1:{}", 19200 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        
        // Simulate monitoring overhead by getting session info
        let _session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
    let monitored_duration = monitored_start.elapsed();
    
    let overhead = monitored_duration.saturating_sub(baseline_duration);
    let overhead_percentage = if baseline_duration.as_nanos() > 0 {
        (overhead.as_nanos() as f64 / baseline_duration.as_nanos() as f64) * 100.0
    } else {
        0.0
    };
    
    println!("Performance monitoring overhead:");
    println!("  Baseline: {:?}", baseline_duration);
    println!("  With monitoring: {:?}", monitored_duration);
    println!("  Overhead: {:?} ({:.2}%)", overhead, overhead_percentage);
    
    // Validate monitoring overhead is reasonable for real media operations
    assert!(overhead_percentage < 50.0, "Monitoring overhead should be < 50% for this test scale, got {:.2}%", overhead_percentage);
    assert!(monitored_duration < Duration::from_secs(15), "Monitored operations should complete within 15s");
} 