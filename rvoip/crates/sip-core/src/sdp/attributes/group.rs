//! SDP Group Attribute Parser
//!
//! Implements parser for group attributes as defined in RFC 5888.
//! Format: a=group:<semantics> <identification-tag> ...

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::take_while1,
    character::complete::space1,
    combinator::{map, verify},
    multi::separated_list0,
    sequence::{pair, preceded},
    IResult,
};

/// Parser for semantics values (like BUNDLE, LS, etc.)
fn semantics_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_')(input)
}

/// Parser for identification tags list (mids)
fn identification_tags_parser(input: &str) -> IResult<&str, Vec<String>> {
    preceded(
        space1,
        separated_list0(
            space1,
            map(
                verify(token, |s: &str| !s.is_empty()),
                |s: &str| s.to_string()
            )
        )
    )(input)
}

/// Main parser for group attribute
fn group_parser(input: &str) -> IResult<&str, (String, Vec<String>)> {
    pair(
        map(semantics_parser, |s: &str| s.to_string()),
        identification_tags_parser
    )(input)
}

/// Parser for the SDP Group Attribute
/// Follows RFC 5888
pub fn parse_group(input: &str) -> Result<(String, Vec<String>)> {
    let trimmed_input = input.trim();
    
    if trimmed_input.is_empty() {
        return Err(Error::SdpParseError("Invalid group format".to_string()));
    }
    
    let parts: Vec<&str> = trimmed_input.splitn(2, ' ').collect();
    
    // Group needs at least a semantics part
    if parts.is_empty() {
        return Err(Error::SdpParseError("Invalid group format".to_string()));
    }
    
    // Check if the semantics part looks like a valid semantics value
    // Must contain only alphanumeric characters, hyphens, or underscores
    if !parts[0].chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(Error::SdpParseError("Invalid semantics value".to_string()));
    }
    
    // According to RFC 5888, semantics values should typically be uppercase tokens
    // like BUNDLE, LS, FID, etc. If the token contains lowercase letters or numbers 
    // and doesn't match known patterns, it might be a misplaced identification tag
    if parts[0].chars().any(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) &&
       !["bundle", "ls", "fid", "srf", "anat"].contains(&parts[0].to_lowercase().as_str()) &&
       !parts[0].starts_with("X-") && !parts[0].starts_with("x-") {
        // Heuristic: If it looks like it might be an identification tag
        // and not a valid semantics value, reject it
        return Err(Error::SdpParseError("Invalid semantics value, possibly misplaced identification tag".to_string()));
    }
    
    let semantics = parts[0].to_string();
    
    // According to RFC 5888, at least one identification tag should be present
    if parts.len() < 2 || parts[1].trim().is_empty() {
        return Err(Error::SdpParseError(
            "Group attribute must have at least one identification tag".to_string(),
        ));
    }
    
    let identification_tags = parts[1]
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    
    Ok((semantics, identification_tags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bundle_group() {
        // BUNDLE semantics (RFC 8843)
        let bundle = "BUNDLE audio video data";
        let result = parse_group(bundle).unwrap();
        
        assert_eq!(result.0, "BUNDLE");
        assert_eq!(result.1, vec!["audio", "video", "data"]);
    }
    
    #[test]
    fn test_parse_lip_sync_group() {
        // LS (Lip Synchronization) semantics from RFC 5888
        let ls = "LS 1 2";
        let result = parse_group(ls).unwrap();
        
        assert_eq!(result.0, "LS");
        assert_eq!(result.1, vec!["1", "2"]);
    }
    
    #[test]
    fn test_parse_flow_identification_group() {
        // FID (Flow Identification) semantics from RFC 5888
        let fid = "FID 1 2";
        let result = parse_group(fid).unwrap();
        
        assert_eq!(result.0, "FID");
        assert_eq!(result.1, vec!["1", "2"]);
    }
    
    #[test]
    fn test_parse_single_reservation_flow_group() {
        // SRF (Single Reservation Flow) semantics
        let srf = "SRF 1 2 3";
        let result = parse_group(srf).unwrap();
        
        assert_eq!(result.0, "SRF");
        assert_eq!(result.1, vec!["1", "2", "3"]);
    }
    
    #[test]
    fn test_parse_alternative_network_address_types_group() {
        // ANAT (Alternative Network Address Types) semantics from RFC 4091
        let anat = "ANAT 1 2";
        let result = parse_group(anat).unwrap();
        
        assert_eq!(result.0, "ANAT");
        assert_eq!(result.1, vec!["1", "2"]);
    }
    
    #[test]
    fn test_case_insensitivity_semantics() {
        // Test case-insensitivity for semantics
        let bundle_lowercase = "bundle audio video";
        let result_lower = parse_group(bundle_lowercase).unwrap();
        
        let bundle_mixed = "BuNdLe audio video";
        let result_mixed = parse_group(bundle_mixed).unwrap();
        
        assert_eq!(result_lower.0, "bundle");
        assert_eq!(result_mixed.0, "BuNdLe");
        
        // Semantics are preserved in their original case
        assert_ne!(result_lower.0, "BUNDLE");
    }
    
    #[test]
    fn test_multiple_identification_tags() {
        // Test with a large number of identification tags
        let many_tags = "BUNDLE 1 2 3 4 5 6 7 8 9 10";
        let result = parse_group(many_tags).unwrap();
        
        assert_eq!(result.0, "BUNDLE");
        assert_eq!(result.1.len(), 10);
        assert_eq!(result.1, vec!["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
    }
    
    #[test]
    fn test_complex_identification_tags() {
        // Test with more complex identification tags (can contain alphanumeric chars and some symbols)
        let complex_tags = "BUNDLE audio-0 video_main data.channel";
        let result = parse_group(complex_tags).unwrap();
        
        assert_eq!(result.0, "BUNDLE");
        assert_eq!(result.1, vec!["audio-0", "video_main", "data.channel"]);
    }
    
    #[test]
    fn test_empty_identification_tags_list() {
        // Test with no identification tags
        // According to RFC 5888, at least one identification tag should be present
        let no_tags = "BUNDLE";
        assert!(parse_group(no_tags).is_err(), "Parser should reject group with no identification tags");
        
        // Verify that a single tag works
        let single_tag = "BUNDLE audio";
        let result = parse_group(single_tag).unwrap();
        assert_eq!(result.0, "BUNDLE");
        assert_eq!(result.1, vec!["audio"]);
    }
    
    #[test]
    fn test_unknown_semantics() {
        // Test with unknown semantics (not an error per spec)
        let custom = "CUSTOM tag1 tag2";
        let result = parse_group(custom).unwrap();
        
        assert_eq!(result.0, "CUSTOM");
        assert_eq!(result.1, vec!["tag1", "tag2"]);
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with extra whitespace
        let extra_space = "  BUNDLE   audio   video   data  ";
        let result = parse_group(extra_space).unwrap();
        
        assert_eq!(result.0, "BUNDLE");
        assert_eq!(result.1, vec!["audio", "video", "data"]);
    }
    
    #[test]
    fn test_hyphenated_semantics() {
        // Test with hyphenated semantics
        let hyphenated = "MY-SEMANTICS tag1 tag2";
        let result = parse_group(hyphenated).unwrap();
        
        assert_eq!(result.0, "MY-SEMANTICS");
        assert_eq!(result.1, vec!["tag1", "tag2"]);
    }
    
    #[test]
    fn test_invalid_formats() {
        // Empty input
        assert!(parse_group("").is_err());
        
        // Only whitespace
        assert!(parse_group("   ").is_err());
        
        // Leading whitespace is trimmed, so this is valid if there's a proper format after
        let with_whitespace = " BUNDLE audio";
        let result = parse_group(with_whitespace).unwrap();
        assert_eq!(result.0, "BUNDLE");
        assert_eq!(result.1, vec!["audio"]);
        
        // Input that looks like tags without a proper semantics - should be rejected
        let invalid_format = "tag1 tag2";
        assert!(parse_group(invalid_format).is_err(), "Parser should reject input without proper semantics");
    }
    
    #[test]
    fn test_direct_parser_functions() {
        // Test semantics_parser directly
        let (remainder, semantics) = semantics_parser("BUNDLE rest").unwrap();
        assert_eq!(semantics, "BUNDLE");
        assert_eq!(remainder, " rest");
        
        // Test identification_tags_parser directly
        let (remainder, tags) = identification_tags_parser(" tag1 tag2 tag3").unwrap();
        assert_eq!(tags, vec!["tag1", "tag2", "tag3"]);
        assert_eq!(remainder, "");
        
        // Test group_parser directly
        let (remainder, (semantics, tags)) = group_parser("BUNDLE tag1 tag2").unwrap();
        assert_eq!(semantics, "BUNDLE");
        assert_eq!(tags, vec!["tag1", "tag2"]);
        assert_eq!(remainder, "");
    }
} 