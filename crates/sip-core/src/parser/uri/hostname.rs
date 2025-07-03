use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{alpha1, alphanumeric1},
    combinator::{map_res, recognize, verify},
    multi::{many0, many1},
    sequence::{pair, preceded, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;
use crate::error::{Error, Result};

// RFC 1034/1035 compliant hostname parser
// hostname = *( domainlabel "." ) toplabel [ "." ]
// domainlabel = alphanum / alphanum *( alphanum / "-" ) alphanum
// toplabel = ALPHA / ALPHA *( alphanum / "-" ) alphanum

// Valid character for a hostname label (alphanumeric or hyphen only)
fn is_label_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-'
}

// Parse a single valid domain label (without dots)
// Cannot start or end with a hyphen per RFC 1034
fn domain_label(input: &[u8]) -> ParseResult<&[u8]> {
    // Verify the label doesn't start or end with hyphen
    verify(
        // Must have at least one character, all of which are valid label chars
        take_while1(is_label_char),
        |label: &[u8]| {
            !label.is_empty() && 
            label[0] != b'-' && 
            label[label.len() - 1] != b'-'
        }
    )(input)
}

// Parse a hostname including handling of trailing dot for FQDN
fn parse_hostname(input: &[u8]) -> ParseResult<&[u8]> {
    // Find the position of a colon, semicolon, or question mark if it exists (for port, params, or headers)
    let port_position = input.iter().position(|&c| c == b':');
    let param_position = input.iter().position(|&c| c == b';');
    let header_position = input.iter().position(|&c| c == b'?');
    
    // Determine the end position - use the earliest of port, param, or header if multiple exist
    let end_position = match (port_position, param_position, header_position) {
        (Some(port_pos), Some(param_pos), Some(header_pos)) => Some(port_pos.min(param_pos).min(header_pos)),
        (Some(port_pos), Some(param_pos), None) => Some(port_pos.min(param_pos)),
        (Some(port_pos), None, Some(header_pos)) => Some(port_pos.min(header_pos)),
        (None, Some(param_pos), Some(header_pos)) => Some(param_pos.min(header_pos)),
        (Some(pos), None, None) => Some(pos),
        (None, Some(pos), None) => Some(pos),
        (None, None, Some(pos)) => Some(pos),
        (None, None, None) => None
    };
    
    // If a terminator exists, only parse up to that point
    let parse_input = match end_position {
        Some(pos) => &input[..pos],
        None => input,
    };
    
    // Reject consecutive dots (empty labels)
    if parse_input.windows(2).any(|window| window == b"..") {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    // Reject underscores in hostnames
    if parse_input.iter().any(|&c| c == b'_') {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    // Empty input is not a valid hostname
    if parse_input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    // Handle single labels (like "localhost") - valid hostname
    if !parse_input.contains(&b'.') {
        return match domain_label(parse_input) {
            Ok(_) => {
                // Return remaining input (which might include port or params)
                if let Some(pos) = end_position {
                    Ok((&input[pos..], parse_input))
                } else {
                    Ok((&[][..], parse_input))
                }
            },
            Err(e) => Err(e)
        };
    }
    
    // Handle trailing dot in FQDN format (e.g., "example.com.")
    let has_trailing_dot = parse_input.len() > 1 && parse_input[parse_input.len() - 1] == b'.';
    
    // Parse main part of the hostname (without the trailing dot)
    let hostname_input = if has_trailing_dot {
        &parse_input[..parse_input.len() - 1]
    } else {
        parse_input
    };
    
    // Leading dot is invalid (empty first label)
    if hostname_input.starts_with(b".") {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    // Split the hostname by dots and verify each label
    let labels: Vec<&[u8]> = hostname_input.split(|&c| c == b'.').collect();
    
    // Check each label for validity
    for label in &labels {
        if label.is_empty() || label[0] == b'-' || label[label.len() - 1] == b'-' || 
           !label.iter().all(|&c| is_label_char(c)) {
            return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
        }
    }
    
    // Don't apply IPv4 detection to hostnames with no chance of being IPv4
    let could_be_ipv4 = hostname_input.iter().all(|&c| c == b'.' || c.is_ascii_digit());
    
    // Only reject hostnames that are definitely meant to be IPv4 addresses
    if could_be_ipv4 && is_likely_actual_ipv4(hostname_input) {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    // Calculate remaining input based on what we parsed and where the terminator is
    if let Some(pos) = end_position {
        // Return the port or param part as remaining input
        Ok((&input[pos..], hostname_input))
    } else if has_trailing_dot {
        // Return the trailing dot as remaining
        Ok((&parse_input[parse_input.len() - 1..], hostname_input))
    } else {
        // No port, params, or trailing dot
        Ok((&[][..], hostname_input))
    }
}

// More precise IPv4 address detection to avoid false positives
// Only marks definite IPv4 addresses, not just numeric domains that look like IPv4
fn is_likely_actual_ipv4(input: &[u8]) -> bool {
    // Must be a specific format: 4 numeric segments separated by dots
    let segments: Vec<&[u8]> = input.split(|&c| c == b'.').collect();
    
    if segments.len() != 4 {
        return false;
    }
    
    // Check that each segment is a valid IPv4 octet
    for segment in segments {
        // Empty segment
        if segment.is_empty() {
            return false;
        }
        
        // Leading zeros (except single 0) not allowed in real IPv4
        if segment.len() > 1 && segment[0] == b'0' {
            return false;
        }
        
        // All characters must be digits
        if !segment.iter().all(|&c| c.is_ascii_digit()) {
            return false;
        }
        
        // Convert to a number and check range (0-255)
        match std::str::from_utf8(segment) {
            Ok(s) => match s.parse::<u8>() {
                Ok(_) => {}, // Valid octet in 0-255 range
                Err(_) => return false, // Outside 0-255 range
            },
            Err(_) => return false, // Not valid UTF-8 (shouldn't happen with ASCII digits)
        }
    }
    
    // Only categorize as IPv4 if it strictly follows IPv4 format
    true
}

// Public hostname parser function
pub fn hostname(input: &[u8]) -> ParseResult<Host> {
    // Handle the case where the input is a single dot
    if input == b"." {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    // Use the internal parser to handle the parsing
    let (remaining, host_bytes) = parse_hostname(input)?;
    
    // Convert the parsed bytes to a domain name
    let domain = match str::from_utf8(host_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Char))),
    };
    
    Ok((remaining, Host::Domain(domain)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostname_basic() {
        // Basic valid hostnames
        assert_eq!(hostname(b"example.com"), Ok((&[][..], Host::Domain("example.com".to_string()))));
        assert_eq!(hostname(b"host1.subdomain.example.co.uk"), Ok((&[][..], Host::Domain("host1.subdomain.example.co.uk".to_string()))));
        assert_eq!(hostname(b"a-valid-host.net"), Ok((&[][..], Host::Domain("a-valid-host.net".to_string()))));
        assert_eq!(hostname(b"xn--ls8h.example"), Ok((&[][..], Host::Domain("xn--ls8h.example".to_string())))); // IDN
        
        // Trailing dot (RFC 1035, FQDN format)
        assert_eq!(hostname(b"example.com."), Ok((&b"."[..], Host::Domain("example.com".to_string()))));
    }
    
    #[test]
    fn test_hostname_invalid_basic() {
        // Invalid cases
        assert!(hostname(b"-invalid.start").is_err());
        assert!(hostname(b"invalid.end-").is_err());
        assert!(hostname(b"invalid..dot").is_err()); // Consecutive dots (empty label)
        assert!(hostname(b".").is_err()); // Just a dot
        assert!(hostname(b".invalid.start").is_err()); // Leading dot
    }
    
    #[test]
    fn test_hostname_rfc3261_compliance() {
        // RFC 3261 Section 25.1 (Core Syntax)
        // The hostname syntax follows RFC 1034/1035 with modifications for SIP
        
        // Valid examples from RFC 3261
        assert_eq!(hostname(b"atlanta.com"), Ok((&[][..], Host::Domain("atlanta.com".to_string()))));
        assert_eq!(hostname(b"biloxi.com"), Ok((&[][..], Host::Domain("biloxi.com".to_string()))));
        
        // Single-label hostnames are valid in RFC 3261
        assert_eq!(hostname(b"localhost"), Ok((&[][..], Host::Domain("localhost".to_string()))));
        
        // Domain labels can be up to 63 characters per RFC 1034
        let long_label = b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijk.com"; // 63 chars + .com
        assert_eq!(hostname(long_label), Ok((&[][..], Host::Domain(String::from_utf8_lossy(long_label).to_string()))));
    }
    
    #[test]
    fn test_hostname_edge_cases() {
        // RFC 1034/1035 edge cases
        
        // Labels should only contain alphanumeric characters and hyphens
        assert!(hostname(b"invalid_underscore.com").is_err()); 
        
        // Modern DNS implementations sometimes allow underscores in certain contexts
        // SIP hostnames are more strict and don't allow this
        assert!(hostname(b"_sip._tcp.example.com").is_err());
        
        // Double dots (empty labels) are invalid
        assert!(hostname(b"example..com").is_err());
        assert!(hostname(b".example.com").is_err()); // Leading dot (empty first label)
    }
    
    #[test]
    fn test_hostname_domain_labels() {
        // RFC 1034/1035 domain label tests
        
        // Domain labels can't start or end with hyphens
        assert!(hostname(b"-invalid.com").is_err());
        assert!(hostname(b"invalid-.com").is_err());
        assert!(hostname(b"valid.com-").is_err());
        
        // Valid domain label with hyphens in the middle
        assert_eq!(hostname(b"this-is-valid.com"), Ok((&[][..], Host::Domain("this-is-valid.com".to_string()))));
    }
    
    #[test]
    fn test_hostname_rfc5890_idn() {
        // RFC 5890-5894 Internationalized Domain Names
        // IDNs in ASCII-compatible encoding (Punycode with xn-- prefix)
        
        // Valid IDNs in Punycode
        assert_eq!(hostname(b"xn--bcher-kva.example"), Ok((&[][..], Host::Domain("xn--bcher-kva.example".to_string())))); // bücher.example
        assert_eq!(hostname(b"xn--caf-dma.example"), Ok((&[][..], Host::Domain("xn--caf-dma.example".to_string())))); // café.example
    }
    
    #[test]
    fn test_hostname_not_ipv4() {
        // Test cases that look like IPv4 addresses but should be parsed as hostnames
        
        // Numeric domains that don't follow IPv4 format should be valid hostnames
        assert_eq!(hostname(b"999.888.777.666"), Ok((&[][..], Host::Domain("999.888.777.666".to_string()))));
        assert_eq!(hostname(b"192.168.1"), Ok((&[][..], Host::Domain("192.168.1".to_string()))));
        assert_eq!(hostname(b"192.168.1."), Ok((&b"."[..], Host::Domain("192.168.1".to_string()))));
    }
    
    #[test]
    fn test_hostname_rfc4475_torture_cases() {
        // RFC 4475 SIP Torture test cases
        
        // Valid hostnames with numeric labels from RFC 4475
        assert_eq!(hostname(b"987.13.55.44"), Ok((&[][..], Host::Domain("987.13.55.44".to_string()))));
        assert_eq!(hostname(b"555.example.com"), Ok((&[][..], Host::Domain("555.example.com".to_string()))));
    }
    
    #[test]
    fn test_hostname_with_port_suffix() {
        // Test hostnames with port suffix to ensure proper parsing
        assert_eq!(hostname(b"example.com:5060"), Ok((&b":5060"[..], Host::Domain("example.com".to_string()))));
        assert_eq!(hostname(b"localhost:5060"), Ok((&b":5060"[..], Host::Domain("localhost".to_string()))));
    }
    
    #[test]
    fn test_hostname_fqdn_format() {
        // Test fully qualified domain names with trailing dots
        assert_eq!(hostname(b"example.com."), Ok((&b"."[..], Host::Domain("example.com".to_string()))));
        assert_eq!(hostname(b"a.b.c.d.e.f."), Ok((&b"."[..], Host::Domain("a.b.c.d.e.f".to_string()))));
    }

    #[test]
    fn test_hostname_with_semicolon() {
        // Test that semicolons are properly recognized as terminators
        let (rem, host) = hostname(b"example.com;transport=tcp").unwrap();
        assert_eq!(rem, b";transport=tcp");
        assert_eq!(host, Host::Domain("example.com".to_string()));

        // Test with both port and params
        let (rem, host) = hostname(b"example.com:5060;transport=tcp").unwrap();
        assert_eq!(rem, b":5060;transport=tcp");
        assert_eq!(host, Host::Domain("example.com".to_string()));

        // Test with multiple params
        let (rem, host) = hostname(b"example.com;transport=tcp;lr").unwrap();
        assert_eq!(rem, b";transport=tcp;lr");
        assert_eq!(host, Host::Domain("example.com".to_string()));
        
        // Test with question marks for headers
        let (rem, host) = hostname(b"example.com?Subject=Hello").unwrap();
        assert_eq!(rem, b"?Subject=Hello");
        assert_eq!(host, Host::Domain("example.com".to_string()));
        
        // Test with both param and headers
        let (rem, host) = hostname(b"example.com;transport=tcp?Subject=Hello").unwrap();
        assert_eq!(rem, b";transport=tcp?Subject=Hello");
        assert_eq!(host, Host::Domain("example.com".to_string()));
    }
} 