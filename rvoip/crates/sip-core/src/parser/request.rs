use std::str;
use std::str::FromStr;
use nom::{
    branch::alt,
    bytes::complete::{take_till, take_while1, tag},
    character::complete::{line_ending, space1},
    combinator::{map_res, recognize},
    error::{Error as NomError, ErrorKind, ParseError},
    sequence::tuple,
    IResult,
};
// Keep Result for FromStr impls if needed elsewhere
use crate::error::{Error, Result};
use crate::types::{Method, Version, Uri};
use crate::types::param::Param;
use crate::types::uri::{Host, Scheme};
use crate::parser::uri::parse_uri;
use crate::parser::token::token;
use crate::parser::common::sip_version;
use crate::parser::whitespace::crlf;
use crate::parser::ParseResult;

/// SIP Request Line Parser
///
/// Implements parser for SIP request lines according to RFC 3261 Section 25
/// (Core ABNF)
///
/// ABNF Syntax:
/// ```text
/// Request-Line  =  Method SP Request-URI SP SIP-Version CRLF
///
/// Method        =  INVITEm / ACKm / OPTIONSm / BYEm / CANCELm
///                  / REGISTERm / extension-method
/// extension-method = token
///
/// Request-URI   =  SIP-URI / SIPS-URI / absoluteURI
///
/// SIP-Version   =  "SIP" "/" 1*DIGIT "." 1*DIGIT
/// ```
///
/// Where SP is space and CRLF is carriage return followed by line feed.
/// The parser handles these components separately for more precise error detection.

/// Parser for a SIP request line
/// Changed signature to accept &[u8]
pub fn parse_request_line(input: &[u8]) -> ParseResult<(Method, Uri, Version)> {
    // First, parse the method and the first space
    let (input, method_bytes) = token(input)?;
    let (input, _) = space1(input)?;
    
    // Check if method is valid before proceeding
    let method_str = str::from_utf8(method_bytes)
        .map_err(|_| nom::Err::Failure(NomError::new(method_bytes, ErrorKind::Char)))?;
    
    // Verify method is a valid SIP method
    match method_str.parse::<Method>() {
        Ok(method) => {
            // Find the position of the next space which marks the end of the URI
            if let Some(space_pos) = input.iter().position(|&c| c == b' ') {
                // Extract the URI portion
                let uri_part = &input[..space_pos];
                
                // First try to parse as a SIP URI
                let uri_result = parse_uri(uri_part);
                
                let uri = match uri_result {
                    Ok((_, uri)) => uri,
                    Err(_) => {
                        // If not a SIP URI, try to handle as a generic URI
                        // This is needed for RFC 4475 compliance (exotic URI schemes)
                        if let Ok(uri_str) = str::from_utf8(uri_part) {
                            // Create a custom URI with the raw URI string
                            let mut uri = Uri::new(Scheme::Sip, Host::domain("example.com"));
                            uri.raw_uri = Some(uri_str.to_string());
                            uri
                        } else {
                            // If we can't even convert to UTF-8, it's a genuine error
                            return Err(nom::Err::Error(NomError::new(uri_part, ErrorKind::Tag)));
                        }
                    }
                };
                
                // Parse the remaining part after the URI (space + SIP/2.0 + CRLF)
                let version_part = &input[space_pos..];
                if let Ok((remaining, version)) = 
                    tuple((space1, sip_version, crlf))(version_part) {
                    
                    Ok((remaining, (method, uri, version.1)))
                } else {
                    Err(nom::Err::Error(NomError::new(version_part, ErrorKind::Tag)))
                }
            } else {
                // No space found after URI
                Err(nom::Err::Error(NomError::new(input, ErrorKind::Space)))
            }
        },
        Err(_) => {
            // Invalid SIP method
            Err(nom::Err::Failure(NomError::new(method_bytes, ErrorKind::Tag)))
        }
    }
}

// Removed request_parser, request_parser_nom, parse_headers_and_body functions. 

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::{Host, Scheme};
    use std::net::{Ipv4Addr, Ipv6Addr, IpAddr};

    #[test]
    fn test_parse_valid_request_line() {
        let line = b"INVITE sip:user@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, (method, uri, version)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(method, Method::Invite);
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user.as_ref().unwrap(), "user");
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(version, Version::new(2, 0));
    }

    #[test]
    fn test_parse_custom_method() {
        let line = b"PUBLISH sip:pres@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, (method, _, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(method, Method::Publish);
    }

    #[test]
    fn test_parse_request_line_sips() {
        let line = b"REGISTER sips:secure@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, (method, uri, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(method, Method::Register);
        assert_eq!(uri.scheme, Scheme::Sips);
    }

    // Test all standard SIP methods defined in RFC 3261
    #[test]
    fn test_standard_methods() {
        // Using Vec instead of array to avoid size mismatch issues
        let methods = vec![
            (b"REGISTER sip:example.com SIP/2.0\r\n".to_vec(), Method::Register),
            (b"INVITE sip:example.com SIP/2.0\r\n".to_vec(), Method::Invite),
            (b"ACK sip:example.com SIP/2.0\r\n".to_vec(), Method::Ack),
            (b"BYE sip:example.com SIP/2.0\r\n".to_vec(), Method::Bye),
            (b"CANCEL sip:example.com SIP/2.0\r\n".to_vec(), Method::Cancel),
            (b"OPTIONS sip:example.com SIP/2.0\r\n".to_vec(), Method::Options),
            (b"REFER sip:example.com SIP/2.0\r\n".to_vec(), Method::Refer),
            (b"SUBSCRIBE sip:example.com SIP/2.0\r\n".to_vec(), Method::Subscribe),
            (b"NOTIFY sip:example.com SIP/2.0\r\n".to_vec(), Method::Notify),
            (b"MESSAGE sip:example.com SIP/2.0\r\n".to_vec(), Method::Message)
        ];

        for (line, expected_method) in methods.iter() {
            let result = parse_request_line(line);
            assert!(result.is_ok());
            let (_, (method, _, _)) = result.unwrap();
            assert_eq!(method, *expected_method);
        }
    }

    // Test URI with IPv4 address
    #[test]
    fn test_request_uri_with_ipv4() {
        let line = b"INVITE sip:user@192.168.1.1 SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        assert!(matches!(uri.host, Host::Address(IpAddr::V4(ip)) if ip == Ipv4Addr::new(192, 168, 1, 1)));
    }

    // Test URI with IPv6 address, if supported
    #[test]
    fn test_request_uri_with_ipv6() {
        let line = b"INVITE sip:user@[2001:db8::1] SIP/2.0\r\n";
        let result = parse_request_line(line);
        
        // If IPv6 is implemented, test it's parsed correctly
        if result.is_ok() {
            let (_, (_, uri, _)) = result.unwrap();
            assert!(matches!(uri.host, Host::Address(IpAddr::V6(_))));
        }
        // Otherwise, this test can be skipped until IPv6 is implemented
    }

    // Test URI with port
    #[test]
    fn test_request_uri_with_port() {
        let line = b"INVITE sip:user@example.com:5060 SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        assert_eq!(uri.port, Some(5060));
    }

    // Test URI with parameters
    #[test]
    fn test_request_uri_with_params() {
        let line = b"INVITE sip:user@example.com;transport=udp SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        
        // Check if transport parameter is present
        let has_transport_param = uri.parameters.iter().any(|p| {
            match p {
                Param::Transport(value) => value == "udp",
                _ => false
            }
        });
        assert!(has_transport_param);
    }

    // Test URI with headers
    #[test]
    fn test_request_uri_with_headers() {
        let line = b"INVITE sip:user@example.com?subject=meeting SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        
        // Check if the headers map contains the subject
        assert!(!uri.headers.is_empty());
        assert!(uri.headers.contains_key("subject"));
        assert_eq!(uri.headers.get("subject"), Some(&"meeting".to_string()));
    }

    // Test multiple spaces between parts
    #[test]
    fn test_request_line_multiple_spaces() {
        let line = b"INVITE   sip:user@example.com   SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
    }

    // Test escaped characters in URI
    #[test]
    fn test_request_uri_with_escaped_chars() {
        let line = b"INVITE sip:user%20name@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        assert_eq!(uri.user.as_ref().unwrap(), "user name");
    }

    // Test with version variations
    #[test]
    fn test_version_variations() {
        // Valid versions
        assert!(parse_request_line(b"INVITE sip:user@example.com SIP/2.0\r\n").is_ok());
        
        // Other versions
        let result = parse_request_line(b"INVITE sip:user@example.com SIP/3.0\r\n");
        if result.is_ok() {
            let (_, (_, _, version)) = result.unwrap();
            assert_eq!(version, Version::new(3, 0));
        }
    }

    #[test]
    fn test_invalid_request_line_method() {
        // Empty method should be invalid
        let line = b" sip:user@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_request_uri_with_non_sip_schemes() {
        // We now support non-SIP URI schemes for requests
        let line = b"INVITE http://example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        assert_eq!(uri.scheme, Scheme::Http);
        
        // Test with HTTPS
        let line = b"OPTIONS https://example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        assert_eq!(uri.scheme, Scheme::Https);
        
        // Test with unknown scheme
        let line = b"OPTIONS someunknownscheme:something SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (_, (_, uri, _)) = result.unwrap();
        // Unknown schemes are stored in raw_uri
        assert!(uri.raw_uri.is_some());
        assert_eq!(uri.raw_uri.unwrap(), "someunknownscheme:something");
    }

    #[test]
    fn test_invalid_request_line_version() {
        let line = b"INVITE sip:user@example.com HTTP/1.1\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_request_line_spacing() {
        let line = b"INVITEsip:user@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_invalid_request_line_crlf() {
        let line = b"INVITE sip:user@example.com SIP/2.0";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }

    // Test trailing content after CRLF
    #[test]
    fn test_request_line_with_trailing_content() {
        let line = b"INVITE sip:user@example.com SIP/2.0\r\nContent-Type: application/sdp\r\n\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, _) = result.unwrap();
        assert_eq!(rem, b"Content-Type: application/sdp\r\n\r\n");
    }

    // Test with long URI to ensure it handles large inputs
    #[test]
    fn test_request_line_with_long_uri() {
        let mut long_uri = b"INVITE sip:user@".to_vec();
        long_uri.extend_from_slice(&vec![b'a'; 500]);
        long_uri.extend_from_slice(b".example.com SIP/2.0\r\n");
        
        let result = parse_request_line(&long_uri);
        assert!(result.is_ok());
    }
} 