//! DTLS handshake management
//!
//! This module provides functions for initiating and monitoring DTLS handshakes.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, error, info, warn};
use tokio::net::UdpSocket;

use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::dtls::DtlsConnection;
use crate::srtp::SrtpContext;
use crate::api::client::security::dtls::connection;
use crate::api::client::security::srtp::keys;

/// Start a DTLS handshake with the remote peer
pub async fn start_handshake(
    remote_addr: &Option<SocketAddr>,
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    // Ensure we have a remote address
    let remote_addr = remote_addr.ok_or_else(|| 
        SecurityError::Configuration("Remote address not set for handshake".to_string()))?;
    
    // Get the connection
    let mut conn_guard = connection.lock().await;
    if let Some(conn) = conn_guard.as_mut() {
        // Start handshake and send ClientHello
        debug!("Starting DTLS handshake with {}", remote_addr);
        
        if let Err(e) = conn.start_handshake(remote_addr).await {
            error!("Failed to start DTLS handshake: {}", e);
            return Err(SecurityError::Handshake(format!("Failed to start DTLS handshake: {}", e)));
        }
        
        debug!("DTLS handshake started successfully");
        Ok(())
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Wait for a DTLS handshake to complete
pub async fn wait_for_handshake(
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    handshake_completed: &Arc<Mutex<bool>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
) -> Result<(), SecurityError> {
    debug!("Waiting for DTLS handshake to complete");
    
    // Get the connection
    let mut conn_guard = connection.lock().await;
    if let Some(conn) = conn_guard.as_mut() {
        // Delegate to the DTLS library's wait_handshake
        match conn.wait_handshake().await {
            Ok(_) => {
                debug!("DTLS handshake completed successfully");
                
                // Set the handshake completed flag
                let mut completed = handshake_completed.lock().await;
                *completed = true;
                
                // Extract SRTP keys if needed
                let mut srtp_guard = srtp_context.lock().await;
                if srtp_guard.is_none() {
                    // Release the SRTP guard before calling extract_srtp_keys
                    // to avoid potential deadlock
                    drop(srtp_guard);
                    
                    // Extract SRTP keys using the dedicated function
                    if let Err(e) = keys::extract_srtp_keys(connection, srtp_context, handshake_completed).await {
                        return Err(e);
                    }
                } else {
                    // We already have SRTP context, just set the completed flag
                    *completed = true;
                }
                
                Ok(())
            },
            Err(e) => {
                Err(SecurityError::Handshake(format!("DTLS handshake failed: {}", e)))
            }
        }
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Start a handshake monitor task
pub async fn start_handshake_monitor(
    handshake_monitor_running: &Arc<AtomicBool>,
    remote_addr: &Arc<Mutex<Option<SocketAddr>>>,
    socket: &Arc<Mutex<Option<SocketHandle>>>,
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    handshake_completed: &Arc<Mutex<bool>>,
) -> Result<(), SecurityError> {
    // If already running, don't start another one
    if handshake_monitor_running.load(Ordering::SeqCst) {
        return Ok(());
    }
    
    debug!("Starting handshake monitoring task");
    
    // Set running flag
    handshake_monitor_running.store(true, Ordering::SeqCst);
    
    // Get what we need for the task - remote address and socket
    let remote_addr_guard = remote_addr.lock().await;
    let remote_addr = match remote_addr_guard.as_ref() {
        Some(addr) => *addr,
        None => {
            handshake_monitor_running.store(false, Ordering::SeqCst);
            return Err(SecurityError::Configuration("Remote address not set for handshake monitor".to_string()));
        }
    };
    drop(remote_addr_guard);
    
    let socket_guard = socket.lock().await;
    let socket = match socket_guard.as_ref() {
        Some(s) => s.socket.clone(),
        None => {
            handshake_monitor_running.store(false, Ordering::SeqCst);
            return Err(SecurityError::Configuration("Socket not set for handshake monitor".to_string()));
        }
    };
    drop(socket_guard);
    
    // Clone all the required Arc references for the task
    let connection_ref = connection.clone();
    let handshake_completed_ref = handshake_completed.clone();
    let handshake_monitor_running_ref = handshake_monitor_running.clone();
    
    // Spawn the monitor task
    tokio::spawn(async move {
        debug!("Handshake monitor task started");
        
        let mut attempt = 1;
        let max_attempts = 3;
        
        // Try the handshake process multiple times
        while attempt <= max_attempts {
            debug!("Handshake attempt #{}", attempt);
            
            // First check if handshake is already complete
            if *handshake_completed_ref.lock().await {
                debug!("Handshake already completed, stopping monitor");
                handshake_monitor_running_ref.store(false, Ordering::SeqCst);
                return;
            }
            
            // Wait to see if the handshake completes in a reasonable time
            let mut completed = false;
            for _ in 0..30 {  // Check status every 100ms for up to 3 seconds
                // Check handshake status
                let conn_guard = connection_ref.lock().await;
                
                if let Some(conn) = conn_guard.as_ref() {
                    // Check connection state first
                    if conn.state() == crate::dtls::connection::ConnectionState::Connected {
                        debug!("Connection state is Connected, handshake likely complete");
                        completed = true;
                        break;
                    }
                    
                    // Check handshake step (it may need assistance)
                    if let Some(step) = conn.handshake_step() {
                        match step {
                            crate::dtls::handshake::HandshakeStep::ReceivedHelloVerifyRequest => {
                                // This requires an explicit restart of handshake with cookie
                                debug!("Detected ReceivedHelloVerifyRequest step - need to restart handshake");
                                drop(conn_guard);
                                
                                // Get fresh connection guard for mutation
                                let mut conn_guard = connection_ref.lock().await;
                                if let Some(conn) = conn_guard.as_mut() {
                                    // HelloVerifyRequest received, restart handshake with cookie
                                    debug!("Restarting handshake with cookie from monitor");
                                    if let Err(e) = conn.start_handshake(remote_addr).await {
                                        warn!("Failed to restart handshake with cookie: {}", e);
                                    } else {
                                        info!("Successfully restarted handshake with cookie");
                                    }
                                }
                                break; // Exit loop to allow handshake to progress
                            },
                            crate::dtls::handshake::HandshakeStep::Complete => {
                                debug!("Handshake step is Complete");
                                completed = true;
                                break;
                            },
                            crate::dtls::handshake::HandshakeStep::Failed => {
                                debug!("Handshake step is Failed, will restart");
                                break;
                            },
                            _ => {
                                // Continue waiting
                                debug!("Waiting for handshake progress, current step: {:?}", step);
                            }
                        }
                    }
                }
                
                // Check for completion flag
                if *handshake_completed_ref.lock().await {
                    debug!("Handshake completed flag is set");
                    completed = true;
                    break;
                }
                
                // Wait a bit before checking again
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            
            // If completed, we're done
            if completed || *handshake_completed_ref.lock().await {
                debug!("Handshake completed successfully, exiting monitor");
                handshake_monitor_running_ref.store(false, Ordering::SeqCst);
                return;
            }
            
            // If we get here, we need to try again with a new connection
            warn!("Creating new DTLS connection for retry (attempt #{})", attempt + 1);
            
            // Create a new connection and transport
            match connection::create_connection(&socket, remote_addr).await {
                Ok(new_conn) => {
                    // Replace the old connection
                    let mut conn_guard = connection_ref.lock().await;
                    *conn_guard = Some(new_conn);
                },
                Err(e) => {
                    error!("Failed to create new connection: {}", e);
                }
            }
            
            // Increment attempt counter
            attempt += 1;
            
            // Wait a bit before next attempt
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        
        warn!("Handshake failed after {} attempts", max_attempts);
        handshake_monitor_running_ref.store(false, Ordering::SeqCst);
    });
    
    Ok(())
}

/// Check if a DTLS handshake is complete
pub fn is_handshake_complete(
    handshake_completed: &Arc<Mutex<bool>>,
) -> bool {
    // Since this is a sync function, we need to block on the async operation
    // We could use a blocking_lock() here, but that could lead to deadlocks
    // Instead, we'll just default to false if we can't get the lock immediately
    match handshake_completed.try_lock() {
        Ok(guard) => *guard,
        Err(_) => false,
    }
} 