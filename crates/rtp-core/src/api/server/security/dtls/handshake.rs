//! DTLS handshake functionality
//!
//! This module handles DTLS handshake processing and state management.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::api::common::error::SecurityError;
use crate::api::server::security::{ClientSecurityContext, SocketHandle};
use crate::dtls::{DtlsConnection, handshake::HandshakeStep};
use crate::srtp::{SrtpContext};
use crate::api::server::security::srtp::keys;

/// Process DTLS handshake steps
pub async fn process_handshake_step(
    conn: &mut DtlsConnection,
    step: HandshakeStep,
    address: SocketAddr,
    handshake_completed: &Arc<Mutex<bool>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
) -> Result<(), SecurityError> {
    debug!("Current handshake step: {:?}", step);
    
    match step {
        HandshakeStep::SentHelloVerifyRequest => {
            debug!("Server sent HelloVerifyRequest, waiting for ClientHello with cookie");
            // No action needed, wait for client to respond with cookie
        },
        HandshakeStep::ReceivedClientHello => {
            debug!("Server received ClientHello, sending ServerHello");
            
            // Continue the handshake to send ServerHello and ServerKeyExchange
            if let Err(e) = conn.continue_handshake().await {
                warn!("Failed to continue handshake after ClientHello: {}", e);
            } else {
                debug!("Successfully sent ServerHello after ClientHello");
            }
        },
        HandshakeStep::ReceivedClientKeyExchange => {
            debug!("Server received ClientKeyExchange, sending ChangeCipherSpec and Finished");
            
            // Continue the handshake to send final messages
            if let Err(e) = conn.continue_handshake().await {
                warn!("Failed to continue handshake after ClientKeyExchange: {}", e);
            } else {
                debug!("Successfully completed handshake from server side");
            }
        },
        HandshakeStep::Complete => {
            debug!("Server handshake complete with client {}", address);
            
            // Set handshake completed flag
            let mut completed = handshake_completed.lock().await;
            if !*completed {
                *completed = true;
                
                // Extract SRTP keys using the srtp module
                match keys::extract_srtp_keys(conn, address, false).await {
                    Ok(ctx) => {
                        // Store SRTP context
                        let mut srtp_guard = srtp_context.lock().await;
                        *srtp_guard = Some(ctx);
                        debug!("Server successfully extracted SRTP keys for client {}", address);
                    },
                    Err(e) => warn!("Failed to extract SRTP keys: {}", e)
                }
            }
        },
        _ => {} // Ignore other steps
    }
    
    Ok(())
}

/// Start a DTLS handshake with a client
pub async fn start_handshake(
    conn: &mut DtlsConnection,
    address: SocketAddr,
) -> Result<(), SecurityError> {
    debug!("Starting DTLS handshake with client {}", address);
    
    match conn.start_handshake(address).await {
        Ok(_) => Ok(()),
        Err(e) => Err(SecurityError::Handshake(format!("Failed to start DTLS handshake: {}", e)))
    }
}

/// Wait for a DTLS handshake to complete
pub async fn wait_for_handshake(
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    address: SocketAddr,
    handshake_completed: &Arc<Mutex<bool>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
) -> Result<(), SecurityError> {
    debug!("Waiting for DTLS handshake completion for client {}", address);
    
    let conn_result = {
        let mut conn_guard = connection.lock().await;
        match conn_guard.as_mut() {
            Some(conn) => conn.wait_handshake().await,
            None => {
                error!("No DTLS connection for client {}", address);
                return Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()));
            }
        }
    };
    
    match conn_result {
        Ok(_) => {
            debug!("DTLS handshake completed for client {}", address);
            
            // Extract SRTP keys using the srtp module
            let conn_guard = connection.lock().await;
            if let Some(conn) = conn_guard.as_ref() {
                match keys::extract_srtp_keys(conn, address, false).await {
                    Ok(srtp_ctx) => {
                        debug!("Created SRTP context for client {}", address);
                        
                        // Store SRTP context
                        let mut srtp_guard = srtp_context.lock().await;
                        *srtp_guard = Some(srtp_ctx);
                        
                        // Set handshake completed flag
                        let mut completed = handshake_completed.lock().await;
                        *completed = true;
                        
                        debug!("DTLS handshake fully completed for client {}", address);
                        Ok(())
                    },
                    Err(e) => {
                        error!("Failed to extract SRTP keys for client {}: {}", address, e);
                        Err(e)
                    }
                }
            } else {
                Err(SecurityError::NotInitialized("DTLS connection not available".to_string()))
            }
        },
        Err(e) => {
            error!("DTLS handshake failed for client {}: {}", address, e);
            Err(SecurityError::Handshake(format!("DTLS handshake failed: {}", e)))
        }
    }
}

/// Process a DTLS packet
pub async fn process_dtls_packet(
    conn: &mut DtlsConnection,
    data: &[u8],
    address: SocketAddr,
    handshake_completed: &Arc<Mutex<bool>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
) -> Result<(), SecurityError> {
    debug!("Server processing DTLS packet of {} bytes from client {}", data.len(), address);
    
    // Process the packet with the DTLS library
    match conn.process_packet(data).await {
        Ok(_) => {
            // Take action based on handshake step
            if let Some(step) = conn.handshake_step() {
                process_handshake_step(conn, step, address, handshake_completed, srtp_context).await?;
            }
            
            Ok(())
        },
        Err(e) => {
            debug!("Error processing DTLS packet: {}", e);
            
            // If this was a cookie validation error, we might need to restart
            if e.to_string().contains("Invalid cookie") {
                debug!("Cookie validation failed, restarting handshake");
                
                // Start a new handshake
                if let Err(restart_err) = conn.start_handshake(address).await {
                    warn!("Failed to restart handshake: {}", restart_err);
                }
            }
            
            // Return success to allow handshake to continue
            Ok(())
        }
    }
} 