// Parser for CSeq header (RFC 3261 Section 20.16)
// CSeq = "CSeq" HCOLON 1*DIGIT LWS Method

use nom::{
    character::complete::{digit1},
    combinator::{map, map_res},
    sequence::{pair, preceded, separated_pair, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError}, // Import NomError
};
use std::str::{self, FromStr};

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::token::token; // Method is a token
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;

// Import types (assuming CSeq struct and Method enum exist)
use crate::types::cseq::CSeq; // Use the specific type
use crate::types::method::Method;

// Parses 1*DIGIT LWS Method
fn cseq_value_method(input: &[u8]) -> ParseResult<(u32, Method)> {
    map_res(
        pair( // Use pair instead of separated_pair if Method is not just token
            digit1, // 1*DIGIT
            preceded(lws, token) // LWS Method (token)
        ),
        |(seq_bytes, method_bytes)| -> Result<(u32, Method), NomError<&[u8]>> { // Return Result<_, NomError>
            // Parse sequence number
            let seq_str = str::from_utf8(seq_bytes)
                .map_err(|_| NomError::from_error_kind(seq_bytes, ErrorKind::Char))?;
            let seq_num = seq_str.parse::<u32>()
                .map_err(|_| NomError::from_error_kind(seq_bytes, ErrorKind::Digit))?;
            
            // Parse method
            let method_str = str::from_utf8(method_bytes)
                 .map_err(|_| NomError::from_error_kind(method_bytes, ErrorKind::Char))?; 
            let method = Method::from_str(method_str)
                 .map_err(|_| NomError::from_error_kind(method_bytes, ErrorKind::Verify))?; // Use Verify for Method parse error
                 
            Ok((seq_num, method))
        }
    )(input)
}

// CSeq = "CSeq" HCOLON CSeq-value 
// Note: HCOLON handled elsewhere.
pub(crate) fn parse_cseq(input: &[u8]) -> ParseResult<CSeq> {
    map(
        cseq_value_method, // Use the combined parser
        |(seq, method)| CSeq::new(seq, method)
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::method::Method;

    #[test]
    fn test_parse_cseq() {
        let input = b"101 INVITE";
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 101);
        assert_eq!(cseq.method, Method::Invite);
    }
    
    #[test]
    fn test_parse_cseq_options() {
         let input = b"2 OPTIONS";
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 2);
        assert_eq!(cseq.method, Method::Options);
    }

    #[test]
    fn test_parse_cseq_extension_method() {
         let input = b"47 PUBLISH"; // Assuming PUBLISH is known or handled as extension
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 47);
        assert_eq!(cseq.method, Method::Extension("PUBLISH".into()));
    }
    
    #[test]
    fn test_invalid_cseq_no_method() {
         let input = b"101";
        assert!(parse_cseq(input).is_err());
    }

     #[test]
    fn test_invalid_cseq_no_seq() {
         let input = b"INVITE";
        assert!(parse_cseq(input).is_err());
    }
} 