// Parser for CSeq header (RFC 3261 Section 20.16)
// CSeq = "CSeq" HCOLON 1*DIGIT LWS Method

use nom::{
    bytes::complete::tag_no_case,
    character::complete::{digit1},
    combinator::{map, map_res, opt, recognize},
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

/// Parses 1*DIGIT LWS Method
/// Returns a tuple of (sequence number, Method)
fn cseq_value_method(input: &[u8]) -> ParseResult<(u32, Method)> {
    map_res(
        pair(
            digit1, // 1*DIGIT
            preceded(lws, token) // LWS Method (token)
        ),
        |(seq_bytes, method_bytes)| -> Result<(u32, Method), NomError<&[u8]>> {
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

/// Parses the CSeq value part (without header name and colon)
/// Expected format: 1*DIGIT LWS Method
/// Returns a CSeq struct
pub fn parse_cseq(input: &[u8]) -> ParseResult<CSeq> {
    map(
        cseq_value_method,
        |(seq, method)| CSeq::new(seq, method)
    )(input)
}

/// Full parser that handles the complete header including name and colon
/// Expected format: "CSeq" HCOLON 1*DIGIT LWS Method
/// Returns a CSeq struct
pub fn full_parse_cseq(input: &[u8]) -> ParseResult<CSeq> {
    preceded(
        pair(tag_no_case(b"CSeq"), hcolon),
        parse_cseq
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
        assert_eq!(cseq.method, Method::Publish);
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
    
    // ABNF compliance tests
    
    #[test]
    fn test_full_header_parsing() {
        // Test with header name and colon
        let input = b"CSeq: 101 INVITE";
        let result = full_parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 101);
        assert_eq!(cseq.method, Method::Invite);
        
        // Test case insensitivity of header name
        let input_case = b"cseq: 102 ACK";
        let result_case = full_parse_cseq(input_case);
        assert!(result_case.is_ok(), "Header name should be case-insensitive");
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with extra whitespace after colon
        let input = b"CSeq:  101 INVITE";
        let result = full_parse_cseq(input);
        assert!(result.is_ok(), "Should handle extra whitespace after colon");
        
        // Test with extra whitespace between sequence and method
        let input_extra = b"CSeq: 101     INVITE";
        let result_extra = full_parse_cseq(input_extra);
        assert!(result_extra.is_ok(), "Should handle extra whitespace between sequence and method");
        
        // Test with tab as whitespace
        let input_tab = b"CSeq: 101\tINVITE";
        let result_tab = full_parse_cseq(input_tab);
        assert!(result_tab.is_ok(), "Should handle tab as whitespace");
    }
    
    #[test]
    fn test_sequence_number_limits() {
        // Test minimum sequence (1 digit)
        let input_min = b"CSeq: 1 ACK";
        let result_min = full_parse_cseq(input_min);
        assert!(result_min.is_ok(), "Should handle minimum sequence number");
        
        // Test large sequence number
        let input_large = b"CSeq: 4294967295 BYE"; // u32::MAX
        let result_large = full_parse_cseq(input_large);
        assert!(result_large.is_ok(), "Should handle maximum u32 sequence number");
        let (_, cseq_large) = result_large.unwrap();
        assert_eq!(cseq_large.seq, u32::MAX);
        
        // Test overflow (should fail or handle gracefully)
        let input_overflow = b"CSeq: 4294967296 BYE"; // u32::MAX + 1
        let result_overflow = full_parse_cseq(input_overflow);
        // This depends on how the implementation handles overflow
        // Either it should fail or truncate to fit u32
        println!("Overflow result: {:?}", result_overflow);
    }
    
    #[test]
    fn test_method_case_sensitivity() {
        // Test method case sensitivity (should be case-sensitive)
        let input_upper = b"CSeq: 101 INVITE";
        let result_upper = full_parse_cseq(input_upper);
        assert!(result_upper.is_ok());
        
        let input_lower = b"CSeq: 101 invite";
        let result_lower = full_parse_cseq(input_lower);
        // This depends on Method::from_str implementation
        // According to RFC 3261, methods are case-sensitive
        println!("Lowercase method result: {:?}", result_lower);
    }
    
    #[test]
    fn test_abnf_edge_cases() {
        // Test with no whitespace between header name and colon (valid)
        let input_no_ws = b"CSeq:101 INVITE";
        let result_no_ws = full_parse_cseq(input_no_ws);
        assert!(result_no_ws.is_ok(), "No whitespace before colon should be valid");
        
        // Test missing colon (invalid)
        let input_no_colon = b"CSeq 101 INVITE";
        let result_no_colon = full_parse_cseq(input_no_colon);
        assert!(result_no_colon.is_err(), "Missing colon should be invalid");
        
        // Test with no method (invalid)
        let input_no_method = b"CSeq: 101";
        let result_no_method = full_parse_cseq(input_no_method);
        assert!(result_no_method.is_err(), "Missing method should be invalid");
        
        // Test with no sequence number (invalid)
        let input_no_seq = b"CSeq: INVITE";
        let result_no_seq = full_parse_cseq(input_no_seq);
        assert!(result_no_seq.is_err(), "Missing sequence number should be invalid");
    }
    
    #[test]
    fn test_rfc_examples() {
        // Examples from RFC 3261
        let example1 = b"CSeq: 4711 INVITE";
        let result1 = full_parse_cseq(example1);
        assert!(result1.is_ok());
        let (_, cseq1) = result1.unwrap();
        assert_eq!(cseq1.seq, 4711);
        assert_eq!(cseq1.method, Method::Invite);
        
        // Test with other methods from RFC
        let test_cases: Vec<(&[u8], Method)> = vec![
            (b"CSeq: 1 REGISTER", Method::Register),
            (b"CSeq: 2 ACK", Method::Ack),
            (b"CSeq: 3 OPTIONS", Method::Options),
            (b"CSeq: 4 BYE", Method::Bye),
            (b"CSeq: 5 CANCEL", Method::Cancel),
            (b"CSeq: 6 REFER", Method::Refer),
            (b"CSeq: 7 SUBSCRIBE", Method::Subscribe),
            (b"CSeq: 8 NOTIFY", Method::Notify),
        ];
        
        for (input, expected_method) in test_cases {
            let result = full_parse_cseq(input);
            assert!(result.is_ok(), "Failed to parse: {:?}", std::str::from_utf8(input));
            let (_, cseq) = result.unwrap();
            assert_eq!(cseq.method, expected_method);
        }
    }

    #[test]
    fn test_custom_extension_method() {
        // Test a custom extension method not predefined in the Method enum
        let input = b"999 CUSTOMMETHOD";
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let (rem, cseq) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(cseq.seq, 999);
        assert_eq!(cseq.method, Method::Extension("CUSTOMMETHOD".into()));
    }
} 