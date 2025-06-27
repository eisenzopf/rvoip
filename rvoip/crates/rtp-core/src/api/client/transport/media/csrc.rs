//! CSRC management functionality
//!
//! This module handles CSRC (Contributing Source) management for RTP packets,
//! including mapping between original SSRCs and CSRCs, and metadata tracking.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, warn, info};

use crate::api::common::error::MediaTransportError;
use crate::{CsrcManager, CsrcMapping, RtpSsrc, RtpCsrc, MAX_CSRC_COUNT};

/// Check if CSRC management is enabled
pub fn is_csrc_management_enabled(
    csrc_management_enabled: &Arc<AtomicBool>,
) -> bool {
    csrc_management_enabled.load(Ordering::SeqCst)
}

/// Enable CSRC management
pub async fn enable_csrc_management(
    csrc_management_enabled: &Arc<AtomicBool>,
) -> Result<bool, MediaTransportError> {
    // Placeholder for the extracted enable_csrc_management functionality
    // Check if already enabled
    if csrc_management_enabled.load(Ordering::SeqCst) {
        return Ok(true);
    }
    
    // Set enabled flag
    csrc_management_enabled.store(true, Ordering::SeqCst);
    
    debug!("Enabled CSRC management");
    Ok(true)
}

/// Add a CSRC mapping for a source
pub async fn add_csrc_mapping(
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
    mapping: CsrcMapping,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted add_csrc_mapping functionality
    // Check if CSRC management is enabled
    if !csrc_management_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Add mapping to the manager
    let mut csrc_manager = csrc_manager.lock().await;
    let mapping_clone = mapping.clone(); // Clone before adding
    csrc_manager.add_mapping(mapping);
    
    debug!("Added CSRC mapping: {:?}", mapping_clone);
    Ok(())
}

/// Add a simple SSRC to CSRC mapping
pub async fn add_simple_csrc_mapping(
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
    original_ssrc: RtpSsrc,
    csrc: RtpCsrc,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted add_simple_csrc_mapping functionality
    // Check if CSRC management is enabled
    if !csrc_management_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Add simple mapping to the manager
    let mut csrc_manager = csrc_manager.lock().await;
    csrc_manager.add_simple_mapping(original_ssrc, csrc);
    
    debug!("Added simple CSRC mapping: {:08x} -> {:08x}", original_ssrc, csrc);
    Ok(())
}

/// Remove a CSRC mapping by SSRC
pub async fn remove_csrc_mapping_by_ssrc(
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
    original_ssrc: RtpSsrc,
) -> Result<Option<CsrcMapping>, MediaTransportError> {
    // Placeholder for the extracted remove_csrc_mapping_by_ssrc functionality
    // Check if CSRC management is enabled
    if !csrc_management_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Remove mapping from the manager
    let mut csrc_manager = csrc_manager.lock().await;
    let removed = csrc_manager.remove_by_ssrc(original_ssrc);
    
    if removed.is_some() {
        debug!("Removed CSRC mapping for SSRC: {:08x}", original_ssrc);
    }
    
    Ok(removed)
}

/// Get a CSRC mapping by SSRC
pub async fn get_csrc_mapping_by_ssrc(
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
    original_ssrc: RtpSsrc,
) -> Result<Option<CsrcMapping>, MediaTransportError> {
    // Placeholder for the extracted get_csrc_mapping_by_ssrc functionality
    // Check if CSRC management is enabled
    if !csrc_management_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Get mapping from the manager
    let csrc_manager = csrc_manager.lock().await;
    let mapping = csrc_manager.get_by_ssrc(original_ssrc).cloned();
    
    Ok(mapping)
}

/// Get all CSRC mappings
pub async fn get_all_csrc_mappings(
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
) -> Result<Vec<CsrcMapping>, MediaTransportError> {
    // Placeholder for the extracted get_all_csrc_mappings functionality
    // Check if CSRC management is enabled
    if !csrc_management_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Get all mappings from the manager
    let csrc_manager = csrc_manager.lock().await;
    let mappings = csrc_manager.get_all_mappings().to_vec();
    
    Ok(mappings)
}

/// Get CSRC values for active sources
pub async fn get_active_csrcs(
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
    active_ssrcs: &[RtpSsrc],
) -> Result<Vec<RtpCsrc>, MediaTransportError> {
    // Placeholder for the extracted get_active_csrcs functionality
    // Check if CSRC management is enabled
    if !csrc_management_enabled.load(Ordering::SeqCst) {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Get active CSRCs from the manager
    let csrc_manager = csrc_manager.lock().await;
    let csrcs = csrc_manager.get_active_csrcs(active_ssrcs);
    
    debug!("Got {} active CSRCs for {} active SSRCs", csrcs.len(), active_ssrcs.len());
    
    Ok(csrcs)
} 