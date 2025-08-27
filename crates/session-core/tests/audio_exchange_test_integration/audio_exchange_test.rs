//! Integration test for bidirectional audio exchange between two sessions at the session-core layer
//! 
//! This test creates two sessions that exchange audio tones:
//! - Session A sends a 440Hz tone (A4 note)
//! - Session B sends a 880Hz tone (A5 note)
//! 
//! Each session saves:
//! - input.wav: The audio they're sending
//! - output.wav: The audio they received from the other session

use rvoip_session_core::{
    SessionManagerBuilder,
    SessionControl,
    MediaControl,
    api::{
        types::{AudioFrame, CallSession, SessionId, IncomingCall, CallDecision, CallState},
        handlers::CallHandler,
    },
};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{sleep, Duration, timeout};
use hound::{WavWriter, WavSpec};
use async_trait::async_trait;

/// Test configuration
const SAMPLE_RATE: u32 = 8000; // G.711 uses 8kHz
const CHANNELS: u16 = 1; // Mono
const BITS_PER_SAMPLE: u16 = 16; // 16-bit PCM
const DURATION_SECS: f32 = 5.0;
const FRAME_DURATION_MS: u32 = 20; // 20ms frames for VoIP

/// Generate a sine wave tone at the specified frequency
fn generate_tone(frequency: f32, sample_rate: u32, duration: f32) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * PI * frequency * t).sin();
        // Convert to i16 range (-32768 to 32767), reduced amplitude to avoid clipping
        let sample_i16 = (sample * 16384.0) as i16;
        samples.push(sample_i16);
    }
    
    samples
}

/// Save audio samples to a WAV file
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

/// Test handler that auto-accepts incoming calls and notifies when calls arrive
#[derive(Debug)]
struct TestHandler {
    incoming_calls: mpsc::Sender<(SessionId, IncomingCall)>,
}

#[async_trait]
impl CallHandler for TestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        tracing::info!("📞 Incoming call from {} to {}", call.from, call.to);
        
        // Send call info to test
        let _ = self.incoming_calls.send((call.id.clone(), call.clone())).await;
        
        // Auto-accept - let session-core generate proper SDP
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, session: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        tracing::info!("📞 Call {} established", session.id);
        // Session-core handles SDP parsing automatically
        // Media flow will be established by the main test logic
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        tracing::info!("📞 Call {} ended: {}", session.id, reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging for debugging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip=info,audio_exchange_test=info,rvoip_media_core::relay::controller=debug")
        .try_init();
    
    // Use absolute paths for the test directories
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/audio_exchange_test");
    let session_a_dir = test_dir.join("session_a");
    let session_b_dir = test_dir.join("session_b");
    
    // Ensure directories exist
    std::fs::create_dir_all(&session_a_dir)?;
    std::fs::create_dir_all(&session_b_dir)?;
    
    // Generate tones
    tracing::info!("🎵 Generating test tones...");
    let tone_440hz = generate_tone(440.0, SAMPLE_RATE, DURATION_SECS);
    let tone_880hz = generate_tone(880.0, SAMPLE_RATE, DURATION_SECS);
    
    // Save input files
    save_wav(&session_a_dir.join("input.wav"), &tone_440hz, SAMPLE_RATE)?;
    save_wav(&session_b_dir.join("input.wav"), &tone_880hz, SAMPLE_RATE)?;
    tracing::info!("💾 Saved input tones to WAV files");
    
    // Create handler for Session A
    let (incoming_tx_a, mut incoming_rx_a) = mpsc::channel(10);
    let handler_a = Arc::new(TestHandler {
        incoming_calls: incoming_tx_a,
    });
    
    // Create handler for Session B
    let (incoming_tx_b, mut incoming_rx_b) = mpsc::channel(10);
    let handler_b = Arc::new(TestHandler {
        incoming_calls: incoming_tx_b,
    });
    
    // Create session coordinators
    tracing::info!("🔧 Creating session coordinators...");
    
    // Session A coordinator (caller)
    let coordinator_a = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:session_a@127.0.0.1:5060")
        .with_local_bind_addr("127.0.0.1:5060".parse().unwrap())
        .with_media_ports(40000, 41000)
        .with_handler(handler_a.clone())
        .build()
        .await?;
    
    // Session B coordinator (receiver) with handler
    let coordinator_b = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:session_b@127.0.0.1:5061")
        .with_local_bind_addr("127.0.0.1:5061".parse().unwrap())
        .with_media_ports(42000, 43000)
        .with_handler(handler_b.clone())
        .build()
        .await?;
    
    // Coordinators are automatically started by SessionManagerBuilder
    tracing::info!("✅ Session coordinators started");
    
    // Session A creates an outgoing call
    tracing::info!("📞 Session A calling Session B...");
    
    // First prepare the call to get SDP
    let prepared_call = coordinator_a.prepare_outgoing_call(
        "sip:session_a@127.0.0.1:5060",
        "sip:session_b@127.0.0.1:5061",
    ).await?;
    
    tracing::info!("Prepared call with RTP port: {}", prepared_call.local_rtp_port);
    
    // Now initiate the call
    let session_a = coordinator_a.initiate_prepared_call(&prepared_call).await?;
    
    // Also establish media flow for session A once we get the response
    // This is done automatically when the 200 OK is received
    
    let session_id_a = session_a.id.clone();
    tracing::info!("✅ Session A created: {}", session_id_a);
    
    // Wait for Session B to receive the incoming call
    let (session_id_b, _incoming_call) = timeout(Duration::from_secs(5), incoming_rx_b.recv())
        .await?
        .ok_or("No incoming call received")?;
    
    tracing::info!("✅ Session B received incoming call: {}", session_id_b);
    
    // Wait for sessions to be fully established
    sleep(Duration::from_secs(2)).await;
    
    // Now establish media flow for session A as well (it needs to know where to send RTP)
    // Session B already established its flow in the on_call_established handler
    // For session A, we need to get the media info and establish flow to session B
    tracing::info!("Establishing bidirectional media flow...");
    
    // Get media info for both sessions to find their RTP ports
    let session_a_media = SessionControl::get_media_info(&coordinator_a, &session_id_a).await.ok().flatten();
    let session_b_media = SessionControl::get_media_info(&coordinator_b, &session_id_b).await.ok().flatten();
    
    if let (Some(media_a), Some(media_b)) = (session_a_media, session_b_media) {
        tracing::info!("Session A RTP port: {:?}", media_a.local_rtp_port);
        tracing::info!("Session B RTP port: {:?}", media_b.local_rtp_port);
        
        // The automatic establish_media_flow in event_handler.rs handles this
        // No need for explicit calls - they were redundant
        tracing::info!("Media flow is established automatically by the event handler");
    } else {
        tracing::error!("Could not get media info for sessions");
    }
    
    // Set up audio capture for both sessions
    let capture_a = Arc::new(Mutex::new(AudioCapture::new(session_a_dir.join("output.wav"))));
    let capture_b = Arc::new(Mutex::new(AudioCapture::new(session_b_dir.join("output.wav"))));
    
    // Start audio transmission for both sessions
    tracing::info!("Starting audio transmission for both sessions");
    match coordinator_a.start_audio_transmission(&session_id_a).await {
        Ok(_) => tracing::info!("✅ Started audio transmission for session A"),
        Err(e) => tracing::error!("Failed to start audio transmission for session A: {}", e),
    }
    match coordinator_b.start_audio_transmission(&session_id_b).await {
        Ok(_) => tracing::info!("✅ Started audio transmission for session B"),
        Err(e) => tracing::error!("Failed to start audio transmission for session B: {}", e),
    }
    
    // Wait for both sessions to be fully established
    tracing::info!("⏳ Waiting for sessions to be fully active...");
    let mut retries = 0;
    loop {
        let session_a_info = SessionControl::get_session(&coordinator_a, &session_id_a).await?;
        let session_b_info = SessionControl::get_session(&coordinator_b, &session_id_b).await?;
        
        if let (Some(info_a), Some(info_b)) = (session_a_info, session_b_info) {
            if info_a.state == CallState::Active && info_b.state == CallState::Active {
                tracing::info!("✅ Both sessions are now active");
                break;
            }
        }
        
        retries += 1;
        if retries > 50 { // 5 seconds timeout
            return Err("Sessions did not become active within timeout".into());
        }
        
        sleep(Duration::from_millis(100)).await;
    }
    
    // Give additional time for media paths to stabilize
    tracing::info!("⏳ Waiting for media paths to stabilize...");
    sleep(Duration::from_millis(500)).await;
    
    // IMPORTANT: Give RTP event handlers and decoders time to fully initialize
    // This prevents packet loss due to handlers not being ready
    tracing::info!("⏳ Waiting for RTP receivers and decoders to initialize...");
    sleep(Duration::from_secs(2)).await;
    
    // Subscribe to audio frames for both sessions after media paths are established
    tracing::info!("📻 Subscribing to audio frames for both sessions");
    let mut subscriber_a = coordinator_a.subscribe_to_audio_frames(&session_id_a).await?;
    let mut subscriber_b = coordinator_b.subscribe_to_audio_frames(&session_id_b).await?;
    
    // Create a channel to signal when to start sending audio
    let (start_tx_a, mut start_rx_a) = mpsc::channel(1);
    let (start_tx_b, mut start_rx_b) = mpsc::channel(1);
    
    // Signal that both calls are established and ready for media
    tracing::info!("📞 Both calls established, signaling to start audio transmission");
    start_tx_a.send(()).await?;
    start_tx_b.send(()).await?;
    
    // Create tasks for sending and receiving audio
    // Session A: Send 440Hz tone and receive from B
    let send_task_a = {
        let session_id = session_id_a.clone();
        let coordinator = coordinator_a.clone();
        let tone_samples = tone_440hz.clone();
        
        tokio::spawn(async move {
            // Wait for the signal to start sending
            tracing::info!("🎵 Session A: Waiting for start signal...");
            if start_rx_a.recv().await.is_none() {
                tracing::error!("Session A: Start signal channel closed");
                return;
            }
            
            tracing::info!("🎵 Session A: Starting to send 440Hz tone");
            let samples_per_frame = (SAMPLE_RATE as usize * FRAME_DURATION_MS as usize) / 1000; // 160 samples for 20ms at 8kHz
            let total_frames = tone_samples.len() / samples_per_frame;
            tracing::info!("🎵 Session A: Will send {} frames ({} samples per frame)", total_frames, samples_per_frame);
            
            for frame_idx in 0..total_frames {
                let start = frame_idx * samples_per_frame;
                let end = start + samples_per_frame;
                
                if end > tone_samples.len() {
                    break;
                }
                
                let frame_samples = tone_samples[start..end].to_vec();
                let audio_frame = AudioFrame::new(
                    frame_samples,
                    SAMPLE_RATE,
                    CHANNELS as u8,
                    (frame_idx as u32) * samples_per_frame as u32,
                );
                
                // Send the audio frame to media-core for encoding and transmission
                match MediaControl::send_audio_frame(&coordinator, &session_id, audio_frame).await {
                    Ok(_) => {
                        if frame_idx == 0 || frame_idx % 50 == 0 || frame_idx == 99 || frame_idx == 100 || frame_idx == 101 {
                            tracing::info!("🎵 Session A: Successfully sent frame {} of {}", frame_idx + 1, total_frames);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to send audio frame {}: {}", frame_idx, e);
                        break;
                    }
                }
                
                // Send frames at 20ms intervals
                tokio::time::sleep(tokio::time::Duration::from_millis(FRAME_DURATION_MS as u64)).await;
            }
            
            tracing::info!("🎵 Session A: Finished sending {} frames", total_frames);
        })
    };
    
    // Session B: Send 880Hz tone
    let send_task_b = {
        let session_id = session_id_b.clone();
        let coordinator = coordinator_b.clone();
        let tone_samples = tone_880hz.clone();
        
        tokio::spawn(async move {
            // Wait for the signal to start sending
            tracing::info!("🎵 Session B: Waiting for start signal...");
            if start_rx_b.recv().await.is_none() {
                tracing::error!("Session B: Start signal channel closed");
                return;
            }
            
            tracing::info!("🎵 Session B: Starting to send 880Hz tone");
            let samples_per_frame = (SAMPLE_RATE as usize * FRAME_DURATION_MS as usize) / 1000; // 160 samples for 20ms at 8kHz
            let total_frames = tone_samples.len() / samples_per_frame;
            tracing::info!("🎵 Session B: Will send {} frames ({} samples per frame)", total_frames, samples_per_frame);
            
            for frame_idx in 0..total_frames {
                let start = frame_idx * samples_per_frame;
                let end = start + samples_per_frame;
                
                if end > tone_samples.len() {
                    break;
                }
                
                let frame_samples = tone_samples[start..end].to_vec();
                let audio_frame = AudioFrame::new(
                    frame_samples,
                    SAMPLE_RATE,
                    CHANNELS as u8,
                    (frame_idx as u32) * samples_per_frame as u32,
                );
                
                // Send the audio frame to media-core for encoding and transmission
                match MediaControl::send_audio_frame(&coordinator, &session_id, audio_frame).await {
                    Ok(_) => {
                        if frame_idx == 0 || frame_idx % 50 == 0 || frame_idx == 99 || frame_idx == 100 || frame_idx == 101 {
                            tracing::info!("🎵 Session B: Successfully sent frame {} of {}", frame_idx + 1, total_frames);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to send audio frame {}: {}", frame_idx, e);
                        break;
                    }
                }
                
                // Send frames at 20ms intervals
                tokio::time::sleep(tokio::time::Duration::from_millis(FRAME_DURATION_MS as u64)).await;
            }
            
            tracing::info!("🎵 Session B: Finished sending {} frames", total_frames);
        })
    };
    
    // Create tasks for receiving audio
    let receive_task_a = {
        let capture = capture_a.clone();
        tokio::spawn(async move {
            let mut frame_count = 0;
            let max_frames = (DURATION_SECS * 1000.0 / FRAME_DURATION_MS as f32) as usize;
            let mut no_data_count = 0;
            
            while frame_count < max_frames {
                // Use tokio::time::timeout for each receive
                match tokio::time::timeout(tokio::time::Duration::from_millis(100), subscriber_a.recv()).await {
                    Ok(Some(frame)) => {
                        frame_count += 1;
                        no_data_count = 0;
                        capture.lock().await.add_frame(&frame);
                        
                        if frame_count % 50 == 0 || frame_count == 100 {
                            tracing::info!("Session A received {} frames", frame_count);
                        }
                    }
                    Ok(None) => {
                        tracing::error!("Session A: Channel closed");
                        break;
                    }
                    Err(_) => {
                        // Timeout - no frame within 100ms
                        no_data_count += 1;
                        
                        if no_data_count > 100 { // 10 seconds of no data
                            tracing::warn!("Session A: No frames received for 10 seconds, stopping");
                            break;
                        }
                    }
                }
            }
            tracing::info!("Session A: Received {} total frames", frame_count);
        })
    };
    
    let receive_task_b = {
        let capture = capture_b.clone();
        tokio::spawn(async move {
            let mut frame_count = 0;
            let max_frames = (DURATION_SECS * 1000.0 / FRAME_DURATION_MS as f32) as usize;
            let mut no_data_count = 0;
            
            while frame_count < max_frames {
                // Use tokio::time::timeout for each receive
                match tokio::time::timeout(tokio::time::Duration::from_millis(100), subscriber_b.recv()).await {
                    Ok(Some(frame)) => {
                        frame_count += 1;
                        no_data_count = 0;
                        capture.lock().await.add_frame(&frame);
                        
                        if frame_count % 50 == 0 || frame_count == 100 {
                            tracing::info!("Session B received {} frames", frame_count);
                        }
                    }
                    Ok(None) => {
                        tracing::error!("Session B: Channel closed");
                        break;
                    }
                    Err(_) => {
                        // Timeout - no frame within 100ms
                        no_data_count += 1;
                        
                        if no_data_count > 100 { // 10 seconds of no data
                            tracing::warn!("Session B: No frames received for 10 seconds, stopping");
                            break;
                        }
                    }
                }
            }
            tracing::info!("Session B: Received {} total frames", frame_count);
        })
    };
    
    let receive_tasks = vec![receive_task_a, receive_task_b];
    
    // Wait for all tasks to complete or timeout
    let all_tasks = vec![send_task_a, send_task_b];
    let all_tasks: Vec<_> = all_tasks.into_iter().chain(receive_tasks).collect();
    let _ = timeout(Duration::from_secs(DURATION_SECS as u64 + 2), futures::future::join_all(all_tasks)).await;
    
    // Stop audio transmission
    let _ = coordinator_a.stop_audio_transmission(&session_id_a).await;
    let _ = coordinator_b.stop_audio_transmission(&session_id_b).await;
    
    // Save captured audio
    tracing::info!("💾 Saving captured audio...");
    capture_a.lock().await.save()?;
    capture_b.lock().await.save()?;
    
    // Wait 10 seconds as requested to let the call run
    tracing::info!("⏳ Letting the call run for 10 seconds...");
    sleep(Duration::from_secs(10)).await;
    
    // Get RTP statistics
    if let Ok(Some(rtp_stats_a)) = MediaControl::get_rtp_statistics(&coordinator_a, &session_id_a).await {
        tracing::info!("📊 Session A RTP stats:");
        tracing::info!("  Packets sent: {}", rtp_stats_a.packets_sent);
        tracing::info!("  Packets received: {}", rtp_stats_a.packets_received);
        tracing::info!("  Packets lost: {}", rtp_stats_a.packets_lost);
    }
    
    if let Ok(Some(rtp_stats_b)) = MediaControl::get_rtp_statistics(&coordinator_b, &session_id_b).await {
        tracing::info!("📊 Session B RTP stats:");
        tracing::info!("  Packets sent: {}", rtp_stats_b.packets_sent);
        tracing::info!("  Packets received: {}", rtp_stats_b.packets_received);
        tracing::info!("  Packets lost: {}", rtp_stats_b.packets_lost);
    }
    
    // Cleanup
    tracing::info!("🧹 Terminating sessions...");
    coordinator_a.terminate_session(&session_id_a).await?;
    sleep(Duration::from_millis(500)).await;
    
    coordinator_a.stop().await?;
    coordinator_b.stop().await?;
    
    // Verify files were created
    let files = vec![
        (session_a_dir.join("input.wav"), "Session A input.wav"),
        (session_a_dir.join("output.wav"), "Session A output.wav"),
        (session_b_dir.join("input.wav"), "Session B input.wav"),
        (session_b_dir.join("output.wav"), "Session B output.wav"),
    ];
    
    for (path, name) in &files {
        if !path.exists() {
            return Err(format!("{} not found", name).into());
        }
        let metadata = std::fs::metadata(path)?;
        tracing::info!("{}: {} bytes", name, metadata.len());
    }
    
    tracing::info!("✅ Test completed successfully!");
    tracing::info!("Check the following files:");
    tracing::info!("  - tests/audio_exchange_test/session_a/input.wav (440Hz tone)");
    tracing::info!("  - tests/audio_exchange_test/session_a/output.wav (should contain 880Hz from Session B)");
    tracing::info!("  - tests/audio_exchange_test/session_b/input.wav (880Hz tone)");
    tracing::info!("  - tests/audio_exchange_test/session_b/output.wav (should contain 440Hz from Session A)");
    
    Ok(())
}