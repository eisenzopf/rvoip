use anyhow::Result;
use clap::Parser;
use rvoip_client_core::{
    ClientConfig, ClientEventHandler, ClientError, 
    IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
    CallAction, CallId, CallState, MediaConfig,
    client::ClientManager,
};
use std::sync::Arc;
use std::collections::{HashSet, HashMap};
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};
use std::path::Path;
use std::error::Error;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct UacArgs {
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
    config: Arc<RwLock<ClientConfig>>,
    server_address: String,
    last_call_ids: Arc<Mutex<HashMap<usize, CallId>>>,
}

impl SimpleUacHandler {
    fn new(rtp_debug: bool, wav_file: Option<&str>, server_address: String) -> Self {
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
            config: Arc::new(RwLock::new(ClientConfig::default())),
            server_address,
            last_call_ids: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    async fn set_client_manager(&self, client: Arc<ClientManager>) {
        *self.client_manager.write().await = Some(client);
    }

    pub async fn run(
        &self,
        num_concurrent_calls: usize,
        call_duration: Duration,
    ) -> Result<(), Box<dyn Error>> {
        info!("🚀 Starting UAC client - making {} concurrent calls", num_concurrent_calls);
        
        // Initialize client manager
        let config = self.config.read().await.clone();
        
        // Set the SIP port from the config (not 0!)
        // We need to get the port from somewhere - let's modify the signature
        // For now, we'll handle this in main instead
        
        let client = ClientManager::new(config).await?;
        
        // Set the event handler
        client.set_event_handler(Arc::new(self.clone())).await;
        
        // Start the client
        client.start().await?;
        info!("✅ UAC client started");
        
        // Store the client
        self.client_manager.write().await.replace(client);
        
        // Create multiple concurrent calls
        let mut handles = Vec::new();
        
        for i in 0..num_concurrent_calls {
            let call_number = i + 1;
            let client_ref = Arc::clone(&self.client_manager);
            let server_addr = self.server_address.clone();
            let active_calls = Arc::clone(&self.active_calls);
            let duration = call_duration;
            let rtp_debug = self.rtp_debug;
            let last_call_ids = Arc::clone(&self.last_call_ids);
            
            let handle = tokio::spawn(async move {
                if let Err(e) = make_single_call(
                    call_number,
                    client_ref,
                    server_addr,
                    active_calls,
                    duration,
                    rtp_debug,
                    last_call_ids
                ).await {
                    error!("Call {} failed: {}", call_number, e);
                }
            });
            
            handles.push(handle);
            
            // Small delay between starting calls
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        // Wait for all calls to complete
        for handle in handles {
            let _ = handle.await;
        }
        
        info!("✅ All calls completed");
        
        // Print final RTP statistics summary
        if let Some(client) = self.client_manager.read().await.as_ref() {
            info!("");
            info!("📊 ========== FINAL RTP STATISTICS SUMMARY ==========");
            let mut total_sent = 0u64;
            let mut total_received = 0u64;
            let mut total_bytes_sent = 0u64;
            let mut total_bytes_received = 0u64;
            
            for (call_num, call_id) in self.last_call_ids.lock().await.iter() {
                if let Ok(Some(stats)) = client.get_rtp_statistics(call_id).await {
                    info!("📞 Call {}: Sent {} packets ({} bytes), Received {} packets ({} bytes)",
                        call_num,
                        stats.packets_sent,
                        stats.bytes_sent,
                        stats.packets_received,
                        stats.bytes_received
                    );
                    total_sent += stats.packets_sent;
                    total_received += stats.packets_received;
                    total_bytes_sent += stats.bytes_sent;
                    total_bytes_received += stats.bytes_received;
                }
            }
            
            info!("──────────────────────────────────────────────────");
            info!("📈 TOTAL: Sent {} packets ({} bytes), Received {} packets ({} bytes)",
                total_sent,
                total_bytes_sent,
                total_received,
                total_bytes_received
            );
            
            if total_received == 0 && total_sent > 0 {
                warn!("⚠️  No RTP packets were received from the server!");
                warn!("    This may indicate a network configuration issue.");
            }
            info!("===================================================");
            info!("");
        }
        
        Ok(())
    }
}

async fn make_single_call(
    call_number: usize,
    client_ref: Arc<RwLock<Option<Arc<ClientManager>>>>,
    server_addr: String,
    active_calls: Arc<Mutex<HashSet<CallId>>>,
    duration: Duration,
    rtp_debug: bool,
    last_call_ids: Arc<Mutex<HashMap<usize, CallId>>>,
) -> Result<(), Box<dyn Error>> {
    info!("📞 Call {}: Initiating call to {}", call_number, server_addr);
    
    let from_uri = format!("sip:uac{}@127.0.0.1", call_number);
    let to_uri = format!("sip:uas{}@{}", call_number, server_addr);
    
    // Make the call
    let call_id = {
        let client = client_ref.read().await;
        let client = client.as_ref().ok_or("Client not initialized")?;
        client.make_call(from_uri, to_uri, None).await?
    };
    
    info!("📞 Call {}: Created with ID: {}", call_number, call_id);
    
    // Add to active calls
    active_calls.lock().await.insert(call_id.clone());
    
    // Store the mapping for final statistics
    last_call_ids.lock().await.insert(call_number, call_id.clone());
    
    // Start statistics monitoring
    let stats_handle = {
        let client_ref = Arc::clone(&client_ref);
        let call_id = call_id.clone();
        let call_num = call_number;
        
        tokio::spawn(async move {
            // Wait a bit for the call to establish
            tokio::time::sleep(Duration::from_secs(2)).await;
            
            // Monitor statistics every second
            loop {
                if let Some(client) = client_ref.read().await.as_ref() {
                    // Get RTP statistics
                    if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
                        info!("📊 Call {} RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes), Lost: {}", 
                            call_num,
                            rtp_stats.packets_sent,
                            rtp_stats.bytes_sent,
                            rtp_stats.packets_received,
                            rtp_stats.bytes_received,
                            rtp_stats.packets_lost
                        );
                    }
                    
                    // Get call statistics for quality metrics
                    if let Ok(Some(call_stats)) = client.get_call_statistics(&call_id).await {
                        if rtp_debug {
                            info!("📈 Call {} Quality - MOS: {:.2}, Jitter: {:.2}ms, Packet Loss: {:.2}%",
                                call_num,
                                call_stats.quality.mos_score,
                                call_stats.quality.jitter_ms,
                                call_stats.quality.packet_loss_rate
                            );
                        }
                    }
                }
                
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
    };
    
    // Let the call run for the specified duration
    info!("📞 Call {}: Running for {:?}...", call_number, duration);
    tokio::time::sleep(duration).await;
    
    // Terminate the call
    info!("📞 Call {}: Terminating...", call_number);
    {
        let client = client_ref.read().await;
        let client = client.as_ref().ok_or("Client not initialized")?;
        client.hangup_call(&call_id).await?;
    }
    
    // Stop statistics monitoring
    stats_handle.abort();
    
    // Remove from active calls
    active_calls.lock().await.remove(&call_id);
    
    info!("✅ Call {}: Completed successfully", call_number);
    Ok(())
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
            calls.insert(status_info.call_id.clone());
            
            // Extract call number from the call ID or from active calls count
            let call_number = calls.len();
            self.last_call_ids.lock().await.insert(call_number, status_info.call_id.clone());
            
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
                            
                            // The remote SDP should now be automatically populated by session-core
                            if media_info.remote_sdp.is_some() {
                                info!("✅ Remote SDP is available - RTP endpoint configured automatically");
                                info!("📡 RTP packets are being sent to the negotiated remote endpoint");
                            } else {
                                warn!("⚠️ Remote SDP not found - this might indicate an issue");
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args = UacArgs::parse();
    
    // Initialize logging
    let log_level = if args.rtp_debug {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_thread_ids(true)
        .with_target(false)
        .init();
    
    info!("Starting UAC client example");
    
    // Create client configuration
    let mut client_config = ClientConfig::default();
    client_config.local_sip_addr = format!("0.0.0.0:{}", args.port).parse().unwrap();
    client_config.user_agent = "rvoip-uac-demo/1.0".to_string();
    
    // Configure media settings
    client_config.media = MediaConfig {
        rtp_port_start: args.media_port,
        rtp_port_end: args.media_port + 100,
        preferred_codecs: vec!["PCMA".to_string(), "PCMU".to_string()],
        echo_cancellation: false,
        noise_suppression: false,
        auto_gain_control: false,
        ..Default::default()
    };
    
    // Create the UAC client
    let client = SimpleUacHandler::new(args.rtp_debug, Some(&args.wav_file), args.server.clone());
    
    // Set the configuration
    *client.config.write().await = client_config;
    
    // Run the client
    let call_duration = Duration::from_secs(args.duration);
    client.run(args.num_calls, call_duration).await?;
    
    info!("UAC client example completed");
    Ok(())
} 