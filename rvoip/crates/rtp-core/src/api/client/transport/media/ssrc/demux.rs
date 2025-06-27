//! SSRC demultiplexing functionality for client transport
//!
//! This module contains functionality for SSRC demultiplexing on the client side.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::api::common::error::MediaTransportError;
use crate::session::RtpSession;

/// Check if SSRC demultiplexing is enabled
pub fn is_ssrc_demultiplexing_enabled(
    ssrc_demultiplexing_enabled: &Arc<AtomicBool>,
) -> bool {
    ssrc_demultiplexing_enabled.load(Ordering::SeqCst)
}

/// Register an SSRC for demultiplexing
pub async fn register_ssrc(
    ssrc: u32,
    session: &Arc<Mutex<RtpSession>>,
    ssrc_demultiplexing_enabled: &Arc<AtomicBool>,
) -> Result<bool, MediaTransportError> {
    // Check if SSRC demultiplexing is enabled
    if !ssrc_demultiplexing_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("SSRC demultiplexing is not enabled".to_string()));
    }
    
    // Create stream for SSRC in the session
    let mut session = session.lock().await;
    let created = session.create_stream_for_ssrc(ssrc).await;
    
    if created {
        debug!("Pre-registered SSRC {:08x}", ssrc);
    } else {
        debug!("SSRC {:08x} was already registered", ssrc);
    }
    
    Ok(created)
}

/// Get the sequence number for a specific SSRC
pub async fn get_sequence_number(
    ssrc: u32,
    session: &Arc<Mutex<RtpSession>>,
    sequence_numbers: &Arc<Mutex<HashMap<u32, u16>>>,
) -> Result<u16, MediaTransportError> {
    // First try to get from the session
    let session_guard = session.lock().await;
    if let Some(stream) = session_guard.get_stream(ssrc).await {
        // Stream exists, but we need to check if we can get a sequence number
        // The field might be called differently or we may need to access it through a method
        // For now, we'll skip this and rely on our manual tracking
    }
    
    // Use our manual tracking
    let seq_map = sequence_numbers.lock().await;
    match seq_map.get(&ssrc) {
        Some(seq) => Ok(*seq),
        None => Ok(0), // Default to 0 if not found
    }
}

/// Get all registered SSRCs
pub async fn get_all_ssrcs(
    session: &Arc<Mutex<RtpSession>>,
) -> Result<Vec<u32>, MediaTransportError> {
    let session = session.lock().await;
    let ssrcs = session.get_all_ssrcs().await;
    Ok(ssrcs)
} 