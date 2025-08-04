//! Long-running RTP test to verify sustained streaming
//!
//! This test creates an RTP sender and receiver that run for over a minute
//! to verify that the RTP event handler doesn't prematurely close channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn, error};

use rvoip_media_core::relay::controller::MediaSessionController;
use rvoip_media_core::relay::controller::types::{MediaConfig, MediaSessionEvent};
use rvoip_media_core::types::{DialogId, AudioFrame};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_long_running_rtp_streaming() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug,long_running_rtp_test=info")
        .try_init();

    info!("🚀 Starting long-running RTP test - targeting 1 minute duration");
    
    // Create media session controller
    let controller = Arc::new(MediaSessionController::new());
    
    // Create two dialogs for bidirectional communication
    let dialog_a = DialogId::new("test_dialog_a");
    let dialog_b = DialogId::new("test_dialog_b");
    
    // Configure media sessions
    let config_a = MediaConfig {
        local_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: None, // Will be set after B starts
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    
    let config_b = MediaConfig {
        local_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: None, // Will be set after A starts
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    
    // Start media sessions
    controller.start_media(dialog_a.clone(), config_a).await
        .expect("Failed to start media for dialog A");
    
    controller.start_media(dialog_b.clone(), config_b).await
        .expect("Failed to start media for dialog B");
    
    // Get allocated ports
    let session_a = controller.get_session_info(&dialog_a).await
        .expect("No session info for A");
    let session_b = controller.get_session_info(&dialog_b).await
        .expect("No session info for B");
    
    let port_a = session_a.rtp_port.expect("No RTP port for A");
    let port_b = session_b.rtp_port.expect("No RTP port for B");
    
    info!("📡 Session A RTP port: {}", port_a);
    info!("📡 Session B RTP port: {}", port_b);
    
    // Update configs with remote addresses
    let updated_config_a = MediaConfig {
        local_addr: format!("127.0.0.1:{}", port_a).parse().unwrap(),
        remote_addr: Some(format!("127.0.0.1:{}", port_b).parse().unwrap()),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    
    let updated_config_b = MediaConfig {
        local_addr: format!("127.0.0.1:{}", port_b).parse().unwrap(),
        remote_addr: Some(format!("127.0.0.1:{}", port_a).parse().unwrap()),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    
    controller.update_media(dialog_a.clone(), updated_config_a).await
        .expect("Failed to update media config for A");
    
    controller.update_media(dialog_b.clone(), updated_config_b).await
        .expect("Failed to update media config for B");
    
    // Set up audio frame callbacks to receive decoded frames
    let (frame_tx_a, mut frame_rx_a) = mpsc::channel::<AudioFrame>(1000);
    let (frame_tx_b, mut frame_rx_b) = mpsc::channel::<AudioFrame>(1000);
    
    controller.set_audio_frame_callback(dialog_a.clone(), frame_tx_a).await
        .expect("Failed to set callback for A");
    
    controller.set_audio_frame_callback(dialog_b.clone(), frame_tx_b).await
        .expect("Failed to set callback for B");
    
    // Counters for tracking progress
    let frames_sent_a = Arc::new(AtomicU64::new(0));
    let frames_sent_b = Arc::new(AtomicU64::new(0));
    let frames_received_a = Arc::new(AtomicU64::new(0));
    let frames_received_b = Arc::new(AtomicU64::new(0));
    
    // Configuration
    const TARGET_DURATION_SECS: u64 = 75; // 1 minute 15 seconds to ensure > 1 minute
    const FRAME_DURATION_MS: u64 = 20; // 20ms frames (standard for VoIP)
    const EXPECTED_FRAMES: u64 = (TARGET_DURATION_SECS * 1000) / FRAME_DURATION_MS;
    const SAMPLE_RATE: u32 = 8000;
    const SAMPLES_PER_FRAME: usize = 160; // 20ms at 8kHz
    
    info!("🎯 Target: {} seconds, expecting ~{} frames", TARGET_DURATION_SECS, EXPECTED_FRAMES);
    
    // Start sending from A
    let send_task_a = {
        let controller = controller.clone();
        let dialog = dialog_a.clone();
        let counter = frames_sent_a.clone();
        
        tokio::spawn(async move {
            info!("🎵 Dialog A: Starting to send audio frames");
            let start_time = Instant::now();
            let mut frame_count = 0u64;
            
            // Generate 440Hz tone
            loop {
                // Check if we've sent enough
                if frame_count >= EXPECTED_FRAMES {
                    break;
                }
                
                // Generate audio frame
                let mut samples = Vec::with_capacity(SAMPLES_PER_FRAME);
                for i in 0..SAMPLES_PER_FRAME {
                    let t = ((frame_count * SAMPLES_PER_FRAME as u64 + i as u64) as f32) / SAMPLE_RATE as f32;
                    let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 8000.0;
                    samples.push(sample as i16);
                }
                
                let audio_frame = AudioFrame::new(
                    samples,
                    SAMPLE_RATE,
                    1,
                    (frame_count * SAMPLES_PER_FRAME as u64) as u32,
                );
                
                // Send via media-core
                if let Err(e) = controller.send_audio_frame(&dialog, audio_frame).await {
                    error!("Failed to send frame {}: {}", frame_count, e);
                    break;
                }
                
                frame_count += 1;
                counter.store(frame_count, Ordering::Relaxed);
                
                // Log progress every second
                if frame_count % 50 == 0 {
                    let elapsed = start_time.elapsed();
                    info!("📤 Dialog A: Sent {} frames ({:.1}s)", frame_count, elapsed.as_secs_f32());
                }
                
                // Important: maintain 20ms pacing
                tokio::time::sleep(Duration::from_millis(FRAME_DURATION_MS)).await;
            }
            
            let total_duration = start_time.elapsed();
            info!("✅ Dialog A: Finished sending {} frames in {:.1}s", frame_count, total_duration.as_secs_f32());
        })
    };
    
    // Start sending from B
    let send_task_b = {
        let controller = controller.clone();
        let dialog = dialog_b.clone();
        let counter = frames_sent_b.clone();
        
        tokio::spawn(async move {
            info!("🎵 Dialog B: Starting to send audio frames");
            let start_time = Instant::now();
            let mut frame_count = 0u64;
            
            // Generate 880Hz tone
            loop {
                // Check if we've sent enough
                if frame_count >= EXPECTED_FRAMES {
                    break;
                }
                
                // Generate audio frame
                let mut samples = Vec::with_capacity(SAMPLES_PER_FRAME);
                for i in 0..SAMPLES_PER_FRAME {
                    let t = ((frame_count * SAMPLES_PER_FRAME as u64 + i as u64) as f32) / SAMPLE_RATE as f32;
                    let sample = (t * 880.0 * 2.0 * std::f32::consts::PI).sin() * 8000.0;
                    samples.push(sample as i16);
                }
                
                let audio_frame = AudioFrame::new(
                    samples,
                    SAMPLE_RATE,
                    1,
                    (frame_count * SAMPLES_PER_FRAME as u64) as u32,
                );
                
                // Send via media-core
                if let Err(e) = controller.send_audio_frame(&dialog, audio_frame).await {
                    error!("Failed to send frame {}: {}", frame_count, e);
                    break;
                }
                
                frame_count += 1;
                counter.store(frame_count, Ordering::Relaxed);
                
                // Log progress every second
                if frame_count % 50 == 0 {
                    let elapsed = start_time.elapsed();
                    info!("📤 Dialog B: Sent {} frames ({:.1}s)", frame_count, elapsed.as_secs_f32());
                }
                
                // Important: maintain 20ms pacing
                tokio::time::sleep(Duration::from_millis(FRAME_DURATION_MS)).await;
            }
            
            let total_duration = start_time.elapsed();
            info!("✅ Dialog B: Finished sending {} frames in {:.1}s", frame_count, total_duration.as_secs_f32());
        })
    };
    
    // Start receiving for A
    let receive_task_a = {
        let counter = frames_received_a.clone();
        
        tokio::spawn(async move {
            info!("👂 Dialog A: Starting to receive audio frames");
            let start_time = Instant::now();
            let mut frame_count = 0u64;
            let mut last_log_time = Instant::now();
            
            loop {
                match tokio::time::timeout(Duration::from_secs(5), frame_rx_a.recv()).await {
                    Ok(Some(frame)) => {
                        frame_count += 1;
                        counter.store(frame_count, Ordering::Relaxed);
                        
                        // Verify it's 880Hz from B
                        if frame_count == 1 {
                            info!("🎵 Dialog A received first frame: {} samples", frame.samples.len());
                        }
                        
                        // Log progress every second
                        if last_log_time.elapsed() >= Duration::from_secs(1) {
                            let elapsed = start_time.elapsed();
                            info!("📥 Dialog A: Received {} frames ({:.1}s)", frame_count, elapsed.as_secs_f32());
                            last_log_time = Instant::now();
                        }
                    }
                    Ok(None) => {
                        warn!("Dialog A: Receive channel closed after {} frames", frame_count);
                        break;
                    }
                    Err(_) => {
                        warn!("Dialog A: Timeout waiting for frames after {} frames", frame_count);
                        break;
                    }
                }
            }
            
            let total_duration = start_time.elapsed();
            info!("🛑 Dialog A: Stopped receiving after {} frames in {:.1}s", frame_count, total_duration.as_secs_f32());
        })
    };
    
    // Start receiving for B
    let receive_task_b = {
        let counter = frames_received_b.clone();
        
        tokio::spawn(async move {
            info!("👂 Dialog B: Starting to receive audio frames");
            let start_time = Instant::now();
            let mut frame_count = 0u64;
            let mut last_log_time = Instant::now();
            
            loop {
                match tokio::time::timeout(Duration::from_secs(5), frame_rx_b.recv()).await {
                    Ok(Some(frame)) => {
                        frame_count += 1;
                        counter.store(frame_count, Ordering::Relaxed);
                        
                        // Verify it's 440Hz from A
                        if frame_count == 1 {
                            info!("🎵 Dialog B received first frame: {} samples", frame.samples.len());
                        }
                        
                        // Log progress every second
                        if last_log_time.elapsed() >= Duration::from_secs(1) {
                            let elapsed = start_time.elapsed();
                            info!("📥 Dialog B: Received {} frames ({:.1}s)", frame_count, elapsed.as_secs_f32());
                            last_log_time = Instant::now();
                        }
                    }
                    Ok(None) => {
                        warn!("Dialog B: Receive channel closed after {} frames", frame_count);
                        break;
                    }
                    Err(_) => {
                        warn!("Dialog B: Timeout waiting for frames after {} frames", frame_count);
                        break;
                    }
                }
            }
            
            let total_duration = start_time.elapsed();
            info!("🛑 Dialog B: Stopped receiving after {} frames in {:.1}s", frame_count, total_duration.as_secs_f32());
        })
    };
    
    // Wait for all tasks to complete
    let (send_a_result, send_b_result, recv_a_result, recv_b_result) = 
        tokio::join!(send_task_a, send_task_b, receive_task_a, receive_task_b);
    
    // Check results
    if let Err(e) = send_a_result {
        error!("Send task A panicked: {:?}", e);
    }
    if let Err(e) = send_b_result {
        error!("Send task B panicked: {:?}", e);
    }
    if let Err(e) = recv_a_result {
        error!("Receive task A panicked: {:?}", e);
    }
    if let Err(e) = recv_b_result {
        error!("Receive task B panicked: {:?}", e);
    }
    
    // Get final statistics
    let sent_a = frames_sent_a.load(Ordering::Relaxed);
    let sent_b = frames_sent_b.load(Ordering::Relaxed);
    let received_a = frames_received_a.load(Ordering::Relaxed);
    let received_b = frames_received_b.load(Ordering::Relaxed);
    
    // Get RTP statistics
    if let Some(stats_a) = controller.get_rtp_statistics(&dialog_a).await {
        info!("📊 Dialog A RTP stats: packets_sent={}, packets_received={}, packets_lost={}", 
              stats_a.packets_sent, stats_a.packets_received, stats_a.packets_lost);
    }
    
    if let Some(stats_b) = controller.get_rtp_statistics(&dialog_b).await {
        info!("📊 Dialog B RTP stats: packets_sent={}, packets_received={}, packets_lost={}", 
              stats_b.packets_sent, stats_b.packets_received, stats_b.packets_lost);
    }
    
    // Clean up
    controller.stop_media(&dialog_a).await.expect("Failed to stop media A");
    controller.stop_media(&dialog_b).await.expect("Failed to stop media B");
    
    // Final report
    info!("📊 === FINAL REPORT ===");
    info!("📊 Expected frames: ~{}", EXPECTED_FRAMES);
    info!("📊 Dialog A: Sent {} frames, Received {} frames", sent_a, received_a);
    info!("📊 Dialog B: Sent {} frames, Received {} frames", sent_b, received_b);
    
    let receive_rate_a = (received_a as f64 / sent_b as f64) * 100.0;
    let receive_rate_b = (received_b as f64 / sent_a as f64) * 100.0;
    
    info!("📊 A receive rate: {:.1}%", receive_rate_a);
    info!("📊 B receive rate: {:.1}%", receive_rate_b);
    
    // Assertions
    assert!(sent_a >= EXPECTED_FRAMES * 95 / 100, 
            "Dialog A should send at least 95% of expected frames, sent {}", sent_a);
    assert!(sent_b >= EXPECTED_FRAMES * 95 / 100, 
            "Dialog B should send at least 95% of expected frames, sent {}", sent_b);
    
    // Check if we hit the 100-frame bug
    if received_a == 100 || received_b == 100 {
        panic!("🐛 BUG REPRODUCED: Receiver stopped at exactly 100 frames!");
    }
    
    // We expect at least 90% delivery rate for localhost communication
    assert!(received_a >= sent_b * 90 / 100, 
            "Dialog A should receive at least 90% of frames sent by B, received {} of {}", received_a, sent_b);
    assert!(received_b >= sent_a * 90 / 100, 
            "Dialog B should receive at least 90% of frames sent by A, received {} of {}", received_b, sent_a);
    
    info!("✅ Long-running RTP test completed successfully!");
}