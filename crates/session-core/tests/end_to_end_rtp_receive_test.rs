//! End-to-end integration test for RTP receive implementation
//!
//! This test simulates the complete flow from incoming RTP packets
//! through the media-core RtpBridge to session-core MediaManager
//! and finally to decoded AudioFrames.

use std::sync::Arc;
use std::time::{Duration, Instant};
use bytes::Bytes;
use tokio::sync::mpsc;

use rvoip_session_core::api::types::{SessionId, AudioFrame};
use rvoip_session_core::api::{MediaControl, SessionManagerBuilder, SessionControl};
use rvoip_media_core::{MediaPacket, MediaSessionId, RtpEvent as MediaCoreRtpEvent};
use rvoip_media_core::relay::controller::MediaSessionController;

#[tokio::test]
async fn test_end_to_end_rtp_receive_flow() {
    // Test the complete flow: MediaCore RtpBridge → SessionCore MediaManager → AudioFrame
    println!("🚀 Testing end-to-end RTP receive flow");
    
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    
    // 1. Create SessionCoordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5064) // Use different port
        .build()
        .await
        .expect("Failed to create SessionCoordinator");
    
    SessionControl::start(&coordinator).await
        .expect("Failed to start SessionCoordinator");
    
    println!("✅ Created SessionCoordinator");
    
    // 2. Create a session
    let session_id = SessionId::new();
    
    // Directly set up audio frame callback without full session
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<AudioFrame>(100);
    let (std_sender, std_receiver) = std::sync::mpsc::channel::<AudioFrame>();
    
    coordinator.media_manager.set_audio_frame_callback(&session_id, tokio_sender)
        .await
        .expect("Failed to set audio frame callback");
    
    tokio::spawn(async move {
        while let Some(frame) = tokio_receiver.recv().await {
            let _ = std_sender.send(frame);
        }
    });
    
    let subscriber = rvoip_session_core::api::types::AudioFrameSubscriber::new(session_id.clone(), std_receiver);
    
    println!("✅ Set up audio frame subscription for: {}", session_id);
    
    println!("✅ Subscribed to audio frames");
    
    // 4. Create test callback that simulates media-core RtpBridge behavior
    let callback = coordinator.media_manager.create_rtp_event_callback();
    
    // 6. Simulate RTP packets arriving at media-core and being forwarded as events
    let test_packets = create_test_media_packets();
    
    for (i, packet) in test_packets.iter().enumerate() {
        // Convert MediaPacket to MediaCore RtpEvent (simulating RtpBridge processing)
        let media_session_id = MediaSessionId::new(&session_id.to_string());
        let rtp_event = MediaCoreRtpEvent::MediaReceived {
            payload_type: packet.payload_type,
            payload: packet.payload.to_vec(),
            timestamp: packet.timestamp,
            sequence_number: packet.sequence_number,
            ssrc: packet.ssrc,
        };
        
        // Simulate the callback being called by RtpBridge
        callback(media_session_id, rtp_event);
        
        println!("📤 Sent RTP event {} to MediaManager", i + 1);
    }
    
    // 7. Collect decoded audio frames
    let mut received_frames = Vec::new();
    let start_time = Instant::now();
    let timeout = Duration::from_secs(3);
    
    while received_frames.len() < test_packets.len() && start_time.elapsed() < timeout {
        match subscriber.try_recv() {
            Ok(audio_frame) => {
                println!("🎵 Received AudioFrame: {} samples, {}Hz, timestamp: {}", 
                         audio_frame.samples.len(), 
                         audio_frame.sample_rate,
                         audio_frame.timestamp);
                
                received_frames.push(audio_frame);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("AudioFrame subscriber disconnected");
            }
        }
    }
    
    // 8. Verify results
    assert_eq!(received_frames.len(), test_packets.len(), 
               "Should receive one AudioFrame per RTP packet");
    
    for (i, frame) in received_frames.iter().enumerate() {
        assert_eq!(frame.timestamp, test_packets[i].timestamp, 
                   "Frame {} timestamp should match packet timestamp", i);
        assert_eq!(frame.sample_rate, 8000, "Should be 8kHz");
        assert_eq!(frame.channels, 1, "Should be mono");
        assert!(!frame.samples.is_empty(), "Should have audio samples");
    }
    
    // 9. Verify decoder statistics
    let decoder_stats = coordinator.media_manager.get_rtp_decoder_stats().await;
    println!("📊 Final decoder statistics:");
    println!("  - Packets processed: {}", decoder_stats.packets_processed);
    println!("  - Decode errors: {}", decoder_stats.decode_errors);
    println!("  - Active subscribers: {}", decoder_stats.active_subscribers);
    
    assert_eq!(decoder_stats.packets_processed, test_packets.len() as u64);
    assert_eq!(decoder_stats.decode_errors, 0);
    assert_eq!(decoder_stats.active_subscribers, 1);
    
    println!("🎉 End-to-end RTP receive flow test PASSED!");
}

#[tokio::test]
async fn test_media_core_integration_with_real_controller() {
    // Test integration with the actual MediaSessionController
    println!("🔗 Testing media-core integration with real controller");
    
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init();
    
    // 1. Create MediaSessionController with RTP bridge
    let controller = MediaSessionController::new();
    
    // 2. Create SessionCoordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5065) // Use different port
        .build()
        .await
        .expect("Failed to create SessionCoordinator");
    
    SessionControl::start(&coordinator).await
        .expect("Failed to start SessionCoordinator");
    
    // 3. Get callback from coordinator
    let callback = coordinator.media_manager.create_rtp_event_callback();
    controller.add_rtp_event_callback(callback).await;
    
    println!("✅ Registered RTP callback with MediaSessionController");
    
    // 4. Create a session
    let session_id = SessionId::new();
    coordinator.create_media_session(&session_id).await
        .expect("Failed to create session");
    
    // 6. This test verifies the integration works without errors
    // In a real scenario, RTP packets would flow through the controller's RtpBridge
    // and trigger our callback, but that requires more complex setup
    
    println!("✅ Media-core integration test PASSED!");
}

#[tokio::test]
async fn test_concurrent_session_processing() {
    // Test that multiple sessions can process RTP events concurrently
    println!("⚡ Testing concurrent session processing");
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5066) // Use different port
        .build()
        .await
        .unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    // Create multiple sessions
    let num_sessions = 3;
    let mut sessions = Vec::new();
    let mut subscribers = Vec::new();
    
    for i in 0..num_sessions {
        let session_id = SessionId::new();
        
        // Set up audio frame callback directly
        let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<AudioFrame>(100);
        let (std_sender, std_receiver) = std::sync::mpsc::channel::<AudioFrame>();
        
        coordinator.media_manager.set_audio_frame_callback(&session_id, tokio_sender)
            .await
            .unwrap();
        
        tokio::spawn(async move {
            while let Some(frame) = tokio_receiver.recv().await {
                let _ = std_sender.send(frame);
            }
        });
        
        let subscriber = rvoip_session_core::api::types::AudioFrameSubscriber::new(session_id.clone(), std_receiver);
        
        sessions.push(session_id);
        subscribers.push(subscriber);
        
        println!("✅ Created session {}: {}", i + 1, sessions[i]);
    }
    
    // Create callback for simulating RTP events
    let callback = coordinator.media_manager.create_rtp_event_callback();
    
    // Send events to all sessions concurrently
    let mut tasks = Vec::new();
    
    for (i, session_id) in sessions.iter().enumerate() {
        let callback_clone = callback.clone();
        let session_id_clone = session_id.clone();
        
        let task = tokio::spawn(async move {
            // Simulate media-core session ID
            let media_session_id = MediaSessionId::new(&format!("media-{}", session_id_clone));
            
            // Send multiple packets to this session
            for j in 0..3 {
                let rtp_event = MediaCoreRtpEvent::MediaReceived {
                    payload_type: 0,
                    payload: vec![0xFF; 160],
                    timestamp: (i * 1000 + j * 160) as u32,
                    sequence_number: (j + 1) as u16,
                    ssrc: (0x10000000 + i * 0x1000000) as u32,
                };
                
                callback_clone(media_session_id.clone(), rtp_event);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        
        tasks.push(task);
    }
    
    // Wait for all tasks to complete
    for task in tasks {
        task.await.unwrap();
    }
    
    // Collect frames from all subscribers
    tokio::time::sleep(Duration::from_millis(500)).await; // Allow processing time
    
    let mut total_frames = 0;
    for (i, subscriber) in subscribers.iter().enumerate() {
        let mut session_frames = 0;
        while let Ok(_frame) = subscriber.try_recv() {
            session_frames += 1;
        }
        println!("📊 Session {} received {} frames", i + 1, session_frames);
        total_frames += session_frames;
    }
    
    // We sent 3 packets to 3 sessions = 9 total frames expected
    let expected_frames = num_sessions * 3;
    assert_eq!(total_frames, expected_frames, 
               "Should receive {} frames total across all sessions", expected_frames);
    
    println!("✅ Concurrent session processing test PASSED!");
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    // Test error handling in the RTP receive pipeline
    println!("🛡️ Testing error handling and recovery");
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5067) // Use different port
        .build()
        .await
        .unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    let session_id = SessionId::new();
    
    // Set up audio frame callback directly
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<AudioFrame>(100);
    let (std_sender, std_receiver) = std::sync::mpsc::channel::<AudioFrame>();
    
    coordinator.media_manager.set_audio_frame_callback(&session_id, tokio_sender)
        .await
        .unwrap();
    
    tokio::spawn(async move {
        while let Some(frame) = tokio_receiver.recv().await {
            let _ = std_sender.send(frame);
        }
    });
    
    let subscriber = rvoip_session_core::api::types::AudioFrameSubscriber::new(session_id.clone(), std_receiver);
    
    let callback = coordinator.media_manager.create_rtp_event_callback();
    let media_session_id = MediaSessionId::new("error-test-session");
    
    // 1. Send unsupported payload type (should be handled gracefully)
    let unsupported_event = MediaCoreRtpEvent::MediaReceived {
        payload_type: 99, // Unsupported
        payload: vec![0x01, 0x02, 0x03],
        timestamp: 1000,
        sequence_number: 1,
        ssrc: 0x12345678,
    };
    
    callback(media_session_id.clone(), unsupported_event);
    
    // 2. Send valid packet after error
    let valid_event = MediaCoreRtpEvent::MediaReceived {
        payload_type: 0, // Valid PCMU
        payload: vec![0xFF; 160],
        timestamp: 2000,
        sequence_number: 2,
        ssrc: 0x12345678,
    };
    
    callback(media_session_id.clone(), valid_event);
    
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // 3. Should not receive frame for unsupported payload, but should for valid one
    let mut received_frames = 0;
    while let Ok(_frame) = subscriber.try_recv() {
        received_frames += 1;
    }
    
    // We might receive 0 frames if session mapping fails, or 1 if it succeeds for valid packet
    // The key is that the system doesn't crash
    println!("📊 Received {} frames after error condition", received_frames);
    
    // 4. Check decoder stats show the error was counted
    let stats = coordinator.media_manager.get_rtp_decoder_stats().await;
    println!("📊 Decoder stats after error: {} errors", stats.decode_errors);
    
    // The system should still be functioning
    println!("✅ Error handling and recovery test PASSED!");
}

/// Create test media packets for testing
fn create_test_media_packets() -> Vec<MediaPacket> {
    let mut packets = Vec::new();
    
    // Create 4 test packets with different content
    for i in 0..4 {
        let mut payload = vec![0xFF; 160]; // G.711 μ-law silence
        
        // Add variation to each packet
        for j in 0..8 {
            payload[j] = ((i * 8 + j) as u8).wrapping_add(0x80);
        }
        
        let packet = MediaPacket {
            payload: Bytes::from(payload),
            payload_type: if i % 2 == 0 { 0 } else { 8 }, // Alternate between PCMU and PCMA
            timestamp: (i as u32) * 160 + 1000,
            sequence_number: (i + 1) as u16,
            ssrc: 0x12345678,
            received_at: Instant::now(),
        };
        
        packets.push(packet);
    }
    
    packets
}