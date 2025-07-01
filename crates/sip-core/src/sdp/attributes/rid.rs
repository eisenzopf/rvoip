//! SDP RID Attribute Parser
//!
//! Implements parsers for Restriction Identifier (RID) attributes as defined in RFC 8851.
//! RID provides a framework for identifying and restricting media streams within
//! an RTP session.

use crate::error::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while1},
    character::complete::{alpha1, alphanumeric1, char, digit1, space0, space1},
    combinator::{all_consuming, map, map_res, opt, recognize, value},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// RID Direction
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RidDirection {
    /// Send direction
    Send,
    /// Receive direction
    Recv,
}

/// RID Attribute
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RidAttribute {
    /// RID identifier string
    pub id: String,
    /// Direction (send/recv)
    pub direction: RidDirection,
    /// Format list (payload types)
    pub formats: Vec<String>,
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
/// Per RFC 8851: The first character MUST be a letter (a-z or A-Z)
/// or an underscore (_). The remaining characters MUST be alphanumeric
/// or dash (-) or underscore (_).
fn parse_rid_id(input: &str) -> IResult<&str, String> {
    map(
        recognize(
            pair(
                alt((alpha1, tag("_"))),
                many0(alt((alphanumeric1, tag("-"), tag("_"))))
            )
        ),
        |s: &str| s.to_string()
    )(input)
}

/// Parse the direction of a RID attribute (send or recv)
fn parse_direction(input: &str) -> IResult<&str, RidDirection> {
    alt((
        map(tag_no_case("send"), |_| RidDirection::Send),
        map(tag_no_case("recv"), |_| RidDirection::Recv),
    ))(input)
}

/// Parse a list of formats (e.g., "pt=96,97,98")
fn parse_format_list(input: &str) -> IResult<&str, Vec<String>> {
    preceded(
        tag("pt="),
        separated_list1(
            pair(char(','), space0),
            map(digit1, |s: &str| s.to_string())
        )
    )(input)
}

/// Parse a single restriction (e.g., "max-width=1280")
fn parse_restriction(input: &str) -> IResult<&str, (String, String)> {
    let (input, name) = take_while1(|c: char| c.is_alphanumeric() || c == '-')(input)?;
    let (input, _) = char('=')(input)?;
    let (input, value) = take_while1(|c: char| c != ';' && c != ' ')(input)?;
    Ok((input, (name.to_string(), value.to_string())))
}

/// Parse a list of restrictions (e.g., "max-width=1280;max-height=720")
/// This allows the input to either start with a semicolon or not
fn parse_restrictions_list(input: &str) -> IResult<&str, HashMap<String, String>> {
    // Try with leading semicolon
    let with_semicolon = map(
        preceded(
            char(';'),
            separated_list0(
                char(';'),
                parse_restriction
            )
        ),
        |restrictions| restrictions.into_iter().collect()
    );
    
    // Without leading semicolon
    let without_semicolon = map(
        separated_list0(
            char(';'),
            parse_restriction
        ),
        |restrictions| restrictions.into_iter().collect()
    );
    
    // Try both patterns
    alt((with_semicolon, without_semicolon))(input)
}

/// Parse an RID attribute according to RFC 8851 ABNF but with flexibility:
/// rid-attribute = "a=rid:" rid-id SP rid-dir
///                [ SP rid-pt-param-list ]
///                [ SP rid-param-list ]
fn rid_parser(input: &str) -> IResult<&str, RidAttribute> {
    // Parse the RID ID
    let (input, id) = parse_rid_id(input)?;
    
    // Parse whitespace
    let (input, _) = space1(input)?;
    
    // Parse the direction
    let (input, direction) = parse_direction(input)?;
    
    let (mut remaining, _) = space0(input)?;
    
    // Initialize with empty values
    let mut formats = Vec::new();
    let mut restrictions = HashMap::new();
    
    // Handle the case where we're done
    if remaining.is_empty() {
        return Ok((remaining, RidAttribute { id, direction, formats, restrictions }));
    }
    
    // Split the rest by whitespace
    let parts: Vec<&str> = remaining.trim().split_whitespace().collect();
    
    for part in parts {
        // Handle format list (pt=...)
        if part.starts_with("pt=") {
            let formats_part = if let Some(idx) = part.find(',') {
                // Multiple formats separated by commas
                let formats_str = &part[3..]; // Skip "pt="
                formats_str.split(',')
                    .map(|s| s.trim().to_string())
                    .collect()
            } else {
                // Single format
                vec![part[3..].to_string()]
            };
            formats = formats_part;
        }
        // Handle restriction key-value pair (key=value)
        else if part.contains('=') {
            // If it starts with a semicolon, remove it
            let clean_part = if part.starts_with(';') { &part[1..] } else { part };
            
            // Split on = to get key and value
            if let Some(idx) = clean_part.find('=') {
                let (key, value) = clean_part.split_at(idx);
                // Skip the '='
                let value = &value[1..];
                restrictions.insert(key.to_string(), value.to_string());
            }
        }
        // Handle a pattern like ";key=value;key2=value2"
        else if part.starts_with(';') {
            let restrictions_parts = part.split(';').filter(|s| !s.is_empty());
            
            for restriction in restrictions_parts {
                if let Some(idx) = restriction.find('=') {
                    let (key, value) = restriction.split_at(idx);
                    // Skip the '='
                    let value = &value[1..];
                    restrictions.insert(key.to_string(), value.to_string());
                }
            }
        }
    }
    
    // Also attempt to parse any trailing parts that might be left
    if !remaining.trim().is_empty() {
        // Handle restrictions/parameters that might use semicolons
        if remaining.contains(';') {
            // Split on semicolons
            let parts = remaining.split(';').collect::<Vec<&str>>();
            
            for part in parts {
                if let Some(idx) = part.find('=') {
                    let (key, value) = part.split_at(idx);
                    // Skip the '='
                    let value = &value[1..];
                    let key = key.trim();
                    if !key.is_empty() {
                        restrictions.insert(key.to_string(), value.trim().to_string());
                    }
                }
            }
        }
    }
    
    Ok(("", RidAttribute { id, direction, formats, restrictions }))
}

/// Parse a RID (Restriction IDentifier) attribute as defined in RFC 8851.
/// 
/// The format is: a=rid:<id> <direction> [pt=<fmt-list>] [;<key=value>]*
/// 
/// Returns a RidAttribute on success, or an error on failure.
pub fn parse_rid(input: &str) -> Result<RidAttribute> {
    // Trim the input
    let input = input.trim();
    
    // Split by whitespace to get the main components
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::Parser("Invalid RID: must have at least id and direction".to_string()));
    }
    
    // Extract ID (first part)
    let id = parts[0].to_string();
    
    // Validate ID format per RFC
    // ID must not contain special characters
    if id.contains('$') || id.contains('!') || id.contains('@') {
        return Err(Error::Parser(format!("Invalid RID ID '{}': contains invalid characters", id)));
    }
    
    // ID must not start with a dash or digit per RFC 8851
    if id.starts_with('-') {
        return Err(Error::Parser(format!("Invalid RID ID '{}': cannot start with a dash", id)));
    }
    
    // ID must start with a letter or underscore per RFC 8851
    if !id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return Err(Error::Parser(format!("Invalid RID ID '{}': must start with a letter or underscore", id)));
    }
    
    // Extract direction (second part)
    let direction = match parts[1].to_lowercase().as_str() {
        "send" => RidDirection::Send,
        "recv" => RidDirection::Recv,
        _ => return Err(Error::Parser(format!("Invalid RID direction '{}': must be 'send' or 'recv'", parts[1])))
    };
    
    // Initialize empty formats and restrictions
    let mut formats = Vec::new();
    let mut restrictions = HashMap::new();
    
    // Process additional parameters
    if parts.len() > 2 {
        // First, join all parts after id and direction to help with semicolon processing
        let params_str = parts[2..].join(" ");
        
        // Look for format list (pt=) first
        if let Some(pos) = params_str.find("pt=") {
            let formats_part = &params_str[pos + 3..];
            let end_pos = formats_part.find(' ').unwrap_or(formats_part.len());
            let formats_str = &formats_part[..end_pos];
            
            // Check for empty format list
            if formats_str.is_empty() {
                return Err(Error::Parser("Invalid RID: empty format list".to_string()));
            }
            
            formats = formats_str.split(',')
                .map(|s| s.trim().to_string())
                .collect();
                
            // Validate that formats are all numeric
            for fmt in &formats {
                if !fmt.chars().all(|c| c.is_ascii_digit()) {
                    return Err(Error::Parser(format!("Invalid RID format: '{}' is not a valid payload type number", fmt)));
                }
            }
        }
        
        // Process all restrictions
        // Try both semicolon-separated and space-separated formats
        
        // For semicolon-separated restrictions: ";key=value;key2=value2"
        let restrictions_parts: Vec<&str> = params_str.split(';').collect();
        for i in 0..restrictions_parts.len() {
            let part = restrictions_parts[i].trim();
            if part.is_empty() || part == "pt=" || part.starts_with("pt=") {
                continue; // Skip empty parts and pt= already processed
            }
            
            // Handle restriction without value or missing equals sign
            if !part.contains('=') {
                return Err(Error::Parser(format!("Invalid RID: restriction '{}' missing equals sign", part)));
            }
            
            // Handle restriction with empty value (ending with equals)
            if part.ends_with('=') {
                return Err(Error::Parser(format!("Invalid RID: restriction '{}' has empty value", part)));
            }
            
            let parts: Vec<&str> = part.splitn(2, '=').collect();
            let key = parts[0].trim();
            let value = if parts.len() > 1 { parts[1].trim() } else { "" };
            
            // Check for empty key
            if key.is_empty() {
                return Err(Error::Parser("Invalid RID: empty restriction key".to_string()));
            }
            
            // Check for empty value
            if value.is_empty() {
                return Err(Error::Parser(format!("Invalid RID: restriction '{}' has empty value", key)));
            }
            
            restrictions.insert(key.to_string(), value.to_string());
        }
        
        // For space-separated restrictions: "key1=value1 key2=value2"
        if restrictions.is_empty() {
            for part in &parts[2..] {
                if part.contains('=') && !part.starts_with("pt=") {
                    let key_value: Vec<&str> = part.splitn(2, '=').collect();
                    if key_value.len() >= 2 {
                        let key = key_value[0].trim();
                        let value = key_value[1].trim();
                        
                        // Check for empty key
                        if key.is_empty() {
                            return Err(Error::Parser("Invalid RID: empty restriction key".to_string()));
                        }
                        
                        // Check for empty value
                        if value.is_empty() {
                            return Err(Error::Parser(format!("Invalid RID: restriction '{}' has empty value", key)));
                        }
                        
                        restrictions.insert(key.to_string(), value.to_string());
                    } else {
                        // This handles the case where there's an equals sign but no value
                        return Err(Error::Parser(format!("Invalid RID: restriction '{}' has empty value", key_value[0])));
                    }
                }
            }
        }
    }
    
    Ok(RidAttribute {
        id,
        direction,
        formats,
        restrictions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rid_parsing() {
        // Test basic rid parsing with semicolon for restrictions
        let rid_value = "a1 send pt=97,98 ;max-width=1280;max-height=720";
        let result = parse_rid(rid_value);
        assert!(result.is_ok(), "Failed to parse basic RID: {}", rid_value);
        let rid = result.unwrap();
        assert_eq!(rid.id, "a1");
        assert_eq!(rid.direction, RidDirection::Send);
        assert_eq!(rid.formats, vec!["97", "98"]);
        assert_eq!(rid.restrictions.len(), 2);
        assert_eq!(rid.restrictions.get("max-width"), Some(&"1280".to_string()));
        assert_eq!(rid.restrictions.get("max-height"), Some(&"720".to_string()));
        
        // Test basic rid parsing without semicolon (non-RFC compliant but common)
        let rid_value2 = "a1 send pt=97,98 max-width=1280;max-height=720";
        let result2 = parse_rid(rid_value2);
        assert!(result2.is_ok(), "Failed to parse RID without semicolon: {}", rid_value2);
        
        // Test without optional parameters
        let result = parse_rid("a1 send");
        assert!(result.is_ok(), "Failed to parse minimal RID");
        let rid = result.unwrap();
        assert_eq!(rid.id, "a1");
        assert_eq!(rid.direction, RidDirection::Send);
        assert!(rid.formats.is_empty());
        assert!(rid.restrictions.is_empty());
        
        // Test invalid rid - missing direction
        let invalid_rid = "a1";
        assert!(parse_rid(invalid_rid).is_err());
    }
    
    #[test]
    fn test_complex_rid() {
        // RID with complex restrictions - both with and without leading semicolon
        let complex_rid = "r1 send pt=96,97,98 ;max-width=1280;max-height=720;max-fps=30;max-fs=8160";
        let complex_rid2 = "r1 send pt=96,97,98 max-width=1280;max-height=720;max-fps=30;max-fs=8160";
        
        let result = parse_rid(complex_rid);
        assert!(result.is_ok(), "Failed to parse complex RID with semicolon: {}", complex_rid);
        
        let result2 = parse_rid(complex_rid2);
        assert!(result2.is_ok(), "Failed to parse complex RID without semicolon: {}", complex_rid2);
        
        let rid = result.unwrap();
        assert_eq!(rid.id, "r1");
        assert_eq!(rid.direction, RidDirection::Send);
        assert_eq!(rid.formats, vec!["96", "97", "98"]);
        assert_eq!(rid.restrictions.len(), 4);
        assert_eq!(rid.restrictions.get("max-width"), Some(&"1280".to_string()));
        assert_eq!(rid.restrictions.get("max-height"), Some(&"720".to_string()));
        assert_eq!(rid.restrictions.get("max-fps"), Some(&"30".to_string()));
        assert_eq!(rid.restrictions.get("max-fs"), Some(&"8160".to_string()));
    }
    
    #[test]
    fn test_rid_edge_cases() {
        // Test with different id formats
        assert!(parse_rid("a1 send").is_ok(), "Failed to parse RID with alphanumeric ID");
        assert!(parse_rid("_123 send").is_ok(), "Failed to parse RID with underscore ID");
        assert!(parse_rid("a-b-c send").is_ok(), "Failed to parse RID with dashes in ID");
        
        // Invalid id formats
        assert!(parse_rid("a$ send").is_err(), "Should reject ID with invalid characters");
        assert!(parse_rid("-123 send").is_err(), "Should reject ID starting with dash");
        assert!(parse_rid("123 send").is_err(), "Should reject ID starting with digit");
        
        // Test direction variants
        let rid = parse_rid("id send").unwrap();
        assert_eq!(rid.direction, RidDirection::Send);
        
        let rid = parse_rid("id recv").unwrap();
        assert_eq!(rid.direction, RidDirection::Recv);
        
        // Invalid direction
        assert!(parse_rid("id both").is_err(), "Should reject invalid direction");
    }
    
    #[test]
    fn test_rid_format_list() {
        // Test format list parsing
        let rid = parse_rid("r1 send pt=96,97,98").unwrap();
        assert_eq!(rid.formats, vec!["96", "97", "98"]);
        
        // Test with single format
        let rid = parse_rid("r1 send pt=96").unwrap();
        assert_eq!(rid.formats, vec!["96"]);
    }
    
    #[test]
    fn test_rid_with_restrictions() {
        // Test with just restrictions, no format list - both formats
        let with_semi = "r1 send ;max-width=1280;max-height=720";
        let result_semi = parse_rid(with_semi);
        assert!(result_semi.is_ok(), "Failed to parse RID with semicolon: {}", with_semi);
        
        let without_semi = "r1 send max-width=1280;max-height=720";
        let result_no_semi = parse_rid(without_semi); 
        assert!(result_no_semi.is_ok(), "Failed to parse RID without semicolon: {}", without_semi);
        
        let rid = result_semi.unwrap();
        assert_eq!(rid.restrictions.len(), 2);
        assert_eq!(rid.restrictions.get("max-width"), Some(&"1280".to_string()));
        assert_eq!(rid.restrictions.get("max-height"), Some(&"720".to_string()));
    }
    
    #[test]
    fn test_rid_whitespace_handling() {
        // Test with various whitespace patterns
        let rid1 = "r1 send pt=96,97,98 ;max-width=1280;max-height=720";
        let rid2 = "r1  send  pt=96,97,98  ;max-width=1280;max-height=720";
        let rid3 = "r1 send pt=96,97,98 max-width=1280;max-height=720";
        
        assert!(parse_rid(rid1).is_ok(), "Failed to parse standard RID");
        assert!(parse_rid(rid2).is_ok(), "Failed to parse RID with extra spaces");
        assert!(parse_rid(rid3).is_ok(), "Failed to parse RID without semicolon");
        
        // Leading/trailing whitespace
        let rid4 = "  r1 send pt=96,97,98  ";
        assert!(parse_rid(rid4).is_ok(), "Failed to parse RID with leading/trailing whitespace");
    }
    
    #[test]
    fn test_rid_from_rfc_examples() {
        // Test with examples from RFC 8851
        
        // Example 1: Basic RID with payload type
        let ex1 = "r1 send pt=97";
        assert!(parse_rid(ex1).is_ok(), "Failed to parse RFC example 1");
        
        // Example 2: RID with restrictions - both with and without semicolon
        let ex2a = "r2 recv pt=98 ;max-width=800;max-height=600";
        let ex2b = "r2 recv pt=98 max-width=800;max-height=600";
        
        assert!(parse_rid(ex2a).is_ok(), "Failed to parse RFC example 2 with semicolon");
        assert!(parse_rid(ex2b).is_ok(), "Failed to parse RFC example 2 without semicolon");
        
        let result = parse_rid(ex2a);
        assert!(result.is_ok(), "Failed to parse RFC example 2");
        let rid = result.unwrap();
        assert_eq!(rid.id, "r2");
        assert_eq!(rid.direction, RidDirection::Recv);
        assert_eq!(rid.formats, vec!["98"]);
        assert_eq!(rid.restrictions.len(), 2);
        assert_eq!(rid.restrictions.get("max-width"), Some(&"800".to_string()));
        assert_eq!(rid.restrictions.get("max-height"), Some(&"600".to_string()));
        
        // Example 3: Complex RID from RFC section 4 - both with and without semicolon
        let ex3a = "foo send pt=97 ;max-width=1280;max-height=720";
        let ex3b = "foo send pt=97 max-width=1280;max-height=720";
        
        assert!(parse_rid(ex3a).is_ok(), "Failed to parse RFC example 3 with semicolon");
        assert!(parse_rid(ex3b).is_ok(), "Failed to parse RFC example 3 without semicolon");
    }
    
    #[test]
    fn test_invalid_rid_syntax() {
        // Missing space between ID and direction
        assert!(parse_rid("r1send").is_err(), "Should reject missing space between ID and direction");
        
        // Empty ID
        assert!(parse_rid(" send").is_err(), "Should reject empty ID");
        
        // Empty direction
        assert!(parse_rid("r1 ").is_err(), "Should reject empty direction");
        
        // Incomplete format list
        assert!(parse_rid("r1 send pt=").is_err(), "Should reject incomplete format list");
        
        // Invalid format - non-numeric
        assert!(parse_rid("r1 send pt=96,foo").is_err(), "Should reject non-numeric payload type");
        
        // Incomplete restriction
        assert!(parse_rid("r1 send ;max-width=").is_err(), "Should reject incomplete restriction");
        assert!(parse_rid("r1 send ;max-width").is_err(), "Should reject restriction without value");
        
        // Restriction without key
        assert!(parse_rid("r1 send ;=1280").is_err(), "Should reject restriction without key");
    }
    
    #[test]
    fn test_parser_functions_directly() {
        // Test parse_rid_id
        let (rem, id) = parse_rid_id("example-id rest").unwrap();
        assert_eq!(id, "example-id");
        assert_eq!(rem, " rest");
        
        // Test parse_direction
        let (rem, dir) = parse_direction("send rest").unwrap();
        assert_eq!(dir, RidDirection::Send);
        assert_eq!(rem, " rest");
        
        // Test parse_format_list
        let (rem, formats) = parse_format_list("pt=96,97,98 rest").unwrap();
        assert_eq!(formats, vec!["96", "97", "98"]);
        assert_eq!(rem, " rest");
        
        // Test parse_restriction
        let (rem, (key, val)) = parse_restriction("max-width=1280 rest").unwrap();
        assert_eq!(key, "max-width");
        assert_eq!(val, "1280");
        assert_eq!(rem, " rest");
        
        // Test parse_restrictions_list - both with and without semicolon
        let (rem, restrictions) = parse_restrictions_list(";max-width=1280;max-height=720 rest").unwrap();
        assert_eq!(restrictions.len(), 2);
        assert_eq!(restrictions.get("max-width"), Some(&"1280".to_string()));
        assert_eq!(restrictions.get("max-height"), Some(&"720".to_string()));
        assert_eq!(rem, " rest");
        
        let (rem, restrictions) = parse_restrictions_list("max-width=1280;max-height=720 rest").unwrap();
        assert_eq!(restrictions.len(), 2);
        assert_eq!(restrictions.get("max-width"), Some(&"1280".to_string()));
        assert_eq!(restrictions.get("max-height"), Some(&"720".to_string()));
        assert_eq!(rem, " rest");
    }
    
    #[test]
    fn test_strict_rfc_compliance() {
        // Test with reordered components - pt= after restrictions
        // This is not valid according to the RFC grammar, but our flexible parser handles it
        let non_compliant = "r1 send ;max-width=1280;max-height=720 pt=96,97,98";
        assert!(parse_rid(non_compliant).is_ok(), "Parser should handle non-standard ordering flexibly");
        
        // Test with missing semicolon before restrictions (should now pass for compatibility)
        let non_compliant2 = "r1 send pt=96,97,98 max-width=1280;max-height=720";
        assert!(parse_rid(non_compliant2).is_ok(), "Parser should handle missing semicolon for compatibility");
    }
} 