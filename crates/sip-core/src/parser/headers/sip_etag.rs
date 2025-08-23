// RFC 3903 Section 5.1 - SIP-ETag Header Parser
//
// SIP-ETag = "SIP-ETag" HCOLON entity-tag
// entity-tag = token

use crate::parser::token::token;
use crate::parser::whitespace::sws;
use crate::parser::ParseResult;
use crate::types::sip_etag::SipETag;
use nom::combinator::{map, opt};
use nom::sequence::terminated;

/// Parse a SIP-ETag header value
///
/// # ABNF
/// ```abnf
/// SIP-ETag = "SIP-ETag" HCOLON entity-tag
/// entity-tag = token
/// ```
///
/// # Example
/// ```text
/// SIP-ETag: dx200xyz
/// ```
pub fn parse_sip_etag(input: &[u8]) -> ParseResult<SipETag> {
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
    
    Ok((input, SipETag::new(tag)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sip_etag_simple() {
        let input = b"dx200xyz";
        let (rem, etag) = parse_sip_etag(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(etag.tag(), "dx200xyz");
    }
    
    #[test]
    fn test_parse_sip_etag_with_spaces() {
        let input = b"  abc123  ";
        let (rem, etag) = parse_sip_etag(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(etag.tag(), "abc123");
    }
    
    #[test]
    fn test_parse_sip_etag_alphanumeric() {
        let input = b"Tag-123.456_xyz";
        let (rem, etag) = parse_sip_etag(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(etag.tag(), "Tag-123.456_xyz");
    }
    
    #[test]
    fn test_parse_sip_etag_empty_fails() {
        let input = b"";
        assert!(parse_sip_etag(input).is_err());
    }
}