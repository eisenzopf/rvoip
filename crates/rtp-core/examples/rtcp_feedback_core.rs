//! RTCP Feedback Core Example
//!
//! This example demonstrates the low-level RTCP feedback packet handling capabilities
//! including Picture Loss Indication (PLI), Full Intra Request (FIR), Receiver Estimated
//! Max Bitrate (REMB), and the core feedback generation algorithms.
//!
//! This is a core-level example showing direct packet manipulation and algorithm usage.

use rvoip_rtp_core::{
    Result, RtpSsrc,
    feedback::{
        FeedbackContext, FeedbackConfig, FeedbackDecision, FeedbackPriority,
        QualityDegradation, CongestionState, FeedbackGenerator, FeedbackGeneratorFactory
    },
    feedback::packets::{FeedbackPacket, PliPacket, FirPacket, RembPacket},
    feedback::algorithms::{GoogleCongestionControl, SimpleBandwidthEstimator, QualityAssessment, QualityMetrics, PacketFeedback},
    api::common::stats::StreamStats,
};
use std::time::{Instant, Duration};
use tracing::{info, debug, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("=== RTCP Feedback Core Example ===");
    info!("Demonstrating low-level RTCP feedback packet handling and algorithms");

    // Test 1: Basic feedback packet creation and serialization
    demonstrate_feedback_packets().await?;

    // Test 2: Feedback generation algorithms
    demonstrate_feedback_generators().await?;

    // Test 3: Bandwidth estimation algorithms
    demonstrate_bandwidth_estimation().await?;

    // Test 4: Quality assessment
    demonstrate_quality_assessment().await?;

    info!("✅ RTCP Feedback Core Example completed successfully");
    Ok(())
}

/// Demonstrate basic feedback packet creation and serialization
async fn demonstrate_feedback_packets() -> Result<()> {
    info!("\n📦 Testing RTCP Feedback Packet Creation and Serialization");

    let sender_ssrc: RtpSsrc = 0x12345678;
    let media_ssrc: RtpSsrc = 0x87654321;

    // 1. Picture Loss Indication (PLI)
    info!("Creating PLI packet...");
    let pli = PliPacket::new(sender_ssrc, media_ssrc);
    let pli_bytes = pli.serialize()?;
    info!("✅ PLI packet created: {} bytes", pli_bytes.len());
    
    // Parse it back to verify
    if let Ok(FeedbackPacket::Pli(parsed_pli)) = FeedbackPacket::parse_from_rtcp(&pli_bytes) {
        debug!("PLI round-trip successful: sender={:08x}, media={:08x}", 
               parsed_pli.sender_ssrc, parsed_pli.media_ssrc);
    } else {
        warn!("PLI round-trip failed");
    }

    // 2. Full Intra Request (FIR)
    info!("Creating FIR packet...");
    let fir = FirPacket::new(sender_ssrc, media_ssrc, 42);
    let fir_bytes = fir.serialize()?;
    info!("✅ FIR packet created: {} bytes, sequence: 42", fir_bytes.len());

    // 3. Receiver Estimated Max Bitrate (REMB)
    info!("Creating REMB packet...");
    let remb = RembPacket::new(sender_ssrc, 2_000_000, vec![media_ssrc]); // 2 Mbps
    let remb_bytes = remb.serialize()?;
    info!("✅ REMB packet created: {} bytes, bitrate: 2 Mbps", remb_bytes.len());
    
    // Parse REMB back to verify bitrate encoding
    if let Ok(FeedbackPacket::Remb(parsed_remb)) = FeedbackPacket::parse_from_rtcp(&remb_bytes) {
        debug!("REMB round-trip successful: bitrate={} bps, SSRCs: {:?}", 
               parsed_remb.bitrate_bps, parsed_remb.ssrcs);
    } else {
        warn!("REMB round-trip failed");
    }

    info!("📦 Feedback packet tests completed");
    Ok(())
}

/// Demonstrate feedback generation algorithms
async fn demonstrate_feedback_generators() -> Result<()> {
    info!("\n🤖 Testing Feedback Generation Algorithms");

    let local_ssrc: RtpSsrc = 0x11111111;
    let media_ssrc: RtpSsrc = 0x22222222;

    // Create feedback context and config
    let mut context = FeedbackContext::new(local_ssrc, media_ssrc);
    let config = FeedbackConfig::default();

    info!("Feedback configuration:");
    info!("  PLI enabled: {}, interval: {}ms", config.enable_pli, config.pli_interval_ms);
    info!("  FIR enabled: {}, interval: {}ms", config.enable_fir, config.fir_interval_ms);
    info!("  REMB enabled: {}, max rate: {} pps", config.enable_remb, config.max_feedback_rate);

    // Test different generators
    let generators = [
        ("Loss Generator", FeedbackGeneratorFactory::create_loss_generator()),
        ("Congestion Generator", FeedbackGeneratorFactory::create_congestion_generator()),
        ("Quality Generator", FeedbackGeneratorFactory::create_quality_generator()),
        ("Comprehensive Generator", FeedbackGeneratorFactory::create_comprehensive_generator()),
    ];

    for (name, mut generator) in generators {
        info!("\nTesting {}:", name);

        // Simulate statistics updates
        for scenario in 1..=4 {
            let stats = create_test_statistics(scenario);
            generator.update_statistics(&stats);

            // Update context congestion state
            let loss_rate = if stats.packet_count > 0 {
                stats.packets_lost as f32 / stats.packet_count as f32
            } else {
                0.0
            };
            context.update_congestion_state(loss_rate, 100, stats.jitter_ms as u32);

            // Generate feedback
            let decision = generator.generate_feedback(&context, &config)?;
            
            match decision {
                FeedbackDecision::None => {
                    debug!("  Scenario {}: No feedback needed", scenario);
                }
                FeedbackDecision::Pli { priority, reason } => {
                    info!("  Scenario {}: PLI recommended (priority: {:?}, reason: {:?})", 
                          scenario, priority, reason);
                }
                FeedbackDecision::Fir { priority, sequence_number } => {
                    info!("  Scenario {}: FIR recommended (priority: {:?}, seq: {})", 
                          scenario, priority, sequence_number);
                }
                FeedbackDecision::Remb { bitrate_bps, confidence } => {
                    info!("  Scenario {}: REMB recommended ({:.1} Mbps, confidence: {:.1}%)", 
                          scenario, bitrate_bps as f32 / 1_000_000.0, confidence * 100.0);
                }
                FeedbackDecision::Multiple(decisions) => {
                    info!("  Scenario {}: Multiple feedback recommended ({} types)", 
                          scenario, decisions.len());
                }
            }

            // Small delay between scenarios
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    info!("🤖 Feedback generation tests completed");
    Ok(())
}

/// Demonstrate bandwidth estimation algorithms
async fn demonstrate_bandwidth_estimation() -> Result<()> {
    info!("\n📊 Testing Bandwidth Estimation Algorithms");

    // Test 1: Google Congestion Control (GCC)
    info!("Testing Google Congestion Control...");
    let mut gcc = GoogleCongestionControl::new(1_000_000); // Start with 1 Mbps

    // Simulate packet feedback
    let feedback_packets = create_transport_feedback();
    for (i, batch) in feedback_packets.chunks(5).enumerate() {
        let estimated_bandwidth = gcc.update_with_feedback(batch);
        info!("  Batch {}: Estimated bandwidth: {:.1} Mbps (state: {:?})", 
              i + 1, estimated_bandwidth as f32 / 1_000_000.0, gcc.state());
    }

    // Test 2: Simple Bandwidth Estimator
    info!("Testing Simple Bandwidth Estimator...");
    let mut simple = SimpleBandwidthEstimator::new(1_000_000);

    for scenario in 1..=5 {
        let (bytes, window_ms, rtt_ms, loss_rate) = match scenario {
            1 => (125_000, 1000, 50, 0.0),    // Good conditions: 1 Mbps
            2 => (100_000, 1000, 100, 0.01),  // Light congestion
            3 => (75_000, 1000, 200, 0.03),   // Moderate congestion
            4 => (50_000, 1000, 400, 0.08),   // Heavy congestion
            5 => (150_000, 1000, 30, 0.0),    // Recovery
            _ => unreachable!(),
        };

        simple.update(bytes, window_ms, rtt_ms, loss_rate);
        
        info!("  Scenario {}: Est: {:.1} Mbps, Confidence: {:.1}% (RTT: {}ms, Loss: {:.1}%)",
              scenario, 
              simple.current_estimate() as f32 / 1_000_000.0,
              simple.confidence() * 100.0,
              rtt_ms,
              loss_rate * 100.0);
    }

    info!("📊 Bandwidth estimation tests completed");
    Ok(())
}

/// Demonstrate quality assessment
async fn demonstrate_quality_assessment() -> Result<()> {
    info!("\n🎯 Testing Quality Assessment");

    let qa = QualityAssessment::default();

    let test_scenarios = [
        ("Excellent", QualityMetrics { loss_rate: 0.0, jitter_ms: 5.0, rtt_ms: 20.0, bandwidth_utilization: 0.8 }),
        ("Good", QualityMetrics { loss_rate: 0.005, jitter_ms: 15.0, rtt_ms: 50.0, bandwidth_utilization: 0.9 }),
        ("Fair", QualityMetrics { loss_rate: 0.02, jitter_ms: 30.0, rtt_ms: 100.0, bandwidth_utilization: 0.95 }),
        ("Poor", QualityMetrics { loss_rate: 0.05, jitter_ms: 60.0, rtt_ms: 200.0, bandwidth_utilization: 1.0 }),
        ("Critical", QualityMetrics { loss_rate: 0.15, jitter_ms: 120.0, rtt_ms: 500.0, bandwidth_utilization: 1.0 }),
    ];

    for (label, metrics) in &test_scenarios {
        let quality_score = qa.calculate_quality(metrics);
        let mos_score = qa.quality_to_mos(quality_score);
        let needs_feedback = qa.requires_feedback(quality_score, 0.6);

        info!("  {}: Quality={:.2}, MOS={:.1}, Feedback needed: {}", 
              label, quality_score, mos_score, needs_feedback);
        
        debug!("    Metrics: Loss={:.1}%, Jitter={:.1}ms, RTT={:.1}ms, BW Util={:.1}%",
               metrics.loss_rate * 100.0, metrics.jitter_ms, metrics.rtt_ms, 
               metrics.bandwidth_utilization * 100.0);
    }

    info!("🎯 Quality assessment tests completed");
    Ok(())
}

/// Create test statistics for different network scenarios
fn create_test_statistics(scenario: u32) -> StreamStats {
    use rvoip_rtp_core::api::common::stats::Direction;
    use rvoip_rtp_core::api::common::frame::MediaFrameType;
    use rvoip_rtp_core::api::common::stats::QualityLevel;
    use std::net::SocketAddr;
    
    let remote_addr = "127.0.0.1:5000".parse::<SocketAddr>().unwrap();
    
    match scenario {
        1 => StreamStats {
            direction: Direction::Inbound,
            ssrc: 0x12345678,
            media_type: MediaFrameType::Video,
            packet_count: 1000,
            byte_count: 1_200_000,
            packets_lost: 0,
            fraction_lost: 0.0,
            jitter_ms: 10.0,
            rtt_ms: Some(50.0),
            mos: Some(4.5),
            remote_addr,
            bitrate_bps: 1_000_000,
            discard_rate: 0.0,
            quality: QualityLevel::Excellent,
            available_bandwidth_bps: Some(2_000_000),
        },
        2 => StreamStats {
            direction: Direction::Inbound,
            ssrc: 0x12345678,
            media_type: MediaFrameType::Video,
            packet_count: 1000,
            byte_count: 1_200_000,
            packets_lost: 10,  // 1% loss
            fraction_lost: 0.01,
            jitter_ms: 25.0,
            rtt_ms: Some(100.0),
            mos: Some(3.8),
            remote_addr,
            bitrate_bps: 1_000_000,
            discard_rate: 0.01,
            quality: QualityLevel::Good,
            available_bandwidth_bps: Some(1_500_000),
        },
        3 => StreamStats {
            direction: Direction::Inbound,
            ssrc: 0x12345678,
            media_type: MediaFrameType::Video,
            packet_count: 1000,
            byte_count: 1_200_000,
            packets_lost: 50,  // 5% loss
            fraction_lost: 0.05,
            jitter_ms: 60.0,
            rtt_ms: Some(200.0),
            mos: Some(2.5),
            remote_addr,
            bitrate_bps: 800_000,
            discard_rate: 0.05,
            quality: QualityLevel::Fair,
            available_bandwidth_bps: Some(1_000_000),
        },
        4 => StreamStats {
            direction: Direction::Inbound,
            ssrc: 0x12345678,
            media_type: MediaFrameType::Video,
            packet_count: 1000,
            byte_count: 1_200_000,
            packets_lost: 150, // 15% loss
            fraction_lost: 0.15,
            jitter_ms: 120.0,
            rtt_ms: Some(400.0),
            mos: Some(1.8),
            remote_addr,
            bitrate_bps: 500_000,
            discard_rate: 0.15,
            quality: QualityLevel::Poor,
            available_bandwidth_bps: Some(600_000),
        },
        _ => StreamStats {
            direction: Direction::Inbound,
            ssrc: 0x12345678,
            media_type: MediaFrameType::Video,
            packet_count: 0,
            byte_count: 0,
            packets_lost: 0,
            fraction_lost: 0.0,
            jitter_ms: 0.0,
            rtt_ms: None,
            mos: None,
            remote_addr,
            bitrate_bps: 0,
            discard_rate: 0.0,
            quality: QualityLevel::Unknown,
            available_bandwidth_bps: None,
        },
    }
}

/// Create transport feedback for GCC testing
fn create_transport_feedback() -> Vec<PacketFeedback> {
    let mut feedback = Vec::new();
    let base_time = 1000i64;
    
    // Simulate 20 packets with varying delays
    for i in 0..20 {
        let send_time = base_time + (i * 20); // 20ms intervals
        let arrival_delay = match i {
            0..=5 => 0,      // Good network
            6..=10 => i * 2, // Increasing delay
            11..=15 => 20,   // High but stable delay
            _ => 20 - (i - 15) * 5, // Improving conditions
        };
        let arrival_time = send_time + arrival_delay;
        
        feedback.push(PacketFeedback {
            sequence_number: i as u16,
            send_time_ms: send_time,
            arrival_time_ms: arrival_time,
            size_bytes: 1200, // Typical RTP packet size
        });
    }
    
    feedback
} 