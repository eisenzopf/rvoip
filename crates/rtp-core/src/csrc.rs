//! CSRC (Contributing Source) management for RTP
//!
//! This module provides constants and utilities for working with CSRC identifiers
//! in mixed RTP streams, as defined in RFC 3550.
//!
//! In conferencing applications where multiple users are participating in a session,
//! an RTP mixer can combine media from multiple sources into a single output stream.
//! The mixer becomes the synchronization source (SSRC) of the new stream, and the
//! original sources are listed as contributing sources (CSRCs) in the RTP header.

use crate::RtpCsrc;
use crate::RtpSsrc;

/// Maximum number of CSRCs allowed in an RTP packet (as per RFC 3550)
pub const MAX_CSRC_COUNT: u8 = 15;

/// Represents a source mapping for RTP mixers
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrcMapping {
    /// The original SSRC of the contributing source
    pub original_ssrc: RtpSsrc,
    
    /// The CSRC value used in the mixed stream
    pub csrc: RtpCsrc,
    
    /// Optional canonical name (CNAME) for this source
    pub cname: Option<String>,
    
    /// Optional display name for this source
    pub display_name: Option<String>,
}

impl CsrcMapping {
    /// Create a new CSRC mapping with just the SSRC and CSRC values
    pub fn new(original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Self {
        Self {
            original_ssrc,
            csrc,
            cname: None,
            display_name: None,
        }
    }
    
    /// Create a new CSRC mapping with CNAME
    pub fn with_cname(original_ssrc: RtpSsrc, csrc: RtpCsrc, cname: String) -> Self {
        Self {
            original_ssrc,
            csrc,
            cname: Some(cname),
            display_name: None,
        }
    }
    
    /// Create a new CSRC mapping with CNAME and display name
    pub fn with_names(
        original_ssrc: RtpSsrc, 
        csrc: RtpCsrc, 
        cname: String, 
        display_name: String
    ) -> Self {
        Self {
            original_ssrc,
            csrc,
            cname: Some(cname),
            display_name: Some(display_name),
        }
    }
    
    /// Add or update the CNAME for this mapping
    pub fn set_cname(&mut self, cname: String) {
        self.cname = Some(cname);
    }
    
    /// Add or update the display name for this mapping
    pub fn set_display_name(&mut self, name: String) {
        self.display_name = Some(name);
    }
}

/// Manages CSRC mappings for an RTP mixer
#[derive(Debug, Default)]
pub struct CsrcManager {
    /// Maps original SSRCs to their CSRC values and metadata
    mappings: Vec<CsrcMapping>,
}

impl CsrcManager {
    /// Create a new CSRC manager
    pub fn new() -> Self {
        Self {
            mappings: Vec::new(),
        }
    }
    
    /// Add a new CSRC mapping
    pub fn add_mapping(&mut self, mapping: CsrcMapping) {
        // Remove any existing mapping with the same original SSRC
        self.remove_by_ssrc(mapping.original_ssrc);
        
        // Add the new mapping
        self.mappings.push(mapping);
    }
    
    /// Add a simple SSRC to CSRC mapping
    pub fn add_simple_mapping(&mut self, original_ssrc: RtpSsrc, csrc: RtpCsrc) {
        self.add_mapping(CsrcMapping::new(original_ssrc, csrc));
    }
    
    /// Remove a mapping by original SSRC
    pub fn remove_by_ssrc(&mut self, original_ssrc: RtpSsrc) -> Option<CsrcMapping> {
        if let Some(index) = self.mappings.iter().position(|m| m.original_ssrc == original_ssrc) {
            Some(self.mappings.remove(index))
        } else {
            None
        }
    }
    
    /// Remove a mapping by CSRC value
    pub fn remove_by_csrc(&mut self, csrc: RtpCsrc) -> Option<CsrcMapping> {
        if let Some(index) = self.mappings.iter().position(|m| m.csrc == csrc) {
            Some(self.mappings.remove(index))
        } else {
            None
        }
    }
    
    /// Get a mapping by original SSRC
    pub fn get_by_ssrc(&self, original_ssrc: RtpSsrc) -> Option<&CsrcMapping> {
        self.mappings.iter().find(|m| m.original_ssrc == original_ssrc)
    }
    
    /// Get a mapping by CSRC value
    pub fn get_by_csrc(&self, csrc: RtpCsrc) -> Option<&CsrcMapping> {
        self.mappings.iter().find(|m| m.csrc == csrc)
    }
    
    /// Get a list of all CSRC values for active sources
    pub fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Vec<RtpCsrc> {
        active_ssrcs
            .iter()
            .filter_map(|&ssrc| self.get_by_ssrc(ssrc).map(|m| m.csrc))
            .collect()
    }
    
    /// Get a list of all mappings
    pub fn get_all_mappings(&self) -> &[CsrcMapping] {
        &self.mappings
    }
    
    /// Update the CNAME for a source
    pub fn update_cname(&mut self, original_ssrc: RtpSsrc, cname: String) -> bool {
        if let Some(mapping) = self.mappings.iter_mut().find(|m| m.original_ssrc == original_ssrc) {
            mapping.cname = Some(cname);
            true
        } else {
            false
        }
    }
    
    /// Update the display name for a source
    pub fn update_display_name(&mut self, original_ssrc: RtpSsrc, name: String) -> bool {
        if let Some(mapping) = self.mappings.iter_mut().find(|m| m.original_ssrc == original_ssrc) {
            mapping.display_name = Some(name);
            true
        } else {
            false
        }
    }
    
    /// Clear all mappings
    pub fn clear(&mut self) {
        self.mappings.clear();
    }
    
    /// Get the number of mappings
    pub fn len(&self) -> usize {
        self.mappings.len()
    }
    
    /// Check if there are no mappings
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
} 