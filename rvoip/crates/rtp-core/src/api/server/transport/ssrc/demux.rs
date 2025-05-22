//! SSRC demultiplexing functionality
//!
//! This module handles SSRC demultiplexing for multiple streams.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::api::common::error::MediaTransportError;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::{RtpSsrc};

/// Check if SSRC demultiplexing is enabled
pub async fn is_ssrc_demultiplexing_enabled(
    ssrc_demultiplexing_enabled: &Arc<RwLock<bool>>,
) -> Result<bool, MediaTransportError> {
    Ok(*ssrc_demultiplexing_enabled.read().await)
}

/// Enable SSRC demultiplexing
pub async fn enable_ssrc_demultiplexing(
    ssrc_demultiplexing_enabled: &Arc<RwLock<bool>>,
) -> Result<bool, MediaTransportError> {
    // Check if already enabled
    if *ssrc_demultiplexing_enabled.read().await {
        return Ok(true);
    }
    
    // Set enabled flag
    *ssrc_demultiplexing_enabled.write().await = true;
    
    debug!("Enabled SSRC demultiplexing on server");
    Ok(true)
}

/// Register an SSRC for a specific client
///
/// Returns true if the stream was created, false if it already existed or if demultiplexing
/// is disabled.
pub async fn register_client_ssrc(
    client_id: &str,
    ssrc: RtpSsrc,
    ssrc_demultiplexing_enabled: &Arc<RwLock<bool>>,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<bool, MediaTransportError> {
    // Check if SSRC demultiplexing is enabled
    if !*ssrc_demultiplexing_enabled.read().await {
        return Err(MediaTransportError::ConfigError("SSRC demultiplexing is not enabled".to_string()));
    }
    
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Create stream for SSRC in the session
    let mut session = client.session.lock().await;
    let created = session.create_stream_for_ssrc(ssrc).await;
    
    if created {
        debug!("Pre-registered SSRC {:08x} for client {}", ssrc, client_id);
    } else {
        debug!("SSRC {:08x} was already registered for client {}", ssrc, client_id);
    }
    
    Ok(created)
}

/// Get a list of all known SSRCs for a client
///
/// Returns all SSRCs that have been received or manually registered for the specified client.
pub async fn get_client_ssrcs(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<Vec<u32>, MediaTransportError> {
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Get all SSRCs for the client
    let session = client.session.lock().await;
    let ssrcs = session.get_all_ssrcs().await;
    
    Ok(ssrcs)
}

/// Find clients by SSRC
///
/// Returns a list of client IDs that have the given SSRC registered.
pub async fn find_clients_by_ssrc(
    ssrc: RtpSsrc,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<Vec<String>, MediaTransportError> {
    let mut result = Vec::new();
    
    // Search all clients
    let clients_guard = clients.read().await;
    
    for (client_id, client) in clients_guard.iter() {
        if !client.connected {
            continue;
        }
        
        // Get all SSRCs for the client
        let session = client.session.lock().await;
        let ssrcs = session.get_all_ssrcs().await;
        
        // Check if this client has the target SSRC
        if ssrcs.contains(&ssrc) {
            result.push(client_id.clone());
        }
    }
    
    Ok(result)
}

/// Map an SSRC to a client ID
///
/// Returns the client ID for the given SSRC, or None if not found.
pub async fn map_ssrc_to_client_id(
    ssrc: RtpSsrc,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<Option<String>, MediaTransportError> {
    // Find all clients with this SSRC
    let matches = find_clients_by_ssrc(ssrc, clients).await?;
    
    // If there's at least one match, return the first one
    Ok(matches.into_iter().next())
} 