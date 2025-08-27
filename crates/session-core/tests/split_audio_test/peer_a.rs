//! Peer A (UAC) - Makes the call and sends 440Hz tone
//! 
//! This is a split version of the integration test to run as separate processes

use rvoip_session_core::{
    SessionManagerBuilder,
    SessionControl,
    MediaControl,
    api::{
        types::{AudioFrame, CallSession, CallState, IncomingCall, CallDecision},
        handlers::CallHandler,
    },
};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
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

#[derive(Debug)]
struct DummyHandler;

#[async_trait]
impl CallHandler for DummyHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        // Peer A doesn't receive calls
        CallDecision::Reject("Peer A doesn't accept calls".to_string())
    }
    
    async fn on_call_established(&self, _call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {}
    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();
    
    tracing::info!("🎵 Peer A (UAC) starting on port 5060...");
    
    // Create output directory
    let test_dir = PathBuf::from("tests/split_audio_test");
    let session_a_dir = test_dir.join("peer_a");
    std::fs::create_dir_all(&session_a_dir)?;
    
    // Generate 440Hz tone
    let tone_440 = generate_tone(440.0, SAMPLE_RATE, DURATION_SECS);
    save_wav(&session_a_dir.join("input.wav"), &tone_440, SAMPLE_RATE)?;
    tracing::info!("💾 Saved 440Hz input tone");
    
    // Create Peer A coordinator
    let coordinator_a = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:peer_a@127.0.0.1:5060")
        .with_local_bind_addr("127.0.0.1:5060".parse().unwrap())
        .with_media_ports(40000, 41000)
        .with_handler(Arc::new(DummyHandler))
        .build()
        .await?;
    
    tracing::info!("✅ Peer A coordinator started");
    
    // Give Peer B time to start
    sleep(Duration::from_secs(2)).await;
    
    // Make the call
    tracing::info!("📞 Peer A calling Peer B...");
    
    let prepared_call = coordinator_a.prepare_outgoing_call(
        "sip:peer_a@127.0.0.1:5060",
        "sip:peer_b@127.0.0.1:5061",
    ).await?;
    
    tracing::info!("Prepared call with RTP port: {}", prepared_call.local_rtp_port);
    
    let session_a = coordinator_a.initiate_prepared_call(&prepared_call).await?;
    let session_id_a = session_a.id.clone();
    tracing::info!("✅ Call initiated: {}", session_id_a);
    
    // Wait for call to be active
    loop {
        if let Some(session) = SessionControl::get_session(&coordinator_a, &session_id_a).await? {
            if session.state() == &CallState::Active {
                tracing::info!("✅ Call is active");
                break;
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for media to be ready
    sleep(Duration::from_secs(2)).await;
    
    // Set up audio capture
    let capture_a = Arc::new(Mutex::new(AudioCapture::new(session_a_dir.join("output.wav"))));
    
    // Subscribe to audio frames
    tracing::info!("📻 Subscribing to audio frames");
    let mut audio_rx = MediaControl::subscribe_to_audio_frames(&coordinator_a, &session_id_a).await?;
    
    // Send audio frames
    let frames_per_second = 1000 / FRAME_DURATION_MS;
    let total_frames = (frames_per_second * DURATION_SECS as u32) as usize;
    let samples_per_frame = SAMPLE_RATE as usize / frames_per_second as usize;
    
    tracing::info!("🎵 Sending {} frames of 440Hz tone", total_frames);
    
    // Spawn receiver task
    let capture_clone = capture_a.clone();
    let receiver = tokio::spawn(async move {
        let mut count = 0;
        while let Ok(Some(frame)) = timeout(Duration::from_millis(100), audio_rx.recv()).await {
            capture_clone.lock().await.add_frame(&frame);
            count += 1;
        }
        tracing::info!("Peer A received {} frames", count);
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
        
        MediaControl::send_audio_frame(&coordinator_a, &session_id_a, frame).await?;
        sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
    }
    
    tracing::info!("✅ Finished sending audio");
    
    // Wait a bit for remaining frames
    sleep(Duration::from_secs(1)).await;
    
    // Stop receiver
    receiver.abort();
    
    // Save captured audio
    capture_a.lock().await.save()?;
    
    // Terminate session
    tracing::info!("🧹 Terminating session");
    SessionControl::terminate_session(&coordinator_a, &session_id_a).await?;
    
    // Shutdown
    sleep(Duration::from_secs(1)).await;
    coordinator_a.stop().await?;
    
    tracing::info!("✅ Peer A done");
    tracing::info!("Check audio files in: {}", session_a_dir.display());
    
    Ok(())
}