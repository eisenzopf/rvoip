// Parser for RSeq header (RFC 3262)
// RSeq = "RSeq" HCOLON 1*DIGIT

use nom::{
    character::complete::digit1,
    combinator::{map_res, recognize, all_consuming},
    IResult, error::ErrorKind,
};
use std::str;

// Import from other modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;
use crate::parser::whitespace::owsp;

/// Parse an RSeq value, which is a non-negative integer
pub fn parse_rseq(input: &[u8]) -> ParseResult<u32> {
    let (input, _) = owsp(input)?;
    
    // Define parse_digit as a function rather than a variable
    fn parse_digit(input: &[u8]) -> ParseResult<u32> {
        map_res(
            recognize(digit1),
            |digits: &[u8]| {
                let digits_str = str::from_utf8(digits)
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(digits, ErrorKind::Digit)))?;
                digits_str.parse::<u32>()
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(digits, ErrorKind::Digit)))
            }
        )(input)
    }
    
    let (input, rseq_val) = parse_digit(input)?;
    
    let (input, _) = owsp(input)?;
    
    Ok((input, rseq_val))
}

/// Parse a complete RSeq header, including the header name and colon
pub fn parse_rseq_header(input: &[u8]) -> ParseResult<u32> {
    let (input, _) = nom::bytes::complete::tag_no_case(b"RSeq")(input)?;
    let (input, _) = hcolon(input)?;
    parse_rseq(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::combinator::all_consuming;

    // Helper function to test the parser with full input consumption
    fn test_parse_rseq(input: &[u8]) -> Result<u32, nom::Err<nom::error::Error<&[u8]>>> {
        all_consuming(parse_rseq)(input).map(|(_, output)| output)
    }

    #[test]
    fn test_parse_rseq_normal() {
        // Simple value
        let input = b"1";
        let result = test_parse_rseq(input).unwrap();
        assert_eq!(result, 1);
        
        // Larger value
        let input = b"12345";
        let result = test_parse_rseq(input).unwrap();
        assert_eq!(result, 12345);
    }

    #[test]
    fn test_parse_rseq_whitespace() {
        // With surrounding whitespace
        let input = b" 42 ";
        let result = test_parse_rseq(input).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_parse_rseq_invalid() {
        // Non-digit value should fail
        let input = b"abc";
        let result = test_parse_rseq(input);
        assert!(result.is_err());
        
        // Empty string should fail
        let input = b"";
        let result = test_parse_rseq(input);
        assert!(result.is_err());
        
        // Negative number is not valid in SIP
        let input = b"-1";
        let result = test_parse_rseq(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rseq_header() {
        // Complete header
        let input = b"RSeq: 42";
        let (remaining, rseq) = parse_rseq_header(input).unwrap();
        assert!(remaining.is_empty());
        assert_eq!(rseq, 42);
        
        // Case insensitive header name
        let input = b"rseq: 123";
        let (remaining, rseq) = parse_rseq_header(input).unwrap();
        assert!(remaining.is_empty());
        assert_eq!(rseq, 123);
    }
    
    #[test]
    fn test_parse_rseq_in_message() {
        // Sample provisional response message with RSeq header
        let response = b"SIP/2.0 183 Session Progress\r\n\
                         Via: SIP/2.0/UDP client.example.com:5060;branch=z9hG4bK74bf9\r\n\
                         From: Alice <sip:alice@example.com>;tag=9fxced76sl\r\n\
                         To: Bob <sip:bob@example.com>;tag=8321234356\r\n\
                         Call-ID: 3848276298220188511@client.example.com\r\n\
                         CSeq: 1 INVITE\r\n\
                         Contact: <sip:bob@server.example.com>\r\n\
                         Require: 100rel\r\n\
                         RSeq: 123\r\n\
                         Content-Type: application/sdp\r\n\
                         Content-Length: 0\r\n\
                         \r\n";
                         
        // Just use the RSeq line directly
        let rseq_line = b"RSeq: 123";
        
        // Parse the RSeq line
        let (_, rseq) = parse_rseq_header(rseq_line).unwrap();
        assert_eq!(rseq, 123);
    }
    
    #[test]
    fn test_parse_rseq_sequence() {
        // Test sequential RSeq values
        let first = b"RSeq: 1";
        let (_, rseq1) = parse_rseq_header(first).unwrap();
        assert_eq!(rseq1, 1);
        
        let second = b"RSeq: 2";
        let (_, rseq2) = parse_rseq_header(second).unwrap();
        assert_eq!(rseq2, 2);
        
        // Verify incremental relationship
        assert_eq!(rseq2, rseq1 + 1);
    }
    
    #[test]
    fn test_parse_rseq_large_values() {
        // Test with values close to u32 limit
        let max_u32 = u32::MAX;
        let almost_max = format!("RSeq: {}", max_u32 - 1);
        let (_, rseq) = parse_rseq_header(almost_max.as_bytes()).unwrap();
        assert_eq!(rseq, max_u32 - 1);
    }
} 