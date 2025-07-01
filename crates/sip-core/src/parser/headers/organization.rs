// Parser for Organization header (RFC 3261 Section 20.27)
// Organization = "Organization" HCOLON [TEXT-UTF8-TRIM]

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, opt, map_res, recognize},
    sequence::{pair, preceded, delimited},
    IResult,
};

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::values::text_utf8_trim;
use crate::parser::whitespace::{sws, lws, owsp};
use crate::parser::ParseResult;
use crate::parser::utils::unfold_lws;
use crate::types::Organization;
use std::str;

/// Parses the Organization header value according to RFC 3261 Section 20.27
/// Organization = "Organization" HCOLON [TEXT-UTF8-TRIM]
///
/// Note: This parser handles only the value part ([TEXT-UTF8-TRIM]).
/// The "Organization" token and HCOLON are parsed separately.
/// 
/// The value is optional - an empty value is valid per the RFC.
/// Handles whitespace and line folding according to RFC 3261 Section 7.3.1.
pub fn parse_organization(input: &[u8]) -> ParseResult<Organization> {
    // Handle the empty case first
    if input.is_empty() {
        return Ok((input, Organization::new("")));
    }
    
    // Check if input is all whitespace
    let (remaining, whitespace) = owsp(input)?;
    if whitespace.len() == input.len() {
        // Input is all whitespace, treat as empty Organization
        return Ok((remaining, Organization::new("")));
    }
    
    // Check for semicolon which might indicate parameters
    if let Some(pos) = input.iter().position(|&b| b == b';') {
        let (text_part, params_part) = input.split_at(pos);
        
        // Parse the text part
        let (_, org) = delimited(
            sws,
            map(
                opt(map_res(
                    text_utf8_trim,
                    |bytes| {
                        // Apply line unfolding (returns Vec<u8>)
                        let unfolded = unfold_lws(bytes);
                        // Convert to UTF-8 string
                        let s = String::from_utf8(unfolded)
                            .map_err(|_| nom::Err::Error(nom::error::Error::new(bytes, nom::error::ErrorKind::Char)))?;
                        Ok::<String, nom::Err<nom::error::Error<&[u8]>>>(s)
                    }
                )),
                |opt_val| Organization::new(opt_val.unwrap_or_default())
            ),
            sws
        )(text_part)?;
        
        return Ok((params_part, org));
    }
    
    // Otherwise, parse without worrying about semicolons
    delimited(
        sws,
        map(
            opt(map_res(
                text_utf8_trim,
                |bytes| {
                    // Apply line unfolding (returns Vec<u8>)
                    let unfolded = unfold_lws(bytes);
                    // Convert to UTF-8 string
                    let s = String::from_utf8(unfolded)
                        .map_err(|_| nom::Err::Error(nom::error::Error::new(bytes, nom::error::ErrorKind::Char)))?;
                    Ok::<String, nom::Err<nom::error::Error<&[u8]>>>(s)
                }
            )),
            |opt_val| Organization::new(opt_val.unwrap_or_default())
        ),
        sws
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_organization() {
        let input = b"Example Org";
        let (rem, val) = parse_organization(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Organization::new("Example Org"));

        // With internal LWS
        let input_lws = b"Some \t Company, Inc.";
        let (rem_lws, val_lws) = parse_organization(input_lws).unwrap();
        assert!(rem_lws.is_empty());
        assert_eq!(val_lws, Organization::new("Some Company, Inc."));  // Note: internal whitespace compressed
    }

    #[test]
    fn test_parse_organization_empty() {
        // If the header value is empty, it should parse successfully
        // as RFC 3261 marks the TEXT-UTF8-TRIM as optional with [...]
        let input = b""; 
        let (rem, val) = parse_organization(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Organization::new(""));
        
        // Also test with just whitespace - should be treated as empty
        let input_ws = b"  \t  ";
        let (rem_ws, val_ws) = parse_organization(input_ws).unwrap();
        assert!(rem_ws.is_empty());
        assert_eq!(val_ws, Organization::new(""));
        
        // Test more complex whitespace patterns
        let complex_whitespace_tests = [
            &b"   "[..],            // Multiple spaces
            &b"\t\t\t"[..],         // Multiple tabs
            &b" \t \t "[..],        // Mixed spaces and tabs
            &b"\r\n "[..],          // CRLF followed by space (line folding)
            &b" \r\n \t"[..],       // Space, CRLF, space, tab
            &b"\r\n \r\n \t"[..],   // Multiple line folding with whitespace
        ];
        
        for test_case in &complex_whitespace_tests {
            let (rem, val) = parse_organization(test_case).unwrap();
            assert!(rem.is_empty(), "Failed to consume all input for {:?}", test_case);
            assert_eq!(val, Organization::new(""), "Whitespace input not treated as empty Organization for {:?}", test_case);
        }
    }
    
    #[test]
    fn test_line_folding() {
        // RFC 3261 Section 7.3.1 requires handling of folded lines
        // "a\r\n b" is equivalent to "a b"
        let input_folded = b"Example\r\n Organization";
        let (rem, val) = parse_organization(input_folded).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Organization::new("Example Organization"));
        
        // Multiple line folding
        let input_multi_fold = b"Big\r\n Company\r\n Inc.";
        let (rem_multi, val_multi) = parse_organization(input_multi_fold).unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(val_multi, Organization::new("Big Company Inc."));
    }
    
    #[test]
    fn test_utf8_characters() {
        // Organization header can contain UTF-8 characters (TEXT-UTF8-TRIM)
        let input_utf8 = b"Acme \xc3\x9c\xc3\x96 GmbH"; // Acme ÜÖ GmbH
        let (rem, val) = parse_organization(input_utf8).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Organization::new("Acme ÜÖ GmbH"));
        
        // Additional UTF-8 test with various characters
        let input_utf8_mix = b"\xe4\xbc\x81\xe6\xa5\xad"; // 企業 (Japanese for "enterprise")
        let (rem_mix, val_mix) = parse_organization(input_utf8_mix).unwrap();
        assert!(rem_mix.is_empty());
        assert_eq!(val_mix, Organization::new("企業"));
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Leading and trailing whitespace should be properly handled
        let input_ws = b"  Example Corp  ";
        let (rem, val) = parse_organization(input_ws).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Organization::new("Example Corp"));
        
        // Mix of spaces and tabs
        let input_mixed_ws = b" \t Example \t Corp \t ";
        let (rem_mixed, val_mixed) = parse_organization(input_mixed_ws).unwrap();
        assert!(rem_mixed.is_empty());
        assert_eq!(val_mixed, Organization::new("Example Corp"));  // Note: internal whitespace compressed
    }
    
    #[test]
    fn test_remaining_input() {
        // Test that the parser correctly handles remaining input
        let input_rem = b"Example Corp;param=value";
        let (rem, org) = parse_organization(input_rem).unwrap();
        assert_eq!(org, Organization::new("Example Corp"));
        
        // With our current implementation, the delimiter is recognized and separated
        assert_eq!(rem, b";param=value");
    }
    
    #[test]
    fn test_rfc_examples() {
        // While RFC 3261 doesn't have specific examples for Organization,
        // we can test reasonable examples that would be RFC compliant
        
        let examples = [
            &b"Cisco Systems, Inc."[..],
            &b"IETF"[..],
            &b"3Com"[..],
            &b""[..],  // Empty is valid
            &b"  "[..], // Whitespace only is equivalent to empty
        ];
        
        for example in &examples {
            // Just verify that parsing succeeds
            assert!(parse_organization(example).is_ok());
        }
    }
    
    #[test]
    fn test_abnf_compliance() {
        // ABNF defines Organization = "Organization" HCOLON [TEXT-UTF8-TRIM]
        // TEXT-UTF8-TRIM = 1*TEXT-UTF8char *(*LWS TEXT-UTF8char)
        
        // Test complex case with whitespace and UTF-8
        let abnf_test = b"Company \xc2\xa9 2023 \r\n Incorporated";
        let (rem, val) = parse_organization(abnf_test).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Organization::new("Company © 2023 Incorporated"));
    }
} 