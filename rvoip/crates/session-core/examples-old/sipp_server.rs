//! SIPp-Compatible Server
//!
//! This example demonstrates a production-ready SIP server that works with SIPp
//! for testing and validation. It uses the new session-core notification system
//! for automatic call handling with ServerManager policy decisions.
//!
//! **Architectural Features:**
//! - Automatic call handling via IncomingCallNotification system
//! - ServerManager makes policy decisions (accept/reject based on capacity, business rules)
//! - SessionManager implements SIP operations (SDP processing, response building)
//! - Clean separation of concerns with proper delegation patterns
//! - Memory leak prevention with automatic cleanup systems
//!
//! Usage:
//!   cargo run --example sipp_server
//!
//! Test with SIPp:
//!   sipp -sn uac 127.0.0.1:5060 -m 1 -d 5000
//!
//! This will send an INVITE, receive automatic 200 OK with real audio, send ACK, 
//! wait 5 seconds with RTP audio transmission, then send BYE for automatic cleanup.

use std::time::Duration;
use anyhow::Result;
use tracing::{info, debug, warn, error};
use tokio::signal;
use tokio::time::{sleep, timeout};

use rvoip_session_core::api::{create_sip_server, ServerConfig, TransportProtocol};
use rvoip_sip_core::StatusCode;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize comprehensive logging for SIP debugging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    info!("🚀 Starting SIPp-Compatible SIP Server");
    info!("📞 Ready to handle real SIP traffic with automatic call processing");
    info!("🎯 ServerManager policy: auto-accept calls based on server capacity and business rules");

    // Create production server configuration
    let config = ServerConfig {
        bind_address: "127.0.0.1:5060".parse().unwrap(),
        transport_protocol: TransportProtocol::Udp,
        max_sessions: 1000, // Support many concurrent calls
        session_timeout: Duration::from_secs(300), // 5 minutes
        transaction_timeout: Duration::from_secs(32), // RFC 3261 Timer B
        enable_media: true,
        server_name: "SIPp-Compatible-Server/1.0".to_string(),
        contact_uri: Some("sip:server@127.0.0.1:5060".to_string()),
    };

    info!("📋 Server Configuration:");
    info!("  📍 Bind Address: {}", config.bind_address);
    info!("  🚛 Transport: {}", config.transport_protocol);
    info!("  📊 Max Sessions: {}", config.max_sessions);
    info!("  ⏱️  Session Timeout: {:?}", config.session_timeout);
    info!("  🎵 Media Enabled: {}", config.enable_media);

    // Create the SIP server using session-core API
    let mut server = match timeout(Duration::from_secs(10), create_sip_server(config)).await {
        Ok(Ok(server)) => {
            info!("✅ SIP server created successfully");
            info!("🎯 Server ready for SIPp testing on 127.0.0.1:5060");
            server
        },
        Ok(Err(e)) => {
            error!("❌ Failed to create SIP server: {}", e);
            return Err(e);
        },
        Err(_) => {
            error!("❌ Timeout creating SIP server");
            return Err(anyhow::anyhow!("Timeout creating SIP server"));
        }
    };

    // **CRITICAL**: Start the managers to activate automatic cleanup and prevent memory leaks
    info!("🧹 Starting automatic cleanup systems...");
    
    // Start ServerManager with automatic cleanup (fixes memory leaks)
    if let Err(e) = server.server_manager().start().await {
        error!("❌ Failed to start ServerManager cleanup: {}", e);
        return Err(anyhow::anyhow!("Failed to start ServerManager cleanup: {}", e));
    }
    
    // Start SessionManager with automatic cleanup (if not already started)
    if let Err(e) = server.session_manager().start().await {
        error!("❌ Failed to start SessionManager cleanup: {}", e);
        return Err(anyhow::anyhow!("Failed to start SessionManager cleanup: {}", e));
    }
    
    info!("✅ Automatic cleanup systems started - memory leaks prevented");
    info!("🎯 Cleanup runs every 30 seconds to remove terminated sessions/dialogs");

    info!("");
    info!("🧪 SIPp Test Commands:");
    info!("  📞 Basic call test (automatic accept/media/cleanup):");
    info!("    sipp -sn uac 127.0.0.1:5060 -m 1 -d 5000");
    info!("  📊 Load test (10 automatic calls):");
    info!("    sipp -sn uac 127.0.0.1:5060 -m 10 -r 1");
    info!("  🔄 Continuous test (automatic handling):");
    info!("    sipp -sn uac 127.0.0.1:5060 -d 10000");
    info!("  🚀 High-volume test (1000 automatic calls):");
    info!("    sipp -sf basic_call.xml -m 1000 -r 100 127.0.0.1:5060");
    info!("  💡 All calls handled automatically: INVITE→policy decision→200 OK→media→BYE→cleanup");
    info!("");

    // Start the call handling task
    let server_manager = server.server_manager();
    let call_handler = tokio::spawn(async move {
        info!("🎧 Starting automatic call monitoring task");
        
        loop {
            // Monitor active sessions periodically (for logging/debugging only)
            let active_sessions = server_manager.get_active_sessions().await;
            
            if !active_sessions.is_empty() {
                debug!("📊 Active sessions: {}", active_sessions.len());
                
                // Log session states for monitoring (no manual intervention needed)
                for session_id in &active_sessions {
                    if let Some(session) = server_manager.get_session(session_id).await {
                        let state = session.state().await;
                        debug!("📞 Session {} state: {}", session_id, state);
                    }
                }
            }
            
            // Brief sleep to avoid busy waiting
            sleep(Duration::from_millis(1000)).await; // Reduced frequency since no manual handling needed
        }
    });

    // Start the main server event processing
    let event_handler = tokio::spawn(async move {
        info!("🎧 Starting server event processing");
        
        match server.run().await {
            Ok(()) => {
                info!("✅ Server event processing completed normally");
            },
            Err(e) => {
                error!("❌ Server event processing error: {}", e);
            }
        }
    });

    // Wait for shutdown signal
    info!("🎯 SIP Server is running with automatic call handling");
    info!("📡 Listening for SIP messages on 127.0.0.1:5060");
    info!("🤖 ServerManager will automatically accept calls based on policy");
    info!("🎵 SessionManager will automatically handle SIP operations and media");
    info!("🛑 Press Ctrl+C to shutdown");

    // Handle graceful shutdown
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("🛑 Shutdown signal received");
        },
        Err(err) => {
            error!("❌ Unable to listen for shutdown signal: {}", err);
        },
    }

    info!("🔄 Shutting down server...");
    
    // Cancel tasks
    call_handler.abort();
    event_handler.abort();
    
    // Wait a moment for cleanup
    sleep(Duration::from_millis(500)).await;
    
    info!("✅ SIP Server shutdown complete");
    Ok(())
} 