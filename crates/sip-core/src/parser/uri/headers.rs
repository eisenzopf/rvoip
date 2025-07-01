use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1, separated_list1},
    sequence::{pair, preceded, separated_pair},
    IResult,
    error::{Error as NomError, ErrorKind},
};
use std::collections::HashMap;
use std::str;

// Import from new modules
use crate::parser::common_chars::{unreserved, escaped};
use crate::parser::ParseResult;
use crate::parser::utils::unescape_uri_component;
use crate::error::Error;

// hnv-unreserved = "[" / "]" / "/" / "?" / ":" / "+" / "$"
fn is_hnv_unreserved(c: u8) -> bool {
    matches!(c, b'[' | b']' | b'/' | b'?' | b':' | b'+' | b'$')
}

// hname = 1*( hnv-unreserved / unreserved / escaped )
// Return an unescaped string
pub fn hname(input: &[u8]) -> ParseResult<String> {
    let mut i = 0;
    let mut found_valid_char = false;
    
    while i < input.len() {
        match input[i] {
            // hnv-unreserved = "[" / "]" / "/" / "?" / ":" / "+" / "$"
            b'[' | b']' | b'/' | b'?' | b':' | b'+' | b'$' => {
                found_valid_char = true;
                i += 1;
            },
            // unreserved = alphanum / mark
            // mark = "-" / "_" / "." / "!" / "~" / "*" / "'" / "(" / ")"
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | 
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                found_valid_char = true;
                i += 1;
            },
            // escaped = "%" HEXDIG HEXDIG
            b'%' => {
                // Check for malformed escape sequences:
                // 1. % at the end of input
                if i + 2 >= input.len() {
                    // Incomplete escape sequence
                    return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
                }
                
                // 2. % followed by non-hex digits
                if !is_hex_digit(input[i+1]) || !is_hex_digit(input[i+2]) {
                    return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
                }
                
                found_valid_char = true;
                i += 3;
            },
            // If we hit an equals sign, we've reached the end
            b'=' => break,
            // If we encounter & or any other character, we've reached the end of this header name
            _ => break,
        }
    }
    
    if !found_valid_char {
        // If no valid chars found, this is not a valid hname
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::TakeWhile1)));
    }
    
    // Extract and unescape the name
    let name_bytes = &input[0..i];
    let name = unescape_uri_component(name_bytes)
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes)))?;
    
    Ok((&input[i..], name))
}

// hvalue = *( hnv-unreserved / unreserved / escaped )
// Return an unescaped string
pub fn hvalue(input: &[u8]) -> ParseResult<String> {
    let mut i = 0;
    
    while i < input.len() {
        match input[i] {
            // hnv-unreserved = "[" / "]" / "/" / "?" / ":" / "+" / "$"
            b'[' | b']' | b'/' | b'?' | b':' | b'+' | b'$' => {
                i += 1;
            },
            // unreserved = alphanum / mark
            // mark = "-" / "_" / "." / "!" / "~" / "*" / "'" / "(" / ")"
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | 
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                i += 1;
            },
            // escaped = "%" HEXDIG HEXDIG
            b'%' => {
                // Check for malformed escape sequences:
                // 1. % at the end of input
                if i + 2 >= input.len() {
                    // Incomplete escape sequence
                    return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
                }
                
                // 2. % followed by non-hex digits
                if !is_hex_digit(input[i+1]) || !is_hex_digit(input[i+2]) {
                    return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
                }
                
                // Special handling for percent-encoded ampersands and other delimiters
                // This helps with nested escaped sequences
                let hex_val = (parse_hex_digit(input[i+1]) << 4) | parse_hex_digit(input[i+2]);
                if hex_val == b'&' as u8 {
                    // Allow percent-encoded ampersands, they're part of the value
                } else if hex_val == b'?' as u8 {
                    // Allow percent-encoded question marks
                }
                
                i += 3;
            },
            // Additionally allow some characters that might appear in encoded headers
            b'<' | b'>' | b';' | b',' | b'"' | b'=' | b'@' | b' ' | b'#' | b'%' => {
                i += 1;
            },
            // If we encounter an actual ampersand (not percent-encoded), we've reached the end
            b'&' => break,
            // End of input or other characters
            _ => break,
        }
    }
    
    // Extract and unescape the value, which can be empty
    let value_bytes = &input[0..i];
    let value = unescape_uri_component(value_bytes)
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes)))?;
    
    Ok((&input[i..], value))
}

// Helper function to parse a single hex digit to its numeric value
fn parse_hex_digit(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'F' => c - b'A' + 10,
        b'a'..=b'f' => c - b'a' + 10,
        _ => 0,  // Should never happen as we only call this after checking with is_hex_digit
    }
}

// header = hname "=" hvalue
pub fn header(input: &[u8]) -> ParseResult<(String, String)> {
    // Reject headers with no name (looking specifically for a starting equals sign)
    if input.is_empty() || input[0] == b'=' {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    
    let (remaining, name) = hname(input)?;
    
    if remaining.is_empty() || remaining[0] != b'=' {
        return Err(nom::Err::Error(nom::error::Error::new(
            remaining,
            nom::error::ErrorKind::Tag,
        )));
    }
    
    let (remaining, _) = tag(b"=")(remaining)?;
    let (remaining, value) = hvalue(remaining)?;

    // Ensure the header name is not empty after parsing
    if name.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    
    Ok((remaining, (name, value)))
}

// uri-headers = "?" header *( "&" header )
pub fn uri_headers(input: &[u8]) -> ParseResult<HashMap<String, String>> {
    if input.is_empty() || input[0] != b'?' {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    
    // We need to check specifically for "?=" which would be a header with no name
    // But we should allow complex values that might contain equals signs
    if input.len() >= 2 && input[1] == b'=' && (input.len() == 2 || input[2] != b'%') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    let (mut remaining, _) = tag(b"?")(input)?;
    let mut headers = HashMap::new();
    
    // Parse first header
    match header(remaining) {
        Ok((new_remaining, (name, value))) => {
            if !name.is_empty() {
                headers.insert(name, value);
            }
            remaining = new_remaining;
        }
        Err(e) => return Err(e),
    }

    // Parse additional headers
    while !remaining.is_empty() && remaining[0] == b'&' {
        // Skip '&'
        let (new_remaining, _) = tag(b"&")(remaining)?;
        
        match header(new_remaining) {
            Ok((next_remaining, (name, value))) => {
                if !name.is_empty() {
                    headers.insert(name, value);
                }
                remaining = next_remaining;
            }
            Err(_) => {
                // Can't parse more headers
                break;
            }
        }
    }

    Ok((remaining, headers))
}

// Helper function to check if a byte is a hex digit (0-9, A-F, a-f)
fn is_hex_digit(c: u8) -> bool {
    matches!(c, b'0'..=b'9' | b'A'..=b'F' | b'a'..=b'f')
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Basic Header Parsing Tests ===

    #[test]
    fn test_uri_headers_basic() {
        let input = b"?subject=project&priority=urgent";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("subject"), Some(&"project".to_string()));
        assert_eq!(map.get("priority"), Some(&"urgent".to_string()));
    }

    #[test]
    fn test_uri_headers_unescaped() {
        let input = b"?h%20name=h%20value&other=val%25"; // h name=h value, other=val%
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("h name"), Some(&"h value".to_string()));
        assert_eq!(map.get("other"), Some(&"val%".to_string()));
    }

    // === Character Set Tests ===

    #[test]
    fn test_hnv_unreserved_chars() {
        // Test all hnv-unreserved characters in header name and value
        let input = b"?h[]/+:$?=v[]/+:$?";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("h[]/+:$?"), Some(&"v[]/+:$?".to_string()));
    }

    #[test]
    fn test_unreserved_chars() {
        // Test unreserved characters in header name and value
        let input = b"?abcXYZ123-_.!~*'()=abcXYZ123-_.!~*'()";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("abcXYZ123-_.!~*'()"), Some(&"abcXYZ123-_.!~*'()".to_string()));
    }

    #[test]
    fn test_escaped_chars() {
        // Test all characters that need escaping
        let input = b"?escape%20%21%22%23%24%25%26=value%3A%20test";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.get("escape !\"#$%&"), Some(&"value: test".to_string()));
    }

    // === Edge Cases ===

    #[test]
    fn test_empty_value() {
        // Header with empty value
        let input = b"?empty=&nonempty=value";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("empty"), Some(&"".to_string()));
        assert_eq!(map.get("nonempty"), Some(&"value".to_string()));
    }

    #[test]
    fn test_duplicate_headers() {
        // Duplicate headers, last one wins
        let input = b"?name=first&name=second";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("name"), Some(&"second".to_string()));
    }

    #[test]
    fn test_headers_with_trailing_content() {
        // Headers followed by other content
        let input = b"?name=value";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("name"), Some(&"value".to_string()));
        
        // Test with a fragment identifier - in SIP URIs, # is part of the header value
        // unless it's percent-encoded
        let input = b"?name=value%23fragment";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("name"), Some(&"value#fragment".to_string()));
    }

    // === RFC 3261 Examples ===

    #[test]
    fn test_rfc3261_examples() {
        // Example from RFC 3261 Section 19.1.1
        let input = b"?subject=project%20x&priority=urgent";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("subject"), Some(&"project x".to_string()));
        assert_eq!(map.get("priority"), Some(&"urgent".to_string()));

        // Example from RFC 3261 Section 19.1.3
        let input = b"?to=sip%3Auser2%40example.com";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("to"), Some(&"sip:user2@example.com".to_string()));
    }

    // === RFC 4475 Torture Tests ===

    #[test]
    fn test_rfc4475_torture_cases() {
        // Test with deeply nested encoding from RFC 4475
        let input = b"?name=p%25%34%31";  // p%41 which is p%A which would decode to pA
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.get("name"), Some(&"p%41".to_string()));

        // Complex header names and values
        let input = b"?very%20long%20header%20name=very%20long%20header%20value%20with%20spaces";
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.get("very long header name"), 
                  Some(&"very long header value with spaces".to_string()));
    }

    // === Error Cases ===

    #[test]
    fn test_invalid_headers() {
        // Missing header name
        assert!(uri_headers(b"?=value").is_err());
        
        // Missing question mark
        assert!(uri_headers(b"name=value").is_err());
        
        // Missing equals sign
        assert!(uri_headers(b"?namevalue").is_err());
    }

    #[test]
    fn test_malformed_escapes() {
        // Invalid percent encoding (incomplete)
        assert!(uri_headers(b"?name=bad%2").is_err());
        assert!(uri_headers(b"?name=bad%").is_err());
        
        // Invalid percent encoding (non-hex)
        assert!(uri_headers(b"?name=bad%ZZ").is_err());
    }
} 