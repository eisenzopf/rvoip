use clap::Parser;
use std::net::SocketAddr;
use tracing::{info, debug, error, warn, Level};
use tracing_subscriber::FmtSubscriber;

// Updated imports from the refactored SIP client library
use rvoip_sip_client::{
    UserAgent, ClientConfig, 
    CallEvent, CallState, Result,
    call_registry::CallRegistry,
};

/// SIP Call Receiver - Listens for incoming calls
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Local address to bind to
    #[arg(short = 'a', long, default_value = "127.0.0.1:5071")]
    local_addr: String,
    
    /// Username
    #[arg(short, long, default_value = "bob")]
    username: String,
    
    /// Domain
    #[arg(short, long, default_value = "rvoip.local")]
    domain: String,
    
    /// Auto-answer calls
    #[arg(short = 'o', long, default_value_t = true)]
    auto_answer: bool,
    
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
    
    info!("Starting SIP call receiver on {}", local_addr);
    info!("Username: {}, Domain: {}", args.username, args.domain);
    info!("Auto-answer: {}", args.auto_answer);
    
    // Create a call registry for persisting call and dialog state
    let registry = CallRegistry::new(100);
    info!("Created call registry for persistence");
    
    // Create client configuration
    let config = ClientConfig::new()
        .with_local_addr(local_addr)
        .with_username(args.username)
        .with_domain(args.domain);
    
    // Create user agent
    let mut user_agent = UserAgent::new(config).await?;
    
    // Set the call registry to enable persistence
    user_agent.set_call_registry(registry).await;
    info!("Call registry configured for user agent");
    
    // Get call events
    let mut call_events = user_agent.event_stream();
    
    // Start user agent in the background
    user_agent.start().await?;
    
    info!("SIP receiver started and listening for calls...");
    info!("Press Ctrl+C to exit");
    
    // Process call events in the foreground
    while let Some(event) = call_events.recv().await {
        match event {
            CallEvent::Ready => {
                info!("SIP call event system ready");
            },
            CallEvent::IncomingCall(call) => {
                info!("Incoming call from {}", call.remote_uri());
                
                if args.auto_answer {
                    // Check call state before answering
                    let state = call.state().await;
                    if state == CallState::Ringing {
                        info!("Auto-answering call in Ringing state");
                        match call.answer().await {
                            Ok(_) => info!("Call answered successfully"),
                            Err(e) => error!("Failed to answer call: {}", e),
                        }
                    } else {
                        info!("Call already in {} state, not sending explicit answer", state);
                    }
                } else {
                    info!("Call ringing - auto-answer is disabled");
                }
            },
            CallEvent::StateChanged { call, previous, current } => {
                info!("Call state changed: {} -> {}", previous, current);
            },
            CallEvent::MediaAdded { call, media_type } => {
                info!("Media added to call: {:?}", media_type);
            },
            CallEvent::MediaRemoved { call, media_type } => {
                info!("Media removed from call: {:?}", media_type);
            },
            CallEvent::DtmfReceived { call, digit } => {
                info!("DTMF received: {}", digit);
            },
            CallEvent::ResponseReceived { call, response, transaction_id } => {
                info!("Response received: {} (transaction ID: {})", response.status, transaction_id);
            },
            CallEvent::Terminated { call, reason } => {
                info!("Call terminated: {}", reason);
            },
            CallEvent::Error { call, error } => {
                error!("Call error: {}", error);
            },
        }
    }
    
    // If we get here, the event channel was closed
    // Wait for Ctrl+C to exit (this keeps us running)
    info!("Event channel closed, now waiting for Ctrl+C to exit");
    match tokio::signal::ctrl_c().await {
        Ok(()) => info!("Received Ctrl+C, shutting down"),
        Err(e) => error!("Error waiting for Ctrl+C: {}", e),
    }
    
    // Stop the user agent
    user_agent.stop().await?;
    
    Ok(())
} 