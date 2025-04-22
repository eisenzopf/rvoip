// Parser for CSeq header (RFC 3261 Section 20.16)
// CSeq = "CSeq" HCOLON 1*DIGIT LWS Method

use nom::{
    bytes::complete::{tag_no_case, take_while1},
    character::complete::digit1,
    combinator::{map, map_res},
    sequence::{pair, preceded, terminated},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::token::token; // Method is a token
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;

// Import types (assuming CSeq struct and Method enum exist)
use crate::types::cseq::CSeq; // Use the specific type
use crate::types::method::Method;

// CSeq = 1*DIGIT LWS Method
// Renamed internal struct CSeqValue to avoid conflict
#[derive(Debug)]
struct CSeqValue {
    seq: u32,
    method_bytes: Vec<u8>,
}
fn cseq_value_parser(input: &[u8]) -> ParseResult<CSeqValue> { // Return intermediate struct
    map(
        pair(
            map_res(digit1, |b| str::from_utf8(b)?.parse::<u32>()), // Parse sequence number as u32
            preceded(lws, token) // Keep method as bytes initially
        ),
        |(seq, method_bytes)| CSeqValue { seq, method_bytes: method_bytes.to_vec() }
    )(input)
}

// CSeq = "CSeq" HCOLON 1*DIGIT LWS Method
// Note: HCOLON handled elsewhere
pub(crate) fn parse_cseq(input: &[u8]) -> ParseResult<CSeq> { // Return final type
    map_res(cseq_value_parser, |parsed| {
        // Convert method bytes to Method enum
        Method::from_str(str::from_utf8(&parsed.method_bytes)?)
            .map(|method| CSeq { seq: parsed.seq, method })
            // Map FromStr error to a string compatible with map_res
            .map_err(|_| "Invalid Method in CSeq") 
    })(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::method::Method;

    #[test]
    fn test_parse_cseq() {
        let input = b"4711 INVITE";
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap(); // Returns CSeq struct
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 4711);
        assert_eq!(cseq.method, Method::Invite);
    }
    
    #[test]
    fn test_parse_cseq_register() {
        let input = b"1 REGISTER";
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 1);
        assert_eq!(cseq.method, Method::Register);
    }
    
     #[test]
    fn test_parse_cseq_extension_method() {
        let input = b"100 PUBLISH"; // Assuming PUBLISH is extension
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 100);
        assert_eq!(cseq.method, Method::Publish); // Or Method::Other("PUBLISH".into())
    }
    
    #[test]
    fn test_invalid_cseq_no_lws() {
        let input = b"1INVITE";
        assert!(parse_cseq(input).is_err());
    }

    #[test]
    fn test_invalid_cseq_no_method() {
        let input = b"1 ";
        assert!(parse_cseq(input).is_err());
    }
    
    #[test]
    fn test_invalid_cseq_no_digit() {
        let input = b" INVITE";
        assert!(parse_cseq(input).is_err());
    }
} 