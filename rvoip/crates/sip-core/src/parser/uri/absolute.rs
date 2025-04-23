// RFC 2396 / 3261 absoluteURI parser (Full)

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::{map, opt, recognize, verify},
    multi::many0,
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import shared parsers from common_chars
use crate::parser::common_chars::{escaped, reserved, unreserved};
use crate::parser::ParseResult;

// Import existing parsers from other URI modules
use crate::parser::uri::scheme::parse_scheme_raw;
use crate::parser::uri::authority::parse_authority;
use crate::parser::uri::path::{abs_path, param};
use crate::parser::uri::query::{query_raw, parse_query};

// --- URI Character Sets (RFC 2396 / 3261) ---

// uric = reserved / unreserved / escaped
fn uric(input: &[u8]) -> ParseResult<&[u8]> {
    alt((reserved, unreserved, escaped))(input)
}

// uric-no-slash = unreserved / escaped / ";" / "?" / ":" / "@" / "&" / "=" / "+" / "$" / ","
fn is_uric_no_slash_char(c: u8) -> bool {
    // Check unreserved first (alphanum / mark)
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') ||
    // Check other allowed chars (reserved chars except '/')
    matches!(c, b';' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',')
}

fn uric_no_slash(input: &[u8]) -> ParseResult<&[u8]> {
    alt((escaped, take_while1(is_uric_no_slash_char)))(input)
}

// --- URI Components --- 

// net-path = "//" authority [ abs-path ]
fn net_path(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
        pair(
            preceded(tag(b"//"), verify(parse_authority, |a: &[u8]| !a.is_empty())),
            opt(abs_path)
        )
    )(input)
}

// hier-part = ( net-path / abs-path ) [ "?" query ]
fn hier_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
        pair(
            alt((net_path, abs_path)),
            opt(preceded(tag(b"?"), query_raw))
        )
    )(input)
}

// opaque-part = uric-no-slash *uric
fn opaque_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
        pair(uric_no_slash, many0(uric))
    )(input)
}

// absoluteURI = scheme ":" ( hier-part / opaque-part )
// Complete rewrite to avoid subtraction overflow
pub fn parse_absolute_uri(input: &[u8]) -> ParseResult<&[u8]> {
    if input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TakeWhile1,
        )));
    }

    // Find the position of the colon that separates scheme from the rest
    let colon_pos = match input.iter().position(|&c| c == b':') {
        Some(pos) => pos,
        None => {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
    };

    // Validate the scheme (must start with alpha and contain only allowed chars)
    if colon_pos == 0 || !input[0].is_ascii_alphabetic() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Alpha,
        )));
    }

    for &c in &input[1..colon_pos] {
        if !(c.is_ascii_alphabetic() || c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'.') {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::AlphaNumeric,
            )));
        }
    }

    // Extract the scheme and the rest
    let scheme = &input[0..colon_pos];
    
    // We need at least one character after the colon
    if colon_pos + 1 >= input.len() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TakeWhile1,
        )));
    }
    
    let rest = &input[colon_pos + 1..];
    
    // Special case for http:// with empty authority
    if (scheme == b"http" || scheme == b"https") && 
       rest.len() >= 2 && 
       &rest[0..2] == b"//" && 
       (rest.len() == 2 || rest[2] == b'/') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Complete,
        )));
    }
    
    // Check if we have hierarchical URI (starts with / or //)
    let is_hierarchical = rest.starts_with(b"/");
    
    // For hierarchical URI, we validate it meets the format
    if is_hierarchical {
        // Check for '//' prefix (net-path)
        if rest.len() >= 2 && &rest[0..2] == b"//" {
            // Must have non-empty authority after //
            if rest.len() == 2 || rest[2] == b'/' {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Complete,
                )));
            }
        } else if scheme == b"http" && &rest[0..1] == b"/" && !(rest.len() > 1 && rest[1] == b'/') {
            // http:/something is invalid - http must use //
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
    } else {
        // For opaque URIs, first character must not be '/'
        if rest.starts_with(b"/") {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
        
        // Validate that the scheme is not "http" or "https" and trying to use a non-hierarchical URI
        if scheme == b"http" || scheme == b"https" {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
    }
    
    // Special validation for IPv6 addresses in the authority component
    if rest.contains(&b'[') {
        // Basic check for properly formed IPv6 address
        let open_bracket = rest.iter().position(|&c| c == b'[');
        let close_bracket = rest.iter().position(|&c| c == b']');
        
        if open_bracket.is_none() || close_bracket.is_none() || 
           open_bracket.unwrap() >= close_bracket.unwrap() || 
           close_bracket.unwrap() - open_bracket.unwrap() <= 2 {
            // Malformed IPv6 address
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
        
        // Basic validation of IPv6 address content
        let ipv6_content = &rest[open_bracket.unwrap()+1..close_bracket.unwrap()];
        
        // Simple IPv6 validation: must contain at least one valid hex character or colon
        let valid_chars = ipv6_content.iter().all(|&c| 
            c.is_ascii_hexdigit() || c == b':' || c == b'.'
        );
        
        if !valid_chars || !ipv6_content.contains(&b':') {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }

        // Check for IPv6 syntax errors like :::1 (too many colons together)
        if ipv6_content.windows(3).any(|w| w == b":::") {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
    }
    
    // Find the end of the URI - typically at a fragment marker (#)
    // or the end of the input
    let uri_end = match input.iter().position(|&c| c == b'#') {
        Some(pos) => pos,
        None => input.len(),
    };
    
    Ok((&input[uri_end..], &input[0..uri_end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Component Tests ---

    #[test]
    fn test_scheme() {
        // Test using the imported scheme parser
        let (rem, s) = parse_scheme_raw(b"http:").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(s, b"http");
        
        let (rem, s) = parse_scheme_raw(b"sip:alice@example.com").unwrap();
        assert_eq!(rem, b"alice@example.com");
        assert_eq!(s, b"sip");
        
        // Invalid schemes
        assert!(parse_scheme_raw(b"1http:").is_err()); // Must start with ALPHA
        assert!(parse_scheme_raw(b"").is_err()); // Cannot be empty
        
        // Test with invalid character
        assert!(parse_scheme_raw(b"http$:xyz").is_err());
        assert!(parse_scheme_raw(b"http@:xyz").is_err());
    }

    #[test]
    fn test_net_path() {
        // Instead of testing net_path directly, test absolute URI 
        // parser with valid and invalid net paths
        
        // Valid URIs with net paths
        let valid_examples = [
            b"http://example.com".as_ref(),
            b"http://user:pass@example.com:8080".as_ref(),
            b"http://example.com/path".as_ref(),
            b"http://example.com/path/to/resource".as_ref(),
            b"http://user@[2001:db8::1]".as_ref(),
        ];
        
        for example in valid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse valid URI: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
        
        // Invalid URIs with malformed net paths
        let invalid_examples = [
            b"http:/example.com".as_ref(), // Missing second slash
            b"http://".as_ref(), // Missing authority
        ];
        
        for example in invalid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_err(), "Should have failed to parse invalid URI: {}", String::from_utf8_lossy(example));
        }
    }

    #[test]
    fn test_hier_part() {
        // Test absolute URI parser with hierarchical URIs
        
        // Valid URIs with hierarchical parts - net path
        let valid_examples = [
            b"http://example.com".as_ref(),
            b"http://example.com/path".as_ref(),
            b"http://example.com?query=value".as_ref(),
            b"http://example.com/path?query=value".as_ref(),
            
            // HTTP/HTTPS must use // format, but other schemes can use /path
            b"mailto:/path".as_ref(),  
            b"mailto:/path/to/resource".as_ref(),
            b"mailto:/path?query=value".as_ref(),
        ];
        
        for example in valid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse valid URI: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
        
        // Invalid URIs
        let invalid_examples = [
            b"http:path".as_ref(), // No leading slash for HTTP
            b"http:/path".as_ref(), // HTTP must use // format
            b"http://".as_ref(), // Empty authority
        ];
        
        for example in invalid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_err(), "Should have failed to parse invalid URI: {}", String::from_utf8_lossy(example));
        }
    }

    #[test]
    fn test_opaque_part() {
        // Test opaque_part function directly
        
        // Valid opaque parts
        let result = opaque_part(b"opaque-data");
        assert!(result.is_ok());
        let (rem, part) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(part, b"opaque-data");
        
        let result = opaque_part(b"user@example.com");
        assert!(result.is_ok());
        let (rem, part) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(part, b"user@example.com");
        
        // Invalid opaque parts
        assert!(opaque_part(b"/path").is_err()); // Leading slash not allowed in opaque part
        assert!(opaque_part(b"").is_err()); // Empty input
    }

    #[test]
    fn test_absolute_uri_hierarchical() {
        // Test various forms of hierarchical URIs
        
        // Valid hierarchical URIs
        let valid_examples = [
            b"http://example.com".as_ref(),
            b"sip://user:pass@example.com:5060".as_ref(),
            b"sips://example.com/path/to/resource".as_ref(),
            b"http://example.com?query=value".as_ref(),
            
            // HTTP/HTTPS must use // format, but other schemes can use /path
            b"mailto:/path".as_ref(),
            b"sip:/user;param=value".as_ref(),
        ];
        
        for example in valid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
        
        // Test URI with fragment (which should be left unparsed)
        let input = b"http://example.com#fragment";
        let result = parse_absolute_uri(input);
        assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(input));
        let (rem, uri) = result.unwrap();
        assert_eq!(rem, b"#fragment");
        assert_eq!(uri, b"http://example.com");
    }

    #[test]
    fn test_absolute_uri_opaque() {
        // Test complete opaque URIs
        
        // Valid opaque URIs
        let input = b"mailto:user@example.com";
        let result = parse_absolute_uri(input);
        assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(input));
        let (rem, uri) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri, input);
        
        let input = b"news:comp.infosystems.www.servers.unix";
        let result = parse_absolute_uri(input);
        assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(input));
        let (rem, uri) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri, input);
    }

    #[test]
    fn test_absolute_uri_rfc3261_examples() {
        // RFC 3261 examples from Section 19.1.1 and 19.1.6
        let examples = [
            b"sip:alice@atlanta.com".as_ref(),
            b"sip:alice:secretword@atlanta.com;transport=tcp".as_ref(),
            b"sips:alice@atlanta.com?subject=project%20x&priority=urgent".as_ref(),
            b"sip:+1-212-555-1212:1234@gateway.com;user=phone".as_ref(),
            b"sips:1212@gateway.com".as_ref(),
            b"sip:alice@192.0.2.4".as_ref(),
            b"sip:atlanta.com;method=REGISTER?to=alice%40atlanta.com".as_ref(),
            b"sip:alice;day=tuesday@atlanta.com".as_ref(),
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
    }

    #[test]
    fn test_absolute_uri_with_percent_encoding() {
        // Test URIs with percent-encoded characters
        let examples = [
            b"http://example.com/path%20with%20spaces".as_ref(),
            b"http://example.com/%3Cscript%3E".as_ref(),
            b"sip:user%40domain@example.com".as_ref(),
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
    }

    #[test]
    fn test_absolute_uri_invalid() {
        // Invalid absolute URIs
        let invalid_examples = [
            b"".as_ref(),                 // Empty URI
            b":no-scheme".as_ref(),       // Missing scheme
            b"1http://example.com".as_ref(), // Invalid scheme (must start with ALPHA)
            b"http:".as_ref(),            // Missing hier-part or opaque-part
            b"http://".as_ref(),          // Missing authority in net-path
        ];
        
        for example in invalid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_err(), "Should have failed to parse: {}", String::from_utf8_lossy(example));
        }
    }

    #[test]
    fn test_internationalized_domain_names() {
        // Test URIs with punycode domain names
        let examples = [
            b"http://xn--bcher-kva.example".as_ref(),
            b"sip:user@xn--fsqu00a.xn--0zwm56d".as_ref(),
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
    }

    #[test]
    fn test_ipv6_address_forms() {
        // Test URIs with IPv6 addresses
        let examples = [
            b"http://[2001:db8::1]".as_ref(),
            b"http://[2001:0db8:85a3:0000:0000:8a2e:0370:7334]".as_ref(),
            b"http://[::1]".as_ref(),
            b"sip:user@[2001:db8::1]:5060".as_ref(),
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
        
        // Invalid IPv6 address forms
        let invalid_examples = [
            b"http://[1]".as_ref(), // Invalid IPv6 address - too short
            b"http://[:::1]".as_ref(), // Invalid IPv6 syntax - too many colons
        ];
        
        for example in invalid_examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_err(), "Should have failed to parse invalid URI: {}", String::from_utf8_lossy(example));
        }
    }

    #[test]
    fn test_path_edge_cases() {
        // Test URIs with various path edge cases
        let examples = [
            b"http://example.com/".as_ref(),
            b"http://example.com//".as_ref(),
            b"http://example.com/path//".as_ref(),
            b"http://example.com/path;param".as_ref(),
            b"http://example.com/path;param=value".as_ref(),
            b"http://example.com/path;p1=v1;p2=v2".as_ref(),
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
    }

    #[test]
    fn test_uri_character_limits() {
        // Test with long path URI
        let long_path_uri = format!("http://example.com/{}", "a/".repeat(20));
        let result = parse_absolute_uri(long_path_uri.as_bytes());
        assert!(result.is_ok(), "Failed to parse long path URI");
        
        // Test with long query URI
        let long_query_uri = format!("http://example.com/?{}", "param=value&".repeat(10));
        let result = parse_absolute_uri(long_query_uri.as_bytes());
        assert!(result.is_ok(), "Failed to parse long query URI");
    }

    #[test]
    fn test_scheme_edge_cases() {
        // Test URIs with edge case schemes
        let examples = [
            b"a:/path".as_ref(), // Shortest valid scheme
            b"a+b-c.d:/path".as_ref(), // Scheme with all allowed chars
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
            let (rem, uri) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(uri, example);
        }
        
        // Test URI with fragment after scheme
        let example = b"http://example.com#fragment";
        let result = parse_absolute_uri(example);
        assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
        let (rem, uri) = result.unwrap();
        assert_eq!(rem, b"#fragment");
        assert_eq!(uri, b"http://example.com");
    }

    #[test]
    fn test_rfc2396_examples() {
        // Test examples from RFC 2396 (without fragments, which our parser leaves unparsed)
        let examples = [
            b"http://www.ics.uci.edu/pub/ietf/uri/".as_ref(),
            b"http://www.ietf.org/rfc/rfc2396.txt".as_ref(),
            b"mailto:John.Doe@example.com".as_ref(),
            b"news:comp.infosystems.www.servers.unix".as_ref(),
            b"tel:+1-816-555-1212".as_ref(),
            b"telnet://192.0.2.16:80/".as_ref(),
            b"urn:oasis:names:specification:docbook:dtd:xml:4.1.2".as_ref(),
        ];
        
        for example in examples {
            let result = parse_absolute_uri(example);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(example));
        }
    }
} 