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
use bytes::Bytes;
use nom::combinator::rest;

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
    map(
        request_parser_inner, 
        |((method, uri, version), headers, body)| {
            let request = Request {
                method,
                uri,
                version,
                headers,
                body,
            };
            Message::Request(request)
        }
    )(input)
}

// Change return type to tuple of components
fn request_parser_inner(input: &str) -> IResult<&str, ((Method, Uri, Version), Vec<Header>, Bytes), nom::error::Error<&str>> {
    // Use tuple combinator
    map(
        tuple((
            // 1. Parse start line and consume CRLF
            terminated(parse_request_line, super::utils::crlf),
            // 2. Parse headers and consume CRLF
            terminated(many0(super::headers::header_parser), super::utils::crlf),
            // 3. Take the rest as the body (&str)
            rest
        )),
        // Map the resulting tuple ((Method, Uri, Version), Vec<Header>, &str) to include owned Bytes
        |(start_line_components, headers, body_str)| {
            (start_line_components, headers, Bytes::from(body_str))
        }
    )(input)
}

// Keep the public interface named request_parser
pub fn request_parser(input: &str) -> IResult<&str, Message, nom::error::Error<&str>> {
    request_parser_mapped(input)
} 