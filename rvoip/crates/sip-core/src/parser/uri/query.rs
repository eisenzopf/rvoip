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
fn query_raw(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(uric))(input)
}

// Parse a query string with key-value pairs into a HashMap
// Transforms byte sequences into proper strings with URI unescaping
pub fn parse_query_params(input: &[u8]) -> ParseResult<HashMap<String, String>> {
    map_res(
        separated_list0(tag(b"&"), query_param),
        |params| {
            let mut map = HashMap::new();
            for (k, v) in params {
                let key = unescape_uri_component(k)?;
                let value = unescape_uri_component(v)?;
                map.insert(key, value);
            }
            Ok::<_, Error>(map)
        }
    )(input)
}

// Parse the entire query component of a URI, which may be preceded by '?'
// Returns the raw bytes or optionally nothing if there is no query
pub fn parse_query(input: &[u8]) -> ParseResult<Option<&[u8]>> {
    opt(preceded(tag(b"?"), query_raw))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_query_raw() {
        let (rem, parsed) = query_raw(b"param1=value1&param2=value2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"param1=value1&param2=value2");
    }

    #[test]
    fn test_parse_query_params() {
        let (rem, params) = parse_query_params(b"param1=value1&param2=value2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("param1"), Some(&"value1".to_string()));
        assert_eq!(params.get("param2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_parse_query_with_escaped_chars() {
        let (rem, params) = parse_query_params(b"name=user%20name&query=search%3Fterm").unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("name"), Some(&"user name".to_string()));
        assert_eq!(params.get("query"), Some(&"search?term".to_string()));
    }

    #[test]
    fn test_parse_optional_query() {
        let (rem, parsed) = parse_query(b"?param1=value1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, Some(b"param1=value1"));

        let (rem, parsed) = parse_query(b"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, None);
    }
} 