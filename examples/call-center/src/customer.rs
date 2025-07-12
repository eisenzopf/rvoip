//! Call Center Customer
//!
//! This customer:
//! 1. Makes a call to the call center support line
//! 2. Uses real audio devices (microphone and speaker) for communication
//! 3. Supports configurable server addresses for distributed deployment
//! 4. Provides detailed call statistics and real-time audio streaming
//! 5. Stays on the call for a configurable duration

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error, warn, debug};
use clap::Parser;
use std::net::SocketAddr;

use rvoip::{
    client_core::{
        ClientConfig, ClientEventHandler, ClientError, ClientManager,
        IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
        CallAction, CallId, CallState, MediaConfig,
        audio::{AudioDeviceManager, AudioDirection},
        AudioStreamConfig,
    },
};
use async_trait::async_trait;

#[derive(Parser, Debug)]
#[command(author, version, about = "Call Center Customer with Real Audio", long_about = None)]
struct Args {
    /// Customer name
    #[arg(short, long, default_value = "customer")]
    name: String,
    
    /// Call center server address
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server: String,
    
    /// Local IP address for this customer (used for SIP binding and signaling)
    #[arg(short, long, default_value = "127.0.0.1")]
    domain: String,
    
    /// Local SIP port to bind to
    #[arg(short, long, default_value = "5080")]
    port: u16,
    
    /// Call duration in seconds
    #[arg(long, default_value = "30")]
    call_duration: u64,
    
    /// Wait time before making call
    #[arg(long, default_value = "3")]
    wait_time: u64,
    
    /// Support line to call (default: support)
    #[arg(long, default_value = "support")]
    support_line: String,
    
    /// List available audio devices and exit
    #[arg(long)]
    list_devices: bool,
    
    /// Input device ID (use --list-devices to see options)
    #[arg(long)]
    input_device: Option<String>,
    
    /// Output device ID (use --list-devices to see options)
    #[arg(long)]
    output_device: Option<String>,
    
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
    
    /// Enable audio debug logging
    #[arg(long)]
    audio_debug: bool,
}

/// Event handler for the customer with real audio capabilities
#[derive(Clone)]
struct CustomerHandler {
    name: String,
    _domain: String,
    audio_debug: bool,
    call_completed: Arc<tokio::sync::Mutex<bool>>,
    call_connected: Arc<tokio::sync::Mutex<bool>>,
    call_id: Arc<tokio::sync::Mutex<Option<CallId>>>,
    client: Arc<tokio::sync::RwLock<Option<Arc<ClientManager>>>>,
    audio_manager: Arc<tokio::sync::RwLock<Option<Arc<AudioDeviceManager>>>>,
}

impl CustomerHandler {
    fn new(name: String, domain: String, audio_debug: bool) -> Self {
        Self {
            name,
            _domain: domain,
            audio_debug,
            call_completed: Arc::new(tokio::sync::Mutex::new(false)),
            call_connected: Arc::new(tokio::sync::Mutex::new(false)),
            call_id: Arc::new(tokio::sync::Mutex::new(None)),
            client: Arc::new(tokio::sync::RwLock::new(None)),
            audio_manager: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_client(&self, client: Arc<ClientManager>) {
        *self.client.write().await = Some(client);
    }
    
    async fn set_audio_manager(&self, audio_manager: Arc<AudioDeviceManager>) {
        *self.audio_manager.write().await = Some(audio_manager);
    }
    
    async fn is_call_completed(&self) -> bool {
        *self.call_completed.lock().await
    }
    
    async fn is_call_connected(&self) -> bool {
        *self.call_connected.lock().await
    }
    
    async fn set_call_id(&self, call_id: CallId) {
        *self.call_id.lock().await = Some(call_id);
    }
    
    async fn setup_audio_for_call(&self, call_id: &CallId) -> Result<(), anyhow::Error> {
        let client_guard = self.client.read().await;
        let audio_guard = self.audio_manager.read().await;
        
        if let (Some(client), Some(audio_manager)) = (client_guard.as_ref(), audio_guard.as_ref()) {
            // Configure high-quality audio stream
            let config = AudioStreamConfig {
                sample_rate: 8000,  // Standard VoIP sample rate
                channels: 1,        // Mono for VoIP
                codec: "PCMU".to_string(),  // G.711 μ-law
                frame_size_ms: 20,  // 20ms frames (160 samples at 8kHz)
                enable_aec: true,   // Echo cancellation
                enable_agc: true,   // Auto gain control
                enable_vad: true,   // Voice activity detection
            };
            
            info!("🔧 [{}] Configuring audio stream for call {}", self.name, call_id);
            info!("   Sample Rate: {}Hz", config.sample_rate);
            info!("   Channels: {}", config.channels);
            info!("   Codec: {}", config.codec);
            info!("   Frame Size: {}ms", config.frame_size_ms);
            
            // Set audio stream configuration
            client.set_audio_stream_config(call_id, config).await?;
            
            // Start audio streaming
            client.start_audio_stream(call_id).await?;
            info!("✅ [{}] Audio streaming started for call {}", self.name, call_id);
            
            // Set up audio capture (microphone)
            let input_device = audio_manager.get_default_device(AudioDirection::Input).await?;
            info!("🎤 [{}] Using input device: {}", self.name, input_device.info().name);
            client.start_audio_capture(call_id, &input_device.info().id).await?;
            
            // Set up audio playback (speaker)
            let output_device = audio_manager.get_default_device(AudioDirection::Output).await?;
            info!("🔊 [{}] Using output device: {}", self.name, output_device.info().name);
            client.start_audio_playback(call_id, &output_device.info().id).await?;
            
            // Subscribe to incoming audio frames for monitoring
            let audio_subscriber = client.subscribe_to_audio_frames(call_id).await?;
            let name_clone = self.name.clone();
            let call_id_clone = call_id.clone();
            let audio_debug = self.audio_debug;
            
            tokio::spawn(async move {
                let mut _frame_count = 0;
                let _start_time = std::time::Instant::now();
                
                // Move the subscriber into a spawn_blocking task to handle blocking recv
                let handle = tokio::task::spawn_blocking(move || {
                    let mut frames = Vec::new();
                    loop {
                        match audio_subscriber.recv() {
                            Ok(frame) => {
                                frames.push(frame);
                                if frames.len() >= 10 {
                                    break; // Process batches of 10 frames
                                }
                            }
                            Err(_) => break, // Channel closed or error
                        }
                    }
                    frames
                });
                
                // For now, just log that we would process audio frames
                // In a real implementation, this would process the frames
                match handle.await {
                    Ok(frames) => {
                        _frame_count += frames.len();
                        if audio_debug {
                            debug!("🎵 [{}] Would process {} audio frames", name_clone, frames.len());
                        }
                    }
                    Err(_) => {
                        debug!("🔚 [{}] Audio frame monitoring ended for call {}", name_clone, call_id_clone);
                    }
                }
            });
            
            info!("🎵 [{}] Real audio setup complete for call {}", self.name, call_id);
        }
        
        Ok(())
    }
    
    async fn cleanup_audio_for_call(&self, call_id: &CallId) {
        let client_guard = self.client.read().await;
        
        if let Some(client) = client_guard.as_ref() {
            info!("🧹 [{}] Cleaning up audio for call {}", self.name, call_id);
            
            // Stop audio streaming
            if let Err(e) = client.stop_audio_stream(call_id).await {
                warn!("⚠️  [{}] Failed to stop audio stream: {}", self.name, e);
            }
            
            // Stop audio capture
            if let Err(e) = client.stop_audio_capture(call_id).await {
                warn!("⚠️  [{}] Failed to stop audio capture: {}", self.name, e);
            }
            
            // Stop audio playback
            if let Err(e) = client.stop_audio_playback(call_id).await {
                warn!("⚠️  [{}] Failed to stop audio playback: {}", self.name, e);
            }
            
            info!("✅ [{}] Audio cleanup complete for call {}", self.name, call_id);
        }
    }
    
    async fn get_call_statistics(&self, call_id: &CallId) -> Result<(), anyhow::Error> {
        let client_guard = self.client.read().await;
        
        if let Some(client) = client_guard.as_ref() {
            // Get media info
            if let Ok(media_info) = client.get_call_media_info(call_id).await {
                info!("📊 [{}] Media info - Codec: {:?}, Local RTP: {:?}, Remote RTP: {:?}",
                    self.name, media_info.codec, media_info.local_rtp_port, media_info.remote_rtp_port);
            }
            
            // Get RTP statistics
            if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(call_id).await {
                info!("📊 [{}] RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes)",
                    self.name, rtp_stats.packets_sent, rtp_stats.bytes_sent,
                    rtp_stats.packets_received, rtp_stats.bytes_received);
            }
            
            // Get call quality metrics (using available media info)
            if let Ok(media_info) = client.get_call_media_info(call_id).await {
                info!("📊 [{}] Call Quality - Media: {:?}, Local: {:?}, Remote: {:?}",
                    self.name, media_info.codec, media_info.local_rtp_port, media_info.remote_rtp_port);
            }
        }
        
        Ok(())
    }
}

#[async_trait]
impl ClientEventHandler for CustomerHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [{}] Unexpected incoming call from {}", 
            self.name, call_info.caller_uri);
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::Initiating => "🔄",
            CallState::Ringing => "🔔",
            CallState::Connected => "✅",
            CallState::Failed => "❌",
            CallState::Cancelled => "🚫",
            CallState::Terminated => "📴",
            _ => "❓",
        };
        
        info!("{} [{}] Call {} state: {:?} → {:?}", 
            state_emoji, self.name, status_info.call_id, 
            status_info.previous_state, status_info.new_state);
        
        match status_info.new_state {
            CallState::Ringing => {
                info!("🔔 [{}] Call is ringing... waiting for agent to answer", self.name);
            }
            CallState::Connected => {
                info!("🎉 [{}] Connected to agent! Setting up real audio...", self.name);
                *self.call_connected.lock().await = true;
                
                // Setup real audio streaming
                if let Err(e) = self.setup_audio_for_call(&status_info.call_id).await {
                    error!("❌ [{}] Failed to setup audio: {}", self.name, e);
                } else {
                    info!("🎵 [{}] Real audio setup successful", self.name);
                    
                    // Get initial media statistics
                    if let Err(e) = self.get_call_statistics(&status_info.call_id).await {
                        warn!("⚠️  [{}] Failed to get initial call statistics: {}", self.name, e);
                    }
                }
            }
            CallState::Failed => {
                error!("❌ [{}] Call failed: {:?}", self.name, status_info.reason);
                *self.call_completed.lock().await = true;
            }
            CallState::Terminated => {
                info!("📴 [{}] Call terminated, cleaning up audio...", self.name);
                self.cleanup_audio_for_call(&status_info.call_id).await;
                *self.call_completed.lock().await = true;
            }
            _ => {}
        }
    }
    
    async fn on_media_event(&self, event: MediaEventInfo) {
        if self.audio_debug {
            info!("🎵 [{}] Media event for {}: {:?}", 
                self.name, event.call_id, event.event_type);
        }
    }
    
    async fn on_registration_status_changed(&self, _reg_info: RegistrationStatusInfo) {
        // Not needed for customer calls
    }
    
    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ [{}] Client error on call {:?}: {}", self.name, call_id, error);
    }
    
    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        if connected {
            info!("🌐 [{}] Network connected", self.name);
        } else {
            warn!("🔌 [{}] Network disconnected: {}", 
                self.name, reason.unwrap_or("unknown".to_string()));
        }
    }
}

async fn list_audio_devices() -> Result<(), anyhow::Error> {
    println!("🎵 Audio Device Discovery");
    println!("========================");
    
    let audio_manager = AudioDeviceManager::new().await?;
    
    // List input devices
    println!("\n🎤 INPUT DEVICES (Microphones):");
    println!("------------------------------");
    let input_devices = audio_manager.list_devices(AudioDirection::Input).await?;
    if input_devices.is_empty() {
        println!("  No input devices found");
    } else {
        for (i, device) in input_devices.iter().enumerate() {
            let default_marker = if device.is_default { " (DEFAULT)" } else { "" };
            println!("  {}. {}{}", i + 1, device.name, default_marker);
            println!("     ID: {}", device.id);
        }
    }
    
    // List output devices
    println!("\n🔊 OUTPUT DEVICES (Speakers):");
    println!("-----------------------------");
    let output_devices = audio_manager.list_devices(AudioDirection::Output).await?;
    if output_devices.is_empty() {
        println!("  No output devices found");
    } else {
        for (i, device) in output_devices.iter().enumerate() {
            let default_marker = if device.is_default { " (DEFAULT)" } else { "" };
            println!("  {}. {}{}", i + 1, device.name, default_marker);
            println!("     ID: {}", device.id);
        }
    }
    
    println!("\n💡 Usage: Use --input-device and --output-device with the device ID");
    println!("   Example: --input-device {} --output-device {}", 
        input_devices.first().map(|d| d.id.as_str()).unwrap_or("device-id"),
        output_devices.first().map(|d| d.id.as_str()).unwrap_or("device-id"));
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    
    // Handle device listing
    if args.list_devices {
        return list_audio_devices().await;
    }
    
    // Create logs directory
    std::fs::create_dir_all("logs")?;
    
    // Initialize logging with file output
    let file_appender = tracing_appender::rolling::never("logs", "customer.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("call_center_demo=info".parse()?)
                .add_directive(format!("rvoip={}", log_level).parse()?)
        )
        .init();
    
    info!("👤 Starting customer: {}", args.name);
    info!("🏢 Call center server: {}", args.server);
    info!("🌐 Local IP address: {}", args.domain);
    info!("📱 Local SIP port: {}", args.port);
    info!("📞 Support line: {}", args.support_line);
    info!("⏰ Call duration: {}s", args.call_duration);
    info!("⌛ Wait time: {}s", args.wait_time);
    
    // Parse server address
    let _server_addr: SocketAddr = args.server.parse()
        .map_err(|e| anyhow::anyhow!("Invalid server address '{}': {}", args.server, e))?;
    
    // Initialize audio device manager
    info!("🎵 Initializing audio devices...");
    let audio_manager = Arc::new(AudioDeviceManager::new().await?);
    
    // Verify audio devices
    let input_devices = audio_manager.list_devices(AudioDirection::Input).await?;
    let output_devices = audio_manager.list_devices(AudioDirection::Output).await?;
    
    if input_devices.is_empty() {
        error!("❌ No input devices (microphones) found!");
        return Err(anyhow::anyhow!("No input devices available"));
    }
    
    if output_devices.is_empty() {
        error!("❌ No output devices (speakers) found!");
        return Err(anyhow::anyhow!("No output devices available"));
    }
    
    let input_device = if let Some(device_id) = args.input_device {
        input_devices.iter().find(|d| d.id == device_id)
            .ok_or_else(|| anyhow::anyhow!("Input device '{}' not found", device_id))?
    } else {
        input_devices.iter().find(|d| d.is_default)
            .unwrap_or(&input_devices[0])
    };
    
    let output_device = if let Some(device_id) = args.output_device {
        output_devices.iter().find(|d| d.id == device_id)
            .ok_or_else(|| anyhow::anyhow!("Output device '{}' not found", device_id))?
    } else {
        output_devices.iter().find(|d| d.is_default)
            .unwrap_or(&output_devices[0])
    };
    
    info!("🎤 Selected input device: {}", input_device.name);
    info!("🔊 Selected output device: {}", output_device.name);
    
    // Create client configuration
    // Use the domain IP as the local binding address to ensure proper SIP signaling
    let local_sip_addr = format!("{}:{}", args.domain, args.port).parse()?;
    let local_media_addr = format!("{}:{}", args.domain, args.port + 1000).parse()?;
    
    let config = ClientConfig::new()
        .with_sip_addr(local_sip_addr)
        .with_media_addr(local_media_addr)
        .with_user_agent(format!("CallCenter-Customer-{}/1.0", args.name))
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: true,   // Enable echo cancellation
            noise_suppression: true,   // Enable noise suppression
            auto_gain_control: true,   // Enable auto gain control
            rtp_port_start: args.port + 2000,
            rtp_port_end: args.port + 2100,
            ..Default::default()
        });
    
    // Create client and handler
    let client = ClientManager::new(config).await?;
    let handler = Arc::new(CustomerHandler::new(
        args.name.clone(),
        args.domain.clone(),
        args.audio_debug
    ));
    
    handler.set_client(client.clone()).await;
    handler.set_audio_manager(audio_manager).await;
    client.set_event_handler(handler.clone()).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [{}] Client started with real audio capabilities", args.name);
    
    // Wait for the call center to be ready
    info!("⏳ [{}] Waiting {} seconds for call center to be ready...", args.name, args.wait_time);
    sleep(Duration::from_secs(args.wait_time)).await;
    
    // Make a call to the support line
    info!("📞 [{}] Calling call center support line...", args.name);
    let from_uri = format!("sip:{}@{}:{}", args.name, args.domain, args.port);
    let to_uri = format!("sip:{}@{}", args.support_line, args.domain);
    
    let call_id = client.make_call(from_uri, to_uri.clone(), None).await?;
    info!("📞 [{}] Call initiated to {} with ID: {}", args.name, to_uri, call_id);
    
    handler.set_call_id(call_id.clone()).await;
    
    // Wait for call to connect
    info!("⏳ [{}] Waiting for call to connect...", args.name);
    let mut attempts = 0;
    while !handler.is_call_connected().await && !handler.is_call_completed().await && attempts < 60 {
        sleep(Duration::from_millis(500)).await;
        attempts += 1;
    }
    
    if !handler.is_call_connected().await {
        error!("❌ [{}] Call failed to connect after 30 seconds", args.name);
        client.stop().await?;
        return Err(anyhow::anyhow!("Call failed to connect"));
    }
    
    info!("✅ [{}] Call connected! Staying on call for {} seconds...", args.name, args.call_duration);
    
    // Let the call run for the specified duration
    let start_time = std::time::Instant::now();
    let mut last_stats_time = start_time;
    
    while !handler.is_call_completed().await {
        sleep(Duration::from_secs(1)).await;
        
        let elapsed = start_time.elapsed();
        if elapsed >= Duration::from_secs(args.call_duration) {
            info!("⏰ [{}] Call duration reached, hanging up...", args.name);
            break;
        }
        
        // Print periodic statistics
        if elapsed.as_secs() % 10 == 0 && elapsed.as_secs() > last_stats_time.elapsed().as_secs() {
            last_stats_time = std::time::Instant::now();
            if let Err(e) = handler.get_call_statistics(&call_id).await {
                debug!("Failed to get call statistics: {}", e);
            }
        }
    }
    
    // Get final statistics before hanging up
    info!("📊 [{}] Getting final call statistics...", args.name);
    if let Err(e) = handler.get_call_statistics(&call_id).await {
        warn!("⚠️  [{}] Failed to get final statistics: {}", args.name, e);
    }
    
    // Hang up the call if not already completed
    if !handler.is_call_completed().await {
        info!("📴 [{}] Hanging up call...", args.name);
        client.hangup_call(&call_id).await?;
    }
    
    // Wait for call termination
    let mut attempts = 0;
    while !handler.is_call_completed().await && attempts < 20 {
        sleep(Duration::from_millis(500)).await;
        attempts += 1;
    }
    
    if handler.is_call_completed().await {
        info!("✅ [{}] Call completed successfully", args.name);
    } else {
        warn!("⚠️  [{}] Call may not have terminated cleanly", args.name);
    }
    
    // Stop the client
    client.stop().await?;
    info!("👋 [{}] Customer session completed", args.name);
    
    Ok(())
} 