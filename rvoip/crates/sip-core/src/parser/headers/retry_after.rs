// Parser for the Retry-After header (RFC 3261 Section 20.33)
// Retry-After = "Retry-After" HCOLON delta-seconds [ comment ] *( SEMI retry-param )
// retry-param = ("duration" EQUAL delta-seconds) / generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
    combinator::{map, map_res, opt},
    multi::{many0},
    sequence::{pair, preceded, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, equal};
use crate::parser::common_params::generic_param;
use crate::parser::values::delta_seconds;
use crate::parser::quoted::comment;
use crate::parser::whitespace::lws;

use crate::types::param::Param;
// use crate::types::retry_after::{RetryAfter as RetryAfterHeader, RetryAfterValue, RetryParam}; // Removed unused import
use crate::parser::common::*;
use crate::parser::common_params::parse_generic_param;
use crate::parser::quoted::parse_quoted_string;
use crate::parser::values::parse_u32;
use crate::parser::whitespace::ows;
// use crate::types::{HeaderError, RetryParam}; // Removed unused import
use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::combinator::{map, map_res, opt};
use nom::multi::separated_list0;
use nom::sequence::{delimited, pair, preceded, tuple};
use crate::parser::ParseResult;

// Define local RetryParam enum
#[derive(Debug, PartialEq, Clone)]
pub enum RetryParam {
    Duration(u32),
    Generic(Param), // Wraps the generic Param type
}

// retry-param = ("duration" EQUAL delta-seconds) / generic-param
fn retry_param(input: &[u8]) -> ParseResult<RetryParam> { // Returns local RetryParam
    alt((
        map(
            preceded(pair(tag_no_case(b"duration"), equal), delta_seconds),
            RetryParam::Duration
        ),
        // Ensure generic_param only returns Param::Other here if needed
        map(generic_param, RetryParam::Generic)
    ))(input)
}

// Define struct for Retry-After value
#[derive(Debug, PartialEq, Clone)]
pub struct RetryAfterValue {
    pub delay: u32, // delta-seconds
    pub comment: Option<String>,
    pub params: Vec<RetryParam>, // Use local RetryParam
}

/// Parses a Retry-After header value.
// pub fn parse_retry_after(input: &[u8]) -> ParseResult<(u32, Option<&[u8]>, Vec<Param>)> { // Old signature
pub fn parse_retry_after(input: &[u8]) -> ParseResult<RetryAfterValue> { // New signature
    map_res(
        tuple((
            delta_seconds,
            opt(preceded(lws, comment)),
            // This correctly parses Vec<RetryParam> because it uses retry_param
            opt(many0(preceded(semi, retry_param))) 
        )),
        |(delta, comment_bytes_opt, params_opt)| {
            let comment_opt_res = comment_bytes_opt
                .map(|b| str::from_utf8(b).map(String::from))
                .transpose()
                // Map Utf8Error, remove ?
                .map_err(|_e| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)));

            let comment_opt = comment_opt_res?; // Handle Result here

            let params_vec = params_opt.unwrap_or_default();
            Ok::<RetryAfterValue, nom::error::Error<&[u8]>>(RetryAfterValue { delay: delta, comment: comment_opt, params: params_vec })
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};

    #[test]
    fn test_retry_param() {
        let (rem_dur, param_dur) = retry_param(b"duration=60").unwrap();
        assert!(rem_dur.is_empty());
        assert!(matches!(param_dur, RetryParam::Duration(60)));

        let (rem_gen, param_gen) = retry_param(b"reason=Temporarily Unavailable").unwrap();
        assert!(rem_gen.is_empty());
        // Note: Generic param value might be Token or Quoted depending on input
        assert!(matches!(param_gen, RetryParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) 
                         if n == "reason" && v == "Temporarily Unavailable"));
    }

    #[test]
    fn test_parse_retry_after_simple() {
        let input = b"120";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap(); // Now returns RetryAfterValue
        assert!(rem.is_empty());
        assert_eq!(value.delay, 120);
        assert!(value.comment.is_none());
        assert!(value.params.is_empty());
    }
    
    #[test]
    fn test_parse_retry_after_with_comment() {
        let input = b"180 (Call Server Migration)";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap(); // Returns RetryAfterValue
        assert!(rem.is_empty());
        assert_eq!(value.delay, 180);
        assert_eq!(value.comment, Some("Call Server Migration".to_string()));
        assert!(value.params.is_empty());
    }

    #[test]
    fn test_parse_retry_after_with_params() {
        let input = b"5;duration=10;reason=congestion";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap(); // Returns RetryAfterValue
        assert!(rem.is_empty());
        assert_eq!(value.delay, 5);
        assert!(value.comment.is_none());
        assert_eq!(value.params.len(), 2);
        assert!(value.params.contains(&RetryParam::Duration(10)));
        // Adjust test to check the Generic variant correctly
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n == "reason" && v == "congestion")));
    }

     #[test]
    fn test_parse_retry_after_full() {
        let input = b"60 (Please wait) ;duration=90";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap(); // Returns RetryAfterValue
        assert!(rem.is_empty());
        assert_eq!(value.delay, 60);
        assert_eq!(value.comment, Some("Please wait".to_string()));
        assert_eq!(value.params.len(), 1);
        assert!(value.params.contains(&RetryParam::Duration(90)));
    }

    #[test]
    fn test_parse_retry_after_comment_nested() {
        let input = b"120 (Nested (comment) here)";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap(); // Returns RetryAfterValue
        assert!(rem.is_empty());
        assert_eq!(value.delay, 120);
        assert_eq!(value.comment, Some("Nested (comment) here".to_string()));
        assert!(value.params.is_empty());
    }
}
