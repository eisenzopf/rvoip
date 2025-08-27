//! UAS (User Agent Server) example that receives a call and sends 880Hz tone

use rvoip_session_core::api::uas::{SimpleUasServer, UasCallHandler, UasCallDecision};
use rvoip_session_core::api::types::{AudioFrame, IncomingCall, CallState, CallSession, SessionId};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{sleep, Duration, timeout};
use hound::{WavWriter, WavSpec};
use async_trait::async_trait;

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

#[derive(Debug, Clone)]
struct TestHandler {
    call_tx: mpsc::Sender<IncomingCall>,
}

#[async_trait]
impl UasCallHandler for TestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> UasCallDecision {
        tracing::info!("📞 Incoming call from {} to {}", call.from, call.to);
        let _ = self.call_tx.send(call).await;
        UasCallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, _call: CallSession) {}
    async fn on_call_ended(&self, _call: CallSession, _reason: String) {}
    async fn on_dtmf_received(&self, _session_id: SessionId, _digit: char) {}
    async fn on_quality_update(&self, _session_id: SessionId, _mos_score: f32) {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();
    
    tracing::info!("🎵 UAS starting on port 5061...");
    
    // Create output directory
    let output_dir = PathBuf::from("examples/api_uac_uas/uas_output");
    std::fs::create_dir_all(&output_dir)?;
    
    // Generate 880Hz tone
    let tone_880 = generate_tone(880.0, SAMPLE_RATE, DURATION_SECS);
    save_wav(&output_dir.join("input.wav"), &tone_880, SAMPLE_RATE)?;
    tracing::info!("💾 Saved 880Hz input tone");
    
    // Create channel for incoming calls
    let (call_tx, mut call_rx) = mpsc::channel(10);
    
    // Create UAS server
    let handler = TestHandler { call_tx };
    let uas_server = SimpleUasServer::new(
        "127.0.0.1:5061",
        "sip:uas@127.0.0.1:5061",
        handler,
    ).await?;
    
    tracing::info!("✅ UAS server started");
    tracing::info!("⏳ Waiting for incoming call...");
    
    // Wait for incoming call
    let incoming = timeout(Duration::from_secs(10), call_rx.recv())
        .await?
        .ok_or("No incoming call received")?;
    
    tracing::info!("✅ Received incoming call: {}", incoming.id);
    
    // Get the call handle
    let mut call = uas_server.get_call(&incoming.id)
        .ok_or("Call not found")?;
    
    // Wait for call to be active
    loop {
        if call.state() == CallState::Active {
            tracing::info!("✅ Call is active");
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for media to be ready
    sleep(Duration::from_secs(2)).await;
    
    // Set up audio channels
    tracing::info!("📻 Setting up audio channels");
    let (audio_tx, mut audio_rx) = call.audio_channels().await?;
    
    // Set up audio capture
    let capture = Arc::new(Mutex::new(AudioCapture::new(output_dir.join("output.wav"))));
    
    // Send audio frames
    let frames_per_second = 1000 / FRAME_DURATION_MS;
    let total_frames = (frames_per_second * DURATION_SECS as u32) as usize;
    let samples_per_frame = SAMPLE_RATE as usize / frames_per_second as usize;
    
    tracing::info!("🎵 Sending {} frames of 880Hz tone", total_frames);
    
    // Spawn receiver task
    let capture_clone = capture.clone();
    let receiver = tokio::spawn(async move {
        let mut count = 0;
        while let Ok(Some(frame)) = timeout(Duration::from_millis(100), audio_rx.recv()).await {
            capture_clone.lock().await.add_frame(&frame);
            count += 1;
        }
        tracing::info!("UAS received {} frames", count);
    });
    
    // Send frames
    for i in 0..total_frames {
        let start = i * samples_per_frame;
        let end = std::cmp::min(start + samples_per_frame, tone_880.len());
        
        if start >= tone_880.len() {
            break;
        }
        
        let frame_samples = tone_880[start..end].to_vec();
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
    
    // Wait for call to end (UAC will terminate)
    sleep(Duration::from_secs(2)).await;
    
    tracing::info!("✅ UAS done");
    tracing::info!("Check audio files in: {}", output_dir.display());
    
    Ok(())
}