//! Agent Client for E2E Testing
//!
//! This agent client:
//! 1. Registers with the call center server via SIP REGISTER
//! 2. Accepts incoming calls automatically
//! 3. Plays a test tone or silence for audio
//! 4. Hangs up after a configurable duration

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tracing::{info, warn, error};
use clap::Parser;

use rvoip_client_core::{
    client::{ClientBuilder, Client},
    events::ClientEvent,
};

#[derive(Parser, Debug)]
#[command(author, version, about = "SIP Agent Client for Call Center Testing", long_about = None)]
struct Args {
    /// Agent username (e.g., alice, bob)
    #[arg(short, long)]
    username: String,
    
    /// Call center server address
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server: String,
    
    /// Local SIP port to bind to
    #[arg(short, long, default_value = "0")]
    port: u16,
    
    /// Domain name
    #[arg(short, long, default_value = "callcenter.example.com")]
    domain: String,
    
    /// Call duration in seconds (0 for manual hangup)
    #[arg(long, default_value = "10")]
    call_duration: u64,
    
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Initialize logging
    let log_level = if args.verbose { tracing::Level::DEBUG } else { tracing::Level::INFO };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .init();
    
    info!("🤖 Starting agent client for {}", args.username);
    
    // Build SIP URI
    let agent_uri = format!("sip:{}@{}", args.username, args.domain);
    let server_addr: SocketAddr = args.server.parse()?;
    
    // Create client configuration
    let local_addr = format!("0.0.0.0:{}", args.port).parse()?;
    
    // Build the client
    let client = ClientBuilder::new()
        .user_agent("RVoIP-Agent/1.0")
        .local_address(local_addr)
        .build()
        .await?;
    
    // Start the client
    client.start().await?;
    
    // Start event handler
    let event_client = client.clone();
    let call_duration = args.call_duration;
    tokio::spawn(async move {
        handle_client_events(event_client, call_duration).await;
    });
    
    // Register with the server
    info!("📝 Registering as {} with server {}", agent_uri, server_addr);
    
    match client.register(&agent_uri, &server_addr, Duration::from_secs(3600)).await {
        Ok(_) => info!("✅ Successfully registered!"),
        Err(e) => {
            error!("❌ Registration failed: {}", e);
            return Err(e.into());
        }
    }
    
    // Keep the client running
    info!("👂 Agent {} is ready to receive calls...", args.username);
    info!("Press Ctrl+C to stop");
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    
    // Unregister before shutdown
    info!("🔚 Unregistering...");
    client.unregister(&agent_uri, &server_addr).await?;
    
    info!("👋 Agent client shutdown complete");
    Ok(())
}

async fn handle_client_events(client: Arc<Client>, call_duration: u64) {
    let mut event_rx = client.subscribe_events();
    
    while let Ok(event) = event_rx.recv().await {
        match event {
            ClientEvent::IncomingCall { info, priority } => {
                info!("📞 Incoming call {} from {}", info.call_id, info.caller_uri);
                
                // Auto-answer the call
                match client.answer_call(&info.call_id).await {
                    Ok(_) => {
                        info!("✅ Answered call {}", info.call_id);
                        
                        // If call_duration is set, automatically hang up after duration
                        if call_duration > 0 {
                            let client_clone = client.clone();
                            let call_id_clone = info.call_id.clone();
                            tokio::spawn(async move {
                                sleep(Duration::from_secs(call_duration)).await;
                                info!("⏰ Auto-hanging up call {} after {} seconds", 
                                      call_id_clone, call_duration);
                                if let Err(e) = client_clone.hangup_call(&call_id_clone).await {
                                    error!("Failed to hang up call: {}", e);
                                }
                            });
                        }
                    }
                    Err(e) => error!("❌ Failed to answer call {}: {}", info.call_id, e),
                }
            }
            
            ClientEvent::CallStateChanged { info, priority } => {
                use rvoip_client_core::call::CallState;
                match info.new_state {
                    CallState::Connected => {
                        info!("🔊 Call {} established - audio should be flowing", info.call_id);
                    }
                    CallState::Terminated | CallState::Failed | CallState::Cancelled => {
                        info!("📴 Call {} ended: {:?} ({})", 
                              info.call_id, 
                              info.new_state,
                              info.reason.as_deref().unwrap_or("no reason"));
                    }
                    _ => {
                        tracing::debug!("Call {} state changed to {:?}", info.call_id, info.new_state);
                    }
                }
            }
            
            ClientEvent::RegistrationStatusChanged { info, priority } => {
                use rvoip_client_core::registration::RegistrationStatus;
                match info.status {
                    RegistrationStatus::Active => {
                        info!("✅ Registration confirmed: {} (server: {})", 
                              info.user_uri, info.server_uri);
                    }
                    RegistrationStatus::Failed => {
                        error!("❌ Registration failed: {}", 
                               info.reason.as_deref().unwrap_or("unknown reason"));
                    }
                    _ => {
                        tracing::debug!("Registration status: {:?}", info.status);
                    }
                }
            }
            
            ClientEvent::MediaEvent { info, priority } => {
                // Log media events if verbose
                tracing::debug!("🎵 Media event for call {}: {:?}", info.call_id, info.event_type);
            }
            
            _ => {
                // Handle other events
                tracing::debug!("Event: {:?}", event);
            }
        }
    }
} 