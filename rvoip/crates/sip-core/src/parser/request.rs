use std::str::FromStr;
use nom::{
    bytes::complete::{take_till, take_while1},
    character::complete::space1,
    combinator::{map_res, rest},
    sequence::{terminated, tuple},
    multi::many0,
    IResult,
};
use std::result::Result as StdResult;
use crate::error::{Error, Result};
use crate::header::Header;
use crate::types::{Method, Message, Request};
use crate::uri::Uri;
use crate::version::Version;
use super::uri::parse_uri;
use bytes::Bytes;

/// Parser for a SIP request line
/// Returns components needed by IncrementalParser
pub fn parse_request_line(input: &str) -> IResult<&str, (Method, Uri, Version)> {
    let (input, method) = map_res(
        take_while1(|c: char| c.is_alphabetic() || c == '_'),
        |s: &str| Method::from_str(s)
    )(input)?;

    let (input, _) = space1(input)?;

    let (input, uri_str) = take_till(|c| c == ' ')(input)?;
    let uri = match parse_uri(uri_str) {
        Ok(uri) => uri,
        Err(_e) => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag))), // TODO: Better error
    };

    let (input, _) = space1(input)?;

    let (input, version) = map_res(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| Version::from_str(s)
    )(input)?;

    Ok((input, (method, uri, version)))
}

/// Helper to parse headers and body
fn parse_headers_and_body(input: &str) -> IResult<&str, (Vec<Header>, Bytes), nom::error::Error<&str>> {
    map_res(
        tuple((
            terminated(many0(super::headers::header_parser), super::utils::crlf),
            rest
        )),
        |(headers, body_str)| -> StdResult<(Vec<Header>, Bytes), nom::error::Error<&str>> {
            Ok((headers, Bytes::from(body_str)))
        }
    )(input)
}

/// Top-level parser for a complete SIP request
pub fn request_parser(input: &str) -> IResult<&str, Message, nom::error::Error<&str>> {
    // 1. Parse the start line and consume CRLF
    let (rest_after_start_line, (method, uri, version)) = 
        terminated(parse_request_line, super::utils::crlf)(input)?;

    // 2. Parse headers and body from the rest of the input
    let (remaining_input_after_all, (headers, body)) = 
        parse_headers_and_body(rest_after_start_line)?;

    // 3. Construct the Request (all components are now owned)
    let request = Request {
        method,
        uri,
        version,
        headers,
        body,
    };
    
    // 4. Wrap in Message enum
    Ok((remaining_input_after_all, Message::Request(request)))
}