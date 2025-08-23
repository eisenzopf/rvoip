// RFC 3903 Section 5.2 - SIP-If-Match Header Parser
//
// SIP-If-Match = "SIP-If-Match" HCOLON entity-tag
// entity-tag = token

use crate::parser::token::token;
use crate::parser::whitespace::sws;
use crate::parser::ParseResult;
use crate::types::sip_if_match::SipIfMatch;
use nom::combinator::{map, opt};
use nom::sequence::terminated;

/// Parse a SIP-If-Match header value
///
/// # ABNF
/// ```abnf
/// SIP-If-Match = "SIP-If-Match" HCOLON entity-tag
/// entity-tag = token
/// ```
///
/// # Example
/// ```text
/// SIP-If-Match: dx200xyz
/// ```
pub fn parse_sip_if_match(input: &[u8]) -> ParseResult<SipIfMatch> {
    // Parse any leading whitespace
    let (input, _) = sws(input)?;
    
    // Parse the entity tag as a token
    let (input, tag_bytes) = token(input)?;
    
    // Convert to string
    let tag = std::str::from_utf8(tag_bytes)
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Char)))?
        .to_string();
    
    // Parse any trailing whitespace
    let (input, _) = sws(input)?;
    
    Ok((input, SipIfMatch::new(tag)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sip_if_match_simple() {
        let input = b"dx200xyz";
        let (rem, if_match) = parse_sip_if_match(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(if_match.tag(), "dx200xyz");
    }
    
    #[test]
    fn test_parse_sip_if_match_with_spaces() {
        let input = b"  abc123  ";
        let (rem, if_match) = parse_sip_if_match(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(if_match.tag(), "abc123");
    }
    
    #[test]
    fn test_parse_sip_if_match_alphanumeric() {
        let input = b"Tag-123.456_xyz";
        let (rem, if_match) = parse_sip_if_match(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(if_match.tag(), "Tag-123.456_xyz");
    }
    
    #[test]
    fn test_parse_sip_if_match_empty_fails() {
        let input = b"";
        assert!(parse_sip_if_match(input).is_err());
    }
}