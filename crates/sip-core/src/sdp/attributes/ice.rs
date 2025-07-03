//! SDP ICE Attribute Parsers
//!
//! Implements parsers for ICE-related attributes as defined in RFC 8839.
//! These attributes are used in the ICE (Interactive Connectivity Establishment)
//! protocol for NAT traversal.

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::take_while1,
    character::complete::{space0, space1},
    combinator::{map, verify},
    multi::separated_list1,
    sequence::preceded,
    IResult,
};

/// Parser for ICE username fragment (ufrag)
/// The ufrag must be between 4 and 256 characters
fn ice_ufrag_parser(input: &str) -> IResult<&str, &str> {
    verify(
        take_while1(|c: char| c.is_ascii() && !c.is_ascii_control() && !c.is_whitespace()),
        |s: &str| s.len() >= 4 && s.len() <= 256
    )(input)
}

/// Parser for ICE password
/// The password must be between 22 and 256 characters
fn ice_pwd_parser(input: &str) -> IResult<&str, &str> {
    verify(
        take_while1(|c: char| c.is_ascii() && !c.is_ascii_control() && !c.is_whitespace()),
        |s: &str| s.len() >= 22 && s.len() <= 256
    )(input)
}

/// Parser for ICE options (a list of tokens)
fn ice_options_parser(input: &str) -> IResult<&str, Vec<String>> {
    if input.is_empty() {
        return Ok((input, Vec::new()));
    }
    
    separated_list1(
        space1,
        map(token, |s: &str| s.to_string())
    )(input)
}

/// Parses ice-ufrag attribute: a=ice-ufrag:<ufrag>
pub fn parse_ice_ufrag(value: &str) -> Result<String> {
    let trimmed = value.trim();
    
    // Check for control characters explicitly
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(Error::SdpParsingError(format!("Invalid ice-ufrag value contains control characters: {}", value)));
    }
    
    // Check for non-ASCII characters
    if !trimmed.is_ascii() {
        return Err(Error::SdpParsingError(format!("Invalid ice-ufrag value contains non-ASCII characters: {}", value)));
    }
    
    to_result(
        ice_ufrag_parser(trimmed),
        &format!("Invalid ice-ufrag value: {}", value)
    ).map(|s| s.to_string())
}

/// Parses ice-pwd attribute: a=ice-pwd:<pwd>
pub fn parse_ice_pwd(value: &str) -> Result<String> {
    let trimmed = value.trim();
    
    // Check for control characters explicitly
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(Error::SdpParsingError(format!("Invalid ice-pwd value contains control characters: {}", value)));
    }
    
    // Check for non-ASCII characters
    if !trimmed.is_ascii() {
        return Err(Error::SdpParsingError(format!("Invalid ice-pwd value contains non-ASCII characters: {}", value)));
    }
    
    to_result(
        ice_pwd_parser(trimmed),
        &format!("Invalid ice-pwd value: {}", value)
    ).map(|s| s.to_string())
}

/// Parses ice-options attribute: a=ice-options:<option-tag> ...
pub fn parse_ice_options(value: &str) -> Result<Vec<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    
    // Split on whitespace for simple option parsing
    Ok(trimmed.split_whitespace()
       .map(|s| s.to_string())
       .collect())
}

/// Parses end-of-candidates attribute: a=end-of-candidates
/// This is a flag attribute with no value
pub fn parse_end_of_candidates(_value: &str) -> Result<bool> {
    // No parsing needed for flag attributes
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_ice_ufrag() {
        // Test standard ufrag
        let ufrag = "f46daf7e";
        let result = parse_ice_ufrag(ufrag).unwrap();
        assert_eq!(result, ufrag);
        
        // Test ufrag with special characters
        let ufrag_special = "abc+/=XYZ";
        let result = parse_ice_ufrag(ufrag_special).unwrap();
        assert_eq!(result, ufrag_special);
        
        // Test minimum length (4 chars)
        let min_ufrag = "abcd";
        let result = parse_ice_ufrag(min_ufrag).unwrap();
        assert_eq!(result, min_ufrag);
        
        // Test with whitespace (should be trimmed)
        let ufrag_with_space = "  f46daf7e  ";
        let result = parse_ice_ufrag(ufrag_with_space).unwrap();
        assert_eq!(result, "f46daf7e");
    }
    
    #[test]
    fn test_parse_invalid_ice_ufrag() {
        // Test too short ufrag (< 4 chars)
        let too_short = "abc";
        assert!(parse_ice_ufrag(too_short).is_err());
        
        // Test empty ufrag
        let empty = "";
        assert!(parse_ice_ufrag(empty).is_err());
        
        // Test with control characters - need to use an actual ASCII control character
        // that will be rejected by the parser
        let with_control = "abcd\x01ef";
        assert!(parse_ice_ufrag(with_control).is_err());
        
        // Test with non-ASCII characters
        let with_non_ascii = "abcdé";
        assert!(parse_ice_ufrag(with_non_ascii).is_err());
    }
    
    #[test]
    fn test_parse_ice_ufrag_length_limits() {
        // Test exactly 256 characters (maximum allowed)
        let max_length_ufrag = "a".repeat(256);
        let result = parse_ice_ufrag(&max_length_ufrag).unwrap();
        assert_eq!(result, max_length_ufrag);
        
        // Test too long (> 256 chars)
        let too_long = "a".repeat(257);
        assert!(parse_ice_ufrag(&too_long).is_err());
    }
    
    #[test]
    fn test_parse_valid_ice_pwd() {
        // Test standard password
        let pwd = "asd88fgpdd777uzjYhagZfMk1k3o";
        let result = parse_ice_pwd(pwd).unwrap();
        assert_eq!(result, pwd);
        
        // Test password with special characters
        let pwd_special = "abcdefghijklmnopqrstuv+/=";
        let result = parse_ice_pwd(pwd_special).unwrap();
        assert_eq!(result, pwd_special);
        
        // Test minimum length (22 chars)
        let min_pwd = "a".repeat(22);
        let result = parse_ice_pwd(&min_pwd).unwrap();
        assert_eq!(result, min_pwd);
        
        // Test with whitespace (should be trimmed)
        let pwd_with_space = "  asd88fgpdd777uzjYhagZfMk1k3o  ";
        let result = parse_ice_pwd(pwd_with_space).unwrap();
        assert_eq!(result, "asd88fgpdd777uzjYhagZfMk1k3o");
    }
    
    #[test]
    fn test_parse_invalid_ice_pwd() {
        // Test too short password (< 22 chars)
        let too_short = "a".repeat(21);
        assert!(parse_ice_pwd(&too_short).is_err());
        
        // Test empty password
        let empty = "";
        assert!(parse_ice_pwd(empty).is_err());
        
        // Test with control characters - need to use an actual ASCII control character
        let with_control = "a".repeat(21) + "\x01";
        assert!(parse_ice_pwd(&with_control).is_err());
        
        // Test with non-ASCII characters
        let with_non_ascii = "a".repeat(21) + "é";
        assert!(parse_ice_pwd(&with_non_ascii).is_err());
    }
    
    #[test]
    fn test_parse_ice_pwd_length_limits() {
        // Test exactly 256 characters (maximum allowed)
        let max_length_pwd = "a".repeat(256);
        let result = parse_ice_pwd(&max_length_pwd).unwrap();
        assert_eq!(result, max_length_pwd);
        
        // Test too long (> 256 chars)
        let too_long = "a".repeat(257);
        assert!(parse_ice_pwd(&too_long).is_err());
    }
    
    #[test]
    fn test_parse_valid_ice_options() {
        // Test single option
        let options = "trickle";
        let result = parse_ice_options(options).unwrap();
        assert_eq!(result, vec!["trickle"]);
        
        // Test multiple options
        let multiple_options = "trickle ice2";
        let result = parse_ice_options(multiple_options).unwrap();
        assert_eq!(result, vec!["trickle", "ice2"]);
        
        // Test with extra whitespace
        let options_with_space = "  trickle   ice2  renomination ";
        let result = parse_ice_options(options_with_space).unwrap();
        assert_eq!(result, vec!["trickle", "ice2", "renomination"]);
        
        // Test empty options list (should return empty vec)
        let empty = "";
        let result = parse_ice_options(empty).unwrap();
        assert_eq!(result, Vec::<String>::new());
    }
    
    #[test]
    fn test_parse_ice_options_with_special_tokens() {
        // Test options with hyphens and dots
        let options = "trickle ice2 ice-lite rtp.mux";
        let result = parse_ice_options(options).unwrap();
        assert_eq!(result, vec!["trickle", "ice2", "ice-lite", "rtp.mux"]);
        
        // Test complex options with various allowed characters
        let complex = "option-1 option.2 option_3 option+4";
        let result = parse_ice_options(complex).unwrap();
        assert_eq!(result, vec!["option-1", "option.2", "option_3", "option+4"]);
    }
    
    #[test]
    fn test_end_of_candidates() {
        // Test with empty value
        let result = parse_end_of_candidates("").unwrap();
        assert!(result);
        
        // Test with some value (flag attributes should ignore the value)
        let result = parse_end_of_candidates("anything").unwrap();
        assert!(result);
    }
    
    #[test]
    fn test_parser_functions_directly() {
        // Test ice_ufrag_parser directly
        let input = "abcd";
        let (remainder, ufrag) = ice_ufrag_parser(input).unwrap();
        assert_eq!(ufrag, "abcd");
        assert_eq!(remainder, "");
        
        // Test with trailing content
        let input_with_suffix = "abcd rest";
        let result = ice_ufrag_parser(input_with_suffix);
        assert!(result.is_ok());
        let (remainder, ufrag) = result.unwrap();
        assert_eq!(ufrag, "abcd");
        assert_eq!(remainder, " rest");
        
        // Test ice_pwd_parser directly
        let pwd = "a".repeat(22);
        let (remainder, parsed_pwd) = ice_pwd_parser(&pwd).unwrap();
        assert_eq!(parsed_pwd, pwd);
        assert_eq!(remainder, "");
        
        // Test ice_pwd_parser with trailing content
        let pwd_input = format!("{} rest", pwd);
        let (remainder, parsed_pwd) = ice_pwd_parser(&pwd_input).unwrap();
        assert_eq!(parsed_pwd, pwd);
        assert_eq!(remainder, " rest");
        
        // Test ice_options_parser directly with non-empty input
        let options_input = "trickle ice2";
        let (remainder, options) = ice_options_parser(options_input).unwrap();
        assert_eq!(options, vec!["trickle", "ice2"]);
        assert_eq!(remainder, "");
        
        // Test ice_options_parser with empty input
        let empty_input = "";
        let (remainder, options) = ice_options_parser(empty_input).unwrap();
        assert_eq!(options, Vec::<String>::new());
        assert_eq!(remainder, "");
    }
    
    #[test]
    fn test_real_world_examples() {
        // Examples from actual SDP messages
        
        // Example 1: Typical Chrome WebRTC SDP
        let ufrag = "4ZcD";
        let pwd = "dOTZkCxUbWFJfwkEwUMm75Zz";
        let options = "trickle";
        
        assert!(parse_ice_ufrag(ufrag).is_ok());
        assert!(parse_ice_pwd(pwd).is_ok());
        assert_eq!(parse_ice_options(options).unwrap(), vec!["trickle"]);
        
        // Example 2: Firefox WebRTC SDP
        let ufrag2 = "b31c1596";
        let pwd2 = "7f42b2911c3efc8f187341748f54d75a";
        let options2 = "trickle ice2";
        
        assert!(parse_ice_ufrag(ufrag2).is_ok());
        assert!(parse_ice_pwd(pwd2).is_ok());
        assert_eq!(parse_ice_options(options2).unwrap(), vec!["trickle", "ice2"]);
    }
} 