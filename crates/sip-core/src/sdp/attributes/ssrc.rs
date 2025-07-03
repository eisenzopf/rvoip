//! SDP SSRC Attribute Parser
//!
//! Implements parser for SSRC attributes as defined in RFC 5576.
//! Format: a=ssrc:<ssrc-id> <attribute>[:<value>]

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use crate::types::sdp::{SsrcAttribute, ParsedAttribute};
use nom::{
    bytes::complete::{tag, take_till1},
    character::complete::{char, space1},
    combinator::{map, opt},
    sequence::{pair, preceded, separated_pair, tuple},
    IResult,
};

/// Parser for SSRC ID (32-bit unsigned integer)
fn ssrc_id_parser(input: &str) -> IResult<&str, u32> {
    positive_integer(input)
}

/// Parser for SSRC attribute name
fn attribute_name_parser(input: &str) -> IResult<&str, &str> {
    token(input)
}

/// Parser for SSRC attribute value (everything after colon)
fn attribute_value_parser(input: &str) -> IResult<&str, &str> {
    take_till1(|c: char| c.is_whitespace())(input)  // Take until whitespace
}

/// Parser for attribute-name:value pair
fn attribute_pair_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    let (input, attr_name) = attribute_name_parser(input)?;
    
    let (input, attr_value) = opt(preceded(
        char(':'),
        map(
            attribute_value_parser,
            |s: &str| s.to_string()
        )
    ))(input)?;
    
    Ok((input, (attr_name.to_string(), attr_value)))
}

/// Main parser for SSRC attribute
fn ssrc_parser(input: &str) -> IResult<&str, SsrcAttribute> {
    let (input, ssrc_id) = ssrc_id_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, (attribute, value)) = attribute_pair_parser(input)?;
    
    Ok((
        input,
        SsrcAttribute {
            ssrc_id,
            attribute,
            value,
        }
    ))
}

/// Parses SSRC attribute: a=ssrc:<ssrc-id> <attribute>[:<value>]
pub fn parse_ssrc(value: &str) -> Result<ParsedAttribute> {
    match ssrc_parser(value.trim()) {
        Ok((_, ssrc)) => {
            // Basic validation: attribute name shouldn't be empty
            if ssrc.attribute.is_empty() {
                return Err(Error::SdpParsingError(format!("Missing attribute name in ssrc: {}", value)));
            }
            
            Ok(ParsedAttribute::Ssrc(ssrc))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid ssrc format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sdp::ParsedAttribute;
    
    #[test]
    fn test_ssrc_basic_parsing() {
        // Basic SSRC attribute with cname
        let ssrc_value = "12345 cname:user@example.com";
        let result = parse_ssrc(ssrc_value);
        assert!(result.is_ok(), "Failed to parse valid SSRC attribute");
        
        if let Ok(ParsedAttribute::Ssrc(attr)) = result {
            assert_eq!(attr.ssrc_id, 12345, "Incorrect SSRC ID");
            assert_eq!(attr.attribute, "cname", "Incorrect attribute name");
            assert_eq!(attr.value, Some("user@example.com".to_string()), "Incorrect attribute value");
        } else {
            panic!("Expected ParsedAttribute::Ssrc");
        }
        
        // SSRC attribute without a value
        let ssrc_value = "54321 mslabel";
        let result = parse_ssrc(ssrc_value);
        assert!(result.is_ok(), "Failed to parse SSRC attribute without value");
        
        if let Ok(ParsedAttribute::Ssrc(attr)) = result {
            assert_eq!(attr.ssrc_id, 54321, "Incorrect SSRC ID");
            assert_eq!(attr.attribute, "mslabel", "Incorrect attribute name");
            assert_eq!(attr.value, None, "Expected no attribute value");
        } else {
            panic!("Expected ParsedAttribute::Ssrc");
        }
    }
    
    #[test]
    fn test_ssrc_rfc_examples() {
        // Examples from RFC 5576
        // a=ssrc:11111 cname:user@example.com
        let ssrc_value = "11111 cname:user@example.com";
        let result = parse_ssrc(ssrc_value);
        assert!(result.is_ok(), "Failed to parse RFC example 1");
        
        // a=ssrc:22222 cname:another@example.com
        let ssrc_value = "22222 cname:another@example.com";
        let result = parse_ssrc(ssrc_value);
        assert!(result.is_ok(), "Failed to parse RFC example 2");
        
        // a=ssrc:33333 cname:another@example.com
        let ssrc_value = "33333 cname:another@example.com";
        let result = parse_ssrc(ssrc_value);
        assert!(result.is_ok(), "Failed to parse RFC example 3");
    }
    
    #[test]
    fn test_ssrc_invalid_formats() {
        // Empty string
        let empty = "";
        assert!(parse_ssrc(empty).is_err(), "Should reject empty string");
        
        // Missing SSRC ID
        let missing_id = "cname:user@example.com";
        assert!(parse_ssrc(missing_id).is_err(), "Should reject missing SSRC ID");
        
        // Missing attribute name
        let missing_attr = "12345 :value";
        assert!(parse_ssrc(missing_attr).is_err(), "Should reject missing attribute name");
        
        // Invalid SSRC ID (not a number)
        let invalid_id = "abc cname:user@example.com";
        assert!(parse_ssrc(invalid_id).is_err(), "Should reject non-numeric SSRC ID");
        
        // Empty attribute name
        let empty_attr = "12345 ";
        assert!(parse_ssrc(empty_attr).is_err(), "Should reject empty attribute name");
    }
    
    #[test]
    fn test_ssrc_whitespace_handling() {
        // Leading/trailing whitespace
        let with_whitespace = "  12345 cname:user@example.com  ";
        let result = parse_ssrc(with_whitespace);
        assert!(result.is_ok(), "Should handle leading/trailing whitespace");
        
        if let Ok(ParsedAttribute::Ssrc(attr)) = result {
            assert_eq!(attr.ssrc_id, 12345);
            assert_eq!(attr.attribute, "cname");
            assert_eq!(attr.value, Some("user@example.com".to_string()));
        }
        
        // Multiple spaces between parts
        let multiple_spaces = "12345    cname:user@example.com";
        let result = parse_ssrc(multiple_spaces);
        assert!(result.is_ok(), "Should handle multiple spaces between parts");
    }
    
    #[test]
    fn test_ssrc_edge_cases() {
        // Large SSRC ID (max u32)
        let large_id = "4294967295 cname:user@example.com";
        let result = parse_ssrc(large_id);
        assert!(result.is_ok(), "Should handle maximum u32 SSRC ID");
        
        if let Ok(ParsedAttribute::Ssrc(attr)) = result {
            assert_eq!(attr.ssrc_id, 4294967295);
        }
        
        // Various valid attribute names
        let valid_names = [
            "12345 msid:stream-id",
            "12345 label:track-label",
            "12345 mslabel:media-stream",
            "12345 extmap:1"
        ];
        
        for name in valid_names.iter() {
            assert!(parse_ssrc(name).is_ok(), "Should handle valid attribute name: {}", name);
        }
        
        // Special characters in value
        let special_chars = "12345 cname:user+name@example.co.uk";
        let result = parse_ssrc(special_chars);
        assert!(result.is_ok(), "Should handle special characters in value");
        
        if let Ok(ParsedAttribute::Ssrc(attr)) = result {
            assert_eq!(attr.value, Some("user+name@example.co.uk".to_string()));
        }
    }
    
    #[test]
    fn test_parser_functions_directly() {
        // Test ssrc_id_parser
        let (rest, id) = ssrc_id_parser("12345 rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(id, 12345);
        
        // Test attribute_name_parser
        let (rest, name) = attribute_name_parser("cname rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(name, "cname");
        
        // Test attribute_value_parser
        let (rest, value) = attribute_value_parser("user@example.com").unwrap();
        assert_eq!(rest, "");
        assert_eq!(value, "user@example.com");
        
        // Test attribute_pair_parser
        let (rest, (name, value)) = attribute_pair_parser("cname:user@example.com rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(name, "cname");
        assert_eq!(value, Some("user@example.com".to_string()));
        
        // Test ssrc_parser
        let (rest, ssrc) = ssrc_parser("12345 cname:user@example.com").unwrap();
        assert_eq!(rest, "");
        assert_eq!(ssrc.ssrc_id, 12345);
        assert_eq!(ssrc.attribute, "cname");
        assert_eq!(ssrc.value, Some("user@example.com".to_string()));
    }
} 