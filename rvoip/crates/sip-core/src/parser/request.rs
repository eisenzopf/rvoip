use std::str::FromStr;
use nom::{
    bytes::complete::{take_till, take_while1},
    character::complete::space1,
    combinator::{map_res},
    IResult,
};
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