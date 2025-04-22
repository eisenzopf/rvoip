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
    multi::{many0, separated_list0},
    sequence::{pair, preceded},
    IResult,
};
use std::str;
use std::collections::HashMap;
use ordered_float::NotNan;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma};
use crate::parser::token::token;
use crate::parser::common_params::accept_param; // Reuses generic_param, qvalue
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::accept_encoding::AcceptEncoding as AcceptEncodingHeader; // Specific type

// Define EncodingInfo locally and make it public
#[derive(Debug, Clone, PartialEq)] // Add derives
pub struct EncodingInfo {
    pub coding: String,
    pub params: Vec<Param>,
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
pub(crate) fn parse_accept_encoding(input: &[u8]) -> ParseResult<AcceptEncodingHeader> {
    map(
        comma_separated_list0(encoding),
        AcceptEncodingHeader // Wrap Vec<...> in AcceptEncoding newtype
    )(input)
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
        assert!(rem.is_empty());
        assert!(encodings.is_empty());
    }
} 