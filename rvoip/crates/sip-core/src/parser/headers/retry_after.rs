// Parser for the Retry-After header (RFC 3261 Section 20.33)
// Retry-After = "Retry-After" HCOLON delta-seconds [ comment ] *( SEMI retry-param )
// retry-param = ("duration" EQUAL delta-seconds) / generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, tag, take_while},
    combinator::{map, map_res, opt},
    multi::{many0, separated_list0},
    sequence::{pair, preceded, tuple, delimited},
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
use crate::parser::ParseResult;

// Define public RetryParam enum for use in types module
#[derive(Debug, PartialEq, Clone)]
pub enum RetryParam {
    Duration(u32),
    Generic(Param), // Wraps the generic Param type
}

// retry-param = ("duration" EQUAL delta-seconds) / generic-param
pub fn retry_param(input: &[u8]) -> ParseResult<RetryParam> {
    alt((
        // Verify that duration has a valid delta-seconds value
        map_res(
            preceded(pair(tag_no_case(b"duration"), equal), delta_seconds),
            |duration| {
                Ok::<RetryParam, nom::Err<NomError<&[u8]>>>(RetryParam::Duration(duration))
            }
        ),
        map(generic_param, RetryParam::Generic)
    ))(input)
}

// Define struct for Retry-After value
#[derive(Debug, PartialEq, Clone)]
pub struct RetryAfterValue {
    pub delay: u32,            // delta-seconds
    pub comment: Option<String>,
    pub params: Vec<RetryParam>,
}

/// Parses a Retry-After header value.
// pub fn parse_retry_after(input: &[u8]) -> ParseResult<(u32, Option<&[u8]>, Vec<Param>)> { // Old signature
pub fn parse_retry_after(input: &[u8]) -> ParseResult<RetryAfterValue> { // New signature
    map_res(
        tuple((
            delta_seconds,
            opt(preceded(lws, comment)),
            many0(preceded(semi, retry_param))
        )),
        |(delta, comment_bytes_opt, params)| {
            // Convert comment bytes to String if present
            let comment_opt_result = comment_bytes_opt
                .map(|b| {
                    str::from_utf8(b)
                        .map(|s| s.to_string())
                        .map_err(|_| nom::Err::Failure(NomError::from_error_kind(b, ErrorKind::Char)))
                })
                .transpose();

            comment_opt_result.map(|comment_opt| {
                RetryAfterValue {
                    delay: delta,
                    comment: comment_opt,
                    params,
                }
            })
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

        let (rem_gen, param_gen) = retry_param(b"reason=Temporarily").unwrap();
        assert!(rem_gen.is_empty());
        assert!(matches!(param_gen, RetryParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) 
                         if n == "reason" && v == "Temporarily"));
                         
        let (rem_quoted, param_quoted) = retry_param(b"reason=\"Temporarily Unavailable\"").unwrap();
        assert!(rem_quoted.is_empty());
        assert!(matches!(param_quoted, RetryParam::Generic(Param::Other(n, Some(GenericValue::Quoted(v)))) 
                         if n == "reason" && v == "Temporarily Unavailable"));
    }

    #[test]
    fn test_parse_retry_after_simple() {
        let input = b"120";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap();
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
        let (rem, value) = result.unwrap();
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
        let (rem, value) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(value.delay, 5);
        assert!(value.comment.is_none());
        assert_eq!(value.params.len(), 2);
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Duration(10))));
        // Adjust test to check the Generic variant correctly
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n == "reason" && v == "congestion")));
    }

    #[test]
    fn test_parse_retry_after_full() {
        let input = b"60 (Please wait) ;duration=90";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(value.delay, 60);
        assert_eq!(value.comment, Some("Please wait".to_string()));
        assert_eq!(value.params.len(), 1);
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Duration(90))));
    }

    #[test]
    fn test_parse_retry_after_comment_nested() {
        let input = b"120 (Nested (comment) here)";
        let result = parse_retry_after(input);
        assert!(result.is_ok());
        let (rem, value) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(value.delay, 120);
        assert_eq!(value.comment, Some("Nested (comment) here".to_string()));
        assert!(value.params.is_empty());
    }

    #[test]
    fn test_parse_retry_after_rfc_examples() {
        // Test with examples similar to those in RFC 3261
        let input = b"18000 (5 hours)";
        let (_, value) = parse_retry_after(input).unwrap();
        assert_eq!(value.delay, 18000);
        assert_eq!(value.comment, Some("5 hours".to_string()));
        
        let input = b"120";
        let (_, value) = parse_retry_after(input).unwrap();
        assert_eq!(value.delay, 120);
        
        let input = b"3600;duration=1800";
        let (_, value) = parse_retry_after(input).unwrap();
        assert_eq!(value.delay, 3600);
        assert_eq!(value.params.len(), 1);
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Duration(1800))));
    }
    
    #[test]
    fn test_parse_retry_after_multiple_params() {
        let input = b"300;duration=600;reason=maintenance;urgent;retry-id=123";
        let (_, value) = parse_retry_after(input).unwrap();
        assert_eq!(value.delay, 300);
        assert_eq!(value.params.len(), 4);
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Duration(600))));
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n == "reason" && v == "maintenance")));
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Generic(Param::Other(n, None)) if n == "urgent")));
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n == "retry-id" && v == "123")));
    }
    
    #[test]
    fn test_parse_retry_after_parameter_only() {
        // Test with parameter only, no comment
        let input = b"60;duration=120";
        let (_, value) = parse_retry_after(input).unwrap();
        assert_eq!(value.delay, 60);
        assert!(value.comment.is_none());
        assert_eq!(value.params.len(), 1);
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Duration(120))));
    }
    
    #[test]
    fn test_parse_retry_after_quoted_param() {
        // Test with quoted parameter value
        let input = b"60;reason=\"Server Maintenance\"";
        let (_, value) = parse_retry_after(input).unwrap();
        assert_eq!(value.delay, 60);
        assert!(value.params.iter().any(|p| matches!(p, RetryParam::Generic(Param::Other(n, Some(GenericValue::Quoted(v)))) if n == "reason" && v == "Server Maintenance")));
    }
}
