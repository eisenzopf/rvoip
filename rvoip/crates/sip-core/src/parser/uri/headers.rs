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
    recognize(many1(alt((
        take_while1(is_hnv_unreserved),
        unreserved,
        escaped,
    ))))(input)
}

// hvalue = *( hnv-unreserved / unreserved / escaped )
// Returns raw bytes, unescaping happens in uri_headers
fn hvalue(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(alt((
        take_while1(is_hnv_unreserved),
        unreserved,
        escaped,
    ))))(input)
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
        |pairs| {
            let mut map = HashMap::new();
            for (name_bytes, value_bytes) in pairs {
                // *** Unescape name and value ***
                let name = unescape_uri_component(name_bytes)?;
                let value = unescape_uri_component(value_bytes)?;
                map.insert(name, value);
            }
            Ok(map)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_headers_unescaped() {
        let input = b"?h%20name=h%20value&other=val%25"; // h name=h value, other=val%
        let (rem, map) = uri_headers(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("h name"), Some(&"h value".to_string()));
        assert_eq!(map.get("other"), Some(&"val%".to_string()));
    }
} 