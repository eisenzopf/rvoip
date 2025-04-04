use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, Context};
use clap::Parser;
use tokio::signal::ctrl_c;
use tracing::{info, debug, error};

use rvoip_call_engine::CallEngine;
use rvoip_call_engine::engine::CallEngineConfig;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};
use rvoip_sip_transport::UdpTransport;
use rvoip_sip_core;
use rvoip_call_engine::policy::PolicyEngine;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Local SIP signaling address to bind to
    #[arg(short, long, default_value = "0.0.0.0:5060")]
    sip_addr: String,
    
    /// Local media address to bind to
    #[arg(short, long, default_value = "0.0.0.0:10000")]
    media_addr: String,
    
    /// Local domain name
    #[arg(short, long, default_value = "rvoip.local")]
    domain: String,

    /// Disable authentication (for testing)
    #[arg(short, long)]
    no_auth: bool,
}

/// Handles SIGINT (Ctrl+C) for graceful shutdown
async fn handle_shutdown_signal() -> Result<()> {
    ctrl_c().await.context("Failed to listen for ctrl+c signal")?;
    info!("Received shutdown signal, initiating graceful shutdown...");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Parse command line arguments
    let args = Args::parse();
    
    // Parse socket addresses
    let sip_addr: SocketAddr = args.sip_addr.parse()
        .context("Invalid SIP address format")?;
    let media_addr: SocketAddr = args.media_addr.parse()
        .context("Invalid media address format")?;
        
    info!("Starting simple call engine example");
    info!("SIP address: {}", sip_addr);
    info!("Media address: {}", media_addr);
    info!("Domain: {}", args.domain);
    info!("Authentication: {}", if args.no_auth { "disabled" } else { "enabled" });
    
    // Create UDP transport (with default buffer size)
    let (udp_transport, transport_rx) = UdpTransport::bind(sip_addr, None).await
        .context("Failed to bind UDP transport")?;
    
    info!("UDP transport bound to {}", sip_addr);
    
    // Create transaction manager
    // Wrap UDP transport in Arc to share it
    let arc_transport = Arc::new(udp_transport);
    
    // Create the transaction manager
    let (transaction_manager, mut transaction_rx) = TransactionManager::new(
        arc_transport.clone(),
        transport_rx,
        None, // Use default event capacity
    ).await.context("Failed to create transaction manager")?;
    
    // Wrap transaction manager in Arc to share it
    let arc_transaction_manager = Arc::new(transaction_manager);
    
    info!("Transaction manager initialized");
    
    // Create call engine
    let engine_config = CallEngineConfig {
        local_signaling_addr: sip_addr,
        local_media_addr: media_addr,
        local_domain: args.domain,
        user_agent: "RVOIP-Simple-Call-Engine/0.1.0".to_string(),
        max_sessions: 100,
        cleanup_interval: Duration::from_secs(30),
    };

    // Create the policy engine with auth disabled if requested
    let policy_engine = if args.no_auth {
        info!("Creating policy engine with authentication disabled");
        let mut policy = PolicyEngine::new();
        policy.set_auth_required(rvoip_sip_core::Method::Register, false);
        policy.set_auth_required(rvoip_sip_core::Method::Invite, false);
        policy.set_auth_required(rvoip_sip_core::Method::Subscribe, false);
        Some(policy)
    } else {
        None
    };

    // Create the call engine with the policy engine
    let call_engine = Arc::new(CallEngine::new_with_policy(
        engine_config,
        arc_transaction_manager.clone(),
        policy_engine,
    ));

    // Initialize the engine
    call_engine.initialize().await
        .context("Failed to initialize call engine")?;

    info!("Call engine initialized");
    
    // Handle transaction events
    let call_engine_clone = call_engine.clone();
    let transaction_handle = tokio::spawn(async move {
        debug!("Starting transaction event handler");
        
        // Process transaction events
        while let Some(event) = transaction_rx.recv().await {
            match event {
                TransactionEvent::TransactionCreated { transaction_id } => {
                    debug!("Transaction created: {}", transaction_id);
                },
                TransactionEvent::TransactionCompleted { transaction_id, response } => {
                    debug!("Transaction completed: {}, response: {:?}", transaction_id, response);
                },
                TransactionEvent::TransactionTerminated { transaction_id } => {
                    debug!("Transaction terminated: {}", transaction_id);
                },
                TransactionEvent::UnmatchedMessage { message, source } => {
                    debug!("Unmatched message from {}: {:?}", source, message);
                    
                    // Process unmatched message through call engine
                    if let rvoip_sip_core::Message::Request(request) = message {
                        match call_engine_clone.handle_request(request, source).await {
                            Ok(response) => {
                                // Send the response back through the transport
                                debug!("Sending direct response: {:?}", response);
                                let message = rvoip_sip_core::Message::Response(response);
                                if let Err(e) = call_engine_clone.transaction_manager().transport().send_message(message, source).await {
                                    error!("Failed to send response: {}", e);
                                }
                            },
                            Err(e) => {
                                error!("Error handling request: {}", e);
                            }
                        }
                    } else {
                        debug!("Ignoring unmatched response message");
                    }
                },
                TransactionEvent::Error { error, transaction_id } => {
                    error!("Transaction error: {}, id: {:?}", error, transaction_id);
                },
            }
        }
        
        debug!("Transaction event handler stopped");
    });
    
    // Wait for shutdown signal
    handle_shutdown_signal().await?;
    
    // Perform graceful shutdown
    info!("Shutting down call engine...");
    
    // Allow some time for cleanup
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Cancel the transaction handler
    transaction_handle.abort();
    
    info!("Shutdown complete");
    
    Ok(())
}

// Test commands to use with this example:
//
// 1. Send a REGISTER request:
//    echo -n "REGISTER sip:rvoip.local SIP/2.0
//    Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-524287-1
//    Max-Forwards: 70
//    From: <sip:alice@rvoip.local>;tag=12345
//    To: <sip:alice@rvoip.local>
//    Call-ID: register-alice-1@localhost
//    CSeq: 1 REGISTER
//    Contact: <sip:alice@127.0.0.1:5070>
//    Expires: 3600
//    User-Agent: RVOIP-Test-Client/0.1.0
//    Content-Length: 0
//    
//    " | nc -u 127.0.0.1 5060
//
// 2. Send an OPTIONS request:
//    echo -n "OPTIONS sip:rvoip.local SIP/2.0
//    Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-524287-1
//    Max-Forwards: 70
//    From: <sip:alice@rvoip.local>;tag=12345
//    To: <sip:rvoip.local>
//    Call-ID: options-test-1@localhost
//    CSeq: 1 OPTIONS
//    User-Agent: RVOIP-Test-Client/0.1.0
//    Content-Length: 0
//    
//    " | nc -u 127.0.0.1 5060 