// Parser for Content-Encoding header (RFC 3261 Section 20.14)
// Content-Encoding = ( "Content-Encoding" / "e" ) HCOLON content-coding *(COMMA content-coding)
// content-coding = token

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, map_res, opt},
    sequence::{pair, preceded, terminated},
    multi::separated_list1,
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::common::comma_separated_list1; // Requires at least one
use crate::parser::ParseResult;
use crate::parser::whitespace::{lws, owsp, sws};
use std::str;

// content-coding = token
fn content_coding(input: &[u8]) -> ParseResult<String> {
    // Handle any leading whitespace, including line folding
    let (input, _) = opt(lws)(input)?;
    
    // Parse the token
    let (input, coding_bytes) = token(input)?;
    
    // Convert token to string
    let coding = str::from_utf8(coding_bytes)
        .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::AlphaNumeric)))?
        .to_string();
    
    // Handle any trailing whitespace
    let (input, _) = opt(lws)(input)?;
    
    Ok((input, coding))
}

/// Parses the Content-Encoding header value.
/// Content-Encoding = ( "Content-Encoding" / "e" ) HCOLON content-coding *(COMMA content-coding)
/// Note: The header name and HCOLON are handled by the main message parser.
pub fn parse_content_encoding(input: &[u8]) -> ParseResult<Vec<String>> {
    // First check for empty input
    if input.is_empty() {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
    }
    
    // Handle any leading whitespace, including line folding
    let (input, _) = opt(lws)(input)?;
    
    // Parse at least one content-coding, separated by commas
    let (input, codings) = separated_list1(
        // Handle whitespace around the comma
        preceded(opt(sws), terminated(comma, opt(lws))),
        content_coding
    )(input)?;
    
    // Handle any trailing whitespace
    let (input, _) = sws(input)?;
    
    // Check that there's nothing left to parse
    if !input.is_empty() {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Eof)));
    }
    
    Ok((input, codings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_coding() {
        let (rem, coding) = content_coding(b"gzip").unwrap();
        assert!(rem.is_empty());
        assert_eq!(coding, "gzip");
    }

    #[test]
    fn test_parse_content_encoding_single() {
        let input = b"deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["deflate".to_string()]);
    }
    
    #[test]
    fn test_parse_content_encoding_multiple() {
        let input = b"gzip, identity";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "identity".to_string()]);
    }

    #[test]
    fn test_parse_content_encoding_empty_fail() {
        // Header value cannot be empty according to ABNF (1*content-coding)
        let input = b"";
        let result = parse_content_encoding(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_parse_content_encoding_with_whitespace() {
        // Test with surrounding whitespace
        let input = b"  gzip  ,  deflate  ";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "deflate".to_string()]);
        
        // Test with tabs
        let input = b"gzip	,	deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "deflate".to_string()]);
    }
    
    #[test]
    fn test_parse_content_encoding_with_line_folding() {
        // Test with line folding at the beginning
        let input = b"\r\n gzip, deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "deflate".to_string()]);
        
        // Test with line folding around comma
        let input = b"gzip\r\n ,\r\n deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "deflate".to_string()]);
        
        // Test with multiple line folding
        let input = b"\r\n gzip\r\n ,\r\n deflate\r\n ,\r\n identity";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "deflate".to_string(), "identity".to_string()]);
    }
    
    #[test]
    fn test_parse_content_encoding_with_error_cases() {
        // Test with invalid token character
        let input = b"gzip, def@late";
        let result = parse_content_encoding(input);
        assert!(result.is_err());
        
        // Test with missing content coding after comma
        let input = b"gzip,";
        let result = parse_content_encoding(input);
        assert!(result.is_err());
        
        // Test with invalid separator
        let input = b"gzip; deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_parse_content_encoding_multiple_tokens() {
        // Content-coding is just a token, so test various valid token characters
        let input = b"gzip-x.y+z!";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip-x.y+z!".to_string()]);
        
        // Test multiple complex tokens
        let input = b"gzip-v1.2, x-custom.encoding+profile";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip-v1.2".to_string(), "x-custom.encoding+profile".to_string()]);
    }
    
    #[test]
    fn test_parse_content_encoding_rfc3261_examples() {
        // Common encoding types mentioned in RFCs although specific Content-Encoding examples aren't in RFC 3261
        let input = b"gzip";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        
        let input = b"compress";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        
        let input = b"deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        
        let input = b"identity";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_parse_content_encoding_case_insensitivity() {
        // Content-coding tokens should be case-insensitive per the token definition
        let input = b"GZIP, Deflate, Identity";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        
        // Note: case is preserved in the returned values - case conversion happens in application logic if needed
        assert_eq!(codings, vec!["GZIP".to_string(), "Deflate".to_string(), "Identity".to_string()]);
    }
    
    #[test]
    fn test_parse_content_encoding_abnf_compliance() {
        // Test that the parser follows the ABNF grammar
        
        // content-coding = token (test valid token characters)
        assert!(parse_content_encoding(b"a").is_ok());
        assert!(parse_content_encoding(b"A").is_ok());
        assert!(parse_content_encoding(b"0").is_ok());
        assert!(parse_content_encoding(b"a0").is_ok());
        assert!(parse_content_encoding(b"a-b").is_ok());
        assert!(parse_content_encoding(b"a.b").is_ok());
        assert!(parse_content_encoding(b"a!b").is_ok());
        assert!(parse_content_encoding(b"a%b").is_ok());
        assert!(parse_content_encoding(b"a*b").is_ok());
        assert!(parse_content_encoding(b"a_b").is_ok());
        assert!(parse_content_encoding(b"a+b").is_ok());
        assert!(parse_content_encoding(b"a'b").is_ok());
        assert!(parse_content_encoding(b"a~b").is_ok());
        
        // content-coding *(COMMA content-coding) (test multiple codings)
        assert!(parse_content_encoding(b"a,b").is_ok());
        assert!(parse_content_encoding(b"a,b,c").is_ok());
        assert!(parse_content_encoding(b"a, b, c").is_ok());
    }
    
    #[test]
    fn test_parse_content_encoding_common_values() {
        // Test common content-encoding values used in HTTP/SIP
        let common_encodings = ["gzip", "compress", "deflate", "identity", "br", "x-gzip", "x-compress"];
        
        for encoding in common_encodings.iter() {
            let input = encoding.as_bytes();
            let result = parse_content_encoding(input);
            assert!(result.is_ok());
            let (rem, codings) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(codings, vec![encoding.to_string()]);
        }
    }
} 