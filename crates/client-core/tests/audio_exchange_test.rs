//! Integration test for bidirectional audio exchange between two clients
//! 
//! This test creates two clients that exchange audio tones:
//! - Client A sends a 440Hz tone (A4 note)
//! - Client B sends a 880Hz tone (A5 note)
//! 
//! Each client saves:
//! - input.wav: The audio they're sending
//! - output.wav: The audio they received from the other client

use rvoip_client_core::{Client, ClientBuilder, ClientEventHandler, CallAction};
use rvoip_client_core::events::{IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo};
use rvoip_session_core::api::types::AudioFrame;
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::{sleep, Duration, timeout};
use hound::{WavWriter, WavSpec};
use async_trait::async_trait;
use uuid::Uuid;

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

/// Test event handler that auto-answers incoming calls
struct TestEventHandler {
    incoming_calls: mpsc::Sender<Uuid>,
}

#[async_trait]
impl ClientEventHandler for TestEventHandler {
    async fn on_incoming_call(&self, info: IncomingCallInfo) -> CallAction {
        tracing::info!("📞 Incoming call from {} to {}", info.caller_uri, info.callee_uri);
        
        // Send call ID to test
        let _ = self.incoming_calls.send(info.call_id).await;
        
        // Auto-answer the call
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, info: CallStatusInfo) {
        tracing::info!("📞 Call {} state changed to {:?}", info.call_id, info.new_state);
    }
    
    async fn on_registration_status_changed(&self, info: RegistrationStatusInfo) {
        tracing::debug!("Registration status changed to {:?}", info.status);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Run with: cargo test audio_exchange_test -- --ignored --nocapture
async fn test_audio_exchange_between_clients() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging for debugging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip=info,audio_exchange_test=info,rvoip_client_core=debug,rvoip_session_core=debug")
        .try_init();
    
    // Use absolute paths for the test directories
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/audio_exchange_test");
    let client_a_dir = test_dir.join("client_a");
    let client_b_dir = test_dir.join("client_b");
    
    // Ensure directories exist
    std::fs::create_dir_all(&client_a_dir)?;
    std::fs::create_dir_all(&client_b_dir)?;
    
    // Generate tones
    tracing::info!("🎵 Generating test tones...");
    let tone_440hz = generate_tone(440.0, SAMPLE_RATE, DURATION_SECS);
    let tone_880hz = generate_tone(880.0, SAMPLE_RATE, DURATION_SECS);
    
    // Save input files
    save_wav(&client_a_dir.join("input.wav"), &tone_440hz, SAMPLE_RATE)?;
    save_wav(&client_b_dir.join("input.wav"), &tone_880hz, SAMPLE_RATE)?;
    tracing::info!("💾 Saved input tones to WAV files");
    
    // Create clients
    tracing::info!("🔧 Creating clients...");
    let client_a = ClientBuilder::new()
        .local_address("127.0.0.1:25060".parse()?)
        .user_agent("TestClientA/1.0")
        .build()
        .await?;
    
    let client_b = ClientBuilder::new()
        .local_address("127.0.0.1:25061".parse()?)
        .user_agent("TestClientB/1.0")
        .build()
        .await?;
    
    // Start clients
    client_a.start().await?;
    client_b.start().await?;
    tracing::info!("✅ Clients started");
    
    // Set up Client B event handler to auto-answer calls
    let (incoming_tx, mut incoming_rx) = mpsc::channel(10);
    let handler_b = Arc::new(TestEventHandler {
        incoming_calls: incoming_tx,
    });
    client_b.set_event_handler(handler_b).await;
    
    // Client A makes call to Client B
    tracing::info!("📞 Client A calling Client B...");
    let call_id_a = client_a.make_call(
        "sip:client_a@127.0.0.1:25060".to_string(),
        "sip:client_b@127.0.0.1:25061".to_string(),
        None, // SDP will be generated
    ).await?;
    
    // Wait for Client B to receive and auto-answer the call
    let call_id_b = timeout(Duration::from_secs(10), incoming_rx.recv())
        .await?
        .ok_or("No incoming call received")?;
    
    tracing::info!("✅ Call established - Client A ID: {}, Client B ID: {}", call_id_a, call_id_b);
    
    // Wait for call to be fully connected and SDP to be exchanged
    sleep(Duration::from_secs(2)).await;
    
    // Now we need to establish media flow
    // Get media info for both clients to find their RTP ports
    tracing::info!("🔗 Getting media info and establishing bidirectional media flow...");
    
    let media_info_a = client_a.get_call_media_info(&call_id_a).await?;
    let media_info_b = client_b.get_call_media_info(&call_id_b).await?;
    
    tracing::info!("Client A RTP port: {:?}", media_info_a.local_rtp_port);
    tracing::info!("Client B RTP port: {:?}", media_info_b.local_rtp_port);
    
    // Establish bidirectional media flow
    if let Some(port_b) = media_info_b.local_rtp_port {
        let remote_addr_b = format!("127.0.0.1:{}", port_b);
        tracing::info!("Establishing media flow from A to B: {}", remote_addr_b);
        match client_a.establish_media(&call_id_a, &remote_addr_b).await {
            Ok(_) => tracing::info!("✅ Client A: Established media flow to {}", remote_addr_b),
            Err(e) => tracing::error!("❌ Client A: Failed to establish media flow: {}", e),
        }
    } else {
        tracing::error!("❌ Client B has no RTP port!");
    }
    
    if let Some(port_a) = media_info_a.local_rtp_port {
        let remote_addr_a = format!("127.0.0.1:{}", port_a);
        tracing::info!("Establishing media flow from B to A: {}", remote_addr_a);
        match client_b.establish_media(&call_id_b, &remote_addr_a).await {
            Ok(_) => tracing::info!("✅ Client B: Established media flow to {}", remote_addr_a),
            Err(e) => tracing::error!("❌ Client B: Failed to establish media flow: {}", e),
        }
    } else {
        tracing::error!("❌ Client A has no RTP port!");
    }
    
    // Give media paths time to stabilize
    sleep(Duration::from_secs(1)).await;
    
    // Set up audio capture for both clients
    let capture_a = Arc::new(Mutex::new(AudioCapture::new(client_a_dir.join("output.wav"))));
    let capture_b = Arc::new(Mutex::new(AudioCapture::new(client_b_dir.join("output.wav"))));
    
    // Create tasks for sending and receiving audio
    let send_receive_tasks = vec![
        // Client A: Send 440Hz and receive from B
        tokio::spawn({
            let client = client_a.clone();
            let call_id = call_id_a;
            let tone = tone_440hz.clone();
            let capture = capture_a.clone();
            async move {
                // Subscribe to incoming audio first
                let mut subscriber = match client.subscribe_to_audio_frames(&call_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Client A: Failed to subscribe to audio: {}", e);
                        return;
                    }
                };
                
                // Spawn receiver task
                let capture_clone = capture.clone();
                let receiver_task = tokio::spawn(async move {
                    let mut frame_count = 0;
                    let max_frames = (DURATION_SECS * 1000.0 / FRAME_DURATION_MS as f32) as usize;
                    
                    loop {
                        // Use tokio timeout with async recv
                        match timeout(Duration::from_millis(100), subscriber.recv()).await {
                            Ok(Some(frame)) => {
                                frame_count += 1;
                                capture_clone.lock().await.add_frame(&frame);
                                
                                if frame_count % 50 == 0 {
                                    tracing::info!("Client A received {} frames", frame_count);
                                }
                                
                                if frame_count >= max_frames {
                                    break;
                                }
                            }
                            Ok(None) => break, // Channel closed
                            Err(_) => continue, // Timeout, continue waiting
                        }
                    }
                    tracing::info!("Client A: Received {} total frames", frame_count);
                });
                
                // Send audio frames
                let samples_per_frame = (SAMPLE_RATE * FRAME_DURATION_MS / 1000) as usize;
                let mut offset = 0;
                let mut frame_count = 0;
                
                while offset < tone.len() {
                    let end = (offset + samples_per_frame).min(tone.len());
                    let frame_samples = tone[offset..end].to_vec();
                    
                    let frame = AudioFrame::new(
                        frame_samples,
                        SAMPLE_RATE,
                        1,
                        (frame_count * FRAME_DURATION_MS) as u32,
                    );
                    
                    if let Err(e) = client.send_audio_frame(&call_id, frame).await {
                        tracing::error!("Client A: Failed to send audio frame: {}", e);
                        break;
                    }
                    
                    frame_count += 1;
                    if frame_count % 50 == 0 {
                        tracing::info!("Client A sent {} frames", frame_count);
                    }
                    
                    offset += samples_per_frame;
                    sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
                }
                
                tracing::info!("Client A: Sent {} total frames", frame_count);
                
                // Wait for receiver to finish
                let _ = timeout(Duration::from_secs(2), receiver_task).await;
            }
        }),
        
        // Client B: Send 880Hz and receive from A
        tokio::spawn({
            let client = client_b.clone();
            let call_id = call_id_b;
            let tone = tone_880hz.clone();
            let capture = capture_b.clone();
            async move {
                // Subscribe to incoming audio first
                let mut subscriber = match client.subscribe_to_audio_frames(&call_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Client B: Failed to subscribe to audio: {}", e);
                        return;
                    }
                };
                
                // Spawn receiver task
                let capture_clone = capture.clone();
                let receiver_task = tokio::spawn(async move {
                    let mut frame_count = 0;
                    let max_frames = (DURATION_SECS * 1000.0 / FRAME_DURATION_MS as f32) as usize;
                    
                    loop {
                        // Use tokio timeout with async recv
                        match timeout(Duration::from_millis(100), subscriber.recv()).await {
                            Ok(Some(frame)) => {
                                frame_count += 1;
                                capture_clone.lock().await.add_frame(&frame);
                                
                                if frame_count % 50 == 0 {
                                    tracing::info!("Client B received {} frames", frame_count);
                                }
                                
                                if frame_count >= max_frames {
                                    break;
                                }
                            }
                            Ok(None) => break, // Channel closed
                            Err(_) => continue, // Timeout, continue waiting
                        }
                    }
                    tracing::info!("Client B: Received {} total frames", frame_count);
                });
                
                // Send audio frames
                let samples_per_frame = (SAMPLE_RATE * FRAME_DURATION_MS / 1000) as usize;
                let mut offset = 0;
                let mut frame_count = 0;
                
                while offset < tone.len() {
                    let end = (offset + samples_per_frame).min(tone.len());
                    let frame_samples = tone[offset..end].to_vec();
                    
                    let frame = AudioFrame::new(
                        frame_samples,
                        SAMPLE_RATE,
                        1,
                        (frame_count * FRAME_DURATION_MS) as u32,
                    );
                    
                    if let Err(e) = client.send_audio_frame(&call_id, frame).await {
                        tracing::error!("Client B: Failed to send audio frame: {}", e);
                        break;
                    }
                    
                    frame_count += 1;
                    if frame_count % 50 == 0 {
                        tracing::info!("Client B sent {} frames", frame_count);
                    }
                    
                    offset += samples_per_frame;
                    sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
                }
                
                tracing::info!("Client B: Sent {} total frames", frame_count);
                
                // Wait for receiver to finish
                let _ = timeout(Duration::from_secs(2), receiver_task).await;
            }
        }),
    ];
    
    // Wait for all tasks to complete
    for task in send_receive_tasks {
        let _ = task.await;
    }
    
    // Save captured audio
    tracing::info!("💾 Saving captured audio...");
    capture_a.lock().await.save()?;
    capture_b.lock().await.save()?;
    
    // Cleanup
    client_a.hangup_call(&call_id_a).await?;
    sleep(Duration::from_millis(500)).await;
    client_a.stop().await?;
    client_b.stop().await?;
    
    // Verify files were created
    let files = vec![
        (client_a_dir.join("input.wav"), "Client A input.wav"),
        (client_a_dir.join("output.wav"), "Client A output.wav"),
        (client_b_dir.join("input.wav"), "Client B input.wav"),
        (client_b_dir.join("output.wav"), "Client B output.wav"),
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
    tracing::info!("  - tests/audio_exchange_test/client_a/input.wav (440Hz tone)");
    tracing::info!("  - tests/audio_exchange_test/client_a/output.wav (should contain 880Hz from Client B)");
    tracing::info!("  - tests/audio_exchange_test/client_b/input.wav (880Hz tone)");
    tracing::info!("  - tests/audio_exchange_test/client_b/output.wav (should contain 440Hz from Client A)");
    
    Ok(())
}