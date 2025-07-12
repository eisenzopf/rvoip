//! Call Center Agent
//!
//! This agent:
//! 1. Registers with the call center server via SIP REGISTER
//! 2. Accepts incoming calls automatically
//! 3. Handles calls using real audio devices (microphone and speaker)
//! 4. Supports configurable server addresses for distributed deployment
//! 5. Uses real-time audio streaming with hardware devices

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error, debug};
use clap::Parser;
use std::net::SocketAddr;

use rvoip::{
    client_core::{
        ClientConfig, ClientEventHandler, ClientError, ClientManager,
        IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
        CallAction, CallId, CallState, MediaConfig,
        registration::{RegistrationConfig, RegistrationStatus},
        audio::{AudioDeviceManager, AudioDirection},
        AudioStreamConfig,
    },
};
use uuid;
use async_trait::async_trait;

#[derive(Parser, Debug)]
#[command(author, version, about = "Call Center Agent with Real Audio", long_about = None)]
struct Args {
    /// Agent name (e.g., alice, bob)
    #[arg(short, long, default_value = "alice")]
    name: String,
    
    /// Call center server address
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server: String,
    
    /// Local IP address for this agent (used for SIP binding and signaling)
    #[arg(short, long, default_value = "127.0.0.1")]
    domain: String,
    
    /// Local SIP port to bind to
    #[arg(short, long, default_value = "5071")]
    port: u16,
    
    /// Call duration in seconds (0 for indefinite)
    #[arg(long, default_value = "0")]
    call_duration: u64,
    
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

/// Event handler for the agent with real audio capabilities
#[derive(Clone)]
struct AgentHandler {
    name: String,
    _domain: String,
    call_duration: u64,
    audio_debug: bool,
    client: Arc<tokio::sync::RwLock<Option<Arc<ClientManager>>>>,
    audio_manager: Arc<tokio::sync::RwLock<Option<Arc<AudioDeviceManager>>>>,
}

impl AgentHandler {
    fn new(name: String, domain: String, call_duration: u64, audio_debug: bool) -> Self {
        Self {
            name,
            _domain: domain,
            call_duration,
            audio_debug,
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
    
    async fn initialize_audio_devices(&self) -> Result<(), anyhow::Error> {
        let client_guard = self.client.read().await;
        let audio_guard = self.audio_manager.read().await;
        
        if let (Some(client), Some(audio_manager)) = (client_guard.as_ref(), audio_guard.as_ref()) {
            // Get audio devices
            let input_device = audio_manager.get_default_device(AudioDirection::Input).await?;
            info!("🎤 [{}] Activating input device: {}", self.name, input_device.info().name);
            
            let output_device = audio_manager.get_default_device(AudioDirection::Output).await?;
            info!("🔊 [{}] Activating output device: {}", self.name, output_device.info().name);
            
            // Create a "standby" call ID for keeping audio devices active
            let standby_call_id = uuid::Uuid::new_v4();
            
            // Configure audio stream for standby mode
            let config = AudioStreamConfig {
                sample_rate: 8000,  // Standard VoIP sample rate
                channels: 1,        // Mono for VoIP
                codec: "PCMU".to_string(),  // G.711 μ-law
                frame_size_ms: 20,  // 20ms frames (160 samples at 8kHz)
                enable_aec: true,   // Echo cancellation
                enable_agc: true,   // Auto gain control
                enable_vad: true,   // Voice activity detection
            };
            
            // Set audio stream configuration for standby
            client.set_audio_stream_config(&standby_call_id, config).await?;
            
            // Start audio streaming in standby mode
            client.start_audio_stream(&standby_call_id).await?;
            
            // Start audio capture (this should show microphone indicator)
            client.start_audio_capture(&standby_call_id, &input_device.info().id).await?;
            
            // Start audio playback (ready for incoming audio)
            client.start_audio_playback(&standby_call_id, &output_device.info().id).await?;
            
            info!("✅ [{}] Audio devices active - microphone recording, ready for calls", self.name);
        }
        
        Ok(())
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
}

#[async_trait]
impl ClientEventHandler for AgentHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [{}] Incoming call from {} (call_id: {})", 
            self.name, call_info.caller_uri, call_info.call_id);
        
        // Just accept the call - we'll set up audio when Connected
        // The client-core will handle SDP generation automatically
        info!("✅ [{}] Accepting call {} - audio will be set up when connected", self.name, call_info.call_id);
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::IncomingPending => "🔔",
            CallState::Connected => "✅",
            CallState::Failed => "❌",
            CallState::Terminated => "📴",
            _ => "🔄",
        };
        
        info!("{} [{}] Call {} state: {:?} → {:?}", 
            state_emoji, self.name, status_info.call_id, 
            status_info.previous_state, status_info.new_state);
        
        match status_info.new_state {
            CallState::Connected => {
                info!("🎉 [{}] Call {} connected! Activating audio for this call...", self.name, status_info.call_id);
                
                // Audio devices were initialized during registration
                // Now activate audio streaming for this specific call
                if let Err(e) = self.setup_audio_for_call(&status_info.call_id).await {
                    error!("❌ [{}] Failed to setup audio: {}", self.name, e);
                } else {
                    info!("🎵 [{}] Call audio streaming active - microphone and speaker ready", self.name);
                }
                
                // Auto-hangup after call duration if specified
                if self.call_duration > 0 {
                    let client_guard = self.client.read().await;
                    if let Some(client) = client_guard.as_ref() {
                        let client_clone = client.clone();
                        let call_id = status_info.call_id.clone();
                        let name = self.name.clone();
                        let duration = self.call_duration;
                        
                        tokio::spawn(async move {
                            sleep(Duration::from_secs(duration)).await;
                            info!("⏰ [{}] Auto-hanging up call {} after {} seconds", 
                                  name, call_id, duration);
                            
                            if let Err(e) = client_clone.hangup_call(&call_id).await {
                                error!("❌ [{}] Failed to hang up call: {}", name, e);
                            }
                        });
                    }
                }
            }
            CallState::Terminated => {
                info!("📴 [{}] Call {} completed, cleaning up audio...", self.name, status_info.call_id);
                self.cleanup_audio_for_call(&status_info.call_id).await;
            }
            CallState::Failed => {
                error!("❌ [{}] Call {} failed, cleaning up audio...", self.name, status_info.call_id);
                self.cleanup_audio_for_call(&status_info.call_id).await;
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
    
    async fn on_registration_status_changed(&self, reg_info: RegistrationStatusInfo) {
        match reg_info.status {
            RegistrationStatus::Active => {
                info!("✅ [{}] Registration active: {}", self.name, reg_info.user_uri);
            }
            RegistrationStatus::Failed => {
                error!("❌ [{}] Registration failed: {}", 
                    self.name, reg_info.reason.as_deref().unwrap_or("unknown"));
            }
            RegistrationStatus::Expired => {
                warn!("⏰ [{}] Registration expired", self.name);
            }
            _ => {
                info!("🔄 [{}] Registration status: {:?}", self.name, reg_info.status);
            }
        }
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
    let file_appender = tracing_appender::rolling::never("logs", format!("{}.log", args.name));
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
    
    info!("🤖 Starting agent: {}", args.name);
    info!("🏢 Call center server: {}", args.server);
    info!("🌐 Local IP address: {}", args.domain);
    info!("📱 Local SIP port: {}", args.port);
    info!("⏰ Call duration: {}s", if args.call_duration > 0 { args.call_duration.to_string() } else { "indefinite".to_string() });
    
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
    
    // Build URIs
    let agent_uri = format!("sip:{}@{}", args.name, args.domain);
    let server_uri = format!("sip:{}", args.server);
    
    // Create client configuration
    // Use the domain IP as the local binding address to ensure proper SIP signaling
    let local_sip_addr = format!("{}:{}", args.domain, args.port).parse()?;
    let local_media_addr = format!("{}:{}", args.domain, args.port + 1000).parse()?;
    
    let config = ClientConfig::new()
        .with_sip_addr(local_sip_addr)
        .with_media_addr(local_media_addr)
        .with_user_agent(format!("CallCenter-Agent-{}/1.0", args.name))
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: true,   // Enable echo cancellation
            noise_suppression: true,   // Enable noise suppression
            auto_gain_control: true,   // Enable auto gain control
            ..Default::default()
        });
    
    // Create client and handler
    let client = ClientManager::new(config).await?;
    let handler = Arc::new(AgentHandler::new(
        args.name.clone(), 
        args.domain.clone(), 
        args.call_duration,
        args.audio_debug
    ));
    
    handler.set_client(client.clone()).await;
    handler.set_audio_manager(audio_manager).await;
    client.set_event_handler(handler.clone()).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [{}] Client started", args.name);
    
    // Register with the call center server
    info!("📝 [{}] Registering with call center server...", args.name);
    
    let reg_config = RegistrationConfig::new(
        server_uri,
        agent_uri.clone(),
        agent_uri.clone(),
    ).with_expires(300); // 5 minute expiry
    
    let registration_id = client.register(reg_config).await?;
    info!("✅ [{}] Successfully registered with ID: {}", args.name, registration_id);
    
    // Initialize audio devices immediately after registration
    // This ensures the agent is ready for incoming calls without delay
    info!("🎵 [{}] Initializing audio devices for call readiness...", args.name);
    
    if let Err(e) = handler.initialize_audio_devices().await {
        error!("❌ [{}] Failed to initialize audio devices: {}", args.name, e);
        return Err(anyhow::anyhow!("Audio initialization failed: {}", e));
    }
    
    info!("🎤 [{}] Microphone active and ready", args.name);
    info!("🔊 [{}] Speaker active and ready", args.name);
    info!("👂 [{}] Agent ready to receive calls with real audio!", args.name);
    
    // Keep the agent running
    tokio::signal::ctrl_c().await?;
    
    // Cleanup
    info!("🔚 [{}] Shutting down...", args.name);
    if let Err(e) = client.unregister(registration_id).await {
        warn!("⚠️  [{}] Failed to unregister: {}", args.name, e);
    }
    
    client.stop().await?;
    info!("👋 [{}] Agent shutdown complete", args.name);
    
    Ok(())
} 