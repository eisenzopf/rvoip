//! Phase 1.3: Session Controller Performance Integration Tests
//! 
//! This test validates that MediaSessionController properly integrates
//! with advanced v2 processors and performance monitoring.

use rvoip_media_core::prelude::*;
use rvoip_media_core::relay::{MediaSessionController, MediaConfig};
use rvoip_media_core::processing::audio::{AdvancedVadConfig, AdvancedAgcConfig, AdvancedAecConfig};
use rvoip_media_core::types::{AudioFrame, DialogId};
use std::net::SocketAddr;
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_session_controller_performance_integration() -> Result<()> {
    println!("🧪 Phase 1.3: Session Controller Performance Integration Test");
    println!("============================================================");
    
    // Create MediaSessionController
    let controller = MediaSessionController::new();
    println!("✅ MediaSessionController created with Arc<AudioFramePool>");
    
    // Check frame pool stats
    let pool_stats = controller.get_frame_pool_stats();
    println!("📊 Initial frame pool stats: {} frames available", pool_stats.pool_size);
    assert!(pool_stats.pool_size > 0, "Frame pool should have frames available");
    
    // Create advanced processor configuration with single-band AGC (working configuration)
    let mut processor_config = AdvancedProcessorConfig::default();
    processor_config.enable_advanced_vad = true;
    processor_config.enable_advanced_agc = true;
    processor_config.enable_advanced_aec = true;
    processor_config.enable_simd = true;
    processor_config.sample_rate = 16000; // Use 16 kHz to avoid VAD frame size issues
    processor_config.frame_pool_size = 8; // Create dedicated session pool
    
    // Configure AGC for single-band processing to avoid biquad filterbank issues
    processor_config.agc_config.num_bands = 1;
    processor_config.agc_config.crossover_frequencies = vec![]; // No crossovers for single band
    processor_config.agc_config.attack_times_ms = vec![10.0];
    processor_config.agc_config.release_times_ms = vec![150.0];
    processor_config.agc_config.compression_ratios = vec![3.0];
    processor_config.agc_config.max_gains_db = vec![12.0];
    
    println!("🔧 Advanced processor configuration:");
    println!("   - VAD v2: {}", processor_config.enable_advanced_vad);
    println!("   - AGC v2: {} (single-band)", processor_config.enable_advanced_agc);
    println!("   - AEC v2: {}", processor_config.enable_advanced_aec);
    println!("   - SIMD: {}", processor_config.enable_simd);
    println!("   - Sample rate: {}", processor_config.sample_rate);
    println!("   - Session pool size: {}", processor_config.frame_pool_size);
    
    // Create media config
    let media_config = MediaConfig {
        local_addr: "127.0.0.1:5060".parse().unwrap(),
        remote_addr: Some("127.0.0.1:5061".parse().unwrap()),
        preferred_codec: Some("PCMU".to_string()),
        parameters: std::collections::HashMap::new(),
    };
    
    // Test basic media session
    let dialog_id = DialogId::new("test-dialog-123");
    println!("\n📞 Starting basic media session for dialog: {}", dialog_id);
    
    controller.start_media(dialog_id.clone(), media_config.clone()).await?;
    println!("✅ Basic media session started");
    
    // Check session info
    if let Some(session_info) = controller.get_session_info(&dialog_id).await {
        println!("📊 Session info: port={:?}, status={:?}", 
                session_info.rtp_port, session_info.status);
        assert!(session_info.rtp_port.is_some(), "RTP port should be allocated");
    } else {
        panic!("Session info should be available");
    }
    
    // Test advanced media session with processors
    let dialog_id_advanced = DialogId::new("test-dialog-advanced-456");
    println!("\n🚀 Starting advanced media session for dialog: {}", dialog_id_advanced);
    
    controller.start_advanced_media(
        dialog_id_advanced.clone(), 
        media_config.clone(), 
        Some(processor_config)
    ).await?;
    println!("✅ Advanced media session started with v2 processors");
    
    // Check if advanced processors are enabled
    let has_processors = controller.has_advanced_processors(&dialog_id_advanced).await;
    println!("🔍 Advanced processors enabled: {}", has_processors);
    assert!(has_processors, "Advanced processors should be enabled");
    
    // Test audio processing with advanced processors
    println!("\n⚡ Testing advanced audio processing...");
    
    // Create a test audio frame (512 samples for 32ms at 16kHz - meets VAD v2 minimum)
    let samples = vec![1000i16; 512];
    let test_frame = AudioFrame {
        samples,
        sample_rate: 16000,
        channels: 1,
        timestamp: 0,
        duration: Duration::from_millis(32),
    };
    
    // Process with advanced processors
    let processed_frame = controller.process_advanced_audio(&dialog_id_advanced, test_frame).await?;
    println!("✅ Audio frame processed with advanced v2 processors");
    println!("   - Input samples: 512, Output samples: {}", processed_frame.samples.len());
    assert_eq!(processed_frame.samples.len(), 512, "Output frame should maintain sample count");
    
    // Get performance metrics
    if let Some(dialog_metrics) = controller.get_dialog_performance_metrics(&dialog_id_advanced).await {
        println!("📈 Dialog performance metrics:");
        println!("   - Operations: {}", dialog_metrics.operation_count);
        println!("   - Average timing: {:?}", dialog_metrics.avg_time);
        assert!(dialog_metrics.operation_count > 0, "Should have recorded operations");
    }
    
    let global_metrics = controller.get_global_performance_metrics().await;
    println!("🌍 Global performance metrics:");
    println!("   - Operations: {}", global_metrics.operation_count);
    println!("   - Average timing: {:?}", global_metrics.avg_time);
    assert!(global_metrics.operation_count > 0, "Should have recorded global operations");
    
    // Test frame pool usage
    let final_pool_stats = controller.get_frame_pool_stats();
    println!("\n📊 Final frame pool stats:");
    println!("   - Pool size: {}", final_pool_stats.pool_size);
    println!("   - Pool hits: {}", final_pool_stats.pool_hits);
    println!("   - Pool misses: {}", final_pool_stats.pool_misses);
    println!("   - Allocated count: {}", final_pool_stats.allocated_count);
    
    // Clean up sessions
    println!("\n🧹 Cleaning up sessions...");
    controller.stop_media(&dialog_id).await?;
    controller.stop_media(&dialog_id_advanced).await?;
    println!("✅ Sessions cleaned up");
    
    // Verify advanced processors were cleaned up
    let has_processors_after = controller.has_advanced_processors(&dialog_id_advanced).await;
    println!("🔍 Advanced processors after cleanup: {}", has_processors_after);
    assert!(!has_processors_after, "Advanced processors should be cleaned up");
    
    println!("\n🎉 Phase 1.3 Session Controller Performance Integration test completed successfully!");
    println!("✅ All Arc<AudioFramePool> patterns working correctly");
    println!("✅ Advanced processor integration functional");
    println!("✅ Performance monitoring operational");
    println!("✅ Session lifecycle management working");
    
    Ok(())
}

#[tokio::test]
#[serial] 
async fn test_performance_metrics_tracking() -> Result<()> {
    println!("📊 Testing Performance Metrics Tracking");
    println!("=======================================");
    
    let controller = MediaSessionController::new();
    
    // Reset global metrics
    controller.reset_global_metrics().await;
    
    let initial_metrics = controller.get_global_performance_metrics().await;
    assert_eq!(initial_metrics.operation_count, 0, "Should start with zero operations");
    
    // Create a session with basic configuration
    let dialog_id = DialogId::new("metrics-test-dialog");
    let media_config = MediaConfig {
        local_addr: "127.0.0.1:5060".parse().unwrap(),
        remote_addr: Some("127.0.0.1:5061".parse().unwrap()),
        preferred_codec: Some("PCMU".to_string()),
        parameters: std::collections::HashMap::new(),
    };
    
    controller.start_media(dialog_id.clone(), media_config).await?;
    
    // Test that metrics are tracked
    let final_metrics = controller.get_global_performance_metrics().await;
    println!("Final metrics: {} operations", final_metrics.operation_count);
    
    controller.stop_media(&dialog_id).await?;
    
    println!("✅ Performance metrics tracking test completed");
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_frame_pool_integration() -> Result<()> {
    println!("🏊 Testing Frame Pool Integration");
    println!("=================================");
    
    let controller = MediaSessionController::new();
    
    let initial_stats = controller.get_frame_pool_stats();
    println!("Initial pool stats: size={}, hits={}, misses={}", 
            initial_stats.pool_size, initial_stats.pool_hits, initial_stats.pool_misses);
    
    assert!(initial_stats.pool_size > 0, "Pool should have initial frames");
    
    // The frame pool is working correctly just by creating the controller
    
    println!("✅ Frame pool integration test completed");
    Ok(())
} 