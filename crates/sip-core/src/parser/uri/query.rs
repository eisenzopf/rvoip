// Parser for URI query component (RFC 3261/2396)
// query follows a '?' character in a URI

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, separated_list0},
    sequence::{pair, preceded, separated_pair},
    IResult,
};
use std::str;
use std::collections::HashMap;

use crate::parser::common_chars::{escaped, reserved, unreserved};
use crate::parser::ParseResult;
use crate::parser::utils::unescape_uri_component;
use crate::error::Error;

// uric = reserved / unreserved / escaped
fn uric(input: &[u8]) -> ParseResult<&[u8]> {
    alt((reserved, unreserved, escaped))(input)
}

// Parse a single query parameter name or value
fn query_param_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(uric))(input)
}

// Parse a name=value pair in the query string
fn query_param(input: &[u8]) -> ParseResult<(&[u8], &[u8])> {
    separated_pair(
        query_param_part,
        tag(b"="),
        query_param_part
    )(input)
}

// query = *uric
// Returns raw query string as bytes
pub fn query_raw(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(uric))(input)
}

// Parse a query string with key-value pairs into a HashMap
// Transforms byte sequences into proper strings with URI unescaping
pub fn parse_query_params(input: &[u8]) -> ParseResult<HashMap<String, String>> {
    // First use query_raw to consume the entire input
    let (rem, raw_query) = query_raw(input)?;
    
    // Now parse the valid key=value pairs from the raw query
    let mut map = HashMap::new();
    
    // If input is empty, return empty map
    if raw_query.is_empty() {
        return Ok((rem, map));
    }
    
    // Split by '&' and process each part
    let parts = raw_query.split(|&c| c == b'&');
    
    for part in parts {
        // Skip empty parts
        if part.is_empty() {
            continue;
        }
        
        // Find equals sign
        if let Some(pos) = part.iter().position(|&c| c == b'=') {
            let key_bytes = &part[..pos];
            let value_bytes = &part[pos+1..];
            
            // Only process valid key=value pairs (key must not be empty)
            if !key_bytes.is_empty() {
                let key = unescape_uri_component(key_bytes)
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Char)))?;
                let value = unescape_uri_component(value_bytes)
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Char)))?;
                
                map.insert(key, value);
            }
        }
        // Parts without equals sign are ignored for key-value parsing
    }
    
    Ok((rem, map))
}

// Parse the entire query component of a URI, which may be preceded by '?'
// Returns the raw bytes or optionally nothing if there is no query
pub fn parse_query(input: &[u8]) -> ParseResult<Option<&[u8]>> {
    opt(preceded(tag(b"?"), query_raw))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Basic Query Structure Tests ===

    #[test]
    fn test_parse_query_raw() {
        let (rem, parsed) = query_raw(b"param1=value1&param2=value2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"param1=value1&param2=value2");
    }

    #[test]
    fn test_parse_empty_query() {
        let (rem, parsed) = query_raw(b"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"");
    }

    #[test]
    fn test_parse_query_params() {
        let (rem, params) = parse_query_params(b"param1=value1&param2=value2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("param1"), Some(&"value1".to_string()));
        assert_eq!(params.get("param2"), Some(&"value2".to_string()));
    }

    // === Character Handling Tests ===

    #[test]
    fn test_parse_query_with_escaped_chars() {
        let (rem, params) = parse_query_params(b"name=user%20name&query=search%3Fterm").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("name"), Some(&"user name".to_string()));
        assert_eq!(params.get("query"), Some(&"search?term".to_string()));
    }

    #[test]
    fn test_query_with_all_allowed_chars() {
        // Test with unreserved chars
        let (rem, params) = parse_query_params(b"unreserved=abc123-_.!~*'()").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.get("unreserved"), Some(&"abc123-_.!~*'()".to_string()));

        // Test with reserved chars (some need to be escaped in values)
        let (rem, params) = parse_query_params(b"reserved=%3B%2F%3F%3A%40%26%3D%2B%24%2C").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.get("reserved"), Some(&";/?:@&=+$,".to_string()));
    }
    
    #[test]
    fn test_individual_reserved_chars() {
        // Test each reserved character individually
        let inputs = [
            (b"semicolon=%3B".as_ref(), ";"),
            (b"slash=%2F".as_ref(), "/"),
            (b"question=%3F".as_ref(), "?"),
            (b"colon=%3A".as_ref(), ":"),
            (b"at=%40".as_ref(), "@"),
            (b"ampersand=%26".as_ref(), "&"),
            (b"equals=%3D".as_ref(), "="),
            (b"plus=%2B".as_ref(), "+"),
            (b"dollar=%24".as_ref(), "$"),
            (b"comma=%2C".as_ref(), ",")
        ];
        
        for (input, expected) in inputs.iter() {
            let (rem, params) = parse_query_params(input).unwrap();
            assert!(rem.is_empty());
            let param_name = std::str::from_utf8(&input[..input.iter().position(|&c| c == b'=').unwrap()]).unwrap();
            assert_eq!(params.get(param_name), Some(&expected.to_string()));
        }
    }
    
    #[test]
    fn test_query_with_complex_escaping() {
        // Test complex mix of escaped sequences
        let (rem, parsed) = query_raw(b"q=%25%26%2B%20%3D%3F").unwrap(); // %&+ =?
        assert!(rem.is_empty());
        assert_eq!(parsed, b"q=%25%26%2B%20%3D%3F");
        
        let (rem, params) = parse_query_params(b"q=%25%26%2B%20%3D%3F").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.get("q"), Some(&"%&+ =?".to_string()));
    }

    // === Parameter Format Tests ===

    #[test]
    fn test_parse_empty_values() {
        let (rem, params) = parse_query_params(b"empty=&nonempty=value").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("empty"), Some(&"".to_string()));
        assert_eq!(params.get("nonempty"), Some(&"value".to_string()));
    }

    #[test]
    fn test_parse_duplicate_keys() {
        // HashMap keeps last value for duplicate keys
        let (rem, params) = parse_query_params(b"key=first&key=second").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 1);
        assert_eq!(params.get("key"), Some(&"second".to_string()));
    }
    
    #[test]
    fn test_consecutive_ampersands() {
        // Test handling of consecutive ampersands (empty parameters)
        let (rem, params) = parse_query_params(b"a=1&&b=2&&&c=3").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert_eq!(params.get("a"), Some(&"1".to_string()));
        assert_eq!(params.get("b"), Some(&"2".to_string()));
        assert_eq!(params.get("c"), Some(&"3".to_string()));
    }

    // === RFC 3261 Specific Tests ===

    #[test]
    fn test_rfc3261_specific_queries() {
        // RFC 3261 examples with SIP-specific query parameters
        let (rem, params) = parse_query_params(b"transport=tcp&method=INVITE").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("transport"), Some(&"tcp".to_string()));
        assert_eq!(params.get("method"), Some(&"INVITE".to_string()));
        
        // URI parameters used in SIP query strings
        let (rem, params) = parse_query_params(b"user=phone&ttl=15").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("user"), Some(&"phone".to_string()));
        assert_eq!(params.get("ttl"), Some(&"15".to_string()));
    }
    
    #[test]
    fn test_rfc3261_example_uris() {
        // RFC 3261 Section 19.1.1 - SIP URI Examples
        let example_queries = [
            b"transport=udp&user=phone".as_ref(),
            b"method=INVITE".as_ref(),
            b"transport=tcp".as_ref(),
            b"user=phone".as_ref(),
            b"maddr=239.255.255.1&ttl=15".as_ref()
        ];
        
        for query in example_queries {
            let (rem, _) = query_raw(query).unwrap();
            assert!(rem.is_empty());
            
            let (rem, _) = parse_query_params(query).unwrap();
            assert!(rem.is_empty());
        }
    }

    // === Torture Test Cases ===

    #[test]
    fn test_rfc4475_query_torture() {
        // Using examples inspired by RFC 4475 torture test cases
        let (rem, params) = parse_query_params(b"complex=%26%3D%3B&multi%20word=complex%20value").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.get("complex"), Some(&"&=;".to_string()));
        assert_eq!(params.get("multi word"), Some(&"complex value".to_string()));
    }
    
    #[test]
    fn test_very_long_query() {
        // Test with a very long query parameter (implementation limits)
        let mut long_value = vec![b'a'; 1000]; // 1000 'a' characters
        let mut query = b"param=".to_vec();
        query.append(&mut long_value);
        
        let (rem, params) = parse_query_params(&query).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 1);
        assert_eq!(params.get("param").unwrap().len(), 1000);
    }
    
    #[test]
    fn test_many_parameters() {
        // Test with many parameters (50)
        let mut query = Vec::new();
        for i in 0..50 {
            if i > 0 { query.extend_from_slice(b"&"); }
            query.extend_from_slice(format!("p{}=v{}", i, i).as_bytes());
        }
        
        let (rem, params) = parse_query_params(&query).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 50);
        for i in 0..50 {
            let key = format!("p{}", i);
            let value = format!("v{}", i);
            assert_eq!(params.get(&key), Some(&value));
        }
    }

    // === Optional Query Tests ===

    #[test]
    fn test_parse_optional_query() {
        let (rem, parsed) = parse_query(b"?param1=value1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, Some(&b"param1=value1"[..]));

        let (rem, parsed) = parse_query(b"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, None);
    }
    
    #[test]
    fn test_query_with_fragment() {
        // Query followed by fragment
        let (rem, parsed) = parse_query(b"?param=value#fragment").unwrap();
        assert_eq!(rem, b"#fragment");
        assert_eq!(parsed, Some(&b"param=value"[..]));
    }
    
    // === Error Cases and Malformed Input ===
    
    #[test]
    fn test_malformed_query_handling() {
        // This isn't strictly an error per the ABNF, as query = *uric allows any sequence
        // But the parameter parser might reject malformed key=value pairs
        
        // A URI parser would generally accept this raw query
        let (rem, parsed) = query_raw(b"malformed&query&without=equals").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"malformed&query&without=equals");
        
        // But our specific parameter parser that expects key=value pairs would fail
        // The current implementation will skip malformed params without equals signs
        let (rem, params) = parse_query_params(b"valid=param&malformed&also=valid").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("valid"), Some(&"param".to_string()));
        assert_eq!(params.get("also"), Some(&"valid".to_string()));
        assert_eq!(params.get("malformed"), None);
    }
    
    #[test]
    fn test_empty_key_handling() {
        // Test cases with empty keys (e.g., "=value")
        // RFC doesn't explicitly forbid this, but it's ambiguous how to handle it
        let (rem, params) = parse_query_params(b"=value&valid=param").unwrap();
        assert!(rem.is_empty());
        // Our implementation skips param pairs with empty keys
        assert_eq!(params.len(), 1);
        assert_eq!(params.get("valid"), Some(&"param".to_string()));
    }
    
    #[test]
    fn test_trailing_ampersand() {
        // Query with trailing ampersand
        let (rem, params) = parse_query_params(b"a=1&b=2&").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("a"), Some(&"1".to_string()));
        assert_eq!(params.get("b"), Some(&"2".to_string()));
    }
} 