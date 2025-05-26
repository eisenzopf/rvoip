//! SIPp-Compatible Server
//!
//! This example demonstrates a production-ready SIP server that works with SIPp
//! for testing and validation. It uses only the session-core API and handles
//! the complete call lifecycle including INVITE/200 OK/ACK flow.
//!
//! Usage:
//!   cargo run --example sipp_server
//!
//! Test with SIPp:
//!   sipp -sn uac 127.0.0.1:5060 -m 1 -d 5000
//!
//! This will send an INVITE, wait for 200 OK, send ACK, wait 5 seconds, then send BYE.

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
    info!("📞 Ready to handle real SIP traffic from SIPp");

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

    info!("");
    info!("🧪 SIPp Test Commands:");
    info!("  📞 Basic call test:");
    info!("    sipp -sn uac 127.0.0.1:5060 -m 1 -d 5000");
    info!("  📊 Load test (10 calls):");
    info!("    sipp -sn uac 127.0.0.1:5060 -m 10 -r 1");
    info!("  🔄 Continuous test:");
    info!("    sipp -sn uac 127.0.0.1:5060 -d 10000");
    info!("");

    // Start the call handling task
    let server_manager = server.server_manager();
    let call_handler = tokio::spawn(async move {
        info!("🎧 Starting call handler task");
        
        loop {
            // Check for active sessions periodically
            let active_sessions = server_manager.get_active_sessions().await;
            
            if !active_sessions.is_empty() {
                debug!("📊 Active sessions: {}", active_sessions.len());
                
                // Handle each active session
                for session_id in &active_sessions {
                    if let Some(session) = server_manager.get_session(session_id).await {
                        let state = session.state().await;
                        debug!("📞 Session {} state: {}", session_id, state);
                        
                        // Auto-accept incoming calls after a brief delay (simulate human response)
                        if state == rvoip_session_core::session::session_types::SessionState::Ringing {
                            info!("📞 Auto-accepting incoming call for session {}", session_id);
                            
                            // Accept the call with automatic media setup
                            match server_manager.accept_call(session_id).await {
                                Ok(()) => {
                                    info!("✅ Call accepted successfully for session {}", session_id);
                                    info!("🎵 Media automatically set up");
                                    info!("📞 Call is now active - waiting for BYE or timeout");
                                },
                                Err(e) => {
                                    error!("❌ Failed to accept call for session {}: {}", session_id, e);
                                    
                                    // Try to reject the call if accept failed
                                    if let Err(reject_err) = server_manager.reject_call(session_id, StatusCode::ServerInternalError).await {
                                        error!("❌ Failed to reject call after accept failure: {}", reject_err);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // Brief sleep to avoid busy waiting
            sleep(Duration::from_millis(100)).await;
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
    info!("🎯 SIP Server is running and ready for SIPp testing");
    info!("📡 Listening for SIP messages on 127.0.0.1:5060");
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