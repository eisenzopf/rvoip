use clap::Parser;
use std::net::SocketAddr;
use std::time::Duration;
use tracing::{info, error, debug, Level, warn};
use tracing_subscriber::FmtSubscriber;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

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
    #[arg(short, long, default_value = "127.0.0.1")]
    domain: String,
    
    /// Target URI to call
    #[arg(short, long, default_value = "sip:bob@127.0.0.1")]
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
    
    /// DTMF sequence to send (if dtmf is true)
    #[arg(long, default_value = "1 2 3")]
    dtmf_sequence: String,
    
    /// DTMF gap in milliseconds between tones
    #[arg(long, default_value_t = 500)]
    dtmf_gap: u64,
    
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
            match weak_call.hangup().await {
                Ok(_) => info!("Call hung up successfully"),
                Err(e) => {
                    error!("Failed to hang up call: {}", e);
                    // Try to get the current state
                    match weak_call.state().await {
                        Ok(state) => info!("Current call state: {}", state),
                        Err(e) => error!("Could not get call state: {}", e),
                    }
                    
                    // Try to terminate the call through the registry if available
                    match weak_call.registry().await {
                        Ok(Some(registry)) => {
                            info!("Attempting to update call state via registry");
                            let call_id = weak_call.sip_call_id();
                            if let Err(e) = registry.update_call_state(&call_id, CallState::Established, CallState::Terminated).await {
                                error!("Failed to update call state in registry: {}", e);
                            } else {
                                info!("Successfully marked call as terminated in registry");
                            }
                        },
                        _ => error!("No registry available to update call state"),
                    }
                }
            }
        }))
    } else {
        None
    };
    
    // Spawn DTMF sending task
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let weak_call = call.weak_clone();
    let dtmf_task = tokio::spawn(async move {
        // Delay before sending DTMF
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // Wait until established to send DTMF
        info!("Waiting for call to be established before sending DTMF...");
        for i in 0..30 {
            // Check if we should terminate
            if !running_clone.load(Ordering::SeqCst) {
                return;
            }
            
            // Get the current call state
            let current_state = match weak_call.state().await {
                Ok(state) => state,
                Err(e) => {
                    error!("Failed to get call state: {}", e);
                    return;
                }
            };
            
            if i % 10 == 0 {
                debug!("Current call state (iteration {}): {}", i, current_state);
            }
            
            // Check if call is established or ended
            if current_state == CallState::Established {
                info!("Call state now Established at iteration {}, can proceed with DTMF", i);
                break;
            } else if current_state == CallState::Failed || 
                      current_state == CallState::Terminated {
                error!("Call ended (state: {}), stopping DTMF sending", current_state);
                return;
            }
            
            // Wait before checking again
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        // Send DTMF if call is established
        match weak_call.state().await {
            Ok(CallState::Established) => {
                info!("Call established, sending DTMF sequence");
                for digit in args.dtmf_sequence.chars() {
                    if !running_clone.load(Ordering::SeqCst) {
                        break;
                    }
                    
                    if digit.is_whitespace() {
                        continue;
                    }
                    
                    info!("Sending DTMF digit: {}", digit);
                    if let Err(e) = weak_call.send_dtmf(digit).await {
                        error!("Failed to send DTMF: {}", e);
                    }
                    
                    // Delay between digits
                    tokio::time::sleep(Duration::from_millis(args.dtmf_gap)).await;
                }
                info!("DTMF sequence complete");
            },
            Ok(state) => {
                error!("Call not established, current state: {}", state);
            },
            Err(e) => {
                error!("Failed to get call state: {}", e);
            }
        }
    });
    
    // Add a separate task to monitor call state - useful for debugging
    let monitoring_call = call.weak_clone();
    let monitoring_task = tokio::spawn(async move {
        for i in 0..60 {
            match monitoring_call.state().await {
                Ok(state) => {
                    info!("Call state monitor [{}]: State = {}", i, state);
                    
                    // Let's just log if we have a registry available
                    match monitoring_call.registry().await {
                        Ok(Some(_)) => debug!("Call has registry available"),
                        Ok(None) => debug!("Call has no registry available"),
                        Err(e) => debug!("Error accessing registry: {}", e),
                    }
                },
                Err(e) => {
                    error!("Failed to get call state in monitor: {}", e);
                    break;
                }
            }
            
            // Stop if call is in a terminal state
            if let Ok(state) = monitoring_call.state().await {
                if state == CallState::Terminated || state == CallState::Failed {
                    info!("Call reached terminal state {}, stopping monitor", state);
                    break;
                }
            }
            
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    
    // Process client events in the foreground
    while let Ok(event) = client_events.recv().await {
        debug!("Received client event: {:?}", event);
        match event {
            SipClientEvent::Call(call_event) => {
                info!("Call event: {:?}", call_event);
                
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
                } else if let CallEvent::ResponseReceived { call, response, transaction_id } = call_event {
                    info!("SIP response received: {} {} (transaction: {})", 
                         response.status.as_u16(), response.status, transaction_id);
                    if response.status.is_success() && response.status.as_u16() == 200 {
                        info!("Received 200 OK for call {}", call.id());
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
    
    // Always abort the DTMF task since it's not an Option
    dtmf_task.abort();
    
    // Abort the monitoring task
    monitoring_task.abort();
    
    // Stop the client
    client.stop().await?;
    info!("SIP client stopped");
    
    Ok(())
} 