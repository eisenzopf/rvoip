//! Conference Audio Mixing Integration Tests
//!
//! Tests the integration between AudioMixer and MediaSessionController for
//! multi-party conference functionality (Phase 5.2).

use rvoip_media_core::{
    MediaSessionController, MediaConfig,
    types::{AudioFrame, SampleRate, DialogId},
    types::conference::{ConferenceMixingConfig, MixingQuality, ConferenceMixingEvent},
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use serial_test::serial;

/// Test basic conference setup and teardown
#[tokio::test]
#[serial]
async fn test_conference_setup_teardown() {
    let config = ConferenceMixingConfig {
        max_participants: 3,
        output_sample_rate: 8000,
        output_channels: 1,
        output_samples_per_frame: 160,
        enable_voice_activity_mixing: false, // Disabled for simple test
        enable_automatic_gain_control: false,
        enable_noise_reduction: false,
        enable_simd_optimization: false,
        max_concurrent_mixes: 2,
        mixing_quality: MixingQuality::Fast,
        overflow_protection: true,
    };
    
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller with conference mixing");
    
    // Verify conference mixing is enabled
    assert!(controller.is_conference_mixing_enabled());
    
    // Initially no participants
    let participants = controller.get_conference_participants().await.unwrap();
    assert_eq!(participants.len(), 0);
    
    // Stats should show 0 participants
    let stats = controller.get_conference_stats().await.unwrap();
    assert_eq!(stats.active_participants, 0);
}

/// Test adding and removing participants from conference
#[tokio::test]
#[serial]
async fn test_participant_management() {
    let config = ConferenceMixingConfig::default();
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller");
    
    // Create media sessions for three participants
    let participants = vec!["alice", "bob", "charlie"];
    
    for participant in &participants {
        let dialog_id = format!("dialog_{}", participant);
        let media_config = create_test_media_config();
        
        // Start media session
        controller.start_media(DialogId::new(dialog_id.clone()), media_config).await
            .expect("Failed to start media session");
        
        // Add to conference
        controller.add_to_conference(&dialog_id).await
            .expect("Failed to add to conference");
    }
    
    // Verify all participants are in conference
    let conference_participants = controller.get_conference_participants().await.unwrap();
    assert_eq!(conference_participants.len(), 3);
    
    // Verify stats reflect correct participant count
    let stats = controller.get_conference_stats().await.unwrap();
    assert_eq!(stats.active_participants, 3);
    
    // Remove one participant
    controller.remove_from_conference("dialog_bob").await
        .expect("Failed to remove from conference");
    
    // Verify participant count decreased
    let updated_participants = controller.get_conference_participants().await.unwrap();
    assert_eq!(updated_participants.len(), 2);
    assert!(!updated_participants.contains(&"dialog_bob".to_string()));
    
    let updated_stats = controller.get_conference_stats().await.unwrap();
    assert_eq!(updated_stats.active_participants, 2);
}

/// Test conference capacity limits
#[tokio::test]
#[serial]
async fn test_conference_capacity_limits() {
    let config = ConferenceMixingConfig {
        max_participants: 2, // Small limit for testing
        ..ConferenceMixingConfig::default()
    };
    
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller");
    
    // Add participants up to the limit
    for i in 0..2 {
        let dialog_id = format!("dialog_{}", i);
        let media_config = create_test_media_config();
        
        controller.start_media(DialogId::new(dialog_id.clone()), media_config).await
            .expect("Failed to start media session");
        
        controller.add_to_conference(&dialog_id).await
            .expect("Failed to add to conference");
    }
    
    // Verify we're at capacity
    let participants = controller.get_conference_participants().await.unwrap();
    assert_eq!(participants.len(), 2);
    
    // Try to add one more participant (should fail)
    let overflow_dialog = "dialog_overflow".to_string();
    let media_config = create_test_media_config();
    
    controller.start_media(DialogId::new(overflow_dialog.clone()), media_config).await
        .expect("Failed to start media session");
    
    let result = controller.add_to_conference(&overflow_dialog).await;
    assert!(result.is_err(), "Should fail to add participant beyond capacity");
    
    // Verify participant count unchanged
    let final_participants = controller.get_conference_participants().await.unwrap();
    assert_eq!(final_participants.len(), 2);
}

/// Test conference audio processing
#[tokio::test]
#[serial]
async fn test_conference_audio_processing() {
    let config = ConferenceMixingConfig {
        max_participants: 3,
        enable_voice_activity_mixing: false, // Simplify for testing
        enable_automatic_gain_control: false,
        enable_noise_reduction: false,
        mixing_quality: MixingQuality::Fast,
        ..ConferenceMixingConfig::default()
    };
    
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller");
    
    // Set up two participants
    let participants = vec!["alice", "bob"];
    
    for participant in &participants {
        let dialog_id = format!("dialog_{}", participant);
        let media_config = create_test_media_config();
        
        controller.start_media(DialogId::new(dialog_id.clone()), media_config).await
            .expect("Failed to start media session");
        
        controller.add_to_conference(&dialog_id).await
            .expect("Failed to add to conference");
    }
    
    // Generate and process audio frames
    for frame_num in 0..5 {
        for (i, participant) in participants.iter().enumerate() {
            let dialog_id = format!("dialog_{}", participant);
            let frequency = 440.0 + (i as f64 * 110.0); // Different frequencies
            let audio_frame = generate_test_audio_frame(frequency, frame_num);
            
            // Process audio through conference mixer
            controller.process_conference_audio(&dialog_id, audio_frame).await
                .expect("Failed to process conference audio");
        }
        
        // Get mixed audio for each participant
        for participant in &participants {
            let dialog_id = format!("dialog_{}", participant);
            let mixed_audio = controller.get_conference_mixed_audio(&dialog_id).await
                .expect("Failed to get mixed audio");
            
            // Should have mixed audio since other participants are talking
            // Note: In a real test, we'd verify the audio content
            if mixed_audio.is_some() {
                let frame = mixed_audio.unwrap();
                assert_eq!(frame.samples.len(), 160); // 20ms at 8kHz
                assert_eq!(frame.sample_rate, 8000);
                assert_eq!(frame.channels, 1);
            }
        }
    }
    
    // Verify conference statistics
    let stats = controller.get_conference_stats().await.unwrap();
    assert_eq!(stats.active_participants, 2);
    assert!(stats.total_mixes > 0, "Should have performed mixing operations");
}

/// Test conference event monitoring
#[tokio::test]
#[serial]
async fn test_conference_events() {
    let config = ConferenceMixingConfig::default();
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller");
    
    // Set up event monitoring
    let event_receiver = controller.take_conference_event_receiver().await;
    assert!(event_receiver.is_some(), "Should have event receiver");
    
    let mut events = event_receiver.unwrap();
    
    // Prepare media session first
    let dialog_id = "dialog_test".to_string();
    let media_config = create_test_media_config();
    
    controller.start_media(DialogId::new(dialog_id.clone()), media_config).await
        .expect("Failed to start media session");
    
    // Spawn task to collect events AND perform operations
    let dialog_id_for_task = dialog_id.clone();
    let controller_for_task = controller;
    let event_collector = tokio::spawn(async move {
        let mut collected_events = Vec::new();
        
        // Perform operations within the collector task
        controller_for_task.add_to_conference(&dialog_id_for_task).await
            .expect("Failed to add to conference");
        
        // Give some time for event to be sent
        sleep(Duration::from_millis(100)).await;
        
        controller_for_task.remove_from_conference(&dialog_id_for_task).await
            .expect("Failed to remove from conference");
        
        // Give some time for event to be sent
        sleep(Duration::from_millis(100)).await;
        
        // Now collect events with a timeout
        let collection_timeout = timeout(Duration::from_millis(500), async {
            while let Some(event) = events.recv().await {
                collected_events.push(event);
                if collected_events.len() >= 2 { // Expect 2 events (add + remove)
                    break;
                }
            }
        });
        
        let _ = collection_timeout.await;
        collected_events
    });
    
    // Collect and verify events
    let collected_events = event_collector.await.expect("Event collector failed");
    
    // Should have at least one event (participant added)
    assert!(!collected_events.is_empty(), "Should have received events");
    
    // Check if we have the expected event types
    let has_participant_added = collected_events.iter().any(|event| {
        matches!(event, ConferenceMixingEvent::ParticipantAdded { .. })
    });
    
    assert!(has_participant_added, "Should have received ParticipantAdded event");
}

/// Test error handling for invalid operations
#[tokio::test]
#[serial]
async fn test_error_handling() {
    let config = ConferenceMixingConfig::default();
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller");
    
    // Try to add non-existent dialog to conference
    let result = controller.add_to_conference("nonexistent_dialog").await;
    assert!(result.is_err(), "Should fail for non-existent dialog");
    
    // Try to remove non-existent participant
    let result = controller.remove_from_conference("nonexistent_dialog").await;
    assert!(result.is_err(), "Should fail for non-existent participant");
    
    // Try to process audio for non-existent participant
    let audio_frame = generate_test_audio_frame(440.0, 0);
    let result = controller.process_conference_audio("nonexistent_dialog", audio_frame).await;
    assert!(result.is_err(), "Should fail for non-existent participant");
    
    // Try to get mixed audio for non-existent participant
    let result = controller.get_conference_mixed_audio("nonexistent_dialog").await;
    assert!(result.is_err(), "Should fail for non-existent participant");
}

/// Test cleanup of inactive participants
#[tokio::test]
#[serial]
async fn test_participant_cleanup() {
    let config = ConferenceMixingConfig {
        max_participants: 5,
        ..ConferenceMixingConfig::default()
    };
    
    let controller = MediaSessionController::with_conference_mixing(0, 0, config)
        .await
        .expect("Failed to create controller");
    
    // Add some participants
    let participants = vec!["alice", "bob"];
    
    for participant in &participants {
        let dialog_id = format!("dialog_{}", participant);
        let media_config = create_test_media_config();
        
        controller.start_media(DialogId::new(dialog_id.clone()), media_config).await
            .expect("Failed to start media session");
        
        controller.add_to_conference(&dialog_id).await
            .expect("Failed to add to conference");
    }
    
    // Verify participants are active
    let active_participants = controller.get_conference_participants().await.unwrap();
    assert_eq!(active_participants.len(), 2);
    
    // Run cleanup (should not remove active participants)
    let removed = controller.cleanup_conference_participants().await.unwrap();
    assert_eq!(removed.len(), 0, "Should not remove active participants");
    
    // Verify participants still active
    let still_active = controller.get_conference_participants().await.unwrap();
    assert_eq!(still_active.len(), 2);
}

// Helper functions

fn create_test_media_config() -> MediaConfig {
    MediaConfig {
        local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        remote_addr: Some(SocketAddr::from(([127, 0, 0, 1], 5004))),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    }
}

fn generate_test_audio_frame(frequency: f64, frame_number: u32) -> AudioFrame {
    let sample_rate = 8000u32;
    let channels = 1u8;
    let samples_per_frame = 160; // 20ms at 8kHz
    let amplitude = 8000.0;
    
    let mut samples = Vec::with_capacity(samples_per_frame);
    
    for i in 0..samples_per_frame {
        let t = (frame_number * samples_per_frame as u32 + i as u32) as f64 / sample_rate as f64;
        let sample_value = (amplitude * (2.0 * std::f64::consts::PI * frequency * t).sin()) as i16;
        samples.push(sample_value);
    }
    
    let timestamp = frame_number * samples_per_frame as u32;
    AudioFrame::new(samples, sample_rate, channels, timestamp)
} 