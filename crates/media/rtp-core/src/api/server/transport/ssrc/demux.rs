//! SSRC demultiplexing functionality
//!
//! This module handles SSRC demultiplexing for multiple streams.

use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::debug;

use crate::api::common::error::MediaTransportError;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::RtpSsrc;

/// Check if SSRC demultiplexing is enabled
pub async fn is_ssrc_demultiplexing_enabled(
    ssrc_demultiplexing_enabled: &Arc<AtomicBool>,
) -> Result<bool, MediaTransportError> {
    Ok(ssrc_demultiplexing_enabled.load(Ordering::Relaxed))
}

/// Enable SSRC demultiplexing
pub async fn enable_ssrc_demultiplexing(
    ssrc_demultiplexing_enabled: &Arc<AtomicBool>,
) -> Result<bool, MediaTransportError> {
    // CAS so re-enable is a no-op visible via the return value.
    if ssrc_demultiplexing_enabled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(true);
    }
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
    ssrc_demultiplexing_enabled: &Arc<AtomicBool>,
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<bool, MediaTransportError> {
    // Check if SSRC demultiplexing is enabled
    if !ssrc_demultiplexing_enabled.load(Ordering::Relaxed) {
        return Err(MediaTransportError::ConfigError(
            "SSRC demultiplexing is not enabled".to_string(),
        ));
    }

    let session_arc = {
        let client = clients
            .get(client_id)
            .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
        if !client.connected {
            return Err(MediaTransportError::ClientNotConnected(
                client_id.to_string(),
            ));
        }
        client.session.clone()
    };

    let mut session = session_arc.lock().await;
    let created = session.create_stream_for_ssrc(ssrc).await;

    if created {
        debug!("Pre-registered SSRC {:08x} for client {}", ssrc, client_id);
    } else {
        debug!(
            "SSRC {:08x} was already registered for client {}",
            ssrc, client_id
        );
    }

    Ok(created)
}

/// Get a list of all known SSRCs for a client
///
/// Returns all SSRCs that have been received or manually registered for the specified client.
pub async fn get_client_ssrcs(
    client_id: &str,
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<Vec<u32>, MediaTransportError> {
    let session_arc = {
        let client = clients
            .get(client_id)
            .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
        if !client.connected {
            return Err(MediaTransportError::ClientNotConnected(
                client_id.to_string(),
            ));
        }
        client.session.clone()
    };

    let session = session_arc.lock().await;
    let ssrcs = session.get_all_ssrcs().await;

    Ok(ssrcs)
}
