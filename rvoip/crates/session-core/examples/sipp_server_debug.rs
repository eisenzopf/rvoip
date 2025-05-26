//! Debug SIPp-Compatible Server
//!
//! This is a debug version of the SIPp server with extensive logging
//! to help diagnose transaction-core integration issues.

use std::time::Duration;
use anyhow::Result;
use tracing::{info, debug, warn, error};
use tokio::signal;
use tokio::time::{sleep, timeout};

use rvoip_session_core::api::{create_sip_server, ServerConfig, TransportProtocol};
use rvoip_sip_core::StatusCode;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize very detailed logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    info!("🔍 Starting DEBUG SIPp-Compatible SIP Server");
    info!("📞 This version has extensive logging for debugging");

    // Create server configuration
    let config = ServerConfig {
        bind_address: "127.0.0.1:5060".parse().unwrap(),
        transport_protocol: TransportProtocol::Udp,
        max_sessions: 10, // Smaller for debugging
        session_timeout: Duration::from_secs(300),
        transaction_timeout: Duration::from_secs(32),
        enable_media: true,
        server_name: "Debug-SIPp-Server/1.0".to_string(),
        contact_uri: Some("sip:debug@127.0.0.1:5060".to_string()),
    };

    info!("📋 Debug Server Configuration:");
    info!("  📍 Bind Address: {}", config.bind_address);
    info!("  🚛 Transport: {}", config.transport_protocol);
    info!("  📊 Max Sessions: {}", config.max_sessions);

    // Create the SIP server
    info!("🔧 Creating SIP server...");
    let mut server = match timeout(Duration::from_secs(10), create_sip_server(config)).await {
        Ok(Ok(server)) => {
            info!("✅ SIP server created successfully");
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

    info!("🎯 Debug server ready on 127.0.0.1:5060");
    info!("🧪 Send a simple SIPp test:");
    info!("   sipp -sn uac 127.0.0.1:5060 -m 1 -d 3000 -trace_msg");

    // Start a simple call handler that just logs what it sees
    let server_manager = server.server_manager();
    let call_handler = tokio::spawn(async move {
        info!("🎧 Starting debug call handler");
        
        let mut check_count = 0;
        loop {
            check_count += 1;
            
            // Check for active sessions
            let active_sessions = server_manager.get_active_sessions().await;
            
            if check_count % 50 == 0 { // Log every 5 seconds
                debug!("🔍 Periodic check #{}: {} active sessions", check_count, active_sessions.len());
            }
            
            if !active_sessions.is_empty() {
                info!("📊 Found {} active sessions", active_sessions.len());
                
                for session_id in &active_sessions {
                    if let Some(session) = server_manager.get_session(session_id).await {
                        let state = session.state().await;
                        info!("📞 Session {} state: {}", session_id, state);
                        
                        // Auto-accept ringing calls
                        if state == rvoip_session_core::session::session_types::SessionState::Ringing {
                            info!("🔔 Auto-accepting ringing call for session {}", session_id);
                            
                            match server_manager.accept_call(session_id).await {
                                Ok(()) => {
                                    info!("✅ Successfully accepted call for session {}", session_id);
                                },
                                Err(e) => {
                                    error!("❌ Failed to accept call for session {}: {}", session_id, e);
                                }
                            }
                        }
                    }
                }
            }
            
            sleep(Duration::from_millis(100)).await;
        }
    });

    // Start the main server event processing with detailed logging
    let event_handler = tokio::spawn(async move {
        info!("🎧 Starting debug server event processing");
        
        match server.run().await {
            Ok(()) => {
                info!("✅ Server event processing completed normally");
            },
            Err(e) => {
                error!("❌ Server event processing error: {}", e);
            }
        }
    });

    info!("🎯 Debug SIP Server is running");
    info!("📡 Listening for SIP messages on 127.0.0.1:5060");
    info!("🔍 All transaction events will be logged in detail");
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

    info!("🔄 Shutting down debug server...");
    
    // Cancel tasks
    call_handler.abort();
    event_handler.abort();
    
    sleep(Duration::from_millis(500)).await;
    
    info!("✅ Debug SIP Server shutdown complete");
    Ok(())
} 