//! Conference Audio Mixing Demo
//!
//! This example demonstrates the multi-party conference audio mixing capabilities
//! of the MediaSessionController, showing how to:
//! - Enable conference mixing
//! - Add participants to a conference
//! - Process real-time audio mixing
//! - Monitor conference statistics and events
//!
//! Run with: `cargo run --example conference_demo`

use rvoip_media_core::{
    MediaSessionController, MediaConfig,
    types::{AudioFrame, SampleRate, DialogId},
    types::conference::{ConferenceMixingConfig, MixingQuality, ConferenceMixingEvent},
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, interval};
use tracing::{info, warn, error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    info!("🎙️ Starting Conference Audio Mixing Demo");
    
    // Create MediaSessionController with conference mixing enabled
    let controller = create_conference_controller().await?;
    
    // Demonstrate conference functionality
    demo_conference_operations(&controller).await?;
    
    // Simulate conference call with multiple participants
    simulate_conference_call(&controller).await?;
    
    info!("✅ Conference audio mixing demo completed successfully");
    Ok(())
}

/// Create MediaSessionController with conference mixing enabled
async fn create_conference_controller() -> Result<Arc<MediaSessionController>, Box<dyn std::error::Error>> {
    info!("🔧 Creating MediaSessionController with conference mixing");
    
    // Configure conference mixing
    let conference_config = ConferenceMixingConfig {
        max_participants: 5,
        output_sample_rate: 8000, // 8kHz for telephony
        output_channels: 1, // Mono
        output_samples_per_frame: 160, // 20ms at 8kHz
        enable_voice_activity_mixing: true,
        enable_automatic_gain_control: true,
        enable_noise_reduction: false, // Disabled for demo performance
        enable_simd_optimization: true,
        max_concurrent_mixes: 3,
        mixing_quality: MixingQuality::Balanced,
        overflow_protection: true,
    };
    
    // Create controller with conference mixing
    let controller = MediaSessionController::with_conference_mixing(
        10000, // Base RTP port
        20000, // Max RTP port
        conference_config
    ).await?;
    
    info!("✅ MediaSessionController created with conference mixing enabled");
    Ok(Arc::new(controller))
}

/// Demonstrate basic conference operations
async fn demo_conference_operations(controller: &Arc<MediaSessionController>) -> Result<(), Box<dyn std::error::Error>> {
    info!("🎯 Demonstrating conference operations");
    
    // Verify conference mixing is enabled
    assert!(controller.is_conference_mixing_enabled());
    info!("✅ Conference mixing is enabled");
    
    // Create media sessions for participants
    let participants = vec!["alice", "bob", "charlie"];
    
    for participant in &participants {
        let dialog_id = format!("dialog_{}", participant);
        let config = create_media_config(participant).await?;
        
        // Start media session
        controller.start_media(DialogId::new(dialog_id.clone()), config).await?;
        info!("✅ Started media session for participant: {}", participant);
        
        // Add to conference
        controller.add_to_conference(&dialog_id).await?;
        info!("✅ Added {} to conference", participant);
    }
    
    // Check conference participants
    let conference_participants = controller.get_conference_participants().await?;
    info!("📋 Conference participants: {:?}", conference_participants);
    assert_eq!(conference_participants.len(), 3);
    
    // Get conference statistics
    let stats = controller.get_conference_stats().await?;
    info!("📊 Conference stats: {} active participants", stats.active_participants);
    
    // Remove one participant
    controller.remove_from_conference("dialog_bob").await?;
    info!("👋 Removed bob from conference");
    
    let updated_participants = controller.get_conference_participants().await?;
    info!("📋 Updated participants: {:?}", updated_participants);
    assert_eq!(updated_participants.len(), 2);
    
    info!("✅ Conference operations demonstration completed");
    Ok(())
}

/// Simulate a real conference call with audio processing
async fn simulate_conference_call(controller: &Arc<MediaSessionController>) -> Result<(), Box<dyn std::error::Error>> {
    info!("🎵 Simulating conference call with audio processing");
    
    // Set up event monitoring
    let event_receiver = controller.take_conference_event_receiver().await;
    if let Some(mut events) = event_receiver {
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    ConferenceMixingEvent::ParticipantAdded { participant_id, participant_count } => {
                        info!("🎤 Conference event: {} joined (total: {})", participant_id, participant_count);
                    },
                    ConferenceMixingEvent::ParticipantRemoved { participant_id, participant_count } => {
                        info!("👋 Conference event: {} left (total: {})", participant_id, participant_count);
                    },
                    ConferenceMixingEvent::VoiceActivityChanged { participant_id, is_talking } => {
                        info!("🗣️ Conference event: {} {}", participant_id, 
                              if is_talking { "started talking" } else { "stopped talking" });
                    },
                    ConferenceMixingEvent::QualityChanged { old_score, new_score, reason } => {
                        info!("📊 Conference event: Quality changed {:.2} -> {:.2} ({})", 
                              old_score, new_score, reason);
                    },
                    ConferenceMixingEvent::PerformanceWarning { latency_us, cpu_usage, reason } => {
                        warn!("⚠️ Conference event: Performance warning - {}μs latency, {:.1}% CPU ({})", 
                              latency_us, cpu_usage * 100.0, reason);
                    },
                }
            }
        });
    }
    
    // Generate and process audio frames for each participant
    let participants = vec!["dialog_alice", "dialog_charlie"]; // Bob was removed earlier
    let frame_duration = Duration::from_millis(20); // 20ms frames
    let mut interval = interval(frame_duration);
    
    for frame_count in 0..50 { // Simulate 1 second of audio (50 * 20ms)
        interval.tick().await;
        
        for (i, participant) in participants.iter().enumerate() {
            // Generate different frequency audio for each participant
            let frequency = 440.0 + (i as f64 * 110.0); // 440Hz, 550Hz, etc.
            let audio_frame = generate_test_audio_frame(frequency, frame_count)?;
            
            // Process audio through conference mixer
            if let Err(e) = controller.process_conference_audio(participant, audio_frame).await {
                error!("Failed to process audio for {}: {}", participant, e);
            }
        }
        
        // Get mixed audio for each participant (they hear everyone except themselves)
        for participant in &participants {
            if let Ok(Some(mixed_audio)) = controller.get_conference_mixed_audio(participant).await {
                // In a real application, this mixed audio would be sent via RTP
                let sample_count = mixed_audio.samples.len();
                if frame_count % 25 == 0 { // Log every 500ms
                    info!("🎵 Mixed audio for {}: {} samples", participant, sample_count);
                }
            }
        }
        
        // Log conference statistics periodically
        if frame_count % 25 == 0 {
            if let Ok(stats) = controller.get_conference_stats().await {
                info!("📊 Conference stats: {} participants, {:.2}μs avg latency, {:.1}% CPU", 
                      stats.active_participants, stats.avg_mixing_latency_us, stats.cpu_usage * 100.0);
            }
        }
    }
    
    // Clean up conference
    let cleanup_removed = controller.cleanup_conference_participants().await?;
    info!("🧹 Cleaned up participants: {:?}", cleanup_removed);
    
    info!("✅ Conference call simulation completed");
    Ok(())
}

/// Create media configuration for a participant
async fn create_media_config(participant: &str) -> Result<MediaConfig, Box<dyn std::error::Error>> {
    // Generate unique port for each participant (in a real app, this would be allocated)
    let base_port = 5000u16;
    let participant_port = base_port + (participant.len() as u16 * 10);
    
    Ok(MediaConfig {
        local_addr: SocketAddr::from(([127, 0, 0, 1], 0)), // Let system allocate
        remote_addr: Some(SocketAddr::from(([127, 0, 0, 1], participant_port))),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    })
}

/// Generate a test audio frame with a specific frequency
fn generate_test_audio_frame(frequency: f64, frame_number: u32) -> Result<AudioFrame, Box<dyn std::error::Error>> {
    let sample_rate = 8000u32;
    let channels = 1u8;
    let samples_per_frame = 160; // 20ms at 8kHz
    let amplitude = 8000.0; // Moderate amplitude
    
    let mut samples = Vec::with_capacity(samples_per_frame);
    
    for i in 0..samples_per_frame {
        let t = (frame_number * samples_per_frame as u32 + i as u32) as f64 / sample_rate as f64;
        let sample_value = (amplitude * (2.0 * std::f64::consts::PI * frequency * t).sin()) as i16;
        samples.push(sample_value);
    }
    
    let timestamp = frame_number * samples_per_frame as u32;
    Ok(AudioFrame::new(samples, sample_rate, channels, timestamp))
}

/// Demonstration of advanced conference features
#[allow(dead_code)]
async fn demo_advanced_features(controller: &Arc<MediaSessionController>) -> Result<(), Box<dyn std::error::Error>> {
    info!("🚀 Demonstrating advanced conference features");
    
    // Test conference capacity limits
    info!("🔢 Testing conference capacity limits");
    for i in 0..7 { // Try to add more than max_participants (5)
        let dialog_id = format!("overflow_dialog_{}", i);
        let config = create_media_config(&format!("overflow_{}", i)).await?;
        
        if let Err(e) = controller.start_media(DialogId::new(dialog_id.clone()), config).await {
            warn!("Failed to start session {}: {}", i, e);
            continue;
        }
        
        match controller.add_to_conference(&dialog_id).await {
            Ok(_) => info!("✅ Added participant {} to conference", i),
            Err(e) => {
                warn!("❌ Could not add participant {} (expected): {}", i, e);
                // Clean up the session that couldn't be added to conference
                let _ = controller.stop_media(&DialogId::new(dialog_id)).await;
            }
        }
    }
    
    let final_participants = controller.get_conference_participants().await?;
    info!("📋 Final participant count: {} (max: 5)", final_participants.len());
    
    info!("✅ Advanced features demonstration completed");
    Ok(())
} 