// Parser for Subject header (RFC 3261 Section 20.38)
// Subject = ( "Subject" / "s" ) HCOLON [TEXT-UTF8-TRIM]

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, opt, map_res, all_consuming},
    sequence::{pair, preceded, terminated},
    IResult,
};
use std::str;

// Import from other modules
use crate::parser::separators::hcolon;
use crate::parser::values::text_utf8_trim;
use crate::parser::utils::unfold_lws;
use crate::parser::ParseResult;
use crate::types::subject::Subject;

/// Parses the value portion of a Subject header.
/// Returns a Subject type with the parsed text, or an empty Subject if no content.
/// This complies with the ABNF [TEXT-UTF8-TRIM] which is optional.
pub fn parse_subject(input: &[u8]) -> ParseResult<Subject> {
    // Handle the case where input is completely empty (which is valid for optional value)
    if input.is_empty() {
        return Ok((input, Subject::new("")));
    }
    
    // For Subject header, the entire input is considered valid text
    // Even if it's just whitespace, we'll process it as subject text
    // Apply UTF-8 validation and unfold any LWS according to RFC
    let unfolded = unfold_lws(input);
    
    // Convert to UTF-8 string and create a Subject
    match str::from_utf8(&unfolded) {
        Ok(text) => Ok((&input[input.len()..], Subject::new(text))),
        Err(_) => Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Char)))
    }
}

/// Parse a complete Subject header line including the header name and colon.
/// This handles both the standard "Subject:" form and the compact "s:" form.
pub fn parse_subject_header(input: &[u8]) -> ParseResult<Subject> {
    preceded(
        terminated(
            alt((
                tag_no_case(b"Subject"),
                tag_no_case(b"s")
            )),
            hcolon
        ),
        parse_subject
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::combinator::all_consuming;
    use crate::parser::utils::unfold_lws;

    // Test helper for fully consuming the input
    fn test_parse_subject(input: &[u8]) -> Result<Subject, nom::Err<nom::error::Error<&[u8]>>> {
        all_consuming(parse_subject)(input).map(|(_, output)| output)
    }

    #[test]
    fn test_parse_subject_normal() {
        // Standard text without special characters
        let input = b"Urgent Meeting Request";
        let result = test_parse_subject(input).unwrap();
        assert_eq!(result.text(), "Urgent Meeting Request");
    }

    #[test]
    fn test_parse_subject_empty() {
        // Empty subject is valid according to RFC 3261
        let input = b"";
        let result = test_parse_subject(input).unwrap();
        assert_eq!(result.text(), "");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_subject_whitespace() {
        // Subject with leading and trailing whitespace (should be preserved in the unfolded result)
        let input = b"  Whitespace Test  ";
        let result = parse_subject(input).unwrap().1;
        assert_eq!(result.text(), " Whitespace Test ");
    }

    #[test]
    fn test_parse_subject_utf8() {
        // Subject with UTF-8 characters
        let input = b"Meeting about \xE2\x82\xAC currency";  // € symbol
        let result = test_parse_subject(input).unwrap();
        assert_eq!(result.text(), "Meeting about € currency");
    }

    #[test]
    fn test_parse_subject_folded() {
        // Subject with line folding according to RFC 3261
        // "This is a\r\n folded subject" should be parsed as "This is a folded subject"
        let input = b"This is a\r\n folded subject";
        let unfolded_bytes = unfold_lws(input);
        let expected = str::from_utf8(&unfolded_bytes).unwrap();
        
        let result = parse_subject(input).unwrap().1;
        assert_eq!(result.text(), expected);
        assert_eq!(result.text(), "This is a folded subject");
    }

    #[test]
    fn test_parse_subject_complex_folding() {
        // Complex case with multiple foldings and whitespace
        let input = b"Multi\r\n line\r\n\tfolded\r\n  text";
        let unfolded_bytes = unfold_lws(input);
        let expected = str::from_utf8(&unfolded_bytes).unwrap();
        
        let result = parse_subject(input).unwrap().1;
        assert_eq!(result.text(), expected);
        assert_eq!(result.text(), "Multi line folded text");
    }

    #[test]
    fn test_parse_subject_header_standard() {
        // Full header with standard form
        let input = b"Subject: Important Message";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "Important Message");
    }

    #[test]
    fn test_parse_subject_header_compact() {
        // Full header with compact form
        let input = b"s: Important Message";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "Important Message");
    }

    #[test]
    fn test_parse_subject_header_case_insensitive() {
        // Header name should be case-insensitive
        let input = b"SUBJECT: Case Test";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "Case Test");
    }

    #[test]
    fn test_parse_subject_header_empty() {
        // Empty subject after header
        let input = b"Subject: ";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "");
        assert!(subject.is_empty());
    }

    #[test]
    fn test_parse_subject_header_folded() {
        // Folded subject after header
        let input = b"Subject: Folded\r\n line\r\n in header";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "Folded line in header");
    }

    #[test]
    fn test_parse_subject_header_utf8() {
        // UTF-8 characters in header
        let input = b"Subject: UTF-8 \xE2\x9C\x93 test"; // ✓ symbol
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "UTF-8 ✓ test");
    }

    #[test]
    fn test_rfc3261_examples() {
        // Examples from RFC 3261
        let input = b"Subject: Need more boxes";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "Need more boxes");
        
        // RFC example with compact form
        let input = b"s: Call me back";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "Call me back");
    }

    #[test]
    fn test_rfc4475_examples() {
        // Torture test cases from RFC 4475
        
        // Complex folding with extra whitespace
        let input = b"Subject: A \r\n  folded  \r\n   header\r\n   value";
        let (_, subject) = parse_subject_header(input).unwrap();
        assert_eq!(subject.text(), "A folded header value");
    }
} 