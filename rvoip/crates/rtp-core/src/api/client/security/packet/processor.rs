//! Packet processing and handling
//!
//! This module provides functions for processing and handling DTLS packets.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::dtls::DtlsConnection;
use crate::srtp::SrtpContext;
use crate::api::client::security::srtp::keys;
use crate::dtls::record::{Record, ContentType};
use crate::dtls::message::handshake::HandshakeHeader;

/// Process a DTLS packet
pub async fn process_packet(
    data: &[u8],
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    remote_addr: &Arc<Mutex<Option<SocketAddr>>>,
    handshake_completed: &Arc<Mutex<bool>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
) -> Result<(), SecurityError> {
    // Log that we're passing the packet to the DTLS connection
    debug!("Client received packet of {} bytes - delegating to DTLS library", data.len());
    
    // Check if this is a DTLS packet that needs special handling
    if data.len() >= 1 && (data[0] == 20 || data[0] == 22) {
        // Process as DTLS packet
        return process_dtls_packet(data, connection, remote_addr, handshake_completed, srtp_context).await;
    }
    
    // Get connection
    let mut conn_guard = connection.lock().await;
    
    if let Some(conn) = conn_guard.as_mut() {
        // Simply delegate to the underlying DTLS connection
        if let Err(e) = conn.process_packet(data).await {
            warn!("Error processing packet: {}", e);
            return Err(SecurityError::HandshakeError(format!("Failed to process packet: {}", e)));
        }
        
        Ok(())
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Process a specific DTLS packet type
pub async fn process_dtls_packet(
    data: &[u8],
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    remote_addr: &Arc<Mutex<Option<SocketAddr>>>,
    handshake_completed: &Arc<Mutex<bool>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
) -> Result<(), SecurityError> {
    debug!("Client received {} bytes from server", data.len());
    
    // Get connection
    let mut conn_guard = connection.lock().await;
    
    if let Some(conn) = conn_guard.as_mut() {
        // Check for HelloVerifyRequest (ContentType=22, HandshakeType=3)
        if data.len() >= 14 && data[0] == 22 && data[13] == 3 {
            debug!("Detected HelloVerifyRequest packet, will handle specially");
            
            // Process the packet to extract cookie
            if let Err(e) = conn.process_packet(data).await {
                debug!("Error processing HelloVerifyRequest: {}", e);
                return Err(SecurityError::HandshakeError(format!("Failed to process HelloVerifyRequest: {}", e)));
            }
            
            // Get remote address
            let remote_addr_guard = remote_addr.lock().await;
            if let Some(addr) = *remote_addr_guard {
                // Make sure we're in the right state
                if conn.handshake_step() == Some(crate::dtls::handshake::HandshakeStep::ReceivedHelloVerifyRequest) {
                    debug!("Continuing handshake after HelloVerifyRequest");
                    // Continue the handshake explicitly with the cookie
                    if let Err(e) = conn.continue_handshake().await {
                        warn!("Failed to continue handshake with cookie: {}", e);
                        return Err(SecurityError::HandshakeError(format!("Failed to continue handshake with cookie: {}", e)));
                    } else {
                        debug!("Successfully continued handshake with cookie");
                    }
                } else {
                    // Start handshake again with the cookie
                    debug!("Restarting handshake with cookie");
                    if let Err(e) = conn.start_handshake(addr).await {
                        warn!("Failed to restart handshake with cookie: {}", e);
                        return Err(SecurityError::HandshakeError(format!("Failed to restart handshake with cookie: {}", e)));
                    } else {
                        debug!("Successfully restarted handshake with cookie");
                    }
                }
            }
            
            return Ok(());
        }
        
        // For all other packet types, process normally
        if let Err(e) = conn.process_packet(data).await {
            // Log errors but don't fail the method for common error types
            debug!("Error processing DTLS packet: {}", e);
            return Err(SecurityError::HandshakeError(format!("Failed to process DTLS packet: {}", e)));
        }
        
        // Check handshake step to determine next actions
        if let Some(step) = conn.handshake_step() {
            debug!("Current handshake step: {:?}", step);
            
            match step {
                crate::dtls::handshake::HandshakeStep::ReceivedServerHello => {
                    debug!("Received ServerHello");
                    
                    // Need to continue the handshake explicitly
                    if let Err(e) = conn.continue_handshake().await {
                        warn!("Failed to continue handshake after ServerHello: {}", e);
                        return Err(SecurityError::HandshakeError(format!("Failed to continue handshake: {}", e)));
                    } else {
                        debug!("Successfully continued handshake after ServerHello");
                    }
                },
                crate::dtls::handshake::HandshakeStep::SentClientKeyExchange => {
                    debug!("Sent ClientKeyExchange");
                    
                    // Need to complete handshake
                    if let Err(e) = conn.complete_handshake().await {
                        warn!("Failed to complete handshake: {}", e);
                        return Err(SecurityError::HandshakeError(format!("Failed to complete handshake: {}", e)));
                    } else {
                        debug!("Successfully completed handshake");
                    }
                },
                crate::dtls::handshake::HandshakeStep::Complete => {
                    debug!("Handshake complete");
                    
                    // Set handshake completed flag
                    let mut completed = handshake_completed.lock().await;
                    if !*completed {
                        *completed = true;
                        
                        // Extract SRTP keys if the connection is in the connected state
                        if conn.state() == crate::dtls::connection::ConnectionState::Connected {
                            // Drop the connection guard before extracting keys to avoid deadlock
                            drop(conn_guard);
                            
                            // Extract keys
                            if let Err(e) = keys::extract_srtp_keys(connection, srtp_context, handshake_completed).await {
                                warn!("Failed to extract SRTP keys: {}", e);
                                return Err(e);
                            }
                            
                            info!("DTLS handshake completed and SRTP keys extracted");
                        }
                    }
                },
                _ => {} // Ignore other steps
            }
        }
        
        Ok(())
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Handle a HelloVerifyRequest packet
pub async fn handle_hello_verify_request(
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    remote_addr: &Option<SocketAddr>,
) -> Result<(), SecurityError> {
    // Ensure we have a remote address
    let addr = match remote_addr {
        Some(addr) => *addr,
        None => return Err(SecurityError::Configuration("Remote address not set".to_string())),
    };
    
    // Get the connection
    let mut conn_guard = connection.lock().await;
    if let Some(conn) = conn_guard.as_mut() {
        // Make sure we're in the right state
        if conn.handshake_step() == Some(crate::dtls::handshake::HandshakeStep::ReceivedHelloVerifyRequest) {
            debug!("Handling HelloVerifyRequest, restarting handshake with cookie");
            
            // Restart handshake with the cookie
            if let Err(e) = conn.start_handshake(addr).await {
                warn!("Failed to restart handshake with cookie: {}", e);
                return Err(SecurityError::HandshakeError(format!("Failed to restart handshake with cookie: {}", e)));
            }
            
            debug!("Successfully restarted handshake with cookie");
            Ok(())
        } else {
            debug!("Not in ReceivedHelloVerifyRequest state, cannot handle");
            Err(SecurityError::HandshakeError("Not in ReceivedHelloVerifyRequest state".to_string()))
        }
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
} 