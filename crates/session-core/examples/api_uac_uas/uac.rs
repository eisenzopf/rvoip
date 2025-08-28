//! UAC (User Agent Client) example that makes a call and sends 440Hz tone

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::types::{AudioFrame, CallState};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, timeout};
use hound::{WavWriter, WavSpec};

const SAMPLE_RATE: u32 = 8000;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;
const DURATION_SECS: f32 = 5.0;
const FRAME_DURATION_MS: u32 = 20;

fn generate_tone(frequency: f32, sample_rate: u32, duration: f32) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * PI * frequency * t).sin();
        let sample_i16 = (sample * 16384.0) as i16;
        samples.push(sample_i16);
    }
    
    samples
}

fn save_wav(path: &Path, samples: &[i16], sample_rate: u32) -> Result<(), Box<dyn std::error::Error>> {
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
    
    fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.samples.is_empty() {
            tracing::warn!("No audio samples captured for {}", self.path.display());
        } else {
            tracing::info!("Saving {} audio samples to {}", self.samples.len(), self.path.display());
        }
        save_wav(&self.path, &self.samples, SAMPLE_RATE)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();
    
    tracing::info!("🎵 UAC starting on port 5060...");
    
    // Create output directory
    let output_dir = PathBuf::from("examples/api_uac_uas/uac_output");
    std::fs::create_dir_all(&output_dir)?;
    
    // Generate 440Hz tone
    let tone_440 = generate_tone(440.0, SAMPLE_RATE, DURATION_SECS);
    save_wav(&output_dir.join("input.wav"), &tone_440, SAMPLE_RATE)?;
    tracing::info!("💾 Saved 440Hz input tone");
    
    // Create UAC client
    let uac_client = SimpleUacClient::new("uac")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    tracing::info!("✅ UAC client created");
    
    // Give UAS time to start
    sleep(Duration::from_secs(2)).await;
    
    // Make the call
    tracing::info!("📞 UAC calling UAS...");
    let mut call = uac_client.call("sip:uas@127.0.0.1:5061").await?;
    
    tracing::info!("✅ Call initiated: {}", call.id());
    
    // Wait for call to be active
    loop {
        if call.state().await == CallState::Active {
            tracing::info!("✅ Call is active");
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for media to be ready
    sleep(Duration::from_secs(2)).await;
    
    // Set up audio channels
    tracing::info!("📻 Setting up audio channels");
    let (audio_tx, mut audio_rx) = call.audio_channels();
    
    // Set up audio capture
    let capture = Arc::new(Mutex::new(AudioCapture::new(output_dir.join("output.wav"))));
    
    // Send audio frames
    let frames_per_second = 1000 / FRAME_DURATION_MS;
    let total_frames = (frames_per_second * DURATION_SECS as u32) as usize;
    let samples_per_frame = SAMPLE_RATE as usize / frames_per_second as usize;
    
    tracing::info!("🎵 Sending {} frames of 440Hz tone", total_frames);
    
    // Spawn receiver task - made resilient to timeouts
    let capture_clone = capture.clone();
    let receiver = tokio::spawn(async move {
        let mut count = 0;
        let mut consecutive_timeouts = 0;
        
        loop {
            match timeout(Duration::from_millis(100), audio_rx.recv()).await {
                Ok(Some(frame)) => {
                    capture_clone.lock().await.add_frame(&frame);
                    count += 1;
                    consecutive_timeouts = 0;
                }
                Ok(None) => {
                    // Channel closed
                    tracing::info!("UAC receiver channel closed after {} frames", count);
                    break;
                }
                Err(_) => {
                    // Timeout - keep trying during call setup
                    consecutive_timeouts += 1;
                    if consecutive_timeouts > 60 {  // 6 seconds of no audio
                        tracing::info!("UAC receiver timeout after {} frames", count);
                        break;
                    }
                }
            }
        }
        tracing::info!("UAC received {} frames", count);
    });
    
    // Send frames
    for i in 0..total_frames {
        let start = i * samples_per_frame;
        let end = std::cmp::min(start + samples_per_frame, tone_440.len());
        
        if start >= tone_440.len() {
            break;
        }
        
        let frame_samples = tone_440[start..end].to_vec();
        let frame = AudioFrame {
            samples: frame_samples,
            sample_rate: SAMPLE_RATE,
            channels: 1,
            duration: Duration::from_millis(FRAME_DURATION_MS as u64),
            timestamp: (i * samples_per_frame) as u32,
        };
        
        audio_tx.send(frame).await?;
        sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
    }
    
    tracing::info!("✅ Finished sending audio");
    
    // Wait a bit for remaining frames
    sleep(Duration::from_secs(1)).await;
    
    // Stop receiver
    receiver.abort();
    
    // Save captured audio
    capture.lock().await.save()?;
    
    // Terminate call
    tracing::info!("🧹 Terminating call");
    call.hangup().await?;
    
    sleep(Duration::from_secs(1)).await;
    
    tracing::info!("✅ UAC done");
    tracing::info!("Check audio files in: {}", output_dir.display());
    
    Ok(())
}