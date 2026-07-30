//! DTLS handshake management
//!
//! This module provides functions for initiating and monitoring DTLS handshakes.

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error};

use crate::api::client::security::srtp::keys;
use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::dtls::DtlsConnection;
use crate::srtp::SrtpContext;

/// Start a DTLS handshake with the remote peer
pub async fn start_handshake(
    remote_addr: &Option<SocketAddr>,
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    // Ensure we have a remote address
    let remote_addr = remote_addr.ok_or_else(|| {
        SecurityError::Configuration("Remote address not set for handshake".to_string())
    })?;

    // Get the connection
    let mut conn_guard = connection.lock().await;
    if let Some(conn) = conn_guard.as_mut() {
        // Start handshake and send ClientHello
        debug!("Starting DTLS handshake with {}", remote_addr);

        if let Err(e) = conn.start_handshake(remote_addr).await {
            error!("Failed to start DTLS handshake: {}", e);
            return Err(SecurityError::Handshake(format!(
                "Failed to start DTLS handshake: {}",
                e
            )));
        }

        debug!("DTLS handshake started successfully");
        Ok(())
    } else {
        Err(SecurityError::NotInitialized(
            "DTLS connection not initialized".to_string(),
        ))
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
                let srtp_guard = srtp_context.lock().await;
                if srtp_guard.is_none() {
                    // Release the SRTP guard before calling extract_srtp_keys
                    // to avoid potential deadlock
                    drop(srtp_guard);

                    // Extract SRTP keys using the dedicated function
                    if let Err(e) =
                        keys::extract_srtp_keys(connection, srtp_context, handshake_completed).await
                    {
                        return Err(e);
                    }
                } else {
                    // We already have SRTP context, just set the completed flag
                    *completed = true;
                }

                Ok(())
            }
            Err(e) => Err(SecurityError::Handshake(format!(
                "DTLS handshake failed: {}",
                e
            ))),
        }
    } else {
        Err(SecurityError::NotInitialized(
            "DTLS connection not initialized".to_string(),
        ))
    }
}

/// Start a handshake monitor task
pub async fn start_handshake_monitor(
    _handshake_monitor_running: &Arc<AtomicBool>,
    _remote_addr: &Arc<Mutex<Option<SocketAddr>>>,
    _socket: &Arc<Mutex<Option<SocketHandle>>>,
    _connection: &Arc<Mutex<Option<DtlsConnection>>>,
    _handshake_completed: &Arc<Mutex<bool>>,
) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS handshake monitoring is unavailable in rvoip 0.3.5".to_string(),
    ))
}

/// Check if a DTLS handshake is complete
pub fn is_handshake_complete(handshake_completed: &Arc<Mutex<bool>>) -> bool {
    // Since this is a sync function, we need to block on the async operation
    // We could use a blocking_lock() here, but that could lead to deadlocks
    // Instead, we'll just default to false if we can't get the lock immediately
    match handshake_completed.try_lock() {
        Ok(guard) => *guard,
        Err(_) => false,
    }
}
