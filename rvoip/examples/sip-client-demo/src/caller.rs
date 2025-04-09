use clap::Parser;
use std::net::SocketAddr;
use std::time::Duration;
use tracing::{info, error, debug, Level, warn};
use tracing_subscriber::FmtSubscriber;

// Updated imports from the refactored SIP client library
use rvoip_sip_client::{
    SipClient, ClientConfig, SipClientEvent,
    CallConfig, CallState, CallEvent,
    Result,
    call_registry::CallRegistry,
};

/// SIP Call Maker - Makes outgoing calls
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Local address to bind to
    #[arg(short = 'a', long, default_value = "127.0.0.1:5070")]
    local_addr: String,
    
    /// Username
    #[arg(short, long, default_value = "alice")]
    username: String,
    
    /// Domain
    #[arg(short, long, default_value = "rvoip.local")]
    domain: String,
    
    /// Target URI to call
    #[arg(short, long, default_value = "sip:bob@rvoip.local")]
    target_uri: String,
    
    /// Server address to send calls to
    #[arg(short, long, default_value = "127.0.0.1:5071")]
    server_addr: String,
    
    /// Call duration in seconds (0 for manual control)
    #[arg(short = 'r', long, default_value_t = 30)]
    duration: u64,
    
    /// Send DTMF tones during the call
    #[arg(short = 'm', long, default_value_t = true)]
    dtmf: bool,
    
    /// Output log level
    #[arg(short, long, default_value = "info")]
    log_level: Level,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();
    
    // Initialize tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(args.log_level)
        .finish();
    
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| rvoip_sip_client::Error::Other(format!("Failed to set tracing: {}", e)))?;
    
    // Parse local address
    let local_addr: SocketAddr = args.local_addr.parse()?;
    let server_addr: SocketAddr = args.server_addr.parse()?;
    
    info!("Starting SIP caller on {}", local_addr);
    info!("Username: {}, Domain: {}", args.username, args.domain);
    info!("Target URI: {}", args.target_uri);
    info!("Server address: {}", server_addr);
    info!("Call duration: {} seconds", args.duration);
    
    // Create a call registry for persisting call and dialog state
    let registry = CallRegistry::new(100);
    info!("Created call registry for persistence");
    
    // Create client configuration
    let config = ClientConfig::new()
        .with_local_addr(local_addr)
        .with_username(args.username.clone())
        .with_domain(args.domain)
        .with_outbound_proxy(Some(server_addr));
    
    // Create SIP client
    let mut client = SipClient::new(config).await?;
    
    // Set the call registry to enable persistence
    client.set_call_registry(registry);
    info!("Call registry configured for SIP client");
    
    // Get client events
    let mut client_events = client.event_stream();
    
    // Start client in the background
    client.start().await?;
    
    info!("SIP client started");
    
    // Make a call
    info!("Making call to {}", args.target_uri);
    
    // Create call configuration
    let call_config = CallConfig::new()
        .with_audio(true)
        .with_dtmf(args.dtmf);
    
    // Make the call
    let call = client.call(&args.target_uri, call_config).await?;
    let call_id = call.id().to_string();
    
    info!("Call initiated with ID: {}", call_id);
    
    // Set up call duration timeout if specified
    let duration_task = if args.duration > 0 {
        let weak_call = call.weak_clone();
        Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(args.duration)).await;
            info!("Call duration reached, hanging up");
            if let Err(e) = weak_call.hangup().await {
                error!("Failed to hang up call: {}", e);
            }
        }))
    } else {
        None
    };
    
    // Set up DTMF task if enabled
    let dtmf_task = if args.dtmf {
        let weak_call = call.weak_clone();
        Some(tokio::spawn(async move {
            // Wait for call to be established before sending DTMF
            info!("Waiting for call to be established before sending DTMF...");
            
            // Wait a reasonable timeout for the entire call
            let timeout = tokio::time::sleep(Duration::from_secs(15));
            
            tokio::select! {
                _ = timeout => {
                    error!("Call setup timed out, will not send DTMF");
                    return;
                }
                result = weak_call.wait_until_established() => {
                    match result {
                        Ok(_) => {
                            info!("Call established successfully, proceeding with DTMF");
                            
                            // Send DTMF digits '1', '2', '3' with 1 second interval between them
                            for digit in &['1', '2', '3'] {
                                info!("Sending DTMF digit: {}", digit);
                                match weak_call.send_dtmf(*digit).await {
                                    Ok(_) => {
                                        info!("Successfully sent DTMF digit: {}", digit);
                                    },
                                    Err(e) => {
                                        // Check if call failed or just DTMF failed
                                        if e.to_string().contains("Failed state") {
                                            error!("Call failed, stopping DTMF sending: {}", e);
                                            break;
                                        } else {
                                            // Just log the error but continue with next digit
                                            error!("Failed to send DTMF digit {}, but continuing: {}", digit, e);
                                        }
                                    }
                                }
                                
                                // Wait for current call state
                                let current_state = weak_call.state().await;
                                if current_state == CallState::Failed || 
                                   current_state == CallState::Terminated {
                                    error!("Call ended (state: {}), stopping DTMF sending", current_state);
                                    break;
                                }
                                
                                // Wait a bit between digits
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        },
                        Err(e) => {
                            error!("Failed to establish call: {}", e);
                            info!("Current call state: {}", weak_call.state().await);
                            
                            // Try to hang up the failed call
                            warn!("Attempting to hang up the failed call");
                            if let Err(e) = weak_call.hangup().await {
                                error!("Failed to hang up call: {}", e);
                            }
                        }
                    }
                }
            }
        }))
    } else {
        None
    };
    
    // Process client events in the foreground
    while let Ok(event) = client_events.recv().await {
        match event {
            SipClientEvent::Call(call_event) => {
                debug!("Call event: {:?}", call_event);
                
                // Check for call state changes
                if let CallEvent::StateChanged { 
                    call, 
                    previous, 
                    current 
                } = call_event {
                    info!("Call state changed: {} -> {}", previous, current);
                    
                    // If call established, print info
                    if current == CallState::Established {
                        info!("Call established with {}", call.remote_uri());
                    }
                    
                    // If call terminated, exit
                    if current == CallState::Terminated {
                        info!("Call terminated, exiting");
                        break;
                    }
                }
            },
            SipClientEvent::RegistrationState { registered, server, expires, error } => {
                if registered {
                    info!("Registered with {}, expires in {} seconds", server, expires.unwrap_or(0));
                } else if let Some(err) = error {
                    error!("Registration failed: {}", err);
                } else {
                    info!("Unregistered from {}", server);
                }
            },
            SipClientEvent::Error(err) => {
                error!("Client error: {}", err);
            },
        }
    }
    
    // Cancel tasks if they're still running
    if let Some(task) = duration_task {
        task.abort();
    }
    if let Some(task) = dtmf_task {
        task.abort();
    }
    
    // Stop the client
    client.stop().await?;
    info!("SIP client stopped");
    
    Ok(())
} 