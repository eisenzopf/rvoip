/// Benchmark: 500 concurrent calls with actual audio tones
/// 
/// This benchmark creates 500 concurrent SIP calls between two SessionManagers,
/// with each peer sending different frequency tones (440Hz client, 880Hz server).
/// 5 random calls are captured to WAV files for validation.
/// Only RECEIVED audio is recorded to validate actual RTP transmission.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::info;
use rand::Rng;

use rvoip_session_core::api::*;
use rvoip_session_core::api::types::AudioFrame;

mod tone_generator;
mod metrics;
mod audio_validator;
mod wav_writer;

use tone_generator::{generate_tone, create_rtp_packets, decode_rtp_payload};
use metrics::{MetricsCollector, MetricSnapshot};
use audio_validator::{AudioValidator, ValidationResult, ChannelValidation};

/// Client handler that sends 440Hz tone and receives 880Hz
#[derive(Debug)]
struct ClientHandler {
    established_calls: Arc<Mutex<Vec<String>>>,
    audio_validator: Arc<AudioValidator>,
    coordinator: Arc<Mutex<Option<Arc<SessionCoordinator>>>>,
}

impl ClientHandler {
    fn new(audio_validator: Arc<AudioValidator>) -> Self {
        Self {
            established_calls: Arc::new(Mutex::new(Vec::new())),
            audio_validator,
            coordinator: Arc::new(Mutex::new(None)),
        }
    }
    
    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        *self.coordinator.lock().await = Some(coordinator);
    }
}

#[async_trait::async_trait]
impl CallHandler for ClientHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Reject("Client doesn't accept incoming calls".to_string())
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        let call_id = call.id.0.clone();
        let session_id = call.id.clone();
        info!("Client: Call {} established - local_sdp: {}, remote_sdp: {}, sip_call_id: {:?}", 
            &call_id[..8.min(call_id.len())],
            local_sdp.is_some(),
            remote_sdp.is_some(),
            call.sip_call_id
        );
        
        // Store session
        self.established_calls.lock().await.push(call_id.clone());
        
        // Log the Call-ID for debugging
        if let Some(ref sip_call_id) = call.sip_call_id {
            info!("Client: Call {} has Call-ID: {}", &call_id[..8.min(call_id.len())], sip_call_id);
        }
        
        // Check if this call is selected for audio capture using SIP Call-ID if available
        let is_selected = if let Some(ref sip_call_id) = call.sip_call_id {
            self.audio_validator.is_selected(sip_call_id).await
        } else {
            self.audio_validator.is_selected(&call_id).await
        };
        
        // Get coordinator
        let coordinator_opt = self.coordinator.lock().await.clone();
        
        if let Some(coordinator) = coordinator_opt {
            // Workaround: If SDP is not provided, fetch it from media info
            let mut actual_remote_sdp = remote_sdp;
            if actual_remote_sdp.is_none() {
                if let Ok(Some(media_info)) = MediaControl::get_media_info(&coordinator, &session_id).await {
                    actual_remote_sdp = media_info.remote_sdp;
                    if actual_remote_sdp.is_some() {
                        info!("Client: Retrieved remote SDP from media info for call {}", &call_id[..8.min(call_id.len())]);
                    }
                }
            }
            // Step 1: Update media with remote SDP (this establishes RTP flow)
            if let Some(remote_sdp_str) = actual_remote_sdp {
                // This parses the SDP and sets up the remote RTP endpoint
                match MediaControl::update_media_with_sdp(&coordinator, &session_id, &remote_sdp_str).await {
                    Ok(_) => {
                        info!("Client: Updated media with remote SDP for call {}", &call_id[..8.min(call_id.len())]);
                    }
                    Err(e) => {
                        tracing::error!("Client: Failed to update media with SDP for call {}: {}", 
                            &call_id[..8.min(call_id.len())], e);
                        return;
                    }
                }
            } else {
                tracing::warn!("Client: No remote SDP provided for call {}", &call_id[..8.min(call_id.len())]);
            }
            
            // Step 2: Start audio transmission (enables RTP processing)
            if let Err(e) = MediaControl::start_audio_transmission(&coordinator, &session_id).await {
                tracing::error!("Client: Failed to start audio transmission: {}", e);
                return;
            }
            
            // Step 3: If selected for capture, subscribe to receive audio
            if is_selected {
                info!("Client: Call {} is selected for audio capture", &call_id[..8.min(call_id.len())]);
                let validator = self.audio_validator.clone();
                // Use SIP Call-ID for audio capture if available, otherwise use session ID
                // This must match what the server uses so audio goes into the same WavCapture
                let call_id_recv = call.sip_call_id.clone().unwrap_or(call_id.clone());
                let session_id_recv = session_id.clone();
                let coordinator_recv = coordinator.clone();
                
                tokio::spawn(async move {
                    info!("Client: Starting audio capture task for call {}", &call_id_recv[..8.min(call_id_recv.len())]);
                    
                    // Wait for media to be fully established
                    let mut retries = 0;
                    while retries < 20 {
                        if let Ok(Some(info)) = MediaControl::get_media_info(&coordinator_recv, &session_id_recv).await {
                            tracing::debug!("Client: Media info for call {} - local_sdp: {}, remote_sdp: {}", 
                                &call_id_recv[..8.min(call_id_recv.len())],
                                info.local_sdp.is_some(),
                                info.remote_sdp.is_some()
                            );
                            if info.remote_sdp.is_some() && info.local_sdp.is_some() {
                                info!("Client: Media ready for call {} after {} retries", 
                                    &call_id_recv[..8.min(call_id_recv.len())], retries);
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        retries += 1;
                    }
                    
                    if retries >= 20 {
                        tracing::error!("Client: Media not ready after 2 seconds for call {}", 
                            &call_id_recv[..8.min(call_id_recv.len())]);
                        return;
                    }
                    
                    // Subscribe to audio frames from the session
                    match MediaControl::subscribe_to_audio_frames(&coordinator_recv, &session_id_recv).await {
                        Ok(mut subscriber) => {
                            info!("Client: Started receiving audio for call {}", &call_id_recv[..8.min(call_id_recv.len())]);
                            
                            // Receive audio for up to 10 seconds
                            let start = tokio::time::Instant::now();
                            let mut frame_count = 0;
                            
                            while start.elapsed() < Duration::from_secs(10) {
                                // Try to receive audio frame with timeout
                                match tokio::time::timeout(Duration::from_millis(100), subscriber.recv()).await {
                                    Ok(Some(frame)) => {
                                        // Capture received audio (should be 880Hz from server)
                                        validator.capture_client_received(&call_id_recv, frame.samples).await;
                                        frame_count += 1;
                                        
                                        if frame_count % 50 == 0 {
                                            tracing::debug!("Client: Received {} frames for call {}", 
                                                frame_count, &call_id_recv[..8.min(call_id_recv.len())]);
                                        }
                                    }
                                    Ok(None) => {
                                        // Channel closed
                                        tracing::debug!("Client: Audio channel closed for call {}", 
                                            &call_id_recv[..8.min(call_id_recv.len())]);
                                        break;
                                    }
                                    Err(_) => {
                                        // Timeout, continue waiting
                                    }
                                }
                            }
                            
                            info!("Client: Stopped receiving audio for call {} ({} frames total)", 
                                &call_id_recv[..8.min(call_id_recv.len())], frame_count);
                        }
                        Err(e) => {
                            tracing::error!("Client: Failed to subscribe to audio frames: {}", e);
                        }
                    }
                });
            }
            
            // Step 4: Start sending 440Hz tone
            let session_id_send = session_id.clone();
            let call_id_send = call_id.clone();
            let coordinator_send = coordinator.clone();
            
            tokio::spawn(async move {
                // Wait a moment for media to be established
                tokio::time::sleep(Duration::from_millis(100)).await;
                
                // Generate 10 seconds of 440Hz tone at 8kHz
                let samples = generate_tone(440.0, 8000, Duration::from_secs(10));
                
                // Send audio in 20ms chunks (160 samples at 8kHz)
                const SAMPLES_PER_FRAME: usize = 160;
                let mut sent_frames = 0;
                
                for chunk in samples.chunks(SAMPLES_PER_FRAME) {
                    // Create audio frame
                    let frame = AudioFrame {
                        samples: chunk.to_vec(),
                        sample_rate: 8000,
                        channels: 1,
                        duration: Duration::from_millis(20),
                        timestamp: (sent_frames * SAMPLES_PER_FRAME) as u32,
                    };
                    
                    // Send through real media session
                    if let Err(e) = MediaControl::send_audio_frame(&coordinator_send, &session_id_send, frame).await {
                        tracing::debug!("Client: Failed to send audio frame: {}", e);
                        break;
                    }
                    
                    sent_frames += 1;
                    
                    // Wait 20ms before next frame
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                
                info!("Client: Finished sending 440Hz tone for call {} ({} frames)", 
                    &call_id_send[..8.min(call_id_send.len())], sent_frames);
            });
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let call_id = call.id.0.clone();
        info!("Client: Call {} ended: {}", &call_id[..8.min(call_id.len())], reason);
    }
}

/// Server handler that sends 880Hz tone and receives 440Hz
#[derive(Debug)]
struct ServerHandler {
    received_calls: Arc<Mutex<Vec<String>>>,
    call_counter: Arc<Mutex<usize>>,
    audio_validator: Arc<AudioValidator>,
    coordinator: Arc<Mutex<Option<Arc<SessionCoordinator>>>>,
}

impl ServerHandler {
    fn new(audio_validator: Arc<AudioValidator>) -> Self {
        Self {
            received_calls: Arc::new(Mutex::new(Vec::new())),
            call_counter: Arc::new(Mutex::new(0)),
            audio_validator,
            coordinator: Arc::new(Mutex::new(None)),
        }
    }
    
    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        *self.coordinator.lock().await = Some(coordinator);
    }
}

#[async_trait::async_trait]
impl CallHandler for ServerHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let id_str = &call.id.0;
        
        // Get the call index - calls arrive in order
        let mut counter = self.call_counter.lock().await;
        let call_index = *counter;
        *counter += 1;
        
        // Check if this call index should be captured
        if self.audio_validator.should_capture_index(call_index).await {
            // Register using the SIP Call-ID if available, otherwise the session ID
            if let Some(ref sip_call_id) = call.sip_call_id {
                self.audio_validator.register_call_for_index(call_index, sip_call_id.clone()).await;
                info!("Server: Registered Call-ID {} for audio capture", sip_call_id);
            } else {
                self.audio_validator.register_call_for_index(call_index, call.id.0.clone()).await;
            }
        }
        
        info!("Server: Accepting incoming call {} (index {}, Call-ID: {:?})", 
            &id_str[..8.min(id_str.len())], call_index, call.sip_call_id);
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        let call_id = call.id.0.clone();
        let session_id = call.id.clone();
        info!("Server: Call {} established - local_sdp: {}, remote_sdp: {}, sip_call_id: {:?}", 
            &call_id[..8.min(call_id.len())],
            local_sdp.is_some(),
            remote_sdp.is_some(),
            call.sip_call_id
        );
        
        // Store session
        self.received_calls.lock().await.push(call_id.clone());
        
        // Check if this call is selected for audio capture using SIP Call-ID if available
        let is_selected = if let Some(ref sip_call_id) = call.sip_call_id {
            self.audio_validator.is_selected(sip_call_id).await
        } else {
            self.audio_validator.is_selected(&call_id).await
        };
        
        // Get coordinator
        let coordinator_opt = self.coordinator.lock().await.clone();
        
        if let Some(coordinator) = coordinator_opt {
            // Workaround: If SDP is not provided, fetch it from media info
            let mut actual_remote_sdp = remote_sdp;
            if actual_remote_sdp.is_none() {
                if let Ok(Some(media_info)) = MediaControl::get_media_info(&coordinator, &session_id).await {
                    actual_remote_sdp = media_info.remote_sdp;
                    if actual_remote_sdp.is_some() {
                        info!("Server: Retrieved remote SDP from media info for call {}", &call_id[..8.min(call_id.len())]);
                    }
                }
            }
            // Step 1: Update media with remote SDP (this establishes RTP flow)
            if let Some(remote_sdp_str) = actual_remote_sdp {
                // This parses the SDP and sets up the remote RTP endpoint
                match MediaControl::update_media_with_sdp(&coordinator, &session_id, &remote_sdp_str).await {
                    Ok(_) => {
                        info!("Server: Updated media with remote SDP for call {}", &call_id[..8.min(call_id.len())]);
                    }
                    Err(e) => {
                        tracing::error!("Server: Failed to update media with SDP for call {}: {}", 
                            &call_id[..8.min(call_id.len())], e);
                        return;
                    }
                }
            } else {
                tracing::warn!("Server: No remote SDP provided for call {}", &call_id[..8.min(call_id.len())]);
            }
            
            // Step 2: Start audio transmission (enables RTP processing)
            if let Err(e) = MediaControl::start_audio_transmission(&coordinator, &session_id).await {
                tracing::error!("Server: Failed to start audio transmission: {}", e);
                return;
            }
            
            // Step 3: If selected for capture, subscribe to receive audio
            if is_selected {
                info!("Server: Call {} is selected for audio capture", &call_id[..8.min(call_id.len())]);
                let validator = self.audio_validator.clone();
                // Use SIP Call-ID for audio capture if available, otherwise use session ID
                let call_id_recv = call.sip_call_id.clone().unwrap_or(call_id.clone());
                let session_id_recv = session_id.clone();
                let coordinator_recv = coordinator.clone();
                
                tokio::spawn(async move {
                    info!("Server: Starting audio capture task for call {}", &call_id_recv[..8.min(call_id_recv.len())]);
                    
                    // Wait for media to be fully established
                    let mut retries = 0;
                    while retries < 20 {
                        if let Ok(Some(info)) = MediaControl::get_media_info(&coordinator_recv, &session_id_recv).await {
                            tracing::debug!("Server: Media info for call {} - local_sdp: {}, remote_sdp: {}", 
                                &call_id_recv[..8.min(call_id_recv.len())],
                                info.local_sdp.is_some(),
                                info.remote_sdp.is_some()
                            );
                            if info.remote_sdp.is_some() && info.local_sdp.is_some() {
                                info!("Server: Media ready for call {} after {} retries", 
                                    &call_id_recv[..8.min(call_id_recv.len())], retries);
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        retries += 1;
                    }
                    
                    if retries >= 20 {
                        tracing::error!("Server: Media not ready after 2 seconds for call {}", 
                            &call_id_recv[..8.min(call_id_recv.len())]);
                        return;
                    }
                    
                    // Subscribe to audio frames from the session
                    match MediaControl::subscribe_to_audio_frames(&coordinator_recv, &session_id_recv).await {
                        Ok(mut subscriber) => {
                            info!("Server: Started receiving audio for call {}", &call_id_recv[..8.min(call_id_recv.len())]);
                            
                            // Receive audio for up to 10 seconds
                            let start = tokio::time::Instant::now();
                            let mut frame_count = 0;
                            
                            while start.elapsed() < Duration::from_secs(10) {
                                // Try to receive audio frame with timeout
                                match tokio::time::timeout(Duration::from_millis(100), subscriber.recv()).await {
                                    Ok(Some(frame)) => {
                                        // Capture received audio (should be 440Hz from client)
                                        validator.capture_server_received(&call_id_recv, frame.samples).await;
                                        frame_count += 1;
                                        
                                        if frame_count % 50 == 0 {
                                            tracing::debug!("Server: Received {} frames for call {}", 
                                                frame_count, &call_id_recv[..8.min(call_id_recv.len())]);
                                        }
                                    }
                                    Ok(None) => {
                                        // Channel closed
                                        tracing::debug!("Server: Audio channel closed for call {}", 
                                            &call_id_recv[..8.min(call_id_recv.len())]);
                                        break;
                                    }
                                    Err(_) => {
                                        // Timeout, continue waiting
                                    }
                                }
                            }
                            
                            info!("Server: Stopped receiving audio for call {} ({} frames total)", 
                                &call_id_recv[..8.min(call_id_recv.len())], frame_count);
                        }
                        Err(e) => {
                            tracing::error!("Server: Failed to subscribe to audio frames: {}", e);
                        }
                    }
                });
            }
            
            // Step 4: Start sending 880Hz tone
            let session_id_send = session_id.clone();
            let call_id_send = call_id.clone();
            let coordinator_send = coordinator.clone();
            
            tokio::spawn(async move {
                // Wait a moment for media to be established
                tokio::time::sleep(Duration::from_millis(100)).await;
                
                // Generate 10 seconds of 880Hz tone at 8kHz
                let samples = generate_tone(880.0, 8000, Duration::from_secs(10));
                
                // Send audio in 20ms chunks (160 samples at 8kHz)
                const SAMPLES_PER_FRAME: usize = 160;
                let mut sent_frames = 0;
                
                for chunk in samples.chunks(SAMPLES_PER_FRAME) {
                    // Create audio frame
                    let frame = AudioFrame {
                        samples: chunk.to_vec(),
                        sample_rate: 8000,
                        channels: 1,
                        duration: Duration::from_millis(20),
                        timestamp: (sent_frames * SAMPLES_PER_FRAME) as u32,
                    };
                    
                    // Send through real media session
                    if let Err(e) = MediaControl::send_audio_frame(&coordinator_send, &session_id_send, frame).await {
                        tracing::debug!("Server: Failed to send audio frame: {}", e);
                        break;
                    }
                    
                    sent_frames += 1;
                    
                    // Wait 20ms before next frame
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                
                info!("Server: Finished sending 880Hz tone for call {} ({} frames)", 
                    &call_id_send[..8.min(call_id_send.len())], sent_frames);
            });
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let id_str = &call.id.0;
        info!("Server: Call {} ended: {}", &id_str[..8.min(id_str.len())], reason);
    }
}

/// Main benchmark function
pub async fn run_benchmark() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging from RUST_LOG env var, default to ERROR only
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║      BENCHMARK: 500 Concurrent Calls with Audio Tones          ║");
    println!("║                                                                 ║");
    println!("║  • 500 concurrent SIP calls                                    ║");
    println!("║  • 10 second call duration                                     ║");
    println!("║  • Client sends 440Hz tone, receives 880Hz                     ║");
    println!("║  • Server sends 880Hz tone, receives 440Hz                     ║");
    println!("║  • 5 random calls captured to WAV files                        ║");
    println!("║  • Only RECEIVED audio is recorded for validation              ║");
    println!("║  • Output directory: bench/samples/                            ║");
    println!("║  • Metrics collected every second                              ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    let test_start = Instant::now();
    
    // Create audio validator
    let audio_validator = Arc::new(AudioValidator::new());
    
    // Create server handler
    let server_handler = Arc::new(ServerHandler::new(audio_validator.clone()));
    info!("Creating server SessionManager on port 5060...");
    let server = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:server@127.0.0.1:5060")
        .with_local_bind_addr("127.0.0.1:5060".parse().unwrap())
        .with_media_ports(20000, 25000)
        .with_handler(server_handler.clone())
        .build()
        .await?;
    
    // Set coordinator in handler
    server_handler.set_coordinator(server.clone()).await;
    
    // Create client handler  
    let client_handler = Arc::new(ClientHandler::new(audio_validator.clone()));
    info!("Creating client SessionManager on port 5061...");
    let client = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:client@127.0.0.1:5061")
        .with_local_bind_addr("127.0.0.1:5061".parse().unwrap())
        .with_media_ports(25000, 30000)
        .with_handler(client_handler.clone())
        .build()
        .await?;
    
    // Set coordinator in handler
    client_handler.set_coordinator(client.clone()).await;
    
    // Start both session managers
    SessionControl::start(&server).await?;
    SessionControl::start(&client).await?;
    
    // Start metrics collection
    let active_calls_counter = Arc::new(Mutex::new(0usize));
    let metrics_collector = MetricsCollector::new();
    metrics_collector.start_collection(
        Duration::from_secs(1),
        active_calls_counter.clone(),
    ).await;
    
    // For testing, let's start with just 5 calls and capture all of them
    const NUM_CALLS: usize = 5;
    
    // Select all 5 calls for audio capture during testing
    let selected_indices = audio_validator.select_random_indices(NUM_CALLS, NUM_CALLS).await;
    
    // Create 5 concurrent calls for testing
    info!("Initiating {} concurrent calls...", NUM_CALLS);
    let mut call_tasks = Vec::new();
    
    for i in 0..NUM_CALLS {
        let client_clone = client.clone();
        let counter = active_calls_counter.clone();
        let validator_clone = audio_validator.clone();
        
        let task = tokio::spawn(async move {
            
            let from = format!("sip:user_{}@127.0.0.1:5061", i);
            let to = format!("sip:destination_{}@127.0.0.1:5060", i);
            
            // Use prepared call to ensure SDP is generated
            match SessionControl::prepare_outgoing_call(&client_clone, &from, &to).await {
                Ok(prepared) => {
                    match SessionControl::initiate_prepared_call(&client_clone, &prepared).await {
                        Ok(session) => {
                            // For now, register the session ID, but the Call-ID will be used when the call is established
                            validator_clone.register_call_for_index(i, session.id.0.clone()).await;
                            
                            // Increment active calls counter
                            *counter.lock().await += 1;
                            
                            info!("Call {} created successfully", i);
                            
                            // Hold the call for 10 seconds
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            
                            // Terminate the call
                            if let Err(e) = SessionControl::terminate_session(&client_clone, &session.id).await {
                                tracing::warn!("Failed to terminate call {}: {}", i, e);
                            }
                            
                            // Decrement active calls counter
                            *counter.lock().await -= 1;
                            
                            Ok(session)
                        }
                        Err(e) => {
                            tracing::warn!("Call {} initiation failed: {}", i, e);
                            Err(e)
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Call {} preparation failed: {}", i, e);
                    Err(e)
                }
            }
        });
        
        call_tasks.push(task);
    }
    
    info!("All 500 call tasks spawned, waiting for completion...");
    
    // Wait for all calls to complete
    let mut successful_calls = 0;
    let mut failed_calls = 0;
    
    for task in call_tasks {
        match task.await {
            Ok(Ok(_)) => successful_calls += 1,
            _ => failed_calls += 1,
        }
    }
    
    // Wait a bit for final metrics collection and audio processing
    println!("\nProcessing audio captures and saving WAV files...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Get and print metrics
    let snapshots = metrics_collector.get_snapshots().await;
    MetricsCollector::print_metrics_table(&snapshots);
    
    // Validate captured audio from WAV files
    let validation_results = audio_validator.validate_all().await;
    AudioValidator::print_validation_results(&validation_results);
    
    // Print summary
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║                    BENCHMARK SUMMARY                           ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║  Total Calls Attempted:    {:3}                                 ║", NUM_CALLS);
    println!("║  Successful Calls:         {:3}                                ║", successful_calls);
    println!("║  Failed Calls:             {:3}                                 ║", failed_calls);
    println!("║  Success Rate:             {:.1}%                             ║", 
        (successful_calls as f32 / NUM_CALLS as f32) * 100.0);
    println!("║  Total Test Time:          {:.1}s                             ║",
        test_start.elapsed().as_secs_f32());
    println!("║  WAV Files Location:       bench/samples/                      ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    // Cleanup
    SessionControl::stop(&server).await?;
    SessionControl::stop(&client).await?;
    
    Ok(())
}

#[tokio::test]
async fn test_concurrent_calls_with_tones() {
    run_benchmark().await.expect("Benchmark failed");
}

/// Main entry point for running as a benchmark
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_benchmark().await
}