//! Audio utilities for testing - handles tone generation, recording, and verification
//! 
//! This module contains audio handling code for the peer examples.
//! Note: The new session-core-v2 API handles audio differently, so this is simplified.

use rvoip_session_core_v2::{Result, SessionError};
use std::f32::consts::PI;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration};

/// Audio configuration
pub const SAMPLE_RATE: u32 = 8000;  // G.711 uses 8kHz
pub const CHANNELS: u8 = 1;         // Mono
pub const BITS_PER_SAMPLE: u16 = 16;
pub const FRAME_SIZE: usize = 160;  // 20ms at 8kHz
pub const DURATION_SECS: f32 = 2.0; // 2 seconds of audio

/// Audio frame representation for the new API
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u8,
    pub duration: Duration,
    pub timestamp: u32,
}

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

/// Simulated audio exchange for the new API
/// Note: The new session-core-v2 doesn't directly expose audio channels yet
/// This is a placeholder that shows what the audio exchange would look like
pub async fn exchange_audio(
    tx: mpsc::Sender<AudioFrame>,
    mut rx: mpsc::Receiver<AudioFrame>,
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
    
    // Clone for spawning tasks
    let sent_samples_clone = sent_samples.clone();
    let tone_clone = tone.clone();
    
    // Spawn sender task
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
    tx: mpsc::Sender<AudioFrame>,
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
    let timeout_duration = Duration::from_secs(3);
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout_duration {
        match rx.try_recv() {
            Ok(frame) => {
                recording.lock().await.extend_from_slice(&frame.samples);
                received += frame.samples.len();
                
                if received >= expected {
                    break;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                // No data available, wait a bit
                sleep(Duration::from_millis(20)).await;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed
                break;
            }
        }
    }
    
    println!("📥 Received {} samples", received);
}

/// Save audio recordings to WAV files
async fn save_recordings(
    peer_name: &str,
    sent_samples: Arc<Mutex<Vec<i16>>>,
    received_samples: Arc<Mutex<Vec<i16>>>,
) -> Result<()> {
    use std::fs;
    use std::path::Path;
    
    // Create output directory
    fs::create_dir_all("output")?;
    
    // Get samples
    let sent = sent_samples.lock().await.clone();
    let received = received_samples.lock().await.clone();
    
    // Save sent audio
    if !sent.is_empty() {
        let sent_path = format!("output/{}_sent.raw", peer_name);
        save_raw_audio(&sent_path, &sent)?;
        println!("💾 Saved sent audio to {}", sent_path);
    }
    
    // Save received audio
    if !received.is_empty() {
        let received_path = format!("output/{}_received.raw", peer_name);
        save_raw_audio(&received_path, &received)?;
        println!("💾 Saved received audio to {}", received_path);
    }
    
    Ok(())
}

/// Save raw audio samples to a file
fn save_raw_audio(path: &str, samples: &[i16]) -> Result<()> {
    use std::fs::File;
    use std::io::Write;
    
    let mut file = File::create(path)?;
    
    // Convert i16 samples to bytes
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    
    Ok(())
}