use std::str::FromStr;
use nom::{
    bytes::complete::{take_till, take_while1},
    character::complete::space1,
    combinator::{map_res},
    IResult,
};
// Keep Result for FromStr impls if needed elsewhere
use crate::error::{Error, Result};
use crate::types::{Method};
use crate::uri::Uri;
use crate::version::Version;
use super::uri::parse_uri;

/// Parser for a SIP request line
/// Returns components needed by IncrementalParser
pub fn parse_request_line(input: &str) -> IResult<&str, (Method, Uri, Version)> {
    let (input, method) = map_res(
        take_while1(|c: char| c.is_alphabetic() || c == '_'),
        |s: &str| Method::from_str(s)
    )(input)?;

    let (input, _) = space1(input)?;

    let (input, uri_str_raw) = take_till(|c| c == ' ')(input)?;
    let uri = Uri::from_str(uri_str_raw).unwrap_or_else(|_|
        Uri::sip("invalid.uri.placeholder").with_user("parse_failed")
    );

    let (input, _) = space1(input)?;

    let (input, version) = map_res(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| Version::from_str(s)
    )(input)?;

    Ok((input, (method, uri, version)))
}

// Removed request_parser, request_parser_nom, parse_headers_and_body functions. 