//! SDP Scalability Mode Attribute Parser
//!
//! Implements parser for scalability mode attributes used in SVC media codecs
//! Format: a=fmtp:<payload> scalability-mode=<mode>
//!
//! Based on WebRTC specifications for SVC (Scalable Video Coding)
//! as referenced in RFC 8851 and RFC 8853.

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
    
    // Return early if only the pattern character is present
    if mode.len() <= 1 {
        return Ok((pattern, spatial_layers, temporal_layers, extra));
    }
    
    // Get the part after the pattern character
    let rest = &mode[1..];
    
    // Count the number of T characters (case-insensitive)
    let t_count = rest.chars()
        .filter(|&c| c.eq_ignore_ascii_case(&'T'))
        .count();
    
    // Reject if there are multiple T characters
    if t_count > 1 {
        return Err(Error::SdpParsingError("Multiple T markers in scalability mode".to_string()));
    }
    
    // Find the first 'T' or 't' in the string (case-insensitive)
    let t_pos = rest.char_indices()
        .find(|(_, c)| c.eq_ignore_ascii_case(&'T'))
        .map(|(i, _)| i);
    
    if let Some(pos) = t_pos {
        // We found a 'T' marker - split into spatial and temporal parts
        let spatial_part = &rest[..pos];
        let temporal_and_extra = &rest[pos+1..];
        
        // Parse spatial part - must be all digits if present
        if !spatial_part.is_empty() {
            if !spatial_part.chars().all(|c| c.is_ascii_digit()) {
                return Err(Error::SdpParsingError(format!(
                    "Invalid spatial layer in scalability mode: {}", spatial_part
                )));
            }
            spatial_layers = Some(spatial_part.parse::<u32>().unwrap());
        }
        
        // Temporal part must not be empty
        if temporal_and_extra.is_empty() {
            return Err(Error::SdpParsingError("Missing temporal value after T marker".to_string()));
        }
        
        // Find the numeric part of the temporal value
        let numeric_end = temporal_and_extra
            .char_indices()
            .find(|(_, c)| !c.is_ascii_digit())
            .map_or(temporal_and_extra.len(), |(i, _)| i);
        
        let numeric_part = &temporal_and_extra[..numeric_end];
        
        // Must have a valid temporal number
        if numeric_part.is_empty() {
            return Err(Error::SdpParsingError("Missing temporal layer value".to_string()));
        }
        
        temporal_layers = Some(numeric_part.parse::<u32>().unwrap());
        
        // Any remaining content is treated as extra information
        if numeric_end < temporal_and_extra.len() {
            extra = Some(temporal_and_extra[numeric_end..].to_string());
        }
    } else {
        // No 'T' marker found - should be just spatial layers or invalid
        if rest.chars().all(|c| c.is_ascii_digit()) {
            // Just a number, must be spatial layers
            spatial_layers = Some(rest.parse::<u32>().unwrap());
        } else {
            // Non-numeric content is not valid
            return Err(Error::SdpParsingError(format!(
                "Invalid characters in spatial layer: {}", rest
            )));
        }
    }
    
    Ok((pattern, spatial_layers, temporal_layers, extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_scalability_patterns() {
        // Test standard patterns mentioned in the documentation
        
        // L1T2: Spatial layer 1, temporal layer 2
        let result = parse_scalability_mode("L1T2").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(1));
        assert_eq!(result.2, Some(2));
        assert_eq!(result.3, None);
        
        // S2T3: Simulcast with 2 streams, 3 temporal layers
        let result = parse_scalability_mode("S2T3").unwrap();
        assert_eq!(result.0, "S");
        assert_eq!(result.1, Some(2));
        assert_eq!(result.2, Some(3));
        assert_eq!(result.3, None);
        
        // K1: Key frame dependence mode
        let result = parse_scalability_mode("K1").unwrap();
        assert_eq!(result.0, "K");
        assert_eq!(result.1, Some(1));
        assert_eq!(result.2, None);
        assert_eq!(result.3, None);
    }
    
    #[test]
    fn test_lowercase_patterns() {
        // Test lowercase pattern letters (should be handled case-insensitively)
        let result = parse_scalability_mode("l3t2").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(3));
        assert_eq!(result.2, Some(2));
        assert_eq!(result.3, None);
        
        let result = parse_scalability_mode("s2t1").unwrap();
        assert_eq!(result.0, "S");
        assert_eq!(result.1, Some(2));
        assert_eq!(result.2, Some(1));
        assert_eq!(result.3, None);
        
        let result = parse_scalability_mode("k2").unwrap();
        assert_eq!(result.0, "K");
        assert_eq!(result.1, Some(2));
        assert_eq!(result.2, None);
        assert_eq!(result.3, None);
    }
    
    #[test]
    fn test_with_extra_information() {
        // Test with extra information after the standard pattern
        let result = parse_scalability_mode("L2T3_KEY").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(2));
        assert_eq!(result.2, Some(3));
        assert_eq!(result.3, Some("_KEY".to_string()));
        
        // Test with complex extra information that doesn't contain additional 'T' characters
        let result = parse_scalability_mode("S3T2_PARAMS").unwrap();
        assert_eq!(result.0, "S");
        assert_eq!(result.1, Some(3));
        assert_eq!(result.2, Some(2));
        assert_eq!(result.3, Some("_PARAMS".to_string()));
    }
    
    #[test]
    fn test_boundary_cases() {
        // Test large numbers for spatial and temporal layers
        let result = parse_scalability_mode("L999T999").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(999));
        assert_eq!(result.2, Some(999));
        assert_eq!(result.3, None);
        
        // Test with zero (technically should be at least 1, but parsing allows it)
        let result = parse_scalability_mode("L0T0").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(0));
        assert_eq!(result.2, Some(0));
        assert_eq!(result.3, None);
        
        // Test with just the pattern letter
        let result = parse_scalability_mode("L").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, None);
        assert_eq!(result.2, None);
        assert_eq!(result.3, None);
        
        // Test with just T and temporal layer
        let result = parse_scalability_mode("LT3").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, None);  // No spatial layer specified
        assert_eq!(result.2, Some(3));
        assert_eq!(result.3, None);
    }
    
    #[test]
    fn test_invalid_inputs() {
        // Test empty string - should be rejected
        assert!(parse_scalability_mode("").is_err(), "Empty string should be rejected");
        
        // Test invalid pattern character - should be rejected
        assert!(parse_scalability_mode("X1T2").is_err(), "Invalid pattern character X should be rejected");
        assert!(parse_scalability_mode("Z3").is_err(), "Invalid pattern character Z should be rejected");
        
        // These should be considered invalid in a strict parser
        assert!(parse_scalability_mode("LaT2").is_err(), "Non-numeric character in spatial layer should be rejected");
        assert!(parse_scalability_mode("L2Ta").is_err(), "Non-numeric character in temporal layer should be rejected");
    }
    
    #[test]
    fn test_complex_scalability_modes() {
        // Test more complex patterns that might be used in real applications
        
        // L3T2h: Hierarchical temporal structure with 3 spatial and 2 temporal layers
        let result = parse_scalability_mode("L3T2h").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(3));
        assert_eq!(result.2, Some(2));
        assert_eq!(result.3, Some("h".to_string()));
        
        // S2T3_KEY: Simulcast with key frame dependency
        let result = parse_scalability_mode("S2T3_KEY").unwrap();
        assert_eq!(result.0, "S");
        assert_eq!(result.1, Some(2));
        assert_eq!(result.2, Some(3));
        assert_eq!(result.3, Some("_KEY".to_string()));
        
        // L2T3_KEY1SVC: Complex real-world pattern
        let result = parse_scalability_mode("L2T3_KEY1SVC").unwrap();
        assert_eq!(result.0, "L");
        assert_eq!(result.1, Some(2));
        assert_eq!(result.2, Some(3));
        assert_eq!(result.3, Some("_KEY1SVC".to_string()));
    }
    
    #[test]
    fn test_malformed_patterns() {
        // Test patterns that should be rejected as invalid
        
        // Missing temporal value but has T marker
        assert!(parse_scalability_mode("L2T").is_err(), "Missing temporal value should be rejected");
        
        // Multiple T characters
        assert!(parse_scalability_mode("L2T3T4").is_err(), "Multiple T characters should be rejected");
        
        // Non-numeric characters in spatial layer
        assert!(parse_scalability_mode("LxT3").is_err(), "Non-numeric spatial layer should be rejected");
    }
} 