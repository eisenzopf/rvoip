//! CSRC management functionality
//!
//! This module handles Contributing Source (CSRC) identifier management.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::api::common::error::MediaTransportError;
use crate::{CsrcManager, CsrcMapping, RtpSsrc, RtpCsrc};

/// Check if CSRC management is enabled
pub async fn is_csrc_management_enabled(
    csrc_management_enabled: &Arc<RwLock<bool>>,
) -> Result<bool, MediaTransportError> {
    Ok(*csrc_management_enabled.read().await)
}

/// Enable CSRC management
pub async fn enable_csrc_management(
    csrc_management_enabled: &Arc<RwLock<bool>>,
) -> Result<bool, MediaTransportError> {
    // Check if already enabled
    if *csrc_management_enabled.read().await {
        return Ok(true);
    }
    
    // Set enabled flag
    *csrc_management_enabled.write().await = true;
    
    debug!("Enabled CSRC management on server");
    Ok(true)
}

/// Add a CSRC mapping for a source
pub async fn add_csrc_mapping(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    mapping: CsrcMapping,
) -> Result<(), MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Add mapping to the manager
    let mut csrc_manager_guard = csrc_manager.write().await;
    let mapping_clone = mapping.clone(); // Clone before adding
    csrc_manager_guard.add_mapping(mapping);
    
    debug!("Added CSRC mapping: {:?}", mapping_clone);
    Ok(())
}

/// Add a simple SSRC to CSRC mapping
pub async fn add_simple_csrc_mapping(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    original_ssrc: RtpSsrc,
    csrc: RtpCsrc,
) -> Result<(), MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Add simple mapping to the manager
    let mut csrc_manager_guard = csrc_manager.write().await;
    csrc_manager_guard.add_simple_mapping(original_ssrc, csrc);
    
    debug!("Added simple CSRC mapping: {:08x} -> {:08x}", original_ssrc, csrc);
    Ok(())
}

/// Remove a CSRC mapping by SSRC
pub async fn remove_csrc_mapping_by_ssrc(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    original_ssrc: RtpSsrc,
) -> Result<Option<CsrcMapping>, MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Remove mapping from the manager
    let mut csrc_manager_guard = csrc_manager.write().await;
    let removed = csrc_manager_guard.remove_by_ssrc(original_ssrc);
    
    if removed.is_some() {
        debug!("Removed CSRC mapping for SSRC: {:08x}", original_ssrc);
    }
    
    Ok(removed)
}

/// Get a CSRC mapping by SSRC
pub async fn get_csrc_mapping_by_ssrc(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    original_ssrc: RtpSsrc,
) -> Result<Option<CsrcMapping>, MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Get mapping from the manager
    let csrc_manager_guard = csrc_manager.read().await;
    let mapping = csrc_manager_guard.get_by_ssrc(original_ssrc).cloned();
    
    Ok(mapping)
}

/// Get a list of all CSRC mappings
pub async fn get_all_csrc_mappings(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
) -> Result<Vec<CsrcMapping>, MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Get all mappings from the manager
    let csrc_manager_guard = csrc_manager.read().await;
    let mappings = csrc_manager_guard.get_all_mappings().to_vec();
    
    Ok(mappings)
}

/// Update the CNAME for a source
pub async fn update_csrc_cname(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    original_ssrc: RtpSsrc,
    cname: String,
) -> Result<bool, MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Update CNAME in the manager
    let mut csrc_manager_guard = csrc_manager.write().await;
    let updated = csrc_manager_guard.update_cname(original_ssrc, cname.clone());
    
    if updated {
        debug!("Updated CNAME for SSRC {:08x}: {}", original_ssrc, cname);
    }
    
    Ok(updated)
}

/// Update the display name for a source
pub async fn update_csrc_display_name(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    original_ssrc: RtpSsrc,
    name: String,
) -> Result<bool, MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Update display name in the manager
    let mut csrc_manager_guard = csrc_manager.write().await;
    let updated = csrc_manager_guard.update_display_name(original_ssrc, name.clone());
    
    if updated {
        debug!("Updated display name for SSRC {:08x}: {}", original_ssrc, name);
    }
    
    Ok(updated)
}

/// Get CSRC values for active sources
pub async fn get_active_csrcs(
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    active_ssrcs: &[RtpSsrc],
) -> Result<Vec<RtpCsrc>, MediaTransportError> {
    // Check if CSRC management is enabled
    if !*csrc_management_enabled.read().await {
        return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
    }
    
    // Get active CSRCs from the manager
    let csrc_manager_guard = csrc_manager.read().await;
    let csrcs = csrc_manager_guard.get_active_csrcs(active_ssrcs);
    
    debug!("Got {} active CSRCs for {} active SSRCs", csrcs.len(), active_ssrcs.len());
    
    Ok(csrcs)
} 