// Parser for Proxy-Require header (RFC 3261 Section 20.29)
//
// The Proxy-Require header field is used to indicate proxy-sensitive features
// that MUST be supported by the proxy. Any request containing a Proxy-Require
// header field with one or more option-tags that the proxy does not understand
// MUST be rejected with a 420 (Bad Extension) response.
//
// ABNF:
// Proxy-Require = "Proxy-Require" HCOLON option-tag *(COMMA option-tag)
// option-tag = token
//
// Example:
// Proxy-Require: sec-agree, precondition

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::space0,
    combinator::{map, map_res, verify, not, fail, recognize, all_consuming, peek},
    multi::separated_list1,
    sequence::{preceded, terminated, delimited},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::comma;
use crate::parser::token::token;
use crate::parser::whitespace::{sws, lws, owsp};
use crate::parser::ParseResult;

/// Parses an option-tag (token) with surrounding whitespace
/// 
/// RFC 3261 defines option-tag as a token, which cannot contain whitespace
/// This function ensures we only parse valid tokens and strips any surrounding whitespace
fn option_tag(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(
        sws,
        token,
        sws
    )(input)
}

/// Parses an option-tag and converts it to a String
fn option_tag_string(input: &[u8]) -> ParseResult<String> {
    map_res(
        option_tag,
        |tag| str::from_utf8(tag).map(String::from)
    )(input)
}

/// Custom implementation of comma-separated-list for option-tags
/// 
/// This implementation strictly follows RFC 3261 ABNF:
/// - Ensures at least one option-tag
/// - Ensures no empty elements (e.g., rejects "foo,,bar")
/// - Handles whitespace around tokens and commas
fn option_tag_list(input: &[u8]) -> ParseResult<Vec<String>> {
    // Parse first token (required)
    let (mut remaining, first_tag) = option_tag_string(input)?;
    let mut tags = vec![first_tag];
    
    // Parse remaining tokens
    loop {
        // Try to match a comma followed by a tag
        match delimited(
            sws,
            tag(b","),
            sws
        )(remaining) {
            Ok((after_comma, _)) => {
                // After a comma, we must have a tag (not another comma)
                // This prevents "foo,,bar" from being accepted
                match option_tag_string(after_comma) {
                    Ok((new_remaining, next_tag)) => {
                        tags.push(next_tag);
                        remaining = new_remaining;
                    },
                    Err(_) => {
                        // If we find a comma but no valid tag after it, it's an error
                        return Err(nom::Err::Error(nom::error::Error::new(
                            after_comma,
                            nom::error::ErrorKind::Tag
                        )));
                    }
                }
            },
            Err(_) => {
                // No more commas, we're done
                break;
            }
        }
    }
    
    // Final check: ensure there's no remaining input
    if !remaining.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            remaining,
            nom::error::ErrorKind::Verify
        )));
    }
    
    Ok((remaining, tags))
}

/// Parses the value of a Proxy-Require header.
///
/// This parser handles the part after "Proxy-Require:" in the SIP message.
/// The header name and HCOLON are handled by the top-level message_header parser.
///
/// The Proxy-Require header field is used to indicate proxy-sensitive features
/// that MUST be supported by the proxy. The header value contains a list of
/// option-tags, which are tokens representing SIP extensions.
///
/// Returns a vector of option-tags as strings.
pub fn parse_proxy_require(input: &[u8]) -> ParseResult<Vec<String>> {
    // Use our custom option_tag_list parser
    option_tag_list(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::combinator::all_consuming;
    use nom::error::ErrorKind;

    #[test]
    fn test_parse_proxy_require() {
        let input = b"sec-agree";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["sec-agree"]);

        let input_multi = b"foo , bar";
        let (rem_multi, preq_multi) = parse_proxy_require(input_multi).unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(preq_multi, vec!["foo", "bar"]);
    }

    #[test]
    fn test_parse_proxy_require_empty_fail() {
        assert!(parse_proxy_require(b"").is_err());
    }
    
    #[test]
    fn test_parse_proxy_require_case_sensitivity() {
        // RFC 3261 Section 20.29: option-tags are case-sensitive
        let input = b"SEC-AGREE, sec-agree";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["SEC-AGREE", "sec-agree"]);
        assert_ne!(preq_list[0], preq_list[1]);
    }
    
    #[test]
    fn test_parse_proxy_require_with_whitespace() {
        // Test with various whitespace patterns
        let input = b"  foo  ,  bar  ,  baz  ";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["foo", "bar", "baz"]);
    }
    
    #[test]
    fn test_parse_proxy_require_with_line_folding() {
        // Test with line folding (CRLF + WSP)
        let input = b"foo,\r\n bar";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["foo", "bar"]);
    }
    
    #[test]
    fn test_parse_proxy_require_with_multiple_tags() {
        // Test with multiple option-tags
        let input = b"foo,bar,baz,qux,quux,quuz";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["foo", "bar", "baz", "qux", "quux", "quuz"]);
    }
    
    #[test]
    fn test_parse_proxy_require_with_special_chars() {
        // Test with special characters allowed in tokens
        let input = b"foo-bar, bar.baz, baz+qux, qux_quux, quux!quuz";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["foo-bar", "bar.baz", "baz+qux", "qux_quux", "quux!quuz"]);
    }
    
    #[test]
    fn test_parse_proxy_require_invalid_tokens() {
        // Test with invalid token characters
        // Direct token testing without using our parser (which might handle whitespace differently)
        assert!(all_consuming(token)(b"foo bar").is_err());
        
        // Test through our proxy require parser
        let result = parse_proxy_require(b"foo bar");
        assert!(result.is_err());
        
        // Test other invalid characters
        assert!(parse_proxy_require(b"foo@bar").is_err());
        assert!(parse_proxy_require(b"foo\"bar").is_err());
        assert!(parse_proxy_require(b"foo(bar").is_err());
    }
    
    #[test]
    fn test_parse_proxy_require_uncommon_tokens() {
        // Test with some uncommon but valid token characters
        let input = b"method.1, %method, ~method, '123";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["method.1", "%method", "~method", "'123"]);
    }
    
    #[test]
    fn test_parse_proxy_require_rfc_examples() {
        // Example from RFC 3261 Section 20.29 (adapted)
        let input = b"sec-agree, precondition";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["sec-agree", "precondition"]);
    }
    
    #[test]
    fn test_parse_proxy_require_real_world_tags() {
        // Test with some real-world SIP extensions
        let input = b"timer, 100rel, path, gruu, outbound";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["timer", "100rel", "path", "gruu", "outbound"]);
    }
    
    #[test]
    fn test_abnf_compliance() {
        // Test various combinations to ensure ABNF compliance
        
        // Multiple commas should fail (no empty option-tags allowed)
        assert!(parse_proxy_require(b"foo,,bar").is_err());
        
        // Trailing comma should fail
        assert!(parse_proxy_require(b"foo,").is_err());
        
        // Leading comma should fail
        assert!(parse_proxy_require(b",foo").is_err());
        
        // Lone comma should fail
        assert!(parse_proxy_require(b",").is_err());
        
        // Whitespace-only should fail
        assert!(parse_proxy_require(b"  ").is_err());
        
        // Empty quoted string should fail (quoted strings are not tokens)
        assert!(parse_proxy_require(b"\"\"").is_err());
    }
} 