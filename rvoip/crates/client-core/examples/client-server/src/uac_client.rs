use anyhow::Result;
use clap::Parser;
use rvoip_client_core::{
    ClientConfig, ClientEventHandler, ClientError, 
    IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
    CallAction, CallId, CallState, MediaConfig,
    client::ClientManager,
};
use std::sync::Arc;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;
use tracing::{error, info, warn};
use std::path::Path;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server address to call
    #[arg(short, long, default_value = "127.0.0.1:5070")]
    server: String,

    /// Local SIP port
    #[arg(short, long, default_value = "5071")]
    port: u16,

    /// Number of calls to make
    #[arg(short, long, default_value = "1")]
    num_calls: usize,

    /// Call duration in seconds
    #[arg(short, long, default_value = "10")]
    duration: u64,

    /// Media port range start
    #[arg(short, long, default_value = "31000")]
    media_port: u16,

    /// Enable RTP packet logging
    #[arg(short, long)]
    rtp_debug: bool,

    /// Path to WAV file to play
    #[arg(short = 'w', long, default_value = "client_a_440hz_pcma.wav")]
    wav_file: String,
}

/// Simple UAC client that makes calls and sends RTP
#[derive(Clone)]
struct SimpleUacHandler {
    rtp_debug: bool,
    active_calls: Arc<Mutex<HashSet<CallId>>>,
    wav_data: Option<Vec<u8>>,
    client_manager: Arc<RwLock<Option<Arc<ClientManager>>>>,
}

impl SimpleUacHandler {
    fn new(rtp_debug: bool, wav_file: Option<&str>) -> Self {
        // Load WAV file if provided
        let wav_data = wav_file.and_then(|path| {
            if Path::new(path).exists() {
                match std::fs::read(path) {
                    Ok(data) => {
                        info!("📁 Loaded WAV file: {} ({} bytes)", path, data.len());
                        
                        // Try to parse WAV header to verify it's valid
                        if let Ok((header, _samples)) = wav::read(&mut std::io::Cursor::new(&data)) {
                            info!("🎵 WAV format: {} channels, {} Hz, {} bits/sample", 
                                header.channel_count, header.sampling_rate, header.bits_per_sample);
                            info!("🎵 WAV file validated successfully");
                            Some(data)
                        } else {
                            error!("❌ Failed to parse WAV file");
                            None
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to read WAV file {}: {}", path, e);
                        None
                    }
                }
            } else {
                warn!("⚠️ WAV file not found: {}", path);
                None
            }
        });
        
        Self {
            rtp_debug,
            active_calls: Arc::new(Mutex::new(HashSet::new())),
            wav_data,
            client_manager: Arc::new(RwLock::new(None)),
        }
    }
    
    async fn set_client_manager(&self, client: Arc<ClientManager>) {
        *self.client_manager.write().await = Some(client);
    }
}

#[async_trait::async_trait]
impl ClientEventHandler for SimpleUacHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        // UAC shouldn't receive calls, but handle just in case
        warn!("Unexpected incoming call on UAC: {}", call_info.call_id);
        CallAction::Reject
    }

    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        info!(
            "📞 Call {} state changed: {:?} -> {:?}",
            status_info.call_id, 
            status_info.previous_state, 
            status_info.new_state
        );

        if status_info.new_state == CallState::Connected {
            let mut calls = self.active_calls.lock().await;
            calls.insert(status_info.call_id);
            info!("🎵 Call {} connected - starting RTP transmission", status_info.call_id);
            
            // Get the client manager to start audio transmission
            if let Some(client) = self.client_manager.read().await.as_ref() {
                info!("📤 Starting audio transmission for call {}", status_info.call_id);
                
                // Start the audio transmission (sends 440Hz test tone)
                match client.start_audio_transmission(&status_info.call_id).await {
                    Ok(_) => {
                        info!("✅ Audio transmission started - sending 440Hz test tone");
                        
                        // Get media info to see the negotiated parameters
                        if let Ok(media_info) = client.get_call_media_info(&status_info.call_id).await {
                            info!("📊 Media info - Local RTP: {:?}, Remote RTP: {:?}, Codec: {:?}",
                                media_info.local_rtp_port, media_info.remote_rtp_port, media_info.codec);
                            
                            // Debug: Log the full media info
                            info!("📋 Full media info: remote_sdp present: {}", media_info.remote_sdp.is_some());
                            
                            // For this demo, since we're running locally and know the server setup,
                            // we'll establish media flow based on what we know:
                            // The server allocates dynamic RTP ports and we need to extract from SDP
                            
                            // Since remote_sdp is not available in MediaInfo, and for demo purposes,
                            // we'll use a workaround. In a real implementation, this would be
                            // handled by the session-core layer properly exposing the remote endpoint.
                            
                            // For now, let's just log that we need the remote endpoint
                            warn!("⚠️ Demo limitation: MediaInfo doesn't include remote SDP");
                            warn!("⚠️ In production, the remote RTP endpoint would be extracted from SDP negotiation");
                            warn!("⚠️ RTP packets are being generated but not sent due to missing destination");
                            
                            // The RTP session is created and generating packets, just missing the destination
                            info!("📡 RTP session is active and generating 440Hz test tone packets");
                            info!("📡 Packets would be sent if remote endpoint was properly configured");
                            
                            // DEMO WORKAROUND: For testing RTP flow in local environment
                            // Normally this would come from SDP negotiation
                            if std::env::var("DEMO_RTP_HARDCODE").is_ok() {
                                // Hardcode to common server RTP port for demo
                                let remote_addr = "127.0.0.1:30000";
                                info!("🔧 DEMO MODE: Using hardcoded remote RTP endpoint: {}", remote_addr);
                                match client.establish_media(&status_info.call_id, remote_addr).await {
                                    Ok(_) => {
                                        info!("✅ Media flow established to {} (demo mode)", remote_addr);
                                    }
                                    Err(e) => {
                                        error!("❌ Failed to establish media flow: {}", e);
                                    }
                                }
                            }
                        } else {
                            error!("❌ Failed to get media info for call {}", status_info.call_id);
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to start audio transmission: {}", e);
                    }
                }
                
                if self.wav_data.is_some() {
                    info!("💡 Note: Custom WAV playback not yet implemented");
                    info!("🎵 The media session is sending a default 440Hz test tone");
                    // TODO: Future enhancement - implement custom audio source for WAV playback
                }
            } else {
                error!("❌ Client manager not available for audio transmission");
            }
        } else if status_info.new_state == CallState::Terminated {
            let mut calls = self.active_calls.lock().await;
            calls.remove(&status_info.call_id);
            info!("🔚 Call {} terminated", status_info.call_id);
        }
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        if self.rtp_debug {
            info!(
                "🎵 Media event for call {}: {:?}",
                event.call_id, event.event_type
            );
        }

        // Check for MediaNegotiated event which should contain remote address
        if let Some(remote_addr) = event.metadata.get("remote_addr") {
            info!("🎯 Found remote RTP address in media event: {}", remote_addr);
            
            // Get client manager and establish media flow
            if let Some(client) = self.client_manager.read().await.as_ref() {
                info!("📤 Establishing media flow to {} for call {}", remote_addr, event.call_id);
                match client.establish_media(&event.call_id, remote_addr).await {
                    Ok(_) => {
                        info!("✅ Media flow established via media event - RTP packets should now be sent to {}", remote_addr);
                    }
                    Err(e) => {
                        error!("❌ Failed to establish media flow via media event: {}", e);
                    }
                }
            }
        }

        // Log RTP packet statistics
        if let Some(metadata) = event.metadata.get("rtp_stats") {
            info!(
                "📤 RTP stats for call {}: {}",
                event.call_id, metadata
            );
        }
    }

    async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
        // UAC doesn't need registration for this demo
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ Error on call {:?}: {}", call_id, error);
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} Network status changed", status);
        if let Some(reason) = reason {
            info!("💬 Reason: {}", reason);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_client_core=debug".parse()?)
                .add_directive("rvoip_media_core=debug".parse()?)
                .add_directive("rvoip_rtp_core=debug".parse()?)
                .add_directive("uac_client=info".parse()?),
        )
        .init();

    let args = Args::parse();

    info!("🚀 Starting UAC Client");
    info!("📞 Local SIP Port: {}", args.port);
    info!("🎯 Target Server: {}", args.server);
    info!("🎵 Media Port Range: {}-{}", args.media_port, args.media_port + 1000);
    info!("📊 Calls: {}, Duration: {}s", args.num_calls, args.duration);
    info!("🐛 RTP Debug: {}", args.rtp_debug);
    info!("🎵 WAV File: {}", args.wav_file);

    // Create client configuration
    let config = ClientConfig::new()
        .with_sip_addr(format!("0.0.0.0:{}", args.port).parse()?)
        .with_media_addr(format!("0.0.0.0:{}", args.media_port).parse()?)
        .with_user_agent("RVOIP-UAC-Client/1.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMA".to_string(), "PCMU".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            rtp_port_start: args.media_port,
            rtp_port_end: args.media_port + 1000,
            ..Default::default()
        });

    // Create handler with WAV file
    let handler = Arc::new(SimpleUacHandler::new(args.rtp_debug, Some(&args.wav_file)));

    // Build and start client
    let client = ClientManager::new(config).await?;
    
    // Set the client manager reference in the handler
    handler.set_client_manager(client.clone()).await;
    
    client.set_event_handler(handler.clone()).await;
    client.start().await?;

    info!("✅ UAC Client ready");

    // Make calls
    for i in 0..args.num_calls {
        info!("📞 Making call {} of {}", i + 1, args.num_calls);
        
        // Parse the server address
        let to_uri = format!("sip:test@{}", args.server);
        let from_uri = format!("sip:uac@{}:{}", "127.0.0.1", args.port);
        
        match client.make_call(from_uri, to_uri, None).await {
            Ok(call_id) => {
                info!("✅ Call {} initiated successfully", call_id);
                
                // Wait for call duration
                info!("⏳ Call will run for {} seconds...", args.duration);
                sleep(Duration::from_secs(args.duration)).await;
                
                // Hang up
                info!("📞 Hanging up call {}", call_id);
                match client.hangup_call(&call_id).await {
                    Ok(_) => info!("✅ Call {} hung up successfully", call_id),
                    Err(e) => error!("❌ Failed to hang up call {}: {}", call_id, e),
                }
            }
            Err(e) => {
                error!("❌ Failed to make call: {}", e);
            }
        }
        
        // Wait between calls
        if i < args.num_calls - 1 {
            info!("⏳ Waiting 2 seconds before next call...");
            sleep(Duration::from_secs(2)).await;
        }
    }

    info!("✅ All calls completed");
    
    // Give some time for final cleanup
    sleep(Duration::from_secs(2)).await;
    
    // Stop the client
    client.stop().await?;
    
    Ok(())
}

/// Extract RTP address from SDP
fn extract_rtp_address_from_sdp(sdp: &str, _rtp_port: Option<u16>) -> Option<String> {
    // Extract IP from c= line
    let ip = sdp.lines()
        .find(|line| line.starts_with("c="))
        .and_then(|line| {
            // c=IN IP4 192.168.1.100
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                Some(parts[3].to_string())
            } else {
                None
            }
        })?;
    
    // Extract port from m= line
    let port = sdp.lines()
        .find(|line| line.starts_with("m=audio"))
        .and_then(|line| {
            // m=audio 5004 RTP/AVP 0 8
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                parts[1].parse::<u16>().ok()
            } else {
                None
            }
        })?;
    
    Some(format!("{}:{}", ip, port))
} 