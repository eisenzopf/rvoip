// Declare URI sub-modules
pub mod host;
pub mod userinfo;
pub mod params;
pub mod headers;
pub mod absolute;
pub mod authority;
pub mod path;
pub mod query;
pub mod scheme;

// Host sub-modules
pub mod hostname;
pub mod ipv4;
pub mod ipv6;

// Re-export relevant parsers from sub-modules
pub use host::{host, port, hostport};
pub use userinfo::userinfo;
pub use params::uri_parameters;
pub use headers::uri_headers;
pub use absolute::parse_absolute_uri;

// Add imports for combinators and types
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while},
    character::complete::{char, digit1, one_of},
    combinator::{map, map_res, opt, recognize, value, verify, all_consuming, eof},
    error::{Error as NomError, ErrorKind},
    multi::{many0, many1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::str;

use crate::types::uri::{Host, Uri};
use crate::types::param::Param;
use crate::parser::ParseResult;
use crate::types::uri::Scheme;
use crate::error::Error;

use authority::parse_authority;

// SIP-URI = "sip:" [ userinfo ] hostport uri-parameters [ headers ]
pub fn parse_sip_uri(bytes: &[u8]) -> ParseResult<Uri> {
    let bytes_slice = bytes;
    let res = tuple((
        tag_no_case(b"sip:"),
        opt(userinfo),
        hostport,
        opt(uri_parameters),
        opt(uri_headers),
    ))(bytes_slice);

    match res {
        Ok((remaining, (_, user_info_opt, (host, port_opt), params_opt, headers_opt))) => {
            // user_info is already (String, Option<String>)
            let (user, password) = user_info_opt.unwrap_or((String::new(), None));
            
            let uri = Uri {
                scheme: Scheme::Sip,
                user: if user.is_empty() { None } else { Some(user) },
                password,
                host,
                port: port_opt,
                parameters: params_opt.unwrap_or_default(),
                headers: headers_opt.unwrap_or_default(),
                raw_uri: None,
            };
            
            Ok((remaining, uri))
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            Err(nom::Err::Error(e))
        }
        Err(e) => Err(e),
    }
}

// SIPS-URI = "sips:" [ userinfo ] hostport uri-parameters [ headers ]
pub fn parse_sips_uri(bytes: &[u8]) -> ParseResult<Uri> {
    let bytes_slice = bytes;
    let res = tuple((
        tag_no_case(b"sips:"),
        opt(userinfo),
        hostport,
        opt(uri_parameters),
        opt(uri_headers),
    ))(bytes_slice);

    match res {
        Ok((remaining, (_, user_info_opt, (host, port_opt), params_opt, headers_opt))) => {
            // user_info is already (String, Option<String>)
            let (user, password) = user_info_opt.unwrap_or((String::new(), None));
            
            let uri = Uri {
                scheme: Scheme::Sips,
                user: if user.is_empty() { None } else { Some(user) },
                password,
                host,
                port: port_opt,
                parameters: params_opt.unwrap_or_default(),
                headers: headers_opt.unwrap_or_default(),
                raw_uri: None,
            };
            
            Ok((remaining, uri))
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            Err(nom::Err::Error(e))
        }
        Err(e) => Err(e),
    }
}

/// Public entry point for parsing a SIP or SIPS URI
/// Can be re-exported by the main parser mod.rs
pub fn parse_uri(input: &[u8]) -> ParseResult<Uri> {
    // Use alt to try each parser
    alt((parse_sip_uri_fixed, parse_sips_uri_fixed, parse_tel_uri))(input)
}

/// Special version of parse_uri that allows for non-standard schemes
/// This should only be used in contexts where we need to handle arbitrary URIs, 
/// such as in the macro builder or when processing messages from other systems
pub fn parse_uri_lenient(input: &[u8]) -> ParseResult<Uri> {
    // First try the standard parsers (SIP, SIPS, TEL)
    let result = alt((parse_sip_uri_fixed, parse_sips_uri_fixed, parse_tel_uri))(input);
    
    if result.is_ok() {
        return result;
    }
    
    // If standard parsers fail, try our fallback generic URI parser
    parse_generic_uri(input)
}

/// Fallback parser for non-standard URI schemes (RFC 3986 compliant)
fn parse_generic_uri(input: &[u8]) -> ParseResult<Uri> {
    // Try to extract the scheme part (up to the ':')
    if let Some(colon_pos) = input.iter().position(|&c| c == b':') {
        // Convert the raw input to a string for the Uri struct
        if let Ok(uri_str) = std::str::from_utf8(input) {
            // Extract the scheme
            let scheme = if colon_pos > 0 {
                let scheme_str = &uri_str[0..colon_pos];
                match scheme_str.to_lowercase().as_str() {
                    "sip" => Scheme::Sip,
                    "sips" => Scheme::Sips,
                    "tel" => Scheme::Tel,
                    "http" => Scheme::Http,
                    "https" => Scheme::Https,
                    _ => Scheme::Custom(scheme_str.to_string()),
                }
            } else {
                // Colon at position 0 is invalid, but we'll create a custom scheme
                Scheme::Custom("invalid".to_string())
            };
            
            // Create a Uri with the raw text preserved
            let uri = Uri {
                scheme,
                user: None,
                password: None,
                host: Host::domain("example.com"), // Default host
                port: None,
                parameters: Vec::new(),
                headers: HashMap::new(),
                raw_uri: Some(uri_str.to_string()),
            };
            return Ok((b"", uri)); // Successfully consumed all input
        }
    }
    
    // If we can't even extract a scheme, it's not a valid URI
    Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)))
}

// Fixed implementation of SIP URI parser that correctly handles params and headers
fn parse_sip_uri_fixed(input: &[u8]) -> ParseResult<Uri> {
    let (input, _) = tag_no_case(b"sip:")(input)?;
    
    // Get the user info (if any)
    let mut userinfo_present = false;
    let user_info;
    
    // Check if @ symbol exists in input
    if let Some(at_pos) = input.iter().position(|&c| c == b'@') {
        userinfo_present = true;
        match userinfo(&input[..at_pos + 1]) {
            Ok((_, parsed_userinfo)) => {
                user_info = Some(parsed_userinfo);
            },
            _ => {
                // If userinfo fails to parse, treat as no userinfo
                user_info = None;
                userinfo_present = false;
            }
        }
    } else {
        user_info = None;
    }
    
    // Skip past user info if present
    let input = if userinfo_present {
        let at_pos = input.iter().position(|&c| c == b'@').unwrap();
        &input[at_pos + 1..]
    } else {
        input
    };
    
    // Special check for non-numeric port values
    // If there's a colon not followed by digits, treat it as an error
    if let Some(colon_pos) = input.iter().position(|&c| c == b':') {
        // First check if this is inside an IPv6 reference
        // Look for opening bracket before this position
        let has_ipv6_before = input[..colon_pos].iter().any(|&c| c == b'[');
        let ipv6_reference_open = has_ipv6_before && !input[..colon_pos].iter().any(|&c| c == b']');
        
        // Skip validation if we're inside an IPv6 reference (this is not a port colon)
        if !ipv6_reference_open && colon_pos + 1 < input.len() {
            let port_start = &input[colon_pos + 1..];
            
            // Skip the check if the colon is immediately followed by an opening bracket (IPv6 in host)
            if !port_start.is_empty() && port_start[0] != b'[' {
                if port_start.is_empty() || !port_start[0].is_ascii_digit() {
                    // Port starts with non-digit
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Digit);
                    return Err(nom::Err::Error(error));
                }
                
                // Find the end of the port number
                let mut i = 0;
                while i < port_start.len() && port_start[i].is_ascii_digit() {
                    i += 1;
                }
                
                // If port contains non-digit characters before parameter or header delimiter
                if i > 0 && i < port_start.len() && port_start[i] != b';' && port_start[i] != b'?' {
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Digit);
                    return Err(nom::Err::Error(error));
                }
                
                // Now check if the port value exceeds u16 range (0-65535)
                if i > 0 {
                    let port_digits = &port_start[0..i];
                    if port_digits.len() > 5 {  // More than 5 digits definitely exceeds 65535
                        let error = nom::error::Error::new(input, nom::error::ErrorKind::Verify);
                        return Err(nom::Err::Error(error));
                    } else if port_digits.len() == 5 {
                        // Check if the value is > 65535
                        if let Ok(port_str) = std::str::from_utf8(port_digits) {
                            if let Ok(port_value) = port_str.parse::<u32>() {
                                if port_value > 65535 {
                                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Verify);
                                    return Err(nom::Err::Error(error));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Parse hostport
    match hostport(input) {
        Ok((remaining, (host, port))) => {
            // Parse parameters if present (starting with ;)
            let mut params = Vec::new();
            let mut current_remaining = remaining;
            
            if !current_remaining.is_empty() && current_remaining[0] == b';' {
                // Check for parameters with no name (;=)
                if current_remaining.len() >= 2 && current_remaining[1] == b'=' {
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Tag);
                    return Err(nom::Err::Error(error));
                }
                
                match uri_parameters(current_remaining) {
                    Ok((new_remaining, parsed_params)) => {
                        current_remaining = new_remaining;
                        params = parsed_params;
                    },
                    Err(e) => {
                        eprintln!("Parameter parsing failed: {:?}", e);
                        // Return the error instead of silently continuing
                        return Err(e);
                    }
                }
            }
            
            // Parse headers if present (starting with ?)
            let mut headers = HashMap::new();
            if !current_remaining.is_empty() && current_remaining[0] == b'?' {
                // Check for headers with no name (?=)
                if current_remaining.len() >= 2 && current_remaining[1] == b'=' {
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Tag);
                    return Err(nom::Err::Error(error));
                }
                
                match uri_headers(current_remaining) {
                    Ok((new_remaining, parsed_headers)) => {
                        current_remaining = new_remaining;
                        headers = parsed_headers;
                    },
                    Err(e) => {
                        // Return the error instead of silently continuing
                        return Err(e);
                    }
                }
            }
            
            // Create URI
            let (user, password) = user_info.unwrap_or((String::new(), None));
            let uri = Uri {
                scheme: Scheme::Sip,
                user: if user.is_empty() { None } else { Some(user) },
                password,
                host,
                port,
                parameters: params,
                headers,
                raw_uri: None,
            };
            
            Ok((current_remaining, uri))
        },
        Err(e) => {
            eprintln!("Hostport parsing failed: {:?}", e);
            // Just propagate the error
            Err(e)
        }
    }
}

// Fixed implementation of SIPS URI parser that correctly handles params and headers
fn parse_sips_uri_fixed(input: &[u8]) -> ParseResult<Uri> {
    let (input, _) = tag_no_case(b"sips:")(input)?;
    
    // Get the user info (if any)
    let mut userinfo_present = false;
    let user_info;
    
    // Check if @ symbol exists in input
    if let Some(at_pos) = input.iter().position(|&c| c == b'@') {
        userinfo_present = true;
        match userinfo(&input[..at_pos + 1]) {
            Ok((_, parsed_userinfo)) => {
                user_info = Some(parsed_userinfo);
            },
            _ => {
                // If userinfo fails to parse, treat as no userinfo
                user_info = None;
                userinfo_present = false;
            }
        }
    } else {
        user_info = None;
    }
    
    // Skip past user info if present
    let input = if userinfo_present {
        let at_pos = input.iter().position(|&c| c == b'@').unwrap();
        &input[at_pos + 1..]
    } else {
        input
    };
    
    // Special check for non-numeric port values
    // If there's a colon not followed by digits, treat it as an error
    if let Some(colon_pos) = input.iter().position(|&c| c == b':') {
        // First check if this is inside an IPv6 reference
        // Look for opening bracket before this position
        let has_ipv6_before = input[..colon_pos].iter().any(|&c| c == b'[');
        let ipv6_reference_open = has_ipv6_before && !input[..colon_pos].iter().any(|&c| c == b']');
        
        // Skip validation if we're inside an IPv6 reference (this is not a port colon)
        if !ipv6_reference_open && colon_pos + 1 < input.len() {
            let port_start = &input[colon_pos + 1..];
            
            // Skip the check if the colon is immediately followed by an opening bracket (IPv6 in host)
            if !port_start.is_empty() && port_start[0] != b'[' {
                if port_start.is_empty() || !port_start[0].is_ascii_digit() {
                    // Port starts with non-digit
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Digit);
                    return Err(nom::Err::Error(error));
                }
                
                // Find the end of the port number
                let mut i = 0;
                while i < port_start.len() && port_start[i].is_ascii_digit() {
                    i += 1;
                }
                
                // If port contains non-digit characters before parameter or header delimiter
                if i > 0 && i < port_start.len() && port_start[i] != b';' && port_start[i] != b'?' {
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Digit);
                    return Err(nom::Err::Error(error));
                }
                
                // Now check if the port value exceeds u16 range (0-65535)
                if i > 0 {
                    let port_digits = &port_start[0..i];
                    if port_digits.len() > 5 {  // More than 5 digits definitely exceeds 65535
                        let error = nom::error::Error::new(input, nom::error::ErrorKind::Verify);
                        return Err(nom::Err::Error(error));
                    } else if port_digits.len() == 5 {
                        // Check if the value is > 65535
                        if let Ok(port_str) = std::str::from_utf8(port_digits) {
                            if let Ok(port_value) = port_str.parse::<u32>() {
                                if port_value > 65535 {
                                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Verify);
                                    return Err(nom::Err::Error(error));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Parse hostport
    match hostport(input) {
        Ok((remaining, (host, port))) => {
            // Parse parameters if present (starting with ;)
            let mut params = Vec::new();
            let mut current_remaining = remaining;
            
            if !current_remaining.is_empty() && current_remaining[0] == b';' {
                // Check for parameters with no name (;=)
                if current_remaining.len() >= 2 && current_remaining[1] == b'=' {
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Tag);
                    return Err(nom::Err::Error(error));
                }
                
                match uri_parameters(current_remaining) {
                    Ok((new_remaining, parsed_params)) => {
                        current_remaining = new_remaining;
                        params = parsed_params;
                    },
                    Err(e) => {
                        // Return the error instead of silently continuing
                        return Err(e);
                    }
                }
            }
            
            // Parse headers if present (starting with ?)
            let mut headers = HashMap::new();
            if !current_remaining.is_empty() && current_remaining[0] == b'?' {
                // Check for headers with no name (?=)
                if current_remaining.len() >= 2 && current_remaining[1] == b'=' {
                    let error = nom::error::Error::new(input, nom::error::ErrorKind::Tag);
                    return Err(nom::Err::Error(error));
                }
                
                match uri_headers(current_remaining) {
                    Ok((new_remaining, parsed_headers)) => {
                        current_remaining = new_remaining;
                        headers = parsed_headers;
                    },
                    Err(e) => {
                        // Return the error instead of silently continuing
                        return Err(e);
                    }
                }
            }
            
            // Create URI
            let (user, password) = user_info.unwrap_or((String::new(), None));
            let uri = Uri {
                scheme: Scheme::Sips,
                user: if user.is_empty() { None } else { Some(user) },
                password,
                host,
                port,
                parameters: params,
                headers,
                raw_uri: None,
            };
            
            Ok((current_remaining, uri))
        },
        Err(e) => {
            eprintln!("Hostport parsing failed: {:?}", e);
            // Just propagate the error
            Err(e)
        }
    }
}

// Parse tel URI (RFC 3966)
// tel:telephone-subscriber
// telephone-subscriber = global-number / local-number
fn parse_tel_uri(input: &[u8]) -> ParseResult<Uri> {
    // Tag for "tel:"
    let (input, _) = tag_no_case(b"tel:")(input)?;
    
    // In our implementation, we store the telephone number as the host part of the URI
    // while marking the URI scheme as Tel
    // Per RFC 3966, the telephone-subscriber part can include various characters
    // Capture everything until a parameter or end of input
    let (input, number) = recognize(
        take_while(|c| {
            c != b';' && c != b'?' // Stop at parameter or header delimiter
        })
    )(input)?;
    
    // Convert number to string
    let number_str = std::str::from_utf8(number)
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?
        .to_string();
    
    // Parse parameters if present
    let mut params = Vec::new();
    let mut current_input = input;
    
    if !current_input.is_empty() && current_input[0] == b';' {
        match uri_parameters(current_input) {
            Ok((remaining, parsed_params)) => {
                current_input = remaining;
                params = parsed_params;
            },
            Err(e) => return Err(e),
        }
    }
    
    // Create TEL URI
    let uri = Uri {
        scheme: Scheme::Tel,
        user: None, // TEL URI doesn't have a user component in our model
        password: None,
        host: Host::Domain(number_str), // Store the phone number as domain
        port: None, // TEL URI doesn't have a port
        parameters: params,
        headers: HashMap::new(), // TEL URI typically doesn't use headers
        raw_uri: None,
    };
    
    Ok((current_input, uri))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Scheme;
    use crate::types::uri::Host;
    use crate::types::param::Param;
    use crate::types::param::GenericValue;
    use std::net::{Ipv4Addr, IpAddr};
    use nom::error::ErrorKind;

    #[test]
    fn test_parse_sip_uri_fixed_directly() {
        // Test the individual parsing function directly
        let uri_bytes = b"sip:user@example.com;transport=tcp;lr";
        match parse_sip_uri_fixed(uri_bytes) {
            Ok((rem, uri)) => {
                println!("Successfully parsed URI");
                println!("Remaining: {:?}", rem);
                println!("URI: {:?}", uri);
                println!("Parameters: {:?}", uri.parameters);
                
                // Verify the parsed URI is correct
                assert!(rem.is_empty());
                assert_eq!(uri.scheme, Scheme::Sip);
                assert_eq!(uri.user, Some("user".to_string()));
                assert_eq!(uri.password, None);
                assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
                assert_eq!(uri.port, None);
                assert_eq!(uri.parameters.len(), 2);
                assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
                assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
                assert!(uri.headers.is_empty());
            },
            Err(e) => {
                println!("Failed to parse URI with parse_sip_uri_fixed: {:?}", e);
                panic!("parse_sip_uri_fixed failed when it should have succeeded");
            }
        }
    }

    #[test]
    fn test_parse_uri_with_params_debug() {
        // Add a debugging test to help understand the issue
        let uri_bytes = b"sip:user@example.com;transport=tcp;lr";
        match parse_uri(uri_bytes) {
            Ok((rem, uri)) => {
                println!("Successfully parsed URI");
                println!("Remaining: {:?}", rem);
                println!("URI: {:?}", uri);
                println!("Parameters: {:?}", uri.parameters);
            },
            Err(e) => {
                println!("Failed to parse URI: {:?}", e);
                // Try parsing the components separately to see where it's failing
                println!("Trying to parse userinfo...");
                if let Some(at_pos) = uri_bytes.iter().position(|&c| c == b'@') {
                    match userinfo(&uri_bytes[4..at_pos+1]) {
                        Ok((rem, info)) => println!("Userinfo parsed successfully: {:?}", info),
                        Err(e) => println!("Userinfo parsing failed: {:?}", e),
                    }
                }
                
                // Try to find where hostport ends and params begin
                if let Some(semi_pos) = uri_bytes.iter().position(|&c| c == b';') {
                    if let Some(at_pos) = uri_bytes.iter().position(|&c| c == b'@') {
                        // Try to parse just the host part
                        match hostport(&uri_bytes[at_pos+1..semi_pos]) {
                            Ok((rem, (host, port))) => println!("Hostport parsed successfully: {:?}, {:?}", host, port),
                            Err(e) => println!("Hostport parsing failed: {:?}", e),
                        }
                        
                        // Try to parse just the parameters part
                        match uri_parameters(&uri_bytes[semi_pos..]) {
                            Ok((rem, params)) => println!("Parameters parsed successfully: {:?}", params),
                            Err(e) => println!("Parameters parsing failed: {:?}", e),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_parse_simple_sip_uri() {
        let uri_bytes = b"sip:user@example.com";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("user".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
    }
    
    #[test]
    fn test_parse_sips_uri_with_port() {
        let uri_bytes = b"sips:alice@atlanta.com:5061";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sips);
        assert_eq!(uri.user, Some("alice".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "atlanta.com"));
        assert_eq!(uri.port, Some(5061));
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
    }

    #[test]
    fn test_parse_sip_uri_ipv4() {
        let uri_bytes = b"sip:192.168.0.1:8080";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, None);
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Address(addr) if addr == IpAddr::from(Ipv4Addr::new(192, 168, 0, 1))));
        assert_eq!(uri.port, Some(8080));
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
    }

     #[test]
    fn test_parse_sip_uri_with_params() {
        let uri_bytes = b"sip:user@example.com;transport=tcp;lr";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("user".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert_eq!(uri.parameters.len(), 2);
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(uri.headers.is_empty());
    }

     #[test]
    fn test_parse_sip_uri_with_headers() {
        let uri_bytes = b"sip:user@example.com?Subject=Urgent&Priority=High";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("user".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert!(uri.parameters.is_empty());
        assert!(!uri.headers.is_empty());
        let headers = uri.headers;
        assert_eq!(headers.get("Subject"), Some(&"Urgent".to_string()));
        assert_eq!(headers.get("Priority"), Some(&"High".to_string()));
    }
    
    #[test]
    fn test_parse_sip_uri_complex() {
         let uri_bytes = b"sips:bob:password@[fe80::1]:5061;transport=tls;maddr=192.0.2.1?Subject=Hello";
         let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
         assert!(rem.is_empty());
         assert_eq!(uri.scheme, Scheme::Sips);
         assert_eq!(uri.user, Some("bob".to_string()));
         assert_eq!(uri.password, Some("password".to_string()));
         assert!(matches!(uri.host, Host::Address(_)));
         assert_eq!(uri.port, Some(5061));
         assert_eq!(uri.parameters.len(), 2);
         assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tls")));
         assert!(uri.parameters.iter().any(|p| matches!(p, Param::Maddr(s) if s == "192.0.2.1")));
         assert!(!uri.headers.is_empty());
         let headers = uri.headers;
         assert_eq!(headers.get("Subject"), Some(&"Hello".to_string()));
    }

    #[test]
    fn test_invalid_uri_scheme() {
        let uri_bytes = b"http:user@example.com";
        let result = parse_uri(uri_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_uri_no_host() {
        let uri_bytes = b"sip:";
        let result = parse_uri(uri_bytes);
        assert!(result.is_err());
    }

    // === RFC 3261 and ABNF Compliance Tests ===
    
    #[test]
    fn test_rfc3261_example_uris() {
        // Examples directly from RFC 3261 Section 19.1.1
        let examples = [
            b"sip:alice@atlanta.com".as_ref(),
            b"sip:alice:secretword@atlanta.com;transport=tcp".as_ref(),
            b"sips:alice@atlanta.com?subject=project%20x&priority=urgent".as_ref(),
            b"sip:+1-212-555-1212:1234@gateway.com;user=phone".as_ref(),
            b"sips:1212@gateway.com".as_ref(),
            b"sip:alice@192.0.2.4".as_ref(),
            b"sip:atlanta.com;method=REGISTER?to=alice%40atlanta.com".as_ref(),
            b"sip:alice;day=tuesday@atlanta.com".as_ref()
        ];
        
        for example in examples {
            let result = parse_uri(example);
            assert!(result.is_ok(), "Failed to parse RFC 3261 example: {}", String::from_utf8_lossy(example));
            let (rem, _) = result.unwrap();
            assert!(rem.is_empty(), "Remaining bytes should be empty for: {}", String::from_utf8_lossy(example));
        }
    }
    
    #[test]
    fn test_uri_with_all_components() {
        // URI with all possible components as specified in RFC 3261
        // sip:user:password@host:port;uri-parameters?headers
        let uri_bytes = b"sips:alice:password@example.com:5061;transport=tls;method=INVITE;ttl=60;lr;maddr=192.168.1.1;custom=value?subject=Meeting&priority=high&custom=value";
        let (rem, uri) = parse_uri(uri_bytes).expect("Failed to parse complete URI");
        
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sips);
        assert_eq!(uri.user, Some("alice".to_string()));
        assert_eq!(uri.password, Some("password".to_string()));
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, Some(5061));
        
        // Verify all parameters are present
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tls")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Method(s) if s == "INVITE")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Ttl(val) if *val == 60)));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Maddr(s) if s == "192.168.1.1")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" && v == "value")));
        
        // Verify headers
        assert_eq!(uri.headers.get("subject"), Some(&"Meeting".to_string()));
        assert_eq!(uri.headers.get("priority"), Some(&"high".to_string()));
        assert_eq!(uri.headers.get("custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_uri_character_escaping() {
        // Test URI with escaped characters in various components
        // RFC 3261 Section 19.1.2 specifies character escaping rules
        
        // Test user and password with escaped characters
        let uri_bytes = b"sip:alice%20smith:pass%40word@example.com";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with escaped user/password");
        assert_eq!(uri.user, Some("alice smith".to_string()));
        assert_eq!(uri.password, Some("pass@word".to_string()));
        
        // Test parameters with escaped characters
        let uri_bytes = b"sip:alice@example.com;custom=value%20with%20spaces";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with escaped params");
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" && v == "value with spaces")));
        
        // Test headers with escaped characters
        let uri_bytes = b"sip:alice@example.com?subject=urgent%20meeting&location=conference%20room";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with escaped headers");
        assert_eq!(uri.headers.get("subject"), Some(&"urgent meeting".to_string()));
        assert_eq!(uri.headers.get("location"), Some(&"conference room".to_string()));
    }

    #[test]
    fn test_uri_with_ipv6_host() {
        // RFC 3261 supports IPv6 references in the host part
        let uri_bytes = b"sip:alice@[2001:db8::1]:5060";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with IPv6 host");
        assert!(matches!(uri.host, Host::Address(addr) if addr.is_ipv6()));
        assert_eq!(uri.port, Some(5060));
        
        // With parameters and headers
        let uri_bytes = b"sips:bob@[2001:db8::1]:5061;transport=tls?subject=meeting";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse complex URI with IPv6 host");
        assert!(matches!(uri.host, Host::Address(addr) if addr.is_ipv6()));
        assert_eq!(uri.port, Some(5061));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tls")));
        assert_eq!(uri.headers.get("subject"), Some(&"meeting".to_string()));
    }

    #[test]
    fn test_uri_parameter_combinations() {
        // Test different parameter combinations according to RFC 3261
        
        // Multiple standard parameters
        let uri_bytes = b"sip:alice@example.com;transport=tcp;user=phone;method=INVITE;ttl=120;lr";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with multiple params");
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "phone")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Method(s) if s == "INVITE")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Ttl(val) if *val == 120)));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        
        // Mix of standard and custom parameters
        let uri_bytes = b"sip:alice@example.com;transport=tcp;custom1=value1;lr;custom2=value2";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with mixed params");
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom1" && v == "value1")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom2" && v == "value2")));
        
        // Parameters without values
        let uri_bytes = b"sip:alice@example.com;lr;custom;transport=tcp";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with valueless params");
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Other(n, None) if n == "custom")));
    }

    #[test]
    fn test_sip_tel_subscriber_syntax() {
        // Test TEL subscriber syntax in SIP URIs (RFC 3261 Section 19.1.6)
        
        // Global phone number
        let uri_bytes = b"sip:+1-212-555-1212@gateway.com;user=phone";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with global phone number");
        assert_eq!(uri.user, Some("+1-212-555-1212".to_string()));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "phone")));
        
        // Local phone number
        let uri_bytes = b"sip:555-1212@gateway.com;user=phone";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with local phone number");
        assert_eq!(uri.user, Some("555-1212".to_string()));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "phone")));
        
        // Phone number with visual-separators
        let uri_bytes = b"sip:+1(212)555-1212@gateway.com;user=phone";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with phone visual separators");
        assert_eq!(uri.user, Some("+1(212)555-1212".to_string()));
    }

    // Now add the new comprehensive RFC 3261 compliance tests
    #[test]
    fn test_rfc3261_sip_uri_examples() {
        // Examples from RFC 3261 Section 19.1.1
        let uri = "sip:alice@atlanta.com";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "atlanta.com"));

        let uri = "sips:alice@atlanta.com";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sips);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "atlanta.com"));

        let uri = "sip:alice:secretword@atlanta.com;transport=tcp";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert_eq!(parsed.password.as_ref().unwrap(), "secretword");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "atlanta.com"));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));

        let uri = "sips:alice@atlanta.com?subject=project%20x&priority=urgent";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sips);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "atlanta.com"));
        assert_eq!(parsed.headers.get("subject"), Some(&"project x".to_string()));
        assert_eq!(parsed.headers.get("priority"), Some(&"urgent".to_string()));
    }

    #[test]
    fn test_sip_uri_with_ipv4() {
        let uri = "sip:alice@192.168.1.1";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Address(addr) if addr.to_string() == "192.168.1.1"));
    }

    #[test]
    fn test_sip_uri_with_ipv6() {
        let uri = "sip:alice@[2001:db8::1]";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Address(addr) if addr.is_ipv6()));
    }

    #[test]
    fn test_sip_uri_with_complex_parameters() {
        let uri = "sip:alice@example.com;transport=tcp;lr;method=INVITE;ttl=5;maddr=239.255.255.1;user=phone";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "example.com"));
        
        // Check parameters
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Method(s) if s == "INVITE")));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Ttl(val) if *val == 5)));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Maddr(s) if s == "239.255.255.1")));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "phone")));
    }

    #[test]
    fn test_sip_uri_with_complex_headers() {
        // Use a simpler version of the complex header test that doesn't involve as many special characters
        let uri = "sip:alice@example.com?Accept-Contact=personal&Call-Info=photo";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "example.com"));
        
        // Check headers
        assert_eq!(parsed.headers.get("Accept-Contact"), Some(&"personal".to_string()));
        assert_eq!(parsed.headers.get("Call-Info"), Some(&"photo".to_string()));
        
        // Test with a more complex but properly escaped URI
        let uri = "sip:alice@example.com?Subject=Meeting%20Time&Priority=Urgent";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.headers.get("Subject"), Some(&"Meeting Time".to_string()));
        assert_eq!(parsed.headers.get("Priority"), Some(&"Urgent".to_string()));
    }

    #[test]
    fn test_sip_uri_escaped_characters() {
        let uri = "sip:alice%20smith@example.com;transport=tcp";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice smith");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "example.com"));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));

        let uri = "sip:alice@example.com;param1=value%20with%20spaces";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sip);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "example.com"));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Other(n, Some(v)) if n == "param1" && matches!(v, GenericValue::Token(val) if val == "value with spaces"))));
    }

    #[test]
    fn test_full_featured_sip_uri() {
        let uri = "sips:alice:password@example.com:5061;transport=tls;user=phone;method=INVITE?subject=Meeting&priority=urgent&Call-Info=%3chttp://www.example.com/alice/photo.jpg%3e%3bpurpose%3dicon";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.scheme, Scheme::Sips);
        assert_eq!(parsed.user.as_ref().unwrap(), "alice");
        assert_eq!(parsed.password.as_ref().unwrap(), "password");
        assert!(matches!(parsed.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(parsed.port, Some(5061));
        
        // Check parameters
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tls")));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "phone")));
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::Method(s) if s == "INVITE")));
        
        // Check headers
        assert_eq!(parsed.headers.get("subject"), Some(&"Meeting".to_string()));
        assert_eq!(parsed.headers.get("priority"), Some(&"urgent".to_string()));
        assert_eq!(parsed.headers.get("Call-Info"), Some(&"<http://www.example.com/alice/photo.jpg>;purpose=icon".to_string()));
    }

    #[test]
    fn test_user_formats() {
        // Telephone subscriber with visual separators
        let uri = "sip:+1-212-555-0101@example.com;user=phone";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.user.as_ref().unwrap(), "+1-212-555-0101");
        assert!(parsed.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "phone")));

        // User with special characters
        let uri = "sip:alice+weekend@example.com";
        let parsed = parse_uri(uri.as_bytes()).unwrap().1;
        assert_eq!(parsed.user.as_ref().unwrap(), "alice+weekend");
    }

    #[test]
    fn test_uri_case_sensitivity() {
        // Test case sensitivity requirements of RFC 3261
        
        // Scheme is case-insensitive
        let uri_bytes = b"SiP:alice@example.com";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with uppercase scheme");
        assert_eq!(uri.scheme, Scheme::Sip);
        
        let uri_bytes = b"sIpS:alice@example.com";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with mixed case scheme");
        assert_eq!(uri.scheme, Scheme::Sips);
        
        // Parameter names are case-insensitive but values are case-sensitive
        let uri_bytes = b"sip:alice@example.com;TrAnSpOrT=TcP;User=PhOnE";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse URI with mixed case params");
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "TcP")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::User(s) if s == "PhOnE")));
    }
    
    #[test]
    fn test_invalid_uri_formats() {
        // Test various invalid URI formats that should be rejected
        
        // Missing host
        assert!(parse_uri(b"sip:").is_err());
        assert!(parse_uri(b"sip:@").is_err());
        
        // Invalid scheme
        assert!(parse_uri(b"invalid:alice@example.com").is_err());
        
        // Invalid IPv6 reference
        assert!(parse_uri(b"sip:alice@[1:2:3:4:5:6:7]").is_err()); // Malformed IPv6
        assert!(parse_uri(b"sip:alice@[1:2:3:4:5:6:7:8").is_err()); // Missing closing bracket
        
        // Note: Our parser is more lenient with some inputs than the strict RFC
        // interpretation would suggest. While the RFC might consider these invalid:
        
        // - Non-numeric port: sip:alice@example.com:abc
        // - Port exceeds u16 range: sip:alice@example.com:65536
        // - Param with no name: sip:alice@example.com;=value
        // - Header with no name: sip:alice@example.com?=value
        
        // The current implementation handles these cases differently, which may
        // be acceptable for real-world use to increase interoperability.
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_port_non_numeric() {
        // 1. Non-numeric port (should be rejected)
        let result = parse_uri(b"sip:alice@example.com:abc");
        assert!(result.is_err(), "Parser should reject non-numeric port values");
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_port_overflow() {
        // 2. Port exceeds u16 range (should be rejected)
        let result = parse_uri(b"sip:alice@example.com:65536");
        assert!(result.is_err(), "Parser should reject port values > 65535");
        
        let result = parse_uri(b"sip:alice@example.com:999999");
        assert!(result.is_err(), "Parser should reject large port values");
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_param_no_name() {
        // 3. Parameter with no name (should be rejected)
        let result = parse_uri(b"sip:alice@example.com;=value");
        assert!(result.is_err(), "Parser should reject parameters with no name");
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_header_no_name() {
        // 4. Header with no name (should be rejected)
        let result = parse_uri(b"sip:alice@example.com?=value");
        assert!(result.is_err(), "Parser should reject headers with no name");
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_userinfo_multiple_colons() {
        // 5. Multiple colons in userinfo part (only one allowed for user:password)
        let result = parse_uri(b"sip:alice:pass:word@example.com");
        assert!(result.is_err(), "Parser should reject multiple colons in userinfo");
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_host_invalid_chars() {
        // 6. Invalid characters in host
        let result = parse_uri(b"sip:alice@example_com");
        assert!(result.is_err(), "Parser should reject underscores in host");
    }
    
    #[test]
    fn test_strict_rfc_compliance_edge_cases_host_leading_period() {
        // 7. Leading period in domain
        let result = parse_uri(b"sip:alice@.example.com");
        assert!(result.is_err(), "Parser should reject leading periods in domain");
    }
    
    #[test]
    fn test_rfc4475_torture_test_cases() {
        // Test cases from RFC 4475 SIP Torture Tests
        
        // Escaped headers (Section 3.1.1.1)
        let uri_bytes = b"sip:sip%3Aannc%40biloxi.com@atlanta.com";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse RFC 4475 escaped headers");
        assert_eq!(uri.user, Some("sip:annc@biloxi.com".to_string()));
        
        // Escaped semicolons in URI parameters (Section 3.1.1.13)
        let uri_bytes = b"sip:alice@atlanta.com;param=abc%3bdef";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse RFC 4475 escaped semicolons");
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "param" && v == "abc;def")));
    }

    #[test]
    fn test_parse_tel_uri() {
        // Test parsing a simple TEL URI
        let uri_bytes = b"tel:+1-212-555-1234";
        match parse_tel_uri(uri_bytes) {
            Ok((rem, uri)) => {
                assert!(rem.is_empty());
                assert_eq!(uri.scheme, Scheme::Tel);
                assert_eq!(uri.user, None);
                assert!(matches!(uri.host, Host::Domain(d) if d == "+1-212-555-1234"));
                assert!(uri.parameters.is_empty());
            },
            Err(e) => {
                panic!("Failed to parse TEL URI: {:?}", e);
            }
        }

        // Test with parameters
        let uri_bytes = b"tel:+1-212-555-1234;phone-context=nyc.example.com";
        match parse_tel_uri(uri_bytes) {
            Ok((rem, uri)) => {
                assert!(rem.is_empty());
                assert_eq!(uri.scheme, Scheme::Tel);
                assert!(matches!(uri.host, Host::Domain(d) if d == "+1-212-555-1234"));
                assert_eq!(uri.parameters.len(), 1);
                assert!(uri.parameters.iter().any(|p| 
                    matches!(p, Param::Other(k, Some(GenericValue::Token(v))) 
                        if k == "phone-context" && v == "nyc.example.com")
                ));
            },
            Err(e) => {
                panic!("Failed to parse TEL URI with parameters: {:?}", e);
            }
        }

        // Test via the public parse_uri function
        let uri_bytes = b"tel:+1-212-555-1234";
        let (_, uri) = parse_uri(uri_bytes).expect("Failed to parse TEL URI");
        assert_eq!(uri.scheme, Scheme::Tel);
        assert!(matches!(uri.host, Host::Domain(d) if d == "+1-212-555-1234"));
    }
} 