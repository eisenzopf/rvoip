//! Integration test for RTP receive path implementation
//!
//! This test verifies the complete RTP receive pipeline from RTP packets
//! to decoded AudioFrames that can be played on speakers.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use bytes::Bytes;

use rvoip_session_core::api::types::{SessionId, AudioFrame, AudioFrameSubscriber};
use rvoip_session_core::api::{MediaControl, SessionManagerBuilder, SessionControl};
use rvoip_session_core::media::rtp_decoder::{RtpEvent, RtpPayloadDecoder};
use rvoip_media_core::{MediaPacket, MediaSessionId};

#[tokio::test]
async fn test_complete_rtp_receive_pipeline() {
    // Test the complete pipeline: RTP packet → RtpEvent → AudioFrame → AudioFrameSubscriber
    
    // Initialize tracing for test debugging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    
    println!("🧪 Testing complete RTP receive pipeline");
    
    // 1. Create SessionCoordinator using builder pattern
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060) // Use default SIP port for testing
        .build()
        .await
        .expect("Failed to create SessionCoordinator");
    
    // Start the coordinator
    SessionControl::start(&coordinator).await
        .expect("Failed to start SessionCoordinator");
    
    // 2. Create a real media session through the coordinator
    let session_id = SessionId::new();
    let dialog_id = rvoip_media_core::types::DialogId::new(&format!("test-{}", session_id));
    
    // Create the media session in MediaManager
    coordinator.media_manager.create_media_session(&session_id)
        .await
        .expect("Failed to create media session");
    
    println!("✅ Created media session: {}", session_id);
    
    // 3. Set up audio frame subscription
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<AudioFrame>(100);
    let (std_sender, std_receiver) = std::sync::mpsc::channel::<AudioFrame>();
    
    // Set up the callback with MediaManager - this registers the session with the decoder
    coordinator.media_manager.set_audio_frame_callback(&session_id, tokio_sender)
        .await
        .expect("Failed to set audio frame callback");
    
    // Spawn task to forward frames from tokio to std channel
    tokio::spawn(async move {
        while let Some(frame) = tokio_receiver.recv().await {
            let _ = std_sender.send(frame);
        }
    });
    
    let subscriber = rvoip_session_core::api::types::AudioFrameSubscriber::new(session_id.clone(), std_receiver);
    
    println!("✅ Set up audio frame subscription for session: {}", session_id);
    
    // 4. Create test RTP packets with G.711 μ-law payload
    let test_packets = create_test_g711_packets();
    
    // 5. Get the RTP event callback from media manager
    let rtp_callback = coordinator.media_manager.create_rtp_event_callback();
    
    // 6. Process each test packet through the callback (simulating RtpBridge behavior)
    for (i, packet) in test_packets.iter().enumerate() {
        // Create MediaCore RtpEvent
        let media_session_id = MediaSessionId::new(&format!("media-{}", session_id));
        let rtp_event = rvoip_media_core::integration::RtpEvent::MediaReceived {
            payload_type: packet.payload_type,
            payload: packet.payload.to_vec(),
            timestamp: packet.timestamp,
            sequence_number: packet.sequence_number,
            ssrc: packet.ssrc,
        };
        
        // Send through the callback (this is what RtpBridge would do)
        rtp_callback(media_session_id, rtp_event);
        
        println!("✅ Sent RTP packet {} through callback with {} bytes", i + 1, packet.payload.len());
    }
    
    // 7. Receive decoded audio frames
    let mut received_frames = Vec::new();
    let start_time = Instant::now();
    let timeout = Duration::from_secs(2);
    
    while received_frames.len() < test_packets.len() && start_time.elapsed() < timeout {
        match subscriber.try_recv() {
            Ok(audio_frame) => {
                println!("🎵 Received AudioFrame: {} samples, {} Hz, {} channels, timestamp: {}", 
                         audio_frame.samples.len(), 
                         audio_frame.sample_rate, 
                         audio_frame.channels,
                         audio_frame.timestamp);
                
                // Verify frame properties
                assert_eq!(audio_frame.sample_rate, 8000, "Sample rate should be 8kHz for G.711");
                assert_eq!(audio_frame.channels, 1, "G.711 should be mono");
                assert!(!audio_frame.samples.is_empty(), "AudioFrame should have samples");
                
                received_frames.push(audio_frame);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("AudioFrame channel disconnected unexpectedly");
            }
        }
    }
    
    // 8. Verify we received all expected frames
    assert_eq!(received_frames.len(), test_packets.len(), 
               "Should receive one AudioFrame per RTP packet");
    
    // 9. Verify frame timestamps match RTP timestamps
    for (i, frame) in received_frames.iter().enumerate() {
        assert_eq!(frame.timestamp, test_packets[i].timestamp, 
                   "AudioFrame timestamp should match RTP timestamp");
    }
    
    // 10. Get decoder statistics
    let stats = coordinator.media_manager.get_rtp_decoder_stats().await;
    
    println!("📊 RTP Decoder Statistics:");
    println!("  - Packets processed: {}", stats.packets_processed);
    println!("  - Decode errors: {}", stats.decode_errors);
    println!("  - Active subscribers: {}", stats.active_subscribers);
    
    assert_eq!(stats.packets_processed, test_packets.len() as u64);
    assert_eq!(stats.decode_errors, 0);
    assert_eq!(stats.active_subscribers, 1);
    
    println!("🎉 Complete RTP receive pipeline test PASSED!");
}

#[tokio::test]
async fn test_rtp_payload_decoder_g711_ulaw() {
    // Test G.711 μ-law decoding specifically
    println!("🧪 Testing G.711 μ-law RTP payload decoding");
    
    let mut decoder = RtpPayloadDecoder::new();
    let (sender, mut receiver) = mpsc::channel(10);
    let session_id = SessionId::new();
    
    // Register subscriber
    decoder.add_subscriber(session_id.clone(), sender);
    
    // Create test μ-law payload (silence = 0xFF in μ-law)
    let silence_payload = vec![0xFF; 160]; // 20ms of silence at 8kHz
    let rtp_event = RtpEvent::MediaReceived {
        payload_type: 0, // PCMU
        payload: silence_payload.clone(),
        timestamp: 12345,
        sequence_number: 1,
        ssrc: 0x12345678,
    };
    
    // Process the event
    decoder.process_rtp_event(rtp_event, &session_id)
        .await
        .expect("Failed to process μ-law RTP event");
    
    // Receive the decoded frame
    let audio_frame = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("Timeout waiting for audio frame")
        .expect("Failed to receive audio frame");
    
    // Verify decoded frame
    assert_eq!(audio_frame.samples.len(), 160, "Should decode to 160 samples");
    assert_eq!(audio_frame.sample_rate, 8000, "Sample rate should be 8kHz");
    assert_eq!(audio_frame.channels, 1, "Should be mono");
    assert_eq!(audio_frame.timestamp, 12345, "Timestamp should match");
    
    // Verify silence decoding (μ-law silence should decode to approximately 0)
    for sample in &audio_frame.samples[..10] { // Check first 10 samples
        assert!(sample.abs() < 100, "Silence should decode to near-zero values, got: {}", sample);
    }
    
    println!("✅ G.711 μ-law decoding test PASSED!");
}

#[tokio::test]
async fn test_rtp_payload_decoder_g711_alaw() {
    // Test G.711 A-law decoding specifically
    println!("🧪 Testing G.711 A-law RTP payload decoding");
    
    let mut decoder = RtpPayloadDecoder::new();
    let (sender, mut receiver) = mpsc::channel(10);
    let session_id = SessionId::new();
    
    // Register subscriber
    decoder.add_subscriber(session_id.clone(), sender);
    
    // Create test A-law payload (silence = 0xD5 in A-law)
    let silence_payload = vec![0xD5; 160]; // 20ms of silence at 8kHz
    let rtp_event = RtpEvent::MediaReceived {
        payload_type: 8, // PCMA
        payload: silence_payload.clone(),
        timestamp: 67890,
        sequence_number: 2,
        ssrc: 0x87654321,
    };
    
    // Process the event
    decoder.process_rtp_event(rtp_event, &session_id)
        .await
        .expect("Failed to process A-law RTP event");
    
    // Receive the decoded frame
    let audio_frame = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("Timeout waiting for audio frame")
        .expect("Failed to receive audio frame");
    
    // Verify decoded frame
    assert_eq!(audio_frame.samples.len(), 160, "Should decode to 160 samples");
    assert_eq!(audio_frame.sample_rate, 8000, "Sample rate should be 8kHz");
    assert_eq!(audio_frame.channels, 1, "Should be mono");
    assert_eq!(audio_frame.timestamp, 67890, "Timestamp should match");
    
    // Verify silence decoding (A-law silence should decode to approximately 0)
    for sample in &audio_frame.samples[..10] { // Check first 10 samples
        assert!(sample.abs() < 100, "Silence should decode to near-zero values, got: {}", sample);
    }
    
    println!("✅ G.711 A-law decoding test PASSED!");
}

#[tokio::test]
async fn test_multiple_sessions_isolation() {
    // Test that multiple sessions don't interfere with each other
    println!("🧪 Testing multiple session isolation");
    
    let mut decoder = RtpPayloadDecoder::new();
    
    // Create two sessions with their own subscribers
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    
    let (sender1, mut receiver1) = mpsc::channel(10);
    let (sender2, mut receiver2) = mpsc::channel(10);
    
    decoder.add_subscriber(session1.clone(), sender1);
    decoder.add_subscriber(session2.clone(), sender2);
    
    // Send events to different sessions
    let event1 = RtpEvent::MediaReceived {
        payload_type: 0,
        payload: vec![0xFF; 80], // Shorter frame for session 1
        timestamp: 1000,
        sequence_number: 1,
        ssrc: 0x11111111,
    };
    
    let event2 = RtpEvent::MediaReceived {
        payload_type: 8,
        payload: vec![0xD5; 160], // Different codec and size for session 2
        timestamp: 2000,
        sequence_number: 1,
        ssrc: 0x22222222,
    };
    
    // Process events
    decoder.process_rtp_event(event1, &session1).await.unwrap();
    decoder.process_rtp_event(event2, &session2).await.unwrap();
    
    // Verify each session receives only its own frames
    let frame1 = tokio::time::timeout(Duration::from_secs(1), receiver1.recv())
        .await.unwrap().unwrap();
    let frame2 = tokio::time::timeout(Duration::from_secs(1), receiver2.recv())
        .await.unwrap().unwrap();
    
    assert_eq!(frame1.samples.len(), 80, "Session 1 should get 80 samples");
    assert_eq!(frame1.timestamp, 1000, "Session 1 timestamp should match");
    
    assert_eq!(frame2.samples.len(), 160, "Session 2 should get 160 samples");
    assert_eq!(frame2.timestamp, 2000, "Session 2 timestamp should match");
    
    // Verify no cross-contamination
    assert!(receiver1.try_recv().is_err(), "Session 1 should not receive session 2's frames");
    assert!(receiver2.try_recv().is_err(), "Session 2 should not receive session 1's frames");
    
    println!("✅ Multiple session isolation test PASSED!");
}

#[tokio::test]
async fn test_unsupported_payload_types() {
    // Test that unsupported payload types are handled gracefully
    println!("🧪 Testing unsupported payload type handling");
    
    let mut decoder = RtpPayloadDecoder::new();
    let (sender, mut receiver) = mpsc::channel(10);
    let session_id = SessionId::new();
    
    decoder.add_subscriber(session_id.clone(), sender);
    
    // Try to process unsupported payload type
    let unsupported_event = RtpEvent::MediaReceived {
        payload_type: 99, // Unsupported dynamic payload type
        payload: vec![0x00; 160],
        timestamp: 3000,
        sequence_number: 1,
        ssrc: 0x33333333,
    };
    
    // This should return an error
    let result = decoder.process_rtp_event(unsupported_event, &session_id).await;
    assert!(result.is_err(), "Should return error for unsupported payload type");
    
    // Verify no frame was sent
    assert!(receiver.try_recv().is_err(), "Should not receive frame for unsupported payload");
    
    // Verify error was counted in statistics
    let stats = decoder.get_stats();
    assert_eq!(stats.decode_errors, 1, "Should count decode error");
    
    println!("✅ Unsupported payload type handling test PASSED!");
}

#[tokio::test]
async fn test_packet_loss_handling() {
    // Test packet loss event handling
    println!("🧪 Testing packet loss event handling");
    
    let mut decoder = RtpPayloadDecoder::new();
    let (sender, mut receiver) = mpsc::channel(10);
    let session_id = SessionId::new();
    
    decoder.add_subscriber(session_id.clone(), sender);
    
    // Send a packet loss event
    let loss_event = RtpEvent::PacketLost {
        sequence_number: 42,
    };
    
    // This should not fail but also not generate an audio frame
    decoder.process_rtp_event(loss_event, &session_id)
        .await
        .expect("Packet loss event should be handled gracefully");
    
    // Verify no frame was sent
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(receiver.try_recv().is_err(), "Should not receive frame for packet loss");
    
    println!("✅ Packet loss handling test PASSED!");
}

/// Create test RTP packets with G.711 μ-law payload
fn create_test_g711_packets() -> Vec<MediaPacket> {
    let mut packets = Vec::new();
    let base_timestamp = 1000u32;
    
    for i in 0..5 {
        // Create different patterns for each packet to verify decoding
        let mut payload = vec![0xFF; 160]; // Start with silence
        
        // Add some variation to make each packet unique
        for j in 0..10 {
            payload[j] = ((i * 10 + j) as u8).wrapping_add(0x80); // Vary the first few samples
        }
        
        let packet = MediaPacket {
            payload: Bytes::from(payload),
            payload_type: 0, // PCMU (G.711 μ-law)
            timestamp: base_timestamp + (i as u32 * 160), // 20ms increments
            sequence_number: (i + 1) as u16,
            ssrc: 0x12345678,
            received_at: std::time::Instant::now(),
        };
        
        packets.push(packet);
    }
    
    packets
}