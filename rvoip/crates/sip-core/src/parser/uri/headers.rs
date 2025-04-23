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
// Returns raw bytes, unescaping happens in uri_headers
fn hname(input: &[u8]) -> ParseResult<&[u8]> {
    if input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::TakeWhile1)));
    }

    // First verify there are no incomplete escape sequences
    for i in 0..input.len() {
        if input[i] == b'%' {
            // Require at least 2 more bytes
            if i + 2 >= input.len() {
                return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
            }
            // Require both to be hex digits
            if !is_hex_digit(input[i+1]) || !is_hex_digit(input[i+2]) {
                return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
            }
        }
    }

    // Now proceed with parsing
    let mut position = 0;
    let mut found_match = false;

    while position < input.len() {
        // Try each alternation
        if let Ok((remainder, _)) = take_while1::<_, &[u8], nom::error::Error<&[u8]>>(is_hnv_unreserved)(&input[position..]) {
            position = input.len() - remainder.len();
            found_match = true;
            continue;
        }
        
        if let Ok((remainder, _)) = unreserved(&input[position..]) {
            position = input.len() - remainder.len();
            found_match = true;
            continue;
        }
        
        if let Ok((remainder, _)) = escaped(&input[position..]) {
            position = input.len() - remainder.len();
            found_match = true;
            continue;
        }
        
        // If we get here, no alternation matched
        break;
    }
    
    if position == 0 && !found_match {
        // Nothing matched
        Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Alt)))
    } else {
        // Return the consumed part
        Ok((&input[position..], &input[0..position]))
    }
}

// hvalue = *( hnv-unreserved / unreserved / escaped )
// Returns raw bytes, unescaping happens in uri_headers
fn hvalue(input: &[u8]) -> ParseResult<&[u8]> {
    // Allow empty values - important for headers like "?empty=&next=value"
    if input.is_empty() || input[0] == b'&' || input[0] == b'#' {
        return Ok((input, b""));
    }

    // Check for malformed percent encodings first
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            // Check we have at least 2 more characters for the hex digits
            if i + 2 >= input.len() {
                return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
            }
            
            // Check both characters are valid hex digits
            if !is_hex_digit(input[i + 1]) || !is_hex_digit(input[i + 2]) {
                return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
            }
            
            i += 3; // Skip past this valid escape sequence
        } else if input[i] == b'&' || input[i] == b'#' {
            // Found end of value
            break;
        } else {
            i += 1;
        }
    }
    
    // Return the value up to the next delimiter
    Ok((&input[i..], &input[0..i]))
}

// header = hname "=" hvalue
// Returns (name_bytes, value_bytes)
fn header(input: &[u8]) -> ParseResult<(&[u8], &[u8])> {
    separated_pair(hname, tag(b"="), hvalue)(input)
}

// headers = "?" header *( "&" header )
// Returns HashMap<String, String>, handling unescaping.
pub fn uri_headers(input: &[u8]) -> ParseResult<HashMap<String, String>> {
    map_res(
        preceded(
            tag(b"?"),
            separated_list1(tag(b"&"), header)
        ),
        |pairs| -> Result<HashMap<String, String>, NomError<&[u8]>> {
            let mut map = HashMap::new();
            for (name_bytes, value_bytes) in pairs {
                // *** Unescape name and value ***
                let name = unescape_uri_component(name_bytes)
                    .map_err(|_| NomError::new(input, ErrorKind::MapRes))?;
                let value = unescape_uri_component(value_bytes)
                    .map_err(|_| NomError::new(input, ErrorKind::MapRes))?;
                map.insert(name, value);
            }
            Ok(map)
        }
    )(input)
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
        let input = b"?name=value#fragment";
        let (rem, map) = uri_headers(input).unwrap();
        assert_eq!(rem, b"#fragment");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("name"), Some(&"value".to_string()));
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