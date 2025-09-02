//! Audio utilities for testing - handles tone generation, recording, and verification
//! 
//! This module contains all the complex audio handling code so the peer examples
//! can remain simple and focused on demonstrating the API.

use rvoip_session_core::api::{AudioFrame, Result};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration, timeout};
use hound::{WavWriter, WavSpec};

/// Audio configuration
const SAMPLE_RATE: u32 = 8000;  // G.711 uses 8kHz
const CHANNELS: u8 = 1;          // Mono
const BITS_PER_SAMPLE: u16 = 16;
const FRAME_SIZE: usize = 160;   // 20ms at 8kHz
const DURATION_SECS: f32 = 2.0;  // 2 seconds of audio

/// Generate a tone at specified frequency
pub fn generate_tone(frequency: f32, duration_secs: f32, sample_rate: u32) -> Vec<i16> {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (0.3 * f32::sin(2.0 * PI * frequency * t) * i16::MAX as f32) as i16;
        samples.push(sample);
    }
    
    samples
}

/// Simple audio exchange - sends a tone and receives audio
pub async fn exchange_audio(
    tx: mpsc::Sender<AudioFrame>,
    rx: mpsc::Receiver<AudioFrame>,
    frequency: f32,
    peer_name: &str,
) -> Result<()> {
    println!("🎵 {} generating {}Hz tone", peer_name, frequency);
    
    // Generate tone
    let tone = generate_tone(frequency, DURATION_SECS, SAMPLE_RATE);
    let total_samples = tone.len();
    
    // Storage for recording (if enabled by environment variable)
    let record = std::env::var("RECORD_AUDIO").is_ok();
    let sent_samples = Arc::new(Mutex::new(Vec::new()));
    let received_samples = Arc::new(Mutex::new(Vec::new()));
    
    // Spawn sender task
    let sent_samples_clone = sent_samples.clone();
    let tone_clone = tone.clone();
    let sender = tokio::spawn(async move {
        send_audio(tx, tone_clone, sent_samples_clone).await;
    });
    
    // Spawn receiver task
    let received_samples_clone = received_samples.clone();
    let receiver = tokio::spawn(async move {
        receive_audio(rx, received_samples_clone, total_samples).await;
    });
    
    // Wait for both to complete
    let _ = tokio::join!(sender, receiver);
    
    println!("🎵 {} audio exchange complete", peer_name);
    
    // Save recordings if requested
    if record {
        save_recordings(peer_name, sent_samples, received_samples).await?;
    }
    
    Ok(())
}

/// Send audio frames
async fn send_audio(
    mut tx: mpsc::Sender<AudioFrame>,
    tone: Vec<i16>,
    recording: Arc<Mutex<Vec<i16>>>,
) {
    // Store what we're sending
    recording.lock().await.extend_from_slice(&tone);
    
    let total_frames = tone.len() / FRAME_SIZE;
    let mut sent = 0;
    
    for frame_idx in 0..total_frames {
        let start = frame_idx * FRAME_SIZE;
        let end = std::cmp::min(start + FRAME_SIZE, tone.len());
        
        if start >= tone.len() {
            break;
        }
        
        let frame_samples = tone[start..end].to_vec();
        let frame = AudioFrame {
            samples: frame_samples.clone(),
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            duration: Duration::from_millis(20),
            timestamp: (frame_idx * FRAME_SIZE) as u32,
        };
        
        if tx.send(frame).await.is_err() {
            break;
        }
        
        sent += frame_samples.len();
        
        // Pace at 20ms intervals
        sleep(Duration::from_millis(20)).await;
    }
    
    println!("📤 Sent {} samples", sent);
}

/// Receive audio frames
async fn receive_audio(
    mut rx: mpsc::Receiver<AudioFrame>,
    recording: Arc<Mutex<Vec<i16>>>,
    expected: usize,
) {
    let mut received = 0;
    let mut consecutive_timeouts = 0;
    let max_timeouts = 30; // Allow 3 seconds of no data before giving up
    
    loop {
        match timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(frame)) => {
                recording.lock().await.extend_from_slice(&frame.samples);
                received += frame.samples.len();
                consecutive_timeouts = 0; // Reset timeout counter on successful receive
                
                if received >= expected {
                    break;
                }
            }
            Ok(None) => {
                // Channel closed
                break;
            }
            Err(_) => {
                // Timeout - but keep trying for a while
                consecutive_timeouts += 1;
                if consecutive_timeouts >= max_timeouts {
                    println!("⏱️ Receive timeout after {} consecutive timeouts", consecutive_timeouts);
                    break;
                }
            }
        }
    }
    
    println!("📥 Received {} samples", received);
}

/// Save recordings to WAV files
async fn save_recordings(
    peer_name: &str,
    sent: Arc<Mutex<Vec<i16>>>,
    received: Arc<Mutex<Vec<i16>>>,
) -> Result<()> {
    let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/api_peer_audio/output");
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    
    let spec = WavSpec {
        channels: CHANNELS as u16,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    
    // Save sent audio
    let sent_path = output_dir.join(format!("{}_sent.wav", peer_name));
    let mut writer = WavWriter::create(&sent_path, spec)
        .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    
    for sample in sent.lock().await.iter() {
        writer.write_sample(*sample)
            .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    }
    writer.finalize()
        .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    
    // Save received audio
    let received_path = output_dir.join(format!("{}_received.wav", peer_name));
    let mut writer = WavWriter::create(&received_path, spec)
        .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    
    for sample in received.lock().await.iter() {
        writer.write_sample(*sample)
            .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    }
    writer.finalize()
        .map_err(|e| rvoip_session_core::errors::SessionError::Other(e.to_string()))?;
    
    println!("💾 Saved recordings to {}", output_dir.display());
    
    Ok(())
}