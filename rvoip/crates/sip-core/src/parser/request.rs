use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_while, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize, verify},
    multi::{many0, many1, many_till, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    Err, IResult, Needed,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::types::{Message, Request, Method, StatusCode};
use crate::uri::Uri;
use crate::version::Version;
use super::uri::parse_uri;

/// Parser for a SIP request line
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

/// Parser for a complete SIP request, mapped to Message enum
pub fn request_parser_mapped(input: &str) -> IResult<&str, Message, nom::error::Error<&str>> {
    map(request_parser_inner, Message::Request)(input)
}

// Rename original parser to avoid direct recursion if mapping fails
fn request_parser_inner(input: &str) -> IResult<&str, Request, nom::error::Error<&str>> {
    // Parse the request line and consume the following CRLF
    let (input, (method, uri, version)) = terminated(parse_request_line, super::utils::crlf)(input)?;

    // Parse headers using the remaining input
    let (input, headers) = terminated(
        many0(super::headers::header_parser),
        super::utils::crlf
    )(input)?;

    // Create the request
    let mut request = Request {
        method,
        uri,
        version,
        headers: vec![], // Initialize headers vec
        body: Default::default(),
    };

    // Add headers
    request.headers = headers; // Assign parsed headers

    // Parse the body if present - preserve it exactly as is without modifying line endings
    if !input.is_empty() {
        request.body = input.into();
    }

    Ok(("", request))
}

// Keep the public interface named request_parser
pub fn request_parser(input: &str) -> IResult<&str, Message, nom::error::Error<&str>> {
    request_parser_mapped(input)
} 