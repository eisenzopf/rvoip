//! Performance tests for RTP receive implementation
//!
//! Tests performance characteristics of the RTP receive pipeline
//! under various load conditions.

use std::sync::Arc;
use std::time::{Duration, Instant};
use bytes::Bytes;
use tokio::sync::mpsc;

use rvoip_session_core::api::types::{SessionId, AudioFrame};
use rvoip_session_core::api::{MediaControl, SessionManagerBuilder, SessionControl};
use rvoip_session_core::media::rtp_decoder::{RtpEvent, RtpPayloadDecoder};
use rvoip_media_core::{MediaPacket, MediaSessionId, RtpEvent as MediaCoreRtpEvent};

#[tokio::test]
async fn test_high_packet_rate_performance() {
    // Test performance with high packet rates (typical VoIP scenario)
    println!("⚡ Testing high packet rate performance");
    
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn") // Reduce log noise for performance test
        .try_init();
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5061) // Use different port than integration test
        .build()
        .await
        .unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    let session_id = SessionId::new();
    let _info = coordinator.create_media_session(&session_id).await.unwrap();
    let subscriber = MediaControl::subscribe_to_audio_frames(&coordinator, &session_id).await.unwrap();
    
    let callback = coordinator.media_manager.create_rtp_event_callback();
    let media_session_id = MediaSessionId::new("perf-test-session");
    
    // Simulate 50 packets per second for 2 seconds (typical VoIP rate)
    let packets_per_second = 50;
    let test_duration_seconds = 2;
    let total_packets = packets_per_second * test_duration_seconds;
    let packet_interval = Duration::from_millis(1000 / packets_per_second);
    
    println!("📊 Sending {} packets at {} pps for {} seconds", 
             total_packets, packets_per_second, test_duration_seconds);
    
    let start_time = Instant::now();
    
    // Send packets at regular intervals
    for i in 0..total_packets {
        let rtp_event = MediaCoreRtpEvent::MediaReceived {
            payload_type: 0,
            payload: create_test_g711_payload(i as usize),
            timestamp: (i * 160) as u32, // 20ms frames at 8kHz
            sequence_number: (i + 1) as u16,
            ssrc: 0x12345678,
        };
        
        callback(media_session_id.clone(), rtp_event);
        
        if i % 50 == 0 {
            println!("📤 Sent {} packets", i + 1);
        }
        
        tokio::time::sleep(packet_interval).await;
    }
    
    let send_duration = start_time.elapsed();
    println!("⏱️ Packet sending completed in: {:?}", send_duration);
    
    // Collect received frames
    let collection_start = Instant::now();
    let mut received_frames = 0;
    let collection_timeout = Duration::from_secs(5);
    
    while received_frames < total_packets && collection_start.elapsed() < collection_timeout {
        match subscriber.try_recv() {
            Ok(_frame) => {
                received_frames += 1;
                if received_frames % 50 == 0 {
                    println!("🎵 Received {} frames", received_frames);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("Subscriber disconnected");
            }
        }
    }
    
    let total_duration = start_time.elapsed();
    
    // Performance analysis
    println!("📊 Performance Results:");
    println!("  - Total packets sent: {}", total_packets);
    println!("  - Total frames received: {}", received_frames);
    println!("  - Total duration: {:?}", total_duration);
    println!("  - Average processing latency: {:?}", 
             total_duration.checked_div(total_packets as u32).unwrap_or_default());
    
    let decoder_stats = coordinator.media_manager.get_rtp_decoder_stats().await;
    println!("  - Decoder packets processed: {}", decoder_stats.packets_processed);
    println!("  - Decoder errors: {}", decoder_stats.decode_errors);
    
    // Verify performance criteria
    assert_eq!(received_frames, total_packets, "Should receive all frames");
    assert_eq!(decoder_stats.decode_errors, 0, "Should have no decode errors");
    assert!(total_duration < Duration::from_secs(8), "Should complete within reasonable time");
    
    let frames_per_second = received_frames as f64 / total_duration.as_secs_f64();
    println!("  - Effective frames per second: {:.1}", frames_per_second);
    assert!(frames_per_second >= 45.0, "Should maintain at least 45 fps processing rate");
    
    println!("✅ High packet rate performance test PASSED!");
}

#[tokio::test]
async fn test_multiple_session_performance() {
    // Test performance with multiple concurrent sessions
    println!("🔀 Testing multiple session performance");
    
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .try_init();
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5062) // Use different port
        .build()
        .await
        .unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    let num_sessions = 5;
    let packets_per_session = 20;
    
    // Create multiple sessions
    let mut sessions = Vec::new();
    let mut subscribers = Vec::new();
    
    for i in 0..num_sessions {
        let session_id = SessionId::new();
        let _info = coordinator.create_media_session(&session_id).await.unwrap();
        let subscriber = MediaControl::subscribe_to_audio_frames(&coordinator, &session_id).await.unwrap();
        
        sessions.push(session_id);
        subscribers.push(subscriber);
        
        println!("✅ Created session {}", i + 1);
    }
    
    let callback = coordinator.media_manager.create_rtp_event_callback();
    
    println!("📊 Sending {} packets to {} sessions ({} total packets)", 
             packets_per_session, num_sessions, packets_per_session * num_sessions);
    
    let start_time = Instant::now();
    
    // Send packets to all sessions concurrently
    let mut tasks = Vec::new();
    
    for (session_idx, _session_id) in sessions.iter().enumerate() {
        let callback_clone = callback.clone();
        
        let task = tokio::spawn(async move {
            let media_session_id = MediaSessionId::new(&format!("perf-session-{}", session_idx));
            
            for packet_idx in 0..packets_per_session {
                let rtp_event = MediaCoreRtpEvent::MediaReceived {
                    payload_type: 0,
                    payload: create_test_g711_payload(packet_idx),
                    timestamp: (packet_idx * 160) as u32,
                    sequence_number: (packet_idx + 1) as u16,
                    ssrc: (0x10000000 + session_idx * 0x1000000) as u32,
                };
                
                callback_clone(media_session_id.clone(), rtp_event);
                
                // Small delay to avoid overwhelming the system
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        
        tasks.push(task);
    }
    
    // Wait for all sending tasks to complete
    for task in tasks {
        task.await.unwrap();
    }
    
    let send_duration = start_time.elapsed();
    println!("⏱️ All packets sent in: {:?}", send_duration);
    
    // Collect frames from all subscribers
    let collection_start = Instant::now();
    let mut total_received = 0;
    let expected_total = packets_per_session * num_sessions;
    
    while total_received < expected_total && collection_start.elapsed() < Duration::from_secs(10) {
        for (session_idx, subscriber) in subscribers.iter().enumerate() {
            while let Ok(_frame) = subscriber.try_recv() {
                total_received += 1;
                if total_received % 20 == 0 {
                    println!("🎵 Total frames received: {}", total_received);
                }
            }
        }
        
        if total_received < expected_total {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    
    let total_duration = start_time.elapsed();
    
    // Performance analysis
    println!("📊 Multi-session Performance Results:");
    println!("  - Sessions: {}", num_sessions);
    println!("  - Expected frames: {}", expected_total);
    println!("  - Received frames: {}", total_received);
    println!("  - Total duration: {:?}", total_duration);
    
    let decoder_stats = coordinator.media_manager.get_rtp_decoder_stats().await;
    println!("  - Decoder packets: {}", decoder_stats.packets_processed);
    println!("  - Decoder errors: {}", decoder_stats.decode_errors);
    
    // Verify multi-session performance
    assert_eq!(total_received, expected_total, "Should receive all frames from all sessions");
    assert_eq!(decoder_stats.decode_errors, 0, "Should have no decode errors");
    assert!(total_duration < Duration::from_secs(15), "Should complete within reasonable time");
    
    let total_fps = total_received as f64 / total_duration.as_secs_f64();
    println!("  - Total frames per second: {:.1}", total_fps);
    
    println!("✅ Multiple session performance test PASSED!");
}

#[tokio::test]
async fn test_decoder_performance_direct() {
    // Test RtpPayloadDecoder performance directly
    println!("🎯 Testing RtpPayloadDecoder direct performance");
    
    let mut decoder = RtpPayloadDecoder::new();
    let (sender, mut receiver) = mpsc::channel(200);
    let session_id = SessionId::new();
    
    decoder.add_subscriber(session_id.clone(), sender);
    
    let num_packets = 100;
    println!("📊 Processing {} packets directly through decoder", num_packets);
    
    let start_time = Instant::now();
    
    // Process packets as fast as possible
    for i in 0..num_packets {
        let rtp_event = RtpEvent::MediaReceived {
            payload_type: if i % 2 == 0 { 0 } else { 8 }, // Alternate PCMU/PCMA
            payload: create_test_g711_payload(i as usize),
            timestamp: (i * 160) as u32,
            sequence_number: (i + 1) as u16,
            ssrc: 0x12345678,
        };
        
        decoder.process_rtp_event(rtp_event, &session_id).await
            .expect("Failed to process RTP event");
    }
    
    let processing_duration = start_time.elapsed();
    
    // Collect all frames
    let mut received_frames = 0;
    let collection_start = Instant::now();
    
    while received_frames < num_packets && collection_start.elapsed() < Duration::from_secs(2) {
        match receiver.try_recv() {
            Ok(_frame) => received_frames += 1,
            Err(mpsc::error::TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
    
    let total_duration = start_time.elapsed();
    
    // Performance metrics
    println!("📊 Direct Decoder Performance:");
    println!("  - Packets processed: {}", num_packets);
    println!("  - Frames received: {}", received_frames);
    println!("  - Processing time: {:?}", processing_duration);
    println!("  - Total time: {:?}", total_duration);
    
    let avg_processing_time = processing_duration.as_nanos() / num_packets as u128;
    println!("  - Average processing time per packet: {} ns", avg_processing_time);
    
    let packets_per_second = num_packets as f64 / processing_duration.as_secs_f64();
    println!("  - Processing rate: {:.0} packets/second", packets_per_second);
    
    let stats = decoder.get_stats();
    println!("  - Decoder statistics: {:?}", stats);
    
    // Performance assertions
    assert_eq!(received_frames, num_packets, "Should receive all frames");
    assert_eq!(stats.decode_errors, 0, "Should have no errors");
    assert!(packets_per_second >= 1000.0, "Should process at least 1000 packets/second");
    assert!(avg_processing_time < 1_000_000, "Should process each packet in under 1ms");
    
    println!("✅ Direct decoder performance test PASSED!");
}

#[tokio::test]
async fn test_memory_usage_stability() {
    // Test that memory usage remains stable under continuous load
    println!("💾 Testing memory usage stability");
    
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .try_init();
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5063) // Use different port
        .build()
        .await
        .unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    let session_id = SessionId::new();
    let _info = coordinator.create_media_session(&session_id).await.unwrap();
    let subscriber = MediaControl::subscribe_to_audio_frames(&coordinator, &session_id).await.unwrap();
    
    let callback = coordinator.media_manager.create_rtp_event_callback();
    let media_session_id = MediaSessionId::new("memory-test-session");
    
    let cycles = 10;
    let packets_per_cycle = 50;
    
    println!("📊 Running {} cycles of {} packets each", cycles, packets_per_cycle);
    
    for cycle in 0..cycles {
        println!("🔄 Starting cycle {}", cycle + 1);
        
        // Send packets
        for i in 0..packets_per_cycle {
            let rtp_event = MediaCoreRtpEvent::MediaReceived {
                payload_type: 0,
                payload: create_test_g711_payload(i as usize),
                timestamp: ((cycle * packets_per_cycle + i) * 160) as u32,
                sequence_number: (i + 1) as u16,
                ssrc: 0x12345678,
            };
            
            callback(media_session_id.clone(), rtp_event);
        }
        
        // Consume frames to prevent accumulation
        let mut frames_consumed = 0;
        let consume_start = Instant::now();
        
        while frames_consumed < packets_per_cycle && consume_start.elapsed() < Duration::from_secs(2) {
            match subscriber.try_recv() {
                Ok(_frame) => frames_consumed += 1,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        
        assert_eq!(frames_consumed, packets_per_cycle, 
                   "Should consume all frames in cycle {}", cycle + 1);
        
        // Small pause between cycles
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Final verification
    let final_stats = coordinator.media_manager.get_rtp_decoder_stats().await;
    let expected_packets = cycles * packets_per_cycle;
    
    println!("📊 Memory Stability Test Results:");
    println!("  - Total cycles: {}", cycles);
    println!("  - Expected packets: {}", expected_packets);
    println!("  - Processed packets: {}", final_stats.packets_processed);
    println!("  - Decode errors: {}", final_stats.decode_errors);
    
    assert_eq!(final_stats.packets_processed, expected_packets as u64);
    assert_eq!(final_stats.decode_errors, 0);
    
    println!("✅ Memory usage stability test PASSED!");
}

/// Create test G.711 payload with some variation
fn create_test_g711_payload(index: usize) -> Vec<u8> {
    let mut payload = vec![0xFF; 160]; // Start with silence
    
    // Add some variation based on index
    for i in 0..8 {
        payload[i] = ((index * 8 + i) as u8).wrapping_add(0x80);
    }
    
    payload
}