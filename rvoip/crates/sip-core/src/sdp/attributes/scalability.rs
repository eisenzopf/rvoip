//! SDP Scalability Mode Attribute Parser
//!
//! Implements parser for scalability mode attributes used in SVC media codecs
//! Format: a=fmtp:<payload> scalability-mode=<mode>

use crate::error::{Error, Result};

/// Parses scalability mode for AV1, H.264, and VP9: a=fmtp:<payload> scalability-mode=<mode>
/// This is for Scalable Video Coding (SVC) scenarios, often used with simulcast
///
/// Examples:
/// - L1T2: Spatial layer 1, temporal layer 2
/// - S2T3: Simulcast with 2 streams, 3 temporal layers
/// - K1: Key frame dependence mode
pub fn parse_scalability_mode(mode: &str) -> Result<(String, Option<u32>, Option<u32>, Option<String>)> {    
    // Extracts SVC parameters from mode string like "L2T3" or "S2T3"
    // Returns (pattern, spatial_layers, temporal_layers, extra)
    
    if mode.is_empty() {
        return Err(Error::SdpParsingError("Empty scalability mode".to_string()));
    }
    
    // Basic pattern is a letter followed by optional numbers and more patterns
    let pattern_char = mode.chars().next().unwrap().to_ascii_uppercase();
    
    // Validate pattern character
    if !['L', 'S', 'K'].contains(&pattern_char) {
        return Err(Error::SdpParsingError(format!("Invalid scalability mode pattern: {}", pattern_char)));
    }
    
    let pattern = pattern_char.to_string();
    
    // Parse spatial and temporal layers
    let mut spatial_layers: Option<u32> = None;
    let mut temporal_layers: Option<u32> = None;
    let mut extra: Option<String> = None;
    
    // Simple parsing - in practice would use regex
    if mode.len() > 1 {
        let rest = &mode[1..];
        if rest.contains('T') {
            let parts: Vec<&str> = rest.split('T').collect();
            if parts.len() >= 2 {
                // Try to parse spatial layers (before 'T')
                if !parts[0].is_empty() {
                    if let Ok(num) = parts[0].parse::<u32>() {
                        spatial_layers = Some(num);
                    } else {
                        extra = Some(rest.to_string());
                    }
                }
                
                // Parse temporal layers (after 'T')
                let temporal_part = parts[1];
                if !temporal_part.is_empty() {
                    if let Ok(num) = temporal_part.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u32>() {
                        temporal_layers = Some(num);
                    }
                    
                    // Check for extra info
                    let extra_part = temporal_part.chars()
                        .skip_while(|c| c.is_ascii_digit())
                        .collect::<String>();
                    if !extra_part.is_empty() {
                        extra = Some(extra_part);
                    }
                }
            } else {
                extra = Some(rest.to_string());
            }
        } else if rest.chars().all(|c| c.is_ascii_digit()) {
            // Just a number, likely spatial layers
            if let Ok(num) = rest.parse::<u32>() {
                spatial_layers = Some(num);
            }
        } else {
            // Something else, store as extra
            extra = Some(rest.to_string());
        }
    }
    
    Ok((pattern, spatial_layers, temporal_layers, extra))
} 