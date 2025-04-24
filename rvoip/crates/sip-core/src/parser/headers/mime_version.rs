// Parser for MIME-Version header (RFC 3261 Section 20.26)
// MIME-Version = "MIME-Version" HCOLON 1*DIGIT "." 1*DIGIT

use nom::{
    bytes::complete as bytes,
    character::complete::{digit1, space0},
    combinator::{map_res, verify},
    sequence::{separated_pair, delimited},
    IResult,
    error::{ErrorKind, Error as NomError, ParseError},
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;
use crate::types::version::Version as MimeVersion; // Alias to avoid confusion
use crate::error::{Error, Result};

/// Parses the MIME-Version header value according to RFC 3261 Section 20.26
/// MIME-Version = "MIME-Version" HCOLON 1*DIGIT "." 1*DIGIT
///
/// Note: This parser handles only the value part (1*DIGIT "." 1*DIGIT).
/// The "MIME-Version" token and HCOLON are parsed separately.
pub fn parse_mime_version(input: &[u8]) -> ParseResult<MimeVersion> {
    // Wrap the parser in delimited() to handle optional whitespace
    delimited(
        space0,
        // Parse the actual version 
        map_res(
            separated_pair(digit1, bytes::tag(b"."), digit1),
            |(major_bytes, minor_bytes)| {
                // Parse major version
                let major_str = str::from_utf8(major_bytes)
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(major_bytes, ErrorKind::Char)))?;
                let major = major_str.parse::<u32>()
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(major_bytes, ErrorKind::Digit)))?;
                
                // Parse minor version
                let minor_str = str::from_utf8(minor_bytes)
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(minor_bytes, ErrorKind::Char)))?;
                let minor = minor_str.parse::<u32>()
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(minor_bytes, ErrorKind::Digit)))?;
                
                // Convert u32 to u8 with error handling
                let major_u8 = u8::try_from(major)
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(major_bytes, ErrorKind::TooLarge)))?;
                let minor_u8 = u8::try_from(minor)
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(minor_bytes, ErrorKind::TooLarge)))?;
                
                Ok::<MimeVersion, nom::Err<NomError<&[u8]>>>(MimeVersion::new(major_u8, minor_u8))
            }
        ),
        space0
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mime_version() {
        // Standard case from RFC 3261
        let (rem, val) = parse_mime_version(b"1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, MimeVersion::new(1, 0));

        // Multiple digits
        let (rem_multi, val_multi) = parse_mime_version(b"10.25").unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(val_multi, MimeVersion::new(10, 25));
        
        // With leading/trailing whitespace (HCOLON would normally handle this, but we're still testing)
        let (rem_ws, val_ws) = parse_mime_version(b" 1.0 ").unwrap();
        assert!(rem_ws.is_empty());
        assert_eq!(val_ws, MimeVersion::new(1, 0));
    }
    
    #[test]
    fn test_invalid_mime_version() {
        // Missing components
        assert!(parse_mime_version(b"1.").is_err());
        assert!(parse_mime_version(b".0").is_err());
        assert!(parse_mime_version(b"1").is_err());
        assert!(parse_mime_version(b"").is_err());
        
        // Non-digit characters
        assert!(parse_mime_version(b"a.b").is_err());
        assert!(parse_mime_version(b"1.b").is_err());
        assert!(parse_mime_version(b"a.0").is_err());
        
        // Invalid format
        assert!(parse_mime_version(b"1,0").is_err());  // Wrong separator
        assert!(parse_mime_version(b"1..0").is_err()); // Double dot
    }
    
    #[test]
    fn test_boundary_values() {
        // Maximum u8 values
        let (_, val_max) = parse_mime_version(b"255.255").unwrap();
        assert_eq!(val_max, MimeVersion::new(255, 255));
        
        // Overflow cases - beyond u8 max
        assert!(parse_mime_version(b"256.0").is_err());
        assert!(parse_mime_version(b"0.256").is_err());
        assert!(parse_mime_version(b"1000.0").is_err());
    }
    
    #[test]
    fn test_leading_zeros() {
        // Leading zeros should be valid per RFC (1*DIGIT allows any number of digits)
        let (_, val_leading_zeros) = parse_mime_version(b"01.00").unwrap();
        assert_eq!(val_leading_zeros, MimeVersion::new(1, 0));
        
        let (_, val_many_zeros) = parse_mime_version(b"0001.0002").unwrap();
        assert_eq!(val_many_zeros, MimeVersion::new(1, 2));
    }
    
    #[test]
    fn test_remaining_input() {
        // Parser should handle remaining input correctly
        let (rem, val) = parse_mime_version(b"1.0;param=value").unwrap();
        assert_eq!(rem, b";param=value");
        assert_eq!(val, MimeVersion::new(1, 0));
        
        // Trailing whitespace followed by other content
        let (rem_ws, val_ws) = parse_mime_version(b"1.0 ;param=value").unwrap();
        assert_eq!(rem_ws, b";param=value");
        assert_eq!(val_ws, MimeVersion::new(1, 0));
    }
    
    #[test]
    fn test_rfc3261_examples() {
        // RFC 3261 specifies MIME-Version: 1.0
        // Let's check examples from the specification
        let (_, val) = parse_mime_version(b"1.0").unwrap();
        assert_eq!(val, MimeVersion::new(1, 0));
    }
} 