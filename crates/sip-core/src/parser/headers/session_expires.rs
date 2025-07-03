//! # SIP Session-Expires Header Parser
//!
//! This module provides parser functions for the Session-Expires header as defined in 
//! [RFC 4028](https://datatracker.ietf.org/doc/html/rfc4028).
//!
//! The Session-Expires header specifies the lifetime of the session, along with which
//! party is responsible for the refresh.
//!
//! ## ABNF Grammar
//!
//! ```abnf
//! Session-Expires = delta-seconds *(SEMI se-params)
//! se-params = refresher-param / generic-param
//! refresher-param = "refresher" EQUAL ("uas" / "uac")
//! delta-seconds = 1*DIGIT
//! ```

use nom::{
    bytes::complete::{tag, tag_no_case},
    character::complete::digit1,
    combinator::{map, map_res, opt},
    sequence::{pair, preceded, tuple},
    IResult,
    multi::many0,
    branch::alt,
};

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, equal};
use crate::parser::common_params::generic_param;
use crate::parser::ParseResult;
use crate::types::param::Param;
use crate::types::session_expires::Refresher;
use std::str::{self, FromStr};

/// Parse delta-seconds (non-negative decimal integer)
fn delta_seconds(input: &[u8]) -> ParseResult<u32> {
    map_res(
        digit1,
        |digits: &[u8]| {
            let s = str::from_utf8(digits).map_err(|_| "UTF-8 error")?;
            s.parse::<u32>().map_err(|_| "Invalid delta-seconds")
        }
    )(input)
}

// Parse refresher parameter (refresher=uac|uas)
fn refresher_param(input: &[u8]) -> ParseResult<Refresher> {
    map_res(
        preceded(
            pair(tag_no_case(b"refresher"), equal),
            alt((
                tag_no_case(b"uac"),
                tag_no_case(b"uas")
            ))
        ),
        |val| {
            match str::from_utf8(val).map_err(|_| "UTF-8 error")?.to_lowercase().as_str() {
                "uac" => Ok(Refresher::Uac),
                "uas" => Ok(Refresher::Uas),
                _ => Err("Invalid refresher value")
            }
        }
    )(input)
}

// Parse a session-expires parameter (either refresher or generic)
fn se_param(input: &[u8]) -> ParseResult<(Option<Refresher>, Option<Param>)> {
    alt((
        // Try to parse refresher parameter first
        map(refresher_param, |refresher| (Some(refresher), None)),
        // Then try generic parameter
        map(generic_param, |param: Param| (None, Some(param)))
    ))(input)
}

/// Parse a Session-Expires header value according to RFC 4028
/// 
/// Syntax:
/// Session-Expires = delta-seconds *(SEMI se-params)
/// se-params = refresher-param / generic-param
/// refresher-param = "refresher" EQUAL ("uas" / "uac")
///
/// Returns a tuple with (expires_value, refresher, params)
pub fn parse_session_expires(input: &[u8]) -> ParseResult<(u32, Option<Refresher>, Vec<Param>)> {
    let (remaining_input, expires) = delta_seconds(input)?;
    
    // Parse any parameters
    let (remaining_input, params_data) = many0(
        preceded(semi, se_param)
    )(remaining_input)?;
    
    // Extract refresher (take the last one if multiple are specified)
    let mut parsed_refresher: Option<Refresher> = None;
    let mut parsed_generic_params: Vec<Param> = Vec::new();
    
    for (r_opt, param_opt) in params_data {
        if let Some(r_val) = r_opt {
            parsed_refresher = Some(r_val);
        }
        if let Some(p) = param_opt {
            // RFC 4028: "refresher" parameter ... if present with a value other than 'uac' or 'uas' (which is an error)
            // If generic_param parsed a param with key "refresher", it means it wasn't "refresher=uac" or "refresher=uas".
            // This constitutes an error.
            if p.key().eq_ignore_ascii_case("refresher") {
                return Err(nom::Err::Failure(nom::error::make_error(input, nom::error::ErrorKind::Verify)));
            }
            parsed_generic_params.push(p);
        }
    }
    
    Ok((remaining_input, (expires, parsed_refresher, parsed_generic_params)))
}

/// Parse the Session-Expires header, including the header name
pub fn parse_session_expires_header(input: &[u8]) -> ParseResult<(u32, Option<Refresher>, Vec<Param>)> {
    preceded(
        pair(
            alt((
                tag_no_case(b"Session-Expires"),
                tag_no_case(b"x")  // Compact form
            )),
            hcolon
        ),
        parse_session_expires
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::combinator::all_consuming;

    #[test]
    fn test_parse_session_expires_simple() {
        // Basic expires value
        let input = b"3600";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 3600);
        assert_eq!(refresher, None);
        assert_eq!(params.len(), 0);
    }
    
    #[test]
    fn test_parse_session_expires_with_refresher() {
        // Expires with refresher
        let input = b"3600;refresher=uac";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 3600);
        assert_eq!(refresher, Some(Refresher::Uac));
        assert_eq!(params.len(), 0);
    }
    
    #[test]
    fn test_parse_session_expires_with_generic_param() {
        // Expires with a generic parameter
        let input = b"3600;param=value";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 3600);
        assert_eq!(refresher, None);
        assert_eq!(params.len(), 1);
        
        // Check the parameter using key() and value() methods
        assert_eq!(params[0].key(), "param");
        assert_eq!(params[0].value().map(|s_ref| s_ref.to_string()), Some("value".to_string()));
    }
    
    #[test]
    fn test_parse_session_expires_multiple_params() {
        // Expires with refresher and other params
        let input = b"1800;refresher=uas;param1=value1;param2=value2";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 1800);
        assert_eq!(refresher, Some(Refresher::Uas));
        assert_eq!(params.len(), 2);
    }
    
    #[test]
    fn test_parse_session_expires_zero() {
        // Zero is valid for expires
        let input = b"0";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 0);
        assert_eq!(refresher, None);
        assert_eq!(params.len(), 0);
    }
    
    #[test]
    fn test_parse_session_expires_refresher_case_insensitive() {
        // Case-insensitive parsing
        let input1 = b"3600;refresher=UaC";
        let input2 = b"3600;REFRESHER=uas";
        
        let result1 = parse_session_expires(input1);
        assert!(result1.is_ok());
        let (_, (_, refresher1, _)) = result1.unwrap();
        assert_eq!(refresher1, Some(Refresher::Uac));
        
        let result2 = parse_session_expires(input2);
        assert!(result2.is_ok());
        let (_, (_, refresher2, _)) = result2.unwrap();
        assert_eq!(refresher2, Some(Refresher::Uas));
    }
    
    #[test]
    fn test_parse_session_expires_multiple_refresher() {
        // If multiple refresher parameters are present, the last one wins
        let input = b"3600;refresher=uac;refresher=uas";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 3600);
        assert_eq!(refresher, Some(Refresher::Uas)); // Last one wins
        assert_eq!(params.len(), 0);
    }
    
    #[test]
    fn test_parse_session_expires_valueless_param() {
        // Parameter without a value
        let input = b"3600;param";
        let result = parse_session_expires(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 3600);
        assert_eq!(refresher, None);
        assert_eq!(params.len(), 1);
        
        // Check the parameter
        match &params[0] {
            Param::Other(name, None) => {
                assert_eq!(name, "param");
            },
            _ => panic!("Expected Other param without value")
        }
    }
    
    #[test]
    fn test_parse_session_expires_header() {
        // Test with header name
        let input = b"Session-Expires: 3600;refresher=uac";
        let result = parse_session_expires_header(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 3600);
        assert_eq!(refresher, Some(Refresher::Uac));
        assert_eq!(params.len(), 0);
        
        // Test with compact form
        let input = b"x: 1800;refresher=uas";
        let result = parse_session_expires_header(input);
        assert!(result.is_ok());
        let (rem, (expires, refresher, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(expires, 1800);
        assert_eq!(refresher, Some(Refresher::Uas));
        assert_eq!(params.len(), 0);
    }
    
    #[test]
    fn test_parse_session_expires_invalid() {
        // Invalid inputs
        let inputs = [b"" as &[u8], b"abc", b"-1", b"3600;refresher=invalid"];
        
        for input in &inputs {
            let result = all_consuming(parse_session_expires)(*input);
            assert!(result.is_err());
        }
    }
} 