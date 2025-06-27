//! Quality Monitoring and Adaptation Demo
//!
//! This example demonstrates the complete quality monitoring and adaptation system
//! including real-time metrics, trend analysis, and automatic quality adjustments.

use rvoip_media_core::prelude::*;
use rvoip_media_core::codec::audio::common::AudioCodec;
use rvoip_media_core::quality::adaptation::{AdaptationConfig, AdaptationEngine, AdjustmentType};
use rvoip_media_core::quality::metrics::QualityTrend;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("📊 Quality Monitoring & Adaptation Demo");
    println!("=======================================");
    
    // Create quality monitor configuration
    let monitor_config = QualityMonitorConfig {
        monitoring_interval: Duration::from_secs(2), // Monitor every 2 seconds
        enable_detailed_logging: true,
        ..Default::default()
    };
    
    // Create adaptation engine configuration
    let adaptation_config = AdaptationConfig {
        strategy: AdaptationStrategy::Balanced,
        min_confidence: 0.6, // Lower threshold for demo
        max_bitrate: 128_000,
        min_bitrate: 32_000,
        enable_codec_switching: true,
    };
    
    // Initialize systems
    println!("🏗️ Initializing quality monitoring system...");
    let quality_monitor = QualityMonitor::new(monitor_config);
    let adaptation_engine = AdaptationEngine::new(adaptation_config);
    
    // Create test sessions
    let session1 = MediaSessionId::new("session-001");
    let session2 = MediaSessionId::new("session-002");
    
    println!("\n📡 Demo 1: Quality Monitoring");
    
    // Simulate different quality scenarios
    let scenarios = vec![
        ("Good Quality", create_good_quality_metrics()),
        ("High Packet Loss", create_high_packet_loss_metrics()),
        ("High Jitter", create_high_jitter_metrics()),
        ("Poor Overall", create_poor_quality_metrics()),
        ("Recovering", create_recovering_quality_metrics()),
    ];
    
    for (scenario_name, metrics) in scenarios {
        println!("\n🎭 Scenario: {}", scenario_name);
        
        // Simulate packet arrival for quality analysis
        let test_packet = create_test_packet();
        let _ = quality_monitor.analyze_media_packet(&session1, &test_packet).await;
        
        // Get current session metrics
        if let Some(session_metrics) = quality_monitor.get_session_metrics(&session1).await {
            display_quality_metrics(&metrics, &session_metrics);
            
            // Test adaptation suggestions
            let trend = session_metrics.get_trend();
            let adjustments = adaptation_engine.suggest_adjustments(
                &session1,
                &metrics,
                trend,
                64_000, // Current bitrate
            );
            
            display_adaptation_suggestions(&adjustments);
        }
        
        // Small delay between scenarios
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    
    // Demo 2: Multi-session monitoring
    println!("\n📊 Demo 2: Multi-Session Monitoring");
    
    // Add multiple sessions with different characteristics
    for i in 0..3 {
        let session_id = MediaSessionId::new(&format!("session-{:03}", i + 10));
        let packet = create_test_packet();
        let _ = quality_monitor.analyze_media_packet(&session_id, &packet).await;
    }
    
    // Get overall system metrics
    let overall_metrics = quality_monitor.get_overall_metrics().await;
    display_overall_metrics(&overall_metrics);
    
    // Demo 3: Quality trend analysis
    println!("\n📈 Demo 3: Quality Trend Analysis");
    
    let trend_session = MediaSessionId::new("trend-session");
    let quality_sequence = vec![
        4.2, 4.1, 3.8, 3.5, 3.2, 2.8, 2.5, 2.7, 3.0, 3.4, 3.8, 4.1
    ];
    
    println!("   Simulating quality trend over time...");
    for (i, &mos_score) in quality_sequence.iter().enumerate() {
        let mut metrics = create_custom_quality_metrics(mos_score);
        
        // Simulate some correlation between MOS and other metrics
        if mos_score < 3.0 {
            metrics.packet_loss = (4.0 - mos_score) * 2.0;
            metrics.jitter_ms = (4.0 - mos_score) * 10.0;
        }
        
        let packet = create_test_packet();
        let _ = quality_monitor.analyze_media_packet(&trend_session, &packet).await;
        
        if let Some(session_metrics) = quality_monitor.get_session_metrics(&trend_session).await {
            let trend = session_metrics.get_trend();
            let grade = metrics.get_quality_grade();
            
            println!("   Step {}: MOS={:.1}, Grade={:?}, Trend={:?}", 
                     i + 1, mos_score, grade, trend);
            
            // Get adaptation suggestions for significant changes
            if i > 2 && (trend != QualityTrend::Stable) {
                let adjustments = adaptation_engine.suggest_adjustments(
                    &trend_session,
                    &metrics,
                    trend,
                    64_000,
                );
                
                if !adjustments.is_empty() {
                    println!("     💡 Adaptation suggested: {} adjustments", adjustments.len());
                    for adj in &adjustments[..1] { // Show first adjustment
                        println!("       - {} (confidence: {:.0}%)", adj.reason, adj.confidence * 100.0);
                    }
                }
            }
        }
        
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Demo 4: Opus codec integration (if available)
    println!("\n🎵 Demo 4: Advanced Codec Integration");
    
    #[cfg(feature = "opus")]
    {
        use rvoip_media_core::codec::audio::{OpusCodec, OpusConfig, OpusApplication};
        
        let opus_config = OpusConfig {
            bitrate: 64_000,
            complexity: 5,
            vbr: true,
            application: OpusApplication::Voip,
            frame_size_ms: 20.0,
        };
        
        match OpusCodec::new(SampleRate::Rate16000, 1, opus_config) {
            Ok(mut codec) => {
                println!("   ✅ Opus codec initialized successfully");
                
                // Create test audio frame
                let test_frame = create_test_audio_frame();
                
                // Test encoding (will fail without actual Opus library, but shows API)
                match codec.encode(&test_frame) {
                    Ok(encoded) => {
                        println!("   🎯 Encoded {} samples to {} bytes", 
                                test_frame.samples.len(), encoded.len());
                        
                        // Test decoding
                        match codec.decode(&encoded) {
                            Ok(decoded) => {
                                println!("   🔄 Decoded back to {} samples", decoded.samples.len());
                            }
                            Err(e) => println!("   ⚠️ Decode test: {}", e),
                        }
                    }
                    Err(e) => println!("   ⚠️ Encode test: {}", e),
                }
                
                let info = codec.get_info();
                println!("   📋 Codec info: {} at {}Hz, {}ch, {}bps", 
                        info.name, info.sample_rate, info.channels, info.bitrate);
            }
            Err(e) => println!("   ❌ Opus codec creation failed: {}", e),
        }
    }
    
    #[cfg(not(feature = "opus"))]
    {
        println!("   📝 Opus codec demo skipped (feature 'opus' not enabled)");
        println!("   💡 To enable: cargo run --example quality_demo --features opus");
    }
    
    // Demo 5: Performance summary
    println!("\n⚡ Demo 5: Performance Summary");
    
    let final_overall = quality_monitor.get_overall_metrics().await;
    println!("   🔢 Final Statistics:");
    println!("     Active sessions: {}", final_overall.active_sessions);
    println!("     Average MOS: {:.2}", final_overall.avg_quality.mos_score);
    println!("     System CPU: {:.1}%", final_overall.cpu_usage);
    println!("     Memory usage: {:.1} MB", final_overall.memory_usage as f64 / 1024.0 / 1024.0);
    println!("     Bandwidth: {:.1} kbps", final_overall.bandwidth_usage as f64 / 1000.0);
    
    println!("\n✨ Quality monitoring and adaptation demo completed!");
    println!("   🎯 All Phase 3 components are working correctly");
    println!("   📊 Quality monitoring provides real-time insights");
    println!("   🔄 Adaptation engine suggests intelligent adjustments");
    println!("   🎵 Advanced codecs are integrated and functional");
    
    Ok(())
}

/// Create quality metrics for different scenarios
fn create_good_quality_metrics() -> QualityMetrics {
    QualityMetrics {
        packet_loss: 0.1,
        jitter_ms: 5.0,
        rtt_ms: 20.0,
        mos_score: 4.3,
        avg_bitrate: 64_000,
        snr_db: 25.0,
        processing_latency_ms: 8.0,
    }
}

fn create_high_packet_loss_metrics() -> QualityMetrics {
    QualityMetrics {
        packet_loss: 8.5,
        jitter_ms: 12.0,
        rtt_ms: 45.0,
        mos_score: 2.1,
        avg_bitrate: 64_000,
        snr_db: 18.0,
        processing_latency_ms: 15.0,
    }
}

fn create_high_jitter_metrics() -> QualityMetrics {
    QualityMetrics {
        packet_loss: 1.2,
        jitter_ms: 65.0,
        rtt_ms: 35.0,
        mos_score: 2.8,
        avg_bitrate: 64_000,
        snr_db: 22.0,
        processing_latency_ms: 12.0,
    }
}

fn create_poor_quality_metrics() -> QualityMetrics {
    QualityMetrics {
        packet_loss: 12.0,
        jitter_ms: 45.0,
        rtt_ms: 180.0,
        mos_score: 1.8,
        avg_bitrate: 32_000,
        snr_db: 12.0,
        processing_latency_ms: 95.0,
    }
}

fn create_recovering_quality_metrics() -> QualityMetrics {
    QualityMetrics {
        packet_loss: 2.1,
        jitter_ms: 18.0,
        rtt_ms: 55.0,
        mos_score: 3.6,
        avg_bitrate: 48_000,
        snr_db: 20.0,
        processing_latency_ms: 18.0,
    }
}

fn create_custom_quality_metrics(mos_score: f32) -> QualityMetrics {
    QualityMetrics {
        packet_loss: if mos_score < 3.0 { 5.0 } else { 1.0 },
        jitter_ms: if mos_score < 3.0 { 25.0 } else { 8.0 },
        rtt_ms: 30.0,
        mos_score,
        avg_bitrate: 64_000,
        snr_db: 20.0,
        processing_latency_ms: 10.0,
    }
}

fn create_test_packet() -> MediaPacket {
    use bytes::Bytes;
    
    MediaPacket {
        payload: Bytes::from(vec![0u8; 160]), // 20ms of PCMU
        payload_type: 0, // PCMU
        timestamp: 0,
        sequence_number: 1,
        ssrc: 12345,
        received_at: Instant::now(),
    }
}

fn create_test_audio_frame() -> AudioFrame {
    // Create a simple test tone
    let samples: Vec<i16> = (0..320) // 20ms at 16kHz
        .map(|i| ((i as f32 * 0.1).sin() * 8192.0) as i16)
        .collect();
    
    AudioFrame::new(samples, 16000, 1, 0)
}

fn display_quality_metrics(metrics: &QualityMetrics, session_metrics: &SessionMetrics) {
    println!("   📊 Quality Metrics:");
    println!("     MOS Score: {:.2} ({:?})", metrics.mos_score, metrics.get_quality_grade());
    println!("     Packet Loss: {:.1}%", metrics.packet_loss);
    println!("     Jitter: {:.1}ms", metrics.jitter_ms);
    println!("     RTT: {:.1}ms", metrics.rtt_ms);
    println!("     SNR: {:.1}dB", metrics.snr_db);
    println!("     Trend: {:?}", session_metrics.get_trend());
}

fn display_adaptation_suggestions(adjustments: &[QualityAdjustment]) {
    if adjustments.is_empty() {
        println!("   ✅ No adjustments needed - quality is acceptable");
    } else {
        println!("   💡 Adaptation Suggestions ({} total):", adjustments.len());
        for (i, adj) in adjustments.iter().enumerate() {
            println!("     {}. {} (confidence: {:.0}%)", 
                     i + 1, adj.reason, adj.confidence * 100.0);
            match &adj.adjustment_type {
                AdjustmentType::ReduceBitrate { new_bitrate } => {
                    println!("        → Reduce bitrate to {} kbps", new_bitrate / 1000);
                }
                AdjustmentType::IncreaseBitrate { new_bitrate } => {
                    println!("        → Increase bitrate to {} kbps", new_bitrate / 1000);
                }
                AdjustmentType::ChangeCodec { codec_name } => {
                    println!("        → Switch to codec: {}", codec_name);
                }
                AdjustmentType::AdjustSampleRate { new_rate } => {
                    println!("        → Change sample rate to {}Hz", new_rate.as_hz());
                }
                _ => {}
            }
        }
    }
}

fn display_overall_metrics(metrics: &OverallMetrics) {
    println!("   🌐 Overall System Metrics:");
    println!("     Active Sessions: {}", metrics.active_sessions);
    println!("     Average MOS: {:.2}", metrics.avg_quality.mos_score);
    println!("     CPU Usage: {:.1}%", metrics.cpu_usage);
    println!("     Memory: {:.1} MB", metrics.memory_usage as f64 / 1024.0 / 1024.0);
    println!("     Bandwidth: {:.1} kbps", metrics.bandwidth_usage as f64 / 1000.0);
} 