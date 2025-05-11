// Parser for the Accept-Encoding header (RFC 3261 Section 20.2)
// Accept-Encoding = "Accept-Encoding" HCOLON [ encoding *(COMMA encoding) ]
// encoding = codings *(SEMI accept-param)
// codings = content-coding / "*"
// content-coding = token
// accept-param = ("q" EQUAL qvalue) / generic-param

use nom::{
    branch::alt,
    bytes::complete::tag,
    combinator::{map, opt, value},
    multi::{many0, separated_list0, separated_list1},
    sequence::{pair, preceded},
    IResult,
};
use std::str;
use std::collections::HashMap;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma};
use crate::parser::token::token;
use crate::parser::common_params::accept_param; // Reuses generic_param, qvalue
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

use crate::types::param::Param;
// use crate::types::accept_encoding::AcceptEncoding as AcceptEncodingHeader; // Removed unused import

// Define EncodingInfo locally and make it public
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodingInfo {
    pub coding: String,
    pub params: Vec<Param>,
}

impl EncodingInfo {
    // Get effective q-value (defaults to 1.0 if not specified)
    pub fn q_value(&self) -> f32 {
        for param in &self.params {
            if let Param::Q(q) = param {
                return q.into_inner();
            }
        }
        1.0 // Default q-value per RFC 3261
    }
}

impl std::fmt::Display for EncodingInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.coding)?;
        
        for param in &self.params {
            match param {
                Param::Q(q_not_nan) => {
                    let q_val: f32 = q_not_nan.into_inner();
                    if q_val.fract() == 0.0 && q_val >= 0.0 { // Ensure it's a whole number like 0.0, 1.0
                        write!(f, ";q={}", q_val as i32)?;
                    } else {
                        write!(f, ";q={:.3}", q_val)?;
                    }
                }
                Param::Other(name, None) => write!(f, ";{}", name)?,
                Param::Other(name, Some(crate::types::param::GenericValue::Token(value))) => 
                    write!(f, ";{}={}", name, value)?,
                Param::Other(name, Some(crate::types::param::GenericValue::Quoted(value))) => 
                    write!(f, ";{}=\"{}\"", name, value)?,
                _ => {} // Other param types not expected in Accept-Encoding
            }
        }
        
        Ok(())
    }
}

impl std::cmp::Ord for EncodingInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Sort by q-value (highest first), then by coding string for stable ordering
        other.q_value().partial_cmp(&self.q_value())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| self.coding.cmp(&other.coding))
    }
}

impl std::cmp::PartialOrd for EncodingInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// codings = content-coding / "*"
// content-coding = token
// Returns coding as String
fn codings(input: &[u8]) -> ParseResult<String> {
    map(
        alt((token, tag("*"))),
        |bytes| String::from_utf8_lossy(bytes).to_string()
    )(input)
}

// accept-param = ("q" EQUAL qvalue) / generic-param
// REMOVED: Now imported from common_params

// encoding = codings *(SEMI accept-param)
// Returns EncodingInfo { coding: String, params: Vec<Param> }
fn encoding(input: &[u8]) -> ParseResult<EncodingInfo> {
    map(
        pair(
            codings,
            many0(preceded(semi, accept_param))
        ),
        |(coding_str, params_vec)| EncodingInfo { coding: coding_str, params: params_vec }
    )(input)
}

// Define structure for Accept-Encoding header value
#[derive(Debug, PartialEq, Clone)]
pub struct AcceptEncodingValue {
    pub coding: String,
    pub q: Option<NotNan<f32>>,
    pub params: HashMap<String, String>, // Generic params
}

// Accept-Encoding = "Accept-Encoding" HCOLON [ encoding *(COMMA encoding) ]
/// Parses an Accept-Encoding header value.
// pub fn parse_accept_encoding(input: &[u8]) -> ParseResult<Vec<EncodingInfo>> { // Uncommented function
//     separated_list1(comma, encoding)(input)
// }
// Let's adjust the parser to handle the optional nature: [ encoding *(COMMA encoding) ]
// It should return ParseResult<Vec<EncodingInfo>> , empty Vec if header value is empty
pub fn parse_accept_encoding(input: &[u8]) -> ParseResult<Vec<EncodingInfo>> {
    // Use separated_list0 to allow an empty list if the input is empty or just whitespace
    separated_list0(comma, encoding)(input) 
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{Param, GenericValue};
    use ordered_float::NotNan;

    #[test]
    fn test_encoding() {
        let (rem, enc) = encoding(b"gzip;q=1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(enc.coding, "gzip");
        assert_eq!(enc.params.len(), 1);
        assert!(enc.params.contains(&Param::Q(NotNan::new(1.0).unwrap())));

        let (rem_wild, enc_wild) = encoding(b"*").unwrap();
        assert!(rem_wild.is_empty());
        assert_eq!(enc_wild.coding, "*");
        assert!(enc_wild.params.is_empty());
    }

    #[test]
    fn test_parse_accept_encoding() {
        let input = b"compress, gzip, *;q=0.5";
        let result = parse_accept_encoding(input);
        assert!(result.is_ok());
        let (rem, encodings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(encodings.len(), 3);
        
        assert_eq!(encodings[0].coding, "compress");
        assert!(encodings[0].params.is_empty());

        assert_eq!(encodings[1].coding, "gzip");
        assert!(encodings[1].params.is_empty());

        assert_eq!(encodings[2].coding, "*");
        assert_eq!(encodings[2].params.len(), 1);
        assert!(matches!(encodings[2].params[0], Param::Q(q) if q == NotNan::new(0.5).unwrap()));
    }
    
    #[test]
    fn test_parse_accept_encoding_empty() {
        let input = b""; // Empty value allowed
        let result = parse_accept_encoding(input);
        assert!(result.is_ok());
        let (rem, encodings) = result.unwrap();
        assert!(rem.is_empty()); // Should consume empty input
        assert!(encodings.is_empty()); // Should result in an empty Vec
    }
} 