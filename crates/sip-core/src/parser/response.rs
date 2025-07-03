use std::str;
use std::str::FromStr;
use nom::{
    branch::alt,
    bytes::complete::{take_till, take_while_m_n, take_till1},
    character::complete::{digit1, space1, line_ending},
    combinator::{map, map_res, recognize, opt, all_consuming},
    sequence::{tuple, preceded, terminated},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
// Keep Result for FromStr impls if needed elsewhere
use crate::error::{Error, Result};
use crate::types::version::Version;
use crate::types::StatusCode;
use crate::parser::common::sip_version;
use crate::parser::whitespace::crlf;
use crate::parser::ParseResult;

/// Parser for SIP response status line (RFC 3261 Section 7.2)
///
/// ABNF Grammar:
/// Status-Line =  SIP-Version SP Status-Code SP Reason-Phrase CRLF
/// Status-Code =  3DIGIT
/// Reason-Phrase  =  *(reserved / unreserved / escaped / UTF8-NONASCII / UTF8-CONT / SP / HTAB)
///
/// Parser for a SIP response line
/// Returns components needed by IncrementalParser
pub fn parse_response_line(input: &str) -> IResult<&str, (Version, StatusCode, String)> {
    let (input, version) = map_res(
        take_till(|c| c == ' '),
        |s: &str| Version::from_str(s)
    )(input)?;

    let (input, _) = space1(input)?;

    let (input, status_code) = map_res(
        digit1,
        |s: &str| s.parse::<u16>()
    )(input)?;

    let status = match StatusCode::from_u16(status_code) {
        Ok(status) => status,
        // Use Failure for semantic errors, match nom::error::Error structure
        Err(_) => return Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify))), 
    };

    let (input, _) = space1(input)?;

    let (input, reason) = map(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| s.to_string()
    )(input)?;

    // Consume the line ending
    let (input, _) = line_ending(input)?;

    Ok((input, (version, status, reason)))
} 

// Status-Code = 3DIGIT
pub fn status_code(input: &[u8]) -> ParseResult<StatusCode> {
    map_res(
        digit1,
        |code_bytes: &[u8]| -> Result<StatusCode> {
            if code_bytes.len() != 3 {
                return Err(Error::ParseError("Status code must be 3 digits".to_string()));
            }
            let s = str::from_utf8(code_bytes)?;
            let code = s.parse::<u16>()
                .map_err(|e| Error::ParseError(format!("Invalid status code digit: {}", e)))?;
                
            StatusCode::from_u16(code)
                .map_err(|_| Error::InvalidStatusCode(code))
        }
    )(input)
}

// Reason-Phrase = *(reserved / unreserved / escaped / UTF8-NONASCII / UTF8-CONT / SP / HTAB)
// Simplified: take bytes until CRLF
pub fn reason_phrase(input: &[u8]) -> ParseResult<&[u8]> {
    // The reason phrase can be empty, so we use take_till instead of take_till1
    take_till(|c| c == b'\r' || c == b'\n')(input)
}

// Status-Line = SIP-Version SP Status-Code SP Reason-Phrase CRLF
pub fn parse_status_line(input: &[u8]) -> ParseResult<(Version, StatusCode, &[u8])> {
    terminated(
        tuple((
            terminated(sip_version, space1),
            terminated(status_code, space1),
            reason_phrase
        )),
        crlf
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_code() {
        assert_eq!(status_code(b"200 OK"), Ok((&b" OK"[..], StatusCode::Ok)));
        assert_eq!(status_code(b"404 Not Found"), Ok((&b" Not Found"[..], StatusCode::NotFound)));
        assert_eq!(status_code(b"183"), Ok((&[][..], StatusCode::SessionProgress)));
        assert!(status_code(b"20").is_err());
        assert!(status_code(b"2000").is_err());
        assert!(status_code(b"ABC").is_err());
        assert!(status_code(b"603").is_ok()); // Assuming 603 Decline is defined
    }

    #[test]
    fn test_reason_phrase() {
        assert_eq!(reason_phrase(b"OK\r\n"), Ok((&b"\r\n"[..], &b"OK"[..])));
        assert_eq!(reason_phrase(b"Not Found\r\nMore"), Ok((&b"\r\nMore"[..], &b"Not Found"[..])));
        assert_eq!(reason_phrase(b"Session Progress (Early Media)\r\n"), Ok((&b"\r\n"[..], &b"Session Progress (Early Media)"[..])));
        assert_eq!(reason_phrase(b"\r\n"), Ok((&b"\r\n"[..], &b""[..])));
    }

    #[test]
    fn test_parse_status_line() {
        let line = b"SIP/2.0 200 OK\r\n";
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (rem, (version, status, reason)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(version, Version::new(2, 0));
        assert_eq!(status, StatusCode::Ok);
        assert_eq!(reason, b"OK");
    }

    #[test]
    fn test_parse_status_line_404() {
        let line = b"SIP/2.0 404 Not Found\r\n";
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (rem, (_, status, reason)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(status, StatusCode::NotFound);
        assert_eq!(reason, b"Not Found");
    }

    #[test]
    fn test_parse_status_line_empty_reason() {
        let line = b"SIP/1.0 501 \r\n"; // Note space before CRLF
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (rem, (_, status, reason)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(status, StatusCode::NotImplemented);
        assert_eq!(reason, b""); // Empty reason phrase (after trimming trailing space)
    }
    
    #[test]
    fn test_invalid_status_line_version() {
        let line = b"HTTP/1.1 200 OK\r\n";
        assert!(parse_status_line(line).is_err());
    }

    #[test]
    fn test_invalid_status_line_code() {
        let line = b"SIP/2.0 20 OK\r\n";
        assert!(parse_status_line(line).is_err());
        let line = b"SIP/2.0 2000 OK\r\n";
        assert!(parse_status_line(line).is_err());
        let line = b"SIP/2.0 ABC OK\r\n";
        assert!(parse_status_line(line).is_err());
    }

    #[test]
    fn test_invalid_status_line_spacing() {
        let line = b"SIP/2.0 200OK\r\n";
        assert!(parse_status_line(line).is_err());
        let line = b"SIP/2.0200 OK\r\n";
        assert!(parse_status_line(line).is_err());
    }

    #[test]
    fn test_invalid_status_line_crlf() {
        let line = b"SIP/2.0 200 OK";
        assert!(parse_status_line(line).is_err());
    }
    
    #[test]
    fn test_different_sip_versions() {
        let line = b"SIP/1.0 200 OK\r\n";
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (_, (version, _, _)) = result.unwrap();
        assert_eq!(version, Version::new(1, 0));
        
        let line = b"SIP/3.0 200 OK\r\n";
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (_, (version, _, _)) = result.unwrap();
        assert_eq!(version, Version::new(3, 0));
    }
    
    #[test]
    fn test_no_reason_phrase() {
        let line = b"SIP/2.0 200\r\n";
        assert!(parse_status_line(line).is_err()); // Must have space after status code
        
        let line = b"SIP/2.0 200 \r\n";
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (_, (_, _, reason)) = result.unwrap();
        assert_eq!(reason, b"");
    }
    
    #[test]
    fn test_utf8_characters_in_reason() {
        // Test with emoji and other UTF-8 characters
        let line = "SIP/2.0 200 OK 👍 UTF-8 français\r\n".as_bytes();
        let result = parse_status_line(line);
        assert!(result.is_ok());
        let (_, (_, _, reason)) = result.unwrap();
        assert_eq!(reason, "OK 👍 UTF-8 français".as_bytes());
    }
}

// Removed response_parser, response_parser_nom, parse_headers_and_body functions. 