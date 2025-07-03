// Parser for Unsupported header (RFC 3261 Section 20.41)
// Unsupported = "Unsupported" HCOLON option-tag *(COMMA option-tag)
// option-tag = token

use nom::{
    bytes::complete::tag_no_case,
    multi::separated_list1, // Unsupported needs at least one tag
    sequence::{preceded, terminated},
    IResult,
    combinator::{all_consuming, verify},
    error::{ParseError, ErrorKind, make_error},
    Err as NomErr,
};
use std::str;

// Import shared parser
use super::token_list::token_string; // Need underlying token parser
use crate::parser::common::{comma_separated_list1, verify_no_empty_elements};
use crate::parser::ParseResult;
use crate::parser::separators::hcolon;
use crate::parser::utils::unfold_lws;

// Helper to ensure there are no spaces in the input
fn no_spaces(input: &[u8]) -> bool {
    !input.iter().any(|&b| b == b' ')
}

// Unsupported = "Unsupported" HCOLON option-tag *(COMMA option-tag)
// Note: HCOLON handled elsewhere.
pub fn parse_unsupported(input: &[u8]) -> ParseResult<Vec<String>> {
    // Check for line folding first (CRLF or LF followed by whitespace)
    let has_line_folding = input.windows(3).any(|w| 
        (w[0] == b'\r' && w[1] == b'\n' && (w[2] == b' ' || w[2] == b'\t')) ||
        (w[0] == b'\n' && (w[1] == b' ' || w[1] == b'\t'))
    );
    
    // Only process line folding if it exists, otherwise use input directly
    let input_for_parsing = if has_line_folding {
        unfold_lws(input)
    } else {
        input.to_vec()
    };
    
    // First check if the input contains spaces within tokens (not just between tokens)
    if input_for_parsing.contains(&b' ') && 
       !input_for_parsing.starts_with(&[b' ']) && 
       !input_for_parsing.ends_with(&[b' ']) {
        let parts: Vec<&[u8]> = input_for_parsing.split(|&b| b == b',').collect();
        for part in parts {
            let trimmed = part.iter().skip_while(|&&b| b == b' ')
                             .take_while(|&&b| b != b' ')
                             .count();
            // If we find a space in the middle of a token, reject
            if trimmed < part.iter().filter(|&&b| b != b' ').count() {
                return Err(NomErr::Error(make_error(input, ErrorKind::Verify)));
            }
        }
    }
    
    // Use a separate variable to store the result before modifying remaining
    let parsing_result = comma_separated_list1(token_string)(&input_for_parsing);
    
    // Now handle the result, mapping any errors to the original input
    match parsing_result {
        Ok((remaining, tags)) => {
            // Verify that we don't have trailing commas
            if !remaining.is_empty() && remaining[0] == b',' {
                return Err(NomErr::Error(make_error(input, ErrorKind::TakeWhile1)));
            }
            
            // Verify that there are no empty elements like "tag1,,tag2"
            if !verify_no_empty_elements(&tags) {
                return Err(NomErr::Error(make_error(input, ErrorKind::Verify)));
            }
            
            // Map the remaining back to the original input position
            // The simplest way is to just return an empty slice from the original input
            // This is a bit of a hack but it should work for our use case
            let consumed_len = input.len() - remaining.len().min(input.len());
            Ok((&input[consumed_len..], tags))
        },
        Err(e) => Err(e.map_input(|_| input)),
    }
}

/// Parse a complete Unsupported header, including the header name
/// 
/// Format: "Unsupported" HCOLON option-tag *(COMMA option-tag)
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::parser::headers::unsupported::unsupported_header;
/// let input = b"Unsupported: timer, path";
/// let (_, option_tags) = unsupported_header(input).unwrap();
/// assert_eq!(option_tags, vec!["timer".to_string(), "path".to_string()]);
/// ```
pub fn unsupported_header(input: &[u8]) -> IResult<&[u8], Vec<String>> {
    preceded(
        terminated(
            tag_no_case(b"Unsupported"),
            hcolon
        ),
        parse_unsupported
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::ErrorKind;

    #[test]
    fn test_parse_unsupported_basic() {
        let (_, result) = parse_unsupported(b"timer, path").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_empty() {
        let result = parse_unsupported(b"");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unsupported_single_tag() {
        let (_, result) = parse_unsupported(b"timer").unwrap();
        assert_eq!(result, vec!["timer".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_whitespace() {
        let (_, result) = parse_unsupported(b"timer , path ,  100rel").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string(), "100rel".to_string()]);
    }
    
    #[test]
    fn test_parse_unsupported_special_chars() {
        // RFC 3261 defines token as 1*(alphanum / "-" / "." / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~" )
        let (_, result) = parse_unsupported(b"timer-v2.1").unwrap();
        assert_eq!(result, vec!["timer-v2.1".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_special_chars_multiple() {
        let (_, result) = parse_unsupported(b"timer-v2.1, path!_%*+`'~").unwrap();
        assert_eq!(result, vec!["timer-v2.1".to_string(), "path!_%*+`'~".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_trailing_comma() {
        // A trailing comma is not valid according to the RFC 3261 ABNF
        let result = parse_unsupported(b"timer, path,");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unsupported_empty_element() {
        // Empty elements like in "timer,,path" are not valid
        let result = parse_unsupported(b"timer,,path");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unsupported_invalid_chars() {
        // Space within a token is not valid
        let result = parse_unsupported(b"timer foo");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unsupported_case_insensitive() {
        // Option tags are case-sensitive according to RFC 3261
        let (_, result) = parse_unsupported(b"Timer, PATH").unwrap();
        assert_eq!(result, vec!["Timer".to_string(), "PATH".to_string()]);
        
        // These should not match "timer" and "path" in case-sensitive comparison
        assert_ne!(result, vec!["timer".to_string(), "path".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_header_whitespace() {
        let (_, result) = unsupported_header(b"Unsupported: timer, path").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string()]);
        
        // Test with extra whitespace
        let (_, result) = unsupported_header(b"Unsupported:  timer , path ").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_header_case_insensitive() {
        // Header name should be case-insensitive
        let (_, result) = unsupported_header(b"UNSUPPORTED: timer, path").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string()]);
        
        let (_, result) = unsupported_header(b"unsupported: timer, path").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_header_line_folding() {
        // RFC 3261 allows line folding with CRLF followed by whitespace
        let (_, result) = unsupported_header(b"Unsupported: timer,\r\n path").unwrap();
        assert_eq!(result, vec!["timer".to_string(), "path".to_string()]);
    }

    #[test]
    fn test_parse_unsupported_all_consuming() {
        // The all_consuming combinator should reject input with trailing data
        let result = all_consuming(parse_unsupported)(b"timer, path extra");
        assert!(result.is_err());
    }
} 