// Parser for the Reason header as defined in RFC 3326
// Reason = "Reason" HCOLON protocol *(SEMI reason-param)
// protocol = "SIP" / "Q.850" / token
// reason-param = "cause" EQUAL cause / "text" EQUAL quoted-string / generic-param
// cause = 1*DIGIT

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, tag, take_while1},
    character::complete::digit1,
    combinator::{map, map_res, opt},
    multi::many0,
    sequence::{preceded, tuple, pair},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{semi, equal};
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::common_params::generic_param;
use crate::parser::ParseResult;

// Import the Reason type from types
use crate::types::reason::Reason;
use crate::types::param::Param;

// Define parser for the protocol field
fn protocol(input: &[u8]) -> ParseResult<String> {
    map_res(
        take_while1(|c| {
            // Token characters per RFC 3261
            c > 32 && c < 127 && !b"()<>@,;:\\\"/[]?={} \t".contains(&c)
        }),
        |b: &[u8]| {
            str::from_utf8(b)
                .map(|s| s.to_string())
                .map_err(|_| nom::Err::Error(NomError::new(b, ErrorKind::AlphaNumeric)))
        }
    )(input)
}

// Define parser for the cause parameter
fn cause_param(input: &[u8]) -> ParseResult<u16> {
    preceded(
        pair(tag_no_case(b"cause"), equal),
        map_res(
            digit1,
            |b: &[u8]| {
                str::from_utf8(b)
                    .map_err(|_| nom::Err::Error(NomError::new(b, ErrorKind::Digit)))
                    .and_then(|s| s.parse::<u16>().map_err(|_| nom::Err::Error(NomError::new(b, ErrorKind::Digit))))
            }
        )
    )(input)
}

// Define parser for the text parameter
fn text_param(input: &[u8]) -> ParseResult<String> {
    preceded(
        pair(tag_no_case(b"text"), equal),
        map_res(
            quoted_string,
            |b: &[u8]| {
                str::from_utf8(b)
                    .map(|s| s.to_string())
                    .map_err(|_| nom::Err::Error(NomError::new(b, ErrorKind::AlphaNumeric)))
            }
        )
    )(input)
}

// Define parser for reason parameters (cause, text, or generic)
fn reason_param(input: &[u8]) -> ParseResult<ReasonParam> {
    alt((
        map(cause_param, ReasonParam::Cause),
        map(text_param, ReasonParam::Text),
        map(generic_param, ReasonParam::Generic)
    ))(input)
}

// Define enum for reason parameters
#[derive(Debug, PartialEq, Clone)]
enum ReasonParam {
    Cause(u16),
    Text(String),
    Generic(Param),
}

/// Parses a Reason header value as defined in RFC 3326.
/// 
/// # Example
/// 
/// ```
/// use rvoip_sip_core::parser::headers::parse_reason;
/// 
/// let input = b"SIP ;cause=200 ;text=\"Call completed elsewhere\"";
/// let result = parse_reason(input);
/// assert!(result.is_ok());
/// ```
pub fn parse_reason(input: &[u8]) -> ParseResult<Reason> {
    map_res(
        tuple((
            protocol,
            many0(preceded(semi, reason_param))
        )),
        |(protocol_str, params)| {
            let mut cause: Option<u16> = None;
            let mut text: Option<String> = None;
            
            // Extract cause and text from parameters
            for param in params {
                match param {
                    ReasonParam::Cause(c) => cause = Some(c),
                    ReasonParam::Text(t) => text = Some(t),
                    ReasonParam::Generic(_) => (), // Ignore generic parameters
                }
            }
            
            // The cause parameter is mandatory according to RFC 3326
            match cause {
                Some(c) => Ok(Reason::new(protocol_str, c, text)),
                None => Err(nom::Err::Error(NomError::new(input, ErrorKind::Digit)))
            }
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_protocol_parser() {
        let (rem, protocol_str) = protocol(b"SIP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(protocol_str, "SIP");
        
        let (rem, protocol_str) = protocol(b"Q.850").unwrap();
        assert!(rem.is_empty());
        assert_eq!(protocol_str, "Q.850");
        
        let (rem, protocol_str) = protocol(b"CUSTOM").unwrap();
        assert!(rem.is_empty());
        assert_eq!(protocol_str, "CUSTOM");
    }
    
    #[test]
    fn test_cause_param_parser() {
        let (rem, cause) = cause_param(b"cause=200").unwrap();
        assert!(rem.is_empty());
        assert_eq!(cause, 200);
        
        let (rem, cause) = cause_param(b"cause=486").unwrap();
        assert!(rem.is_empty());
        assert_eq!(cause, 486);
        
        let result = cause_param(b"cause=abc");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_text_param_parser() {
        let (rem, text) = text_param(b"text=\"Call completed elsewhere\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(text, "Call completed elsewhere");
        
        let (rem, text) = text_param(b"text=\"Busy Here\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(text, "Busy Here");
    }
    
    #[test]
    fn test_reason_param_parser() {
        let (rem, param) = reason_param(b"cause=200").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, ReasonParam::Cause(200)));
        
        let (rem, param) = reason_param(b"text=\"Busy Here\"").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, ReasonParam::Text(t) if t == "Busy Here"));
        
        // Test generic parameter
        let (rem, param) = reason_param(b"custom=value").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, ReasonParam::Generic(_)));
    }
    
    #[test]
    fn test_parse_reason() {
        // Test with all parameters
        let input = b"SIP ;cause=200 ;text=\"Call completed elsewhere\"";
        let (rem, reason) = parse_reason(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(reason.protocol(), "SIP");
        assert_eq!(reason.cause(), 200);
        assert_eq!(reason.text(), Some("Call completed elsewhere"));
        
        // Test with just mandatory parameters
        let input = b"Q.850;cause=16";
        let (rem, reason) = parse_reason(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(reason.protocol(), "Q.850");
        assert_eq!(reason.cause(), 16);
        assert_eq!(reason.text(), None);
        
        // Test with parameters in different order
        let input = b"SIP ;text=\"Busy Here\" ;cause=486";
        let (rem, reason) = parse_reason(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(reason.protocol(), "SIP");
        assert_eq!(reason.cause(), 486);
        assert_eq!(reason.text(), Some("Busy Here"));
        
        // Test missing cause parameter (should fail)
        let input = b"SIP ;text=\"Busy Here\"";
        let result = parse_reason(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_reason_examples_from_rfc() {
        // Examples from RFC 3326
        let input = b"SIP ;cause=200 ;text=\"Call completed elsewhere\"";
        let (_, reason) = parse_reason(input).unwrap();
        assert_eq!(reason.protocol(), "SIP");
        assert_eq!(reason.cause(), 200);
        assert_eq!(reason.text(), Some("Call completed elsewhere"));
        
        let input = b"Q.850 ;cause=16 ;text=\"Terminated\"";
        let (_, reason) = parse_reason(input).unwrap();
        assert_eq!(reason.protocol(), "Q.850");
        assert_eq!(reason.cause(), 16);
        assert_eq!(reason.text(), Some("Terminated"));
    }
} 