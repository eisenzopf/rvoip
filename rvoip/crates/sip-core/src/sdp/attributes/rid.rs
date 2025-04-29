//! SDP RID Attribute Parser
//!
//! Implements parsers for Restriction Identifier (RID) attributes as defined in RFC 8851.
//! RID provides a framework for identifying and restricting media streams within
//! an RTP session.

use crate::error::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{alpha1, alphanumeric1, char, digit1, space0},
    combinator::{map, map_res, opt, recognize, value},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use std::collections::HashMap;

/// RID Direction
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RidDirection {
    /// Send direction
    Send,
    /// Receive direction
    Recv,
}

/// RID Attribute
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RidAttribute {
    /// RID identifier string
    pub id: String,
    /// Direction (send/recv)
    pub direction: RidDirection,
    /// Format list (optional)
    pub formats: Option<Vec<String>>,
    /// Key-value parameter restrictions
    pub restrictions: HashMap<String, String>,
}

/// Parse token (alphanumeric with limited special chars)
fn parse_token(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_alphanumeric() || "!#$%&'*+-.^_`{|}~".contains(c)
    })(input)
}

/// Parse RID identifier 
fn parse_rid_id(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| 
        (c.is_alphabetic() || c == '_') || // first char
        (c.is_alphanumeric() || c == '_' || c == '-') // rest chars
    )(input)
}

/// Parse RID direction
fn parse_rid_direction(input: &str) -> IResult<&str, RidDirection> {
    alt((
        value(RidDirection::Send, tag("send")),
        value(RidDirection::Recv, tag("recv"))
    ))(input)
}

/// Parse formats list in the form "pt=111,222,333"
fn parse_format_list(input: &str) -> IResult<&str, Vec<String>> {
    preceded(
        pair(tag("pt="), space0),
        separated_list1(
            pair(char(','), space0),
            map(digit1, |s: &str| s.to_string())
        )
    )(input)
}

/// Parse a key-value restriction
fn parse_restriction(input: &str) -> IResult<&str, (String, String)> {
    map(
        separated_pair(
            parse_token, 
            pair(char('='), space0), 
            parse_token
        ),
        |(k, v)| (k.to_string(), v.to_string())
    )(input)
}

/// Parse all restrictions
fn parse_restrictions_list(input: &str) -> IResult<&str, HashMap<String, String>> {
    map(
        separated_list0(
            pair(char(';'), space0),
            parse_restriction
        ),
        |restrictions| {
            restrictions.into_iter().collect()
        }
    )(input)
}

/// Main RID parser
fn rid_parser(input: &str) -> IResult<&str, RidAttribute> {
    let (input, id) = parse_rid_id(input)?;
    let (input, _) = space0(input)?;
    let (input, _) = char(' ')(input)?;
    let (input, direction) = parse_rid_direction(input)?;
    
    let (input, formats) = opt(preceded(
        tuple((space0, char(' '), space0)),
        parse_format_list
    ))(input)?;
    
    let (input, restrictions) = opt(preceded(
        tuple((space0, char(' '), space0)),
        parse_restrictions_list
    ))(input)?;
    
    Ok((
        input,
        RidAttribute {
            id: id.to_string(),
            direction,
            formats,
            restrictions: restrictions.unwrap_or_default(),
        }
    ))
}

/// Parse a RID attribute
///
/// Format: a=rid:<id> <direction> [pt=<fmt-list>] [;<key>=<value>]...
///
/// Example: a=rid:1 send pt=111,112;max-width=1280;max-height=720
pub fn parse_rid_struct(value: &str) -> Result<RidAttribute> {
    match rid_parser(value.trim()) {
        Ok((_, attr)) => Ok(attr),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid rid attribute: {}", value)))
    }
} 

// Old parse_rid implementation for compatibility with existing tests
// In a real codebase, would refactor tests to use the new return type
#[doc(hidden)]
pub fn parse_rid_compat(value: &str) -> Result<(String, String, Vec<String>)> {
    match parse_rid_struct(value) {
        Ok(attr) => {
            let mut restrictions = Vec::new();
            
            // Handle formats
            if let Some(formats) = attr.formats {
                restrictions.push(format!("pt={}", formats.join(",")));
            }
            
            // Convert HashMap restrictions to Vec<String>
            for (key, value) in attr.restrictions {
                restrictions.push(format!("{}={}", key, value));
            }
            
            Ok((
                attr.id,
                match attr.direction {
                    RidDirection::Send => "send".to_string(),
                    RidDirection::Recv => "recv".to_string(),
                },
                restrictions
            ))
        },
        Err(e) => Err(e)
    }
}

// For backward compatibility with existing code
// This will be removed when all code is updated to use the new struct
#[deprecated(since = "0.1.0", note = "Use parse_rid() returning RidAttribute instead")]
pub fn parse_rid(value: &str) -> Result<(String, String, Vec<String>)> {
    parse_rid_compat(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sdp::ParsedAttribute;
    
    #[test]
    fn test_rid_parsing() {
        // Test basic rid parsing
        let rid_value = "1 send pt=97,98 max-width=1280;max-height=720";
        let result = parse_rid(rid_value);
        assert!(result.is_ok());
        let (id, direction, restrictions) = result.unwrap();
        assert_eq!(id, "1");
        assert_eq!(direction, "send");
        assert_eq!(restrictions.len(), 3);
        assert_eq!(restrictions[0], "pt=97,98");
        assert_eq!(restrictions[1], "max-width=1280");
        assert_eq!(restrictions[2], "max-height=720");
        
        // Test invalid rid - missing direction
        let invalid_rid = "1";
        assert!(parse_rid(invalid_rid).is_err());
    }
    
    #[test]
    fn test_complex_rid() {
        // RID with complex restrictions
        let complex_rid = "1 send pt=96,97,98 max-width=1280;max-height=720;max-fps=30;max-fs=8160";
        let result = parse_rid(complex_rid);
        assert!(result.is_ok());
        let (id, direction, restrictions) = result.unwrap();
        assert_eq!(id, "1");
        assert_eq!(direction, "send");
        // According to RFC 8851, these should be 5 separate restrictions:
        // 1. pt=96,97,98 (payload types)
        // 2. max-width=1280
        // 3. max-height=720
        // 4. max-fps=30
        // 5. max-fs=8160
        assert_eq!(restrictions.len(), 5);
        assert_eq!(restrictions[0], "pt=96,97,98");
        assert_eq!(restrictions[1], "max-width=1280");
        assert_eq!(restrictions[2], "max-height=720");
        assert_eq!(restrictions[3], "max-fps=30");
        assert_eq!(restrictions[4], "max-fs=8160");
    }
    
    #[test]
    fn test_rid_edge_cases() {
        // Test with different id formats
        assert!(parse_rid("a1 send").is_ok());
        assert!(parse_rid("_123 send").is_ok());
        assert!(parse_rid("a-b-c send").is_ok());
        
        // Invalid id formats
        assert!(parse_rid("1a$ send").is_err()); // Invalid character
        assert!(parse_rid("-123 send").is_err()); // Starts with non-alpha/underscore
        
        // Test direction variants
        let (_, dir, _) = parse_rid("id send").unwrap();
        assert_eq!(dir, "send");
        
        let (_, dir, _) = parse_rid("id recv").unwrap();
        assert_eq!(dir, "recv");
        
        // Invalid direction
        assert!(parse_rid("id both").is_err());
    }
    
    #[test]
    fn test_rid_format_list() {
        // Test format list parsing
        let (_, _, restrictions) = parse_rid("1 send pt=96,97,98").unwrap();
        assert_eq!(restrictions.len(), 1);
        assert_eq!(restrictions[0], "pt=96,97,98");
        
        // Test with single format
        let (_, _, restrictions) = parse_rid("1 send pt=96").unwrap();
        assert_eq!(restrictions.len(), 1);
        assert_eq!(restrictions[0], "pt=96");
    }
    
    #[test]
    fn test_rid_with_restrictions() {
        // Test with just restrictions, no format list
        let result = parse_rid("1 send max-width=1280;max-height=720");
        assert!(result.is_ok());
        let (_, _, restrictions) = result.unwrap();
        assert_eq!(restrictions.len(), 2);
        assert_eq!(restrictions[0], "max-width=1280");
        assert_eq!(restrictions[1], "max-height=720");
    }
    
    #[test]
    fn test_parse_rid_struct() {
        // Test the new struct-returning function
        let result = parse_rid_struct("1 send pt=96,97,98 max-width=1280;max-height=720");
        assert!(result.is_ok());
        let attr = result.unwrap();
        
        assert_eq!(attr.id, "1");
        assert_eq!(attr.direction, RidDirection::Send);
        
        // Check formats
        assert!(attr.formats.is_some());
        let formats = attr.formats.unwrap();
        assert_eq!(formats, vec!["96", "97", "98"]);
        
        // Check restrictions
        assert_eq!(attr.restrictions.len(), 2);
        assert_eq!(attr.restrictions.get("max-width"), Some(&"1280".to_string()));
        assert_eq!(attr.restrictions.get("max-height"), Some(&"720".to_string()));
    }
} 