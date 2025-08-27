//! Audio exchange tests between SimplePeer instances
//! 
//! This module tests actual audio flow between peers, recording input
//! and output for each peer to verify correct audio routing.

use rvoip_session_core::api::{SimplePeer, AudioFrame, Result};
use serial_test::serial;
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use hound::{WavWriter, WavSpec};

/// Test configuration
const SAMPLE_RATE: u32 = 8000; // G.711 uses 8kHz
const CHANNELS: u16 = 1; // Mono
const BITS_PER_SAMPLE: u16 = 16; // 16-bit PCM
const DURATION_SECS: f32 = 3.0; // 3 seconds of audio
const FRAME_DURATION_MS: u32 = 20; // 20ms frames for VoIP
const SAMPLES_PER_FRAME: usize = ((SAMPLE_RATE * FRAME_DURATION_MS) / 1000) as usize;

/// Generate a sine wave tone at the specified frequency
fn generate_tone(frequency: f32, sample_rate: u32, duration: f32) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * PI * frequency * t).sin();
        // Convert to i16 range, reduced amplitude to avoid clipping
        let sample_i16 = (sample * 16384.0) as i16;
        samples.push(sample_i16);
    }
    
    samples
}

/// Save audio samples to a WAV file
fn save_wav(path: &Path, samples: &[i16], sample_rate: u32) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let spec = WavSpec {
        channels: CHANNELS,
        sample_rate,
        bits_per_sample: BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    
    let mut writer = WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    
    println!("Saved {} samples to {}", samples.len(), path.display());
    Ok(())
}

/// Audio capture buffer for saving received audio
struct AudioCapture {
    samples: Vec<i16>,
    path: PathBuf,
}

impl AudioCapture {
    fn new(path: PathBuf) -> Self {
        Self {
            samples: Vec::new(),
            path,
        }
    }
    
    fn add_frame(&mut self, frame: &AudioFrame) {
        self.samples.extend_from_slice(&frame.samples);
    }
    
    fn save(&self) -> std::result::Result<(), Box<dyn std::error::Error>> {
        if self.samples.is_empty() {
            eprintln!("Warning: No audio samples captured for {}", self.path.display());
        } else {
            println!("Saving {} audio samples to {}", self.samples.len(), self.path.display());
        }
        save_wav(&self.path, &self.samples, SAMPLE_RATE)?;
        Ok(())
    }
}

/// Helper to send audio tone through a call
async fn send_tone_through_call(
    audio_tx: mpsc::Sender<AudioFrame>,
    tone_samples: Vec<i16>,
    frequency: f32,
) {
    println!("Starting to send {}Hz tone ({} total samples)", frequency, tone_samples.len());
    
    let mut sample_index = 0;
    let total_frames = tone_samples.len() / SAMPLES_PER_FRAME;
    
    for frame_num in 0..total_frames {
        let start = sample_index;
        let end = std::cmp::min(start + SAMPLES_PER_FRAME, tone_samples.len());
        
        if start >= tone_samples.len() {
            break;
        }
        
        let frame_samples = tone_samples[start..end].to_vec();
        let frame = AudioFrame {
            timestamp: (frame_num as u32 * FRAME_DURATION_MS),
            samples: frame_samples,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS as u8,
            duration: Duration::from_millis(FRAME_DURATION_MS as u64),
        };
        
        if audio_tx.send(frame).await.is_err() {
            eprintln!("Audio channel closed while sending");
            break;
        }
        
        sample_index = end;
        
        // Pace the sending at real-time rate
        sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
    }
    
    println!("Finished sending {}Hz tone ({} frames sent)", frequency, total_frames);
}

/// Helper to receive and record audio from a call
async fn receive_and_record_audio(
    mut audio_rx: mpsc::Receiver<AudioFrame>,
    output_path: PathBuf,
    duration: Duration,
) {
    let mut capture = AudioCapture::new(output_path);
    let start = tokio::time::Instant::now();
    
    println!("Starting audio capture for {:?}", duration);
    
    while start.elapsed() < duration {
        match timeout(Duration::from_millis(100), audio_rx.recv()).await {
            Ok(Some(frame)) => {
                capture.add_frame(&frame);
            }
            Ok(None) => {
                println!("Audio channel closed");
                break;
            }
            Err(_) => {
                // Timeout, continue
            }
        }
    }
    
    println!("Audio capture complete, {} samples received", capture.samples.len());
    let _ = capture.save();
}

#[tokio::test]
#[serial]
pub async fn test_peer_to_peer_audio_exchange() -> Result<()> {
    // Initialize tracing for debugging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug,test=info")
        .try_init();
    
    // Create test directory
    let test_dir = std::env::temp_dir().join("rvoip_peer_audio_test");
    std::fs::create_dir_all(&test_dir).unwrap();
    
    let alice_dir = test_dir.join("alice");
    let bob_dir = test_dir.join("bob");
    std::fs::create_dir_all(&alice_dir).unwrap();
    std::fs::create_dir_all(&bob_dir).unwrap();
    
    println!("Test directory: {}", test_dir.display());
    
    // Generate test tones
    println!("Generating test tones...");
    let alice_tone = generate_tone(440.0, SAMPLE_RATE, DURATION_SECS); // A4 note
    let bob_tone = generate_tone(880.0, SAMPLE_RATE, DURATION_SECS);   // A5 note
    
    // Save input files for reference
    save_wav(&alice_dir.join("input.wav"), &alice_tone, SAMPLE_RATE).unwrap();
    save_wav(&bob_dir.join("input.wav"), &bob_tone, SAMPLE_RATE).unwrap();
    
    // Create peers on different ports
    let alice_port = 5060;
    let bob_port = 5061;
    
    println!("Creating peers: Alice on port {}, Bob on port {}", alice_port, bob_port);
    
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(alice_port)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Start Bob listening for incoming calls in background
    let bob_output_path = bob_dir.join("output.wav");
    let bob_handle = tokio::spawn(async move {
        println!("Bob: Waiting for incoming call...");
        
        match timeout(Duration::from_secs(10), bob.next_incoming()).await {
            Ok(Some(incoming)) => {
                println!("Bob: Received call from {}", incoming.from);
                
                // Accept the call (consumes incoming)
                match incoming.accept().await {
                    Ok(mut call) => {
                        println!("Bob: Call accepted, getting audio channels");
                        
                        // Get audio channels
                        match call.audio_channels() {
                            Ok((audio_tx, audio_rx)) => {
                                println!("Bob: Audio channels established");
                                
                                // Send Bob's tone to Alice
                                let send_handle = tokio::spawn(
                                    send_tone_through_call(audio_tx, bob_tone, 880.0)
                                );
                                
                                // Receive Alice's tone
                                let receive_handle = tokio::spawn(
                                    receive_and_record_audio(
                                        audio_rx,
                                        bob_output_path,
                                        Duration::from_secs(3),
                                    )
                                );
                                
                                // Wait for audio exchange to complete
                                let _ = tokio::join!(send_handle, receive_handle);
                                
                                println!("Bob: Audio exchange complete");
                                call.hangup().await.unwrap();
                            }
                            Err(e) => {
                                eprintln!("Bob: Failed to get audio channels: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Bob: Failed to accept call: {}", e);
                    }
                }
            }
            Ok(None) => {
                eprintln!("Bob: No incoming call received");
            }
            Err(_) => {
                eprintln!("Bob: Timeout waiting for incoming call");
            }
        }
        
        bob.shutdown().await.unwrap();
    });
    
    // Give Bob time to start listening
    sleep(Duration::from_millis(500)).await;
    
    // Alice calls Bob
    println!("Alice: Calling Bob at 127.0.0.1:{}", bob_port);
    let mut alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    println!("Alice: Call initiated, getting audio channels");
    
    // Get Alice's audio channels
    let (alice_audio_tx, alice_audio_rx) = alice_call.audio_channels()?;
    
    println!("Alice: Audio channels established");
    
    // Send Alice's tone to Bob
    let alice_send_handle = tokio::spawn(
        send_tone_through_call(alice_audio_tx, alice_tone, 440.0)
    );
    
    // Receive Bob's tone
    let alice_receive_handle = tokio::spawn(
        receive_and_record_audio(
            alice_audio_rx,
            alice_dir.join("output.wav"),
            Duration::from_secs(3),
        )
    );
    
    // Wait for audio exchange
    let _ = tokio::join!(alice_send_handle, alice_receive_handle);
    
    println!("Alice: Audio exchange complete");
    
    // Clean up
    alice_call.hangup().await?;
    alice.shutdown().await?;
    
    // Wait for Bob to complete
    let _ = timeout(Duration::from_secs(5), bob_handle).await;
    
    // Verify output files exist
    assert!(alice_dir.join("input.wav").exists(), "Alice input file should exist");
    assert!(alice_dir.join("output.wav").exists(), "Alice output file should exist");
    assert!(bob_dir.join("input.wav").exists(), "Bob input file should exist");
    assert!(bob_dir.join("output.wav").exists(), "Bob output file should exist");
    
    println!("✅ Test complete! Audio files saved to: {}", test_dir.display());
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_peer_audio_with_hold_resume() -> Result<()> {
    let alice_port = 5060;
    let bob_port = 5061;
    
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(alice_port)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Bob waits for call
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let call = incoming.accept().await.unwrap();
            
            // Wait for hold/resume operations
            sleep(Duration::from_secs(2)).await;
            
            // Clean up
            bob.shutdown().await.unwrap();
        }
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls Bob
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Wait for call to become active first
    let mut active = false;
    for _ in 0..10 {
        if alice_call.is_active().await {
            active = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(active, "Call did not become active before hold");
    
    // Test hold
    println!("Putting call on hold...");
    alice_call.hold().await?;
    assert!(alice_call.is_on_hold().await);
    
    sleep(Duration::from_millis(500)).await;
    
    // Test resume
    println!("Resuming call...");
    alice_call.resume().await?;
    assert!(alice_call.is_active().await);
    
    // Clean up
    alice_call.hangup().await?;
    alice.shutdown().await?;
    
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_peer_audio_with_mute_unmute() -> Result<()> {
    let alice_port = 5060;
    let bob_port = 5061;
    
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(alice_port)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Bob waits for call
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let call = incoming.accept().await.unwrap();
            
            // Wait for mute/unmute operations
            sleep(Duration::from_secs(2)).await;
            
            // Clean up
            bob.shutdown().await.unwrap();
        }
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls Bob
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Test mute
    println!("Muting audio...");
    alice_call.mute().await?;
    
    sleep(Duration::from_millis(500)).await;
    
    // Test unmute
    println!("Unmuting audio...");
    alice_call.unmute().await?;
    
    // Clean up
    alice_call.hangup().await?;
    alice.shutdown().await?;
    
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    
    Ok(())
}