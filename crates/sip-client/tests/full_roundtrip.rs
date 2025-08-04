//! Full roundtrip test with two SIP clients exchanging audio through WAV files
#![cfg(feature = "test-audio")]
//!
//! This test creates two SIP clients that:
//! 1. Load different tone WAV files as input
//! 2. Connect to each other via SIP
//! 3. Exchange audio data
//! 4. Save received audio to output WAV files

use rvoip_sip_client::{SipClientBuilder, StreamExt, CallState};
use rvoip_audio_core::types::{AudioFrame, AudioFormat};
use std::sync::Arc;
use std::path::PathBuf;
use std::f32::consts::PI;
use std::collections::VecDeque;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, timeout};
use tracing::{info, debug, error};
use hound::{WavWriter, WavSpec, WavReader};

/// Test configuration
const SAMPLE_RATE: u32 = 8000;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;
const DURATION_SECS: f32 = 3.0;
const FRAME_DURATION_MS: u32 = 20;
const PEER_A_FREQUENCY: f32 = 440.0; // A4 note
const PEER_B_FREQUENCY: f32 = 880.0; // A5 note (octave higher)

/// Generate a sine wave tone
fn generate_tone(frequency: f32, sample_rate: u32, duration: f32) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * PI * frequency * t).sin();
        let sample_i16 = (sample * 16384.0) as i16; // Scale to 16-bit
        samples.push(sample_i16);
    }
    
    samples
}

/// Save audio samples to WAV file
fn save_wav(path: &std::path::Path, samples: &[i16], sample_rate: u32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    Ok(())
}

/// Load audio samples from WAV file
fn load_wav(path: &std::path::Path) -> Result<(Vec<i16>, u32), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();
    
    // Verify format
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err("WAV file must be mono 16-bit".into());
    }
    
    let samples: Vec<i16> = reader.samples::<i16>()
        .map(|s| s.unwrap())
        .collect();
    
    Ok((samples, spec.sample_rate))
}

/// Audio source that reads from WAV file and feeds frames
struct WavAudioSource {
    samples: Vec<i16>,
    sample_rate: u32,
    position: usize,
    frame_count: u32,
}

impl WavAudioSource {
    fn new(samples: Vec<i16>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
            position: 0,
            frame_count: 0,
        }
    }
    
    fn next_frame(&mut self) -> Option<AudioFrame> {
        let samples_per_frame = (self.sample_rate as usize * FRAME_DURATION_MS as usize) / 1000;
        
        if self.position >= self.samples.len() {
            return None;
        }
        
        let end = (self.position + samples_per_frame).min(self.samples.len());
        let frame_samples = self.samples[self.position..end].to_vec();
        
        let frame = AudioFrame::new(
            frame_samples,
            AudioFormat::new(self.sample_rate, 1, 16, FRAME_DURATION_MS),
            self.frame_count * FRAME_DURATION_MS,
        );
        
        self.position = end;
        self.frame_count += 1;
        
        Some(frame)
    }
}

/// Audio sink that collects frames and saves to WAV file
struct WavAudioSink {
    samples: Vec<i16>,
    sample_rate: u32,
}

impl WavAudioSink {
    fn new(sample_rate: u32) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate,
        }
    }
    
    fn add_frame(&mut self, frame: AudioFrame) {
        self.samples.extend_from_slice(&frame.samples);
    }
    
    fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        save_wav(path, &self.samples, self.sample_rate)
    }
}

/// Feed audio frames from WAV source to the test audio buffer
async fn feed_wav_audio(
    name: &str,
    mut source: WavAudioSource,
    test_buffers: Arc<rvoip_sip_client::test_audio::TestAudioBuffers>,
    is_peer_a: bool,
) {
    info!("🎤 {} starting WAV audio feeder", name);
    
    // Get the appropriate buffer based on which peer this is
    let audio_buffer = if is_peer_a {
        test_buffers.a_to_b.clone() // Peer A sends to B
    } else {
        test_buffers.b_to_a.clone() // Peer B sends to A
    };
    
    while let Some(frame) = source.next_frame() {
        audio_buffer.lock().await.push_back(frame);
        
        // Simulate real-time audio capture
        sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
    }
    
    info!("✅ {} WAV audio feeder completed", name);
}

/// Collect audio frames from test audio buffer to WAV sink
async fn collect_wav_audio(
    name: &str,
    test_buffers: Arc<rvoip_sip_client::test_audio::TestAudioBuffers>,
    is_peer_a: bool,
    duration: Duration,
) -> WavAudioSink {
    info!("🔊 {} starting WAV audio collector", name);
    
    // Get the appropriate buffer based on which peer this is
    let audio_buffer = if is_peer_a {
        test_buffers.b_to_a.clone() // Peer A receives from B
    } else {
        test_buffers.a_to_b.clone() // Peer B receives from A
    };
    
    let mut sink = WavAudioSink::new(SAMPLE_RATE);
    let start = std::time::Instant::now();
    
    while start.elapsed() < duration {
        if let Some(frame) = audio_buffer.lock().await.pop_front() {
            sink.add_frame(frame);
        } else {
            // No frame available, wait a bit
            sleep(Duration::from_millis(5)).await;
        }
    }
    
    info!("✅ {} WAV audio collector completed, collected {} samples", 
        name, sink.samples.len());
    
    sink
}

#[tokio::test]
async fn test_full_roundtrip_with_wav_files() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .init();
    
    info!("🚀 Starting full roundtrip test with WAV files");
    
    // Create test directories if they don't exist
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/full_roundtrip");
    let peer_a_dir = test_dir.join("peer_a");
    let peer_b_dir = test_dir.join("peer_b");
    
    // Generate and save input WAV files
    info!("🎵 Generating test tones");
    let peer_a_samples = generate_tone(PEER_A_FREQUENCY, SAMPLE_RATE, DURATION_SECS);
    let peer_b_samples = generate_tone(PEER_B_FREQUENCY, SAMPLE_RATE, DURATION_SECS);
    
    save_wav(&peer_a_dir.join("input.wav"), &peer_a_samples, SAMPLE_RATE)
        .expect("Failed to save peer A input WAV");
    save_wav(&peer_b_dir.join("input.wav"), &peer_b_samples, SAMPLE_RATE)
        .expect("Failed to save peer B input WAV");
    
    info!("✅ Generated input WAV files");
    
    // Create shared test audio buffers
    let test_buffers = Arc::new(rvoip_sip_client::test_audio::TestAudioBuffers::new());
    
    // Create two SIP clients with test audio
    info!("📞 Creating SIP clients");
    
    let client_a = SipClientBuilder::new()
        .sip_identity("sip:peer_a@localhost")
        .local_address("127.0.0.1:5060".parse().unwrap())
        .test_audio_buffers(test_buffers.clone())
        .audio_defaults()
        .build()
        .await
        .expect("Failed to create client A");
    
    let client_b = SipClientBuilder::new()
        .sip_identity("sip:peer_b@localhost")
        .local_address("127.0.0.1:5061".parse().unwrap())
        .test_audio_buffers(test_buffers.clone())
        .audio_defaults()
        .build()
        .await
        .expect("Failed to create client B");
    
    info!("✅ Created both SIP clients");
    
    // Start the SIP clients
    client_a.start().await.expect("Failed to start client A");
    client_b.start().await.expect("Failed to start client B");
    info!("✅ Started both SIP clients");
    
    // Start audio feeders and collectors
    let peer_a_source = WavAudioSource::new(peer_a_samples.clone(), SAMPLE_RATE);
    let peer_b_source = WavAudioSource::new(peer_b_samples.clone(), SAMPLE_RATE);
    
    // Start feeding audio from WAV files
    let feeder_a = tokio::spawn(feed_wav_audio(
        "Peer A",
        peer_a_source,
        test_buffers.clone(),
        true, // is_peer_a
    ));
    
    let feeder_b = tokio::spawn(feed_wav_audio(
        "Peer B", 
        peer_b_source,
        test_buffers.clone(),
        false, // is_peer_a
    ));
    
    // Start collecting audio
    let collector_a = tokio::spawn(collect_wav_audio(
        "Peer A",
        test_buffers.clone(),
        true, // is_peer_a
        Duration::from_secs_f32(DURATION_SECS + 1.0), // Extra time for processing
    ));
    
    let collector_b = tokio::spawn(collect_wav_audio(
        "Peer B",
        test_buffers.clone(),
        false, // is_peer_a
        Duration::from_secs_f32(DURATION_SECS + 1.0),
    ));
    
    // Client B answers incoming calls
    let client_b_clone = client_b.clone();
    let mut events_b = client_b.events();
    let answer_task = tokio::spawn(async move {
        while let Some(event) = events_b.next().await {
            match event {
                Ok(rvoip_sip_client::SipClientEvent::IncomingCall { call, from, .. }) => {
                    info!("📞 Client B: Incoming call from {}", from);
                    match client_b_clone.answer(&call.id).await {
                        Ok(_) => info!("✅ Client B: Answered call"),
                        Err(e) => error!("❌ Client B: Failed to answer: {}", e),
                    }
                }
                _ => {}
            }
        }
    });
    
    // Give clients time to initialize
    sleep(Duration::from_millis(100)).await;
    
    // Client A calls Client B
    info!("📞 Client A calling Client B");
    let call = client_a.call("sip:peer_b@127.0.0.1:5061").await
        .expect("Failed to initiate call");
    
    // Wait for call to be answered
    match timeout(Duration::from_secs(5), call.wait_for_answer()).await {
        Ok(Ok(_)) => info!("✅ Call answered successfully"),
        Ok(Err(e)) => panic!("Call failed: {}", e),
        Err(_) => panic!("Call answer timeout"),
    }
    
    // Let audio exchange happen
    info!("🎵 Exchanging audio for {} seconds", DURATION_SECS);
    sleep(Duration::from_secs_f32(DURATION_SECS + 0.5)).await;
    
    // Hang up the call
    info!("📞 Hanging up");
    client_a.hangup(&call.id).await.expect("Failed to hang up");
    
    // Wait for audio tasks to complete
    let _ = feeder_a.await;
    let _ = feeder_b.await;
    let sink_a = collector_a.await.expect("Collector A failed");
    let sink_b = collector_b.await.expect("Collector B failed");
    
    // Save output WAV files
    info!("💾 Saving output WAV files");
    sink_a.save(&peer_a_dir.join("output.wav"))
        .expect("Failed to save peer A output");
    sink_b.save(&peer_b_dir.join("output.wav"))
        .expect("Failed to save peer B output");
    
    // Clean up
    answer_task.abort();
    
    info!("✅ Test completed successfully!");
    info!("📁 Input and output WAV files saved in:");
    info!("   - {}", peer_a_dir.display());
    info!("   - {}", peer_b_dir.display());
}