//! Connection management for client transport
//!
//! This module handles the establishment and termination of media transport connections,
//! including transport creation, socket management, and security setup.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::time;
use tracing::{debug, error, info, warn};
use uuid;

use crate::transport::{RtpTransport, UdpRtpTransport, RtpTransportConfig, SecurityRtpTransport};
use crate::api::common::error::MediaTransportError;
use crate::api::server::security::SocketHandle;
use crate::api::client::security::ClientSecurityContext;
use crate::api::common::config::SecurityMode;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80};
use crate::srtp::crypto::SrtpCryptoKey;

/// Check if the security mode requires DTLS
pub fn requires_dtls(mode: SecurityMode) -> bool {
    matches!(mode, SecurityMode::DtlsSrtp)
}

/// Connect to the remote peer
///
/// This function establishes a connection with the remote peer by creating
/// a UDP transport, setting up security if enabled, and starting the packet
/// receiver task.
pub async fn connect(
    config_remote_address: SocketAddr,
    config_rtcp_mux: bool,
    security: &Option<Arc<dyn ClientSecurityContext>>,
    security_requires_dtls: bool,
    security_handshake_timeout_secs: u64,
    connected: &Arc<AtomicBool>,
    transport: &Arc<Mutex<Option<Arc<dyn RtpTransport>>>>,
    connect_callbacks: &Arc<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
    start_receive_task: impl Fn(Arc<dyn RtpTransport>) -> Result<(), MediaTransportError> + Send + 'static,
    srtp_key: Option<Vec<u8>>,
) -> Result<(), MediaTransportError> {
    if connected.load(Ordering::SeqCst) {
        debug!("Already connected, returning early");
        return Ok(());
    }
    
    info!("Connecting client to remote address: {}", config_remote_address);
    
    // Create UDP transport
    let transport_config = RtpTransportConfig {
        local_rtp_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
        local_rtcp_addr: None,
        symmetric_rtp: true,
        rtcp_mux: config_rtcp_mux,
        session_id: Some(format!("client-{}", uuid::Uuid::new_v4())),
        use_port_allocator: true,
    };
    
    // Create base UDP transport
    let udp_transport = UdpRtpTransport::new(transport_config).await
        .map_err(|e| MediaTransportError::ConnectionError(format!("Failed to create UDP transport: {}", e)))?;
    
    // Determine if SRTP is enabled
    let is_srtp_enabled = security.is_some();
    
    // Wrap with SecurityRtpTransport
    let transport_instance = SecurityRtpTransport::new(Arc::new(udp_transport), is_srtp_enabled).await
        .map_err(|e| MediaTransportError::ConnectionError(format!("Failed to create security transport: {}", e)))?;
    
    let transport_instance: Arc<dyn RtpTransport> = Arc::new(transport_instance);
    
    // Set the transport
    let mut transport_guard = transport.lock().await;
    *transport_guard = Some(transport_instance.clone());
    drop(transport_guard);
    
    // Wire SRTP context if security is enabled
    if let Some(security_ctx) = security {
        // Check if this is SRTP mode (not DTLS-SRTP) and we have keys
        if security_ctx.get_security_info_sync().mode == SecurityMode::Srtp {
            debug!("Configuring SRTP context with pre-shared keys");
            
            if let Some(combined_key) = &srtp_key {
                debug!("Creating SRTP context from {} byte key", combined_key.len());
                
                // Extract key and salt (first 16 bytes = key, next 14 bytes = salt)
                if combined_key.len() >= 30 {
                    let key = combined_key[0..16].to_vec();
                    let salt = combined_key[16..30].to_vec();
                    
                    debug!("Creating SrtpCryptoKey with key: {} bytes, salt: {} bytes", key.len(), salt.len());
                    let crypto_key = SrtpCryptoKey::new(key, salt);
                    
                    match SrtpContext::new(SRTP_AES128_CM_SHA1_80, crypto_key) {
                        Ok(srtp_context) => {
                            debug!("Successfully created SRTP context");
                            
                            // Set SRTP context on security transport
                            if let Some(sec_transport) = transport_instance.as_any()
                                .downcast_ref::<SecurityRtpTransport>() {
                                sec_transport.set_srtp_context(srtp_context).await;
                                info!("SRTP context successfully configured on client transport");
                            } else {
                                warn!("Failed to downcast to SecurityRtpTransport for SRTP context setting");
                            }
                        },
                        Err(e) => {
                            error!("Failed to create SRTP context: {}", e);
                            return Err(MediaTransportError::Security(format!("SRTP context creation failed: {}", e)));
                        }
                    }
                } else {
                    error!("SRTP key too short: {} bytes (expected at least 30)", combined_key.len());
                    return Err(MediaTransportError::Security("SRTP key too short".to_string()));
                }
            } else {
                warn!("SRTP mode enabled but no key provided");
            }
        }
    }
    
    // Get socket handle from the underlying UDP transport
    let socket_arc = if let Some(sec_transport) = transport_instance.as_any().downcast_ref::<SecurityRtpTransport>() {
        sec_transport.inner_transport().get_socket()
    } else {
        return Err(MediaTransportError::ConnectionError("Failed to get socket handle".to_string()));
    };

    // Create a proper SocketHandle
    let socket_handle = SocketHandle {
        socket: socket_arc,
        remote_addr: None,
    };
    
    // If security is enabled, set up the security context
    if let Some(security) = security {
        // Set remote address
        security.set_remote_address(config_remote_address).await
            .map_err(|e| MediaTransportError::Security(format!("Failed to set remote address: {}", e)))?;
            
        // Set socket
        security.set_socket(socket_handle).await
            .map_err(|e| MediaTransportError::Security(format!("Failed to set socket: {}", e)))?;
            
        // Start handshake
        security.start_handshake().await
            .map_err(|e| MediaTransportError::Security(format!("Failed to start handshake: {}", e)))?;
            
        // Only wait for handshake completion if DTLS is required
        if security_requires_dtls {
            debug!("DTLS required - waiting for handshake completion");
            let handshake_timeout = Duration::from_secs(security_handshake_timeout_secs);
            match tokio::time::timeout(handshake_timeout, wait_for_handshake_completion(security)).await {
                Ok(result) => {
                    result.map_err(|e| MediaTransportError::Security(format!("Handshake failed: {}", e)))?;
                },
                Err(_) => {
                    return Err(MediaTransportError::Security(format!("Handshake timed out after {} seconds", security_handshake_timeout_secs)));
                }
            }
        } else {
            debug!("SRTP pre-shared keys - no handshake wait needed");
        }
    }
    
    // Start receive task with the transport
    start_receive_task(transport_instance.clone())?;
    
    // Set connected flag
    connected.store(true, Ordering::SeqCst);
    
    // Notify callbacks
    let callbacks = connect_callbacks.lock().await;
    for callback in &*callbacks {
        callback();
    }
    
    info!("Client successfully connected to {}", config_remote_address);
    
    Ok(())
}

/// Wait for the DTLS handshake to complete
async fn wait_for_handshake_completion(security: &Arc<dyn ClientSecurityContext>) -> Result<(), MediaTransportError> {
    while !security.is_handshake_complete().await
        .map_err(|e| MediaTransportError::Security(format!("Failed to check handshake status: {}", e)))? {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    debug!("DTLS handshake completed successfully");
    Ok(())
}

/// Disconnect from the remote peer
///
/// This function terminates the connection with the remote peer by closing
/// the security context if enabled, closing the transport, and updating the
/// connected flag.
pub async fn disconnect(
    security: &Option<Arc<dyn ClientSecurityContext>>,
    connected: &Arc<AtomicBool>,
    transport: &Arc<Mutex<Option<Arc<dyn RtpTransport>>>>,
    disconnect_callbacks: &Arc<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
) -> Result<(), MediaTransportError> {
    if !connected.load(Ordering::SeqCst) {
        return Ok(());
    }
    
    // Close security context
    if let Some(security) = security {
        security.close().await
            .map_err(|e| MediaTransportError::Security(format!("Failed to close security context: {}", e)))?;
    }
    
    // Close transport
    let mut transport_guard = transport.lock().await;
    if let Some(transport) = transport_guard.as_ref() {
        if let Err(e) = transport.close().await {
            warn!("Failed to close transport: {}", e);
        }
    }
    *transport_guard = None;
    
    // Update connected flag
    connected.store(false, Ordering::SeqCst);
    
    // Notify callbacks
    let callbacks = disconnect_callbacks.lock().await;
    for callback in &*callbacks {
        callback();
    }
    
    Ok(())
}

/// Get the local address currently bound to
///
/// This function returns the actual bound address of the transport, which may be
/// different from the configured address if dynamic port allocation is used.
pub async fn get_local_address(
    transport: &Arc<Mutex<Option<Arc<dyn RtpTransport>>>>,
) -> Result<SocketAddr, MediaTransportError> {
    let transport_guard = transport.lock().await;
    if let Some(transport) = transport_guard.as_ref() {
        transport.local_rtp_addr()
            .map_err(|e| MediaTransportError::Transport(format!("Failed to get local address: {}", e)))
    } else {
        Err(MediaTransportError::Transport("Transport not initialized. Connect first to bind to a port.".to_string()))
    }
}

/// Check if the client is connected
///
/// This function returns true if the client is connected to the remote peer.
pub fn is_connected(
    connected: &Arc<AtomicBool>,
) -> bool {
    connected.load(Ordering::SeqCst)
} 