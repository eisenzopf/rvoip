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
// Takes owned String as input
pub fn request_parser_mapped(input: String) -> IResult<String, Message, nom::error::Error<String>> {
    map(
        // Call inner parser with &str
        |s: String| request_parser_inner(s.as_str()), 
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
    // How to apply map to the result of the inner call? This structure is wrong.
    // We need to call inner first, then map the Result.
    )(input) // Pass owned String
}

// Keep the public interface named request_parser
// Takes owned String as input
pub fn request_parser(input: String) -> IResult<String, Message, nom::error::Error<String>> {
    // Directly call inner parser and map the result, avoiding mapped function for now.
    match request_parser_inner(input.as_str()) {
        Ok((_remaining_str, ((method, uri, version), headers, body))) => {
             let request = Request {
                method,
                uri,
                version,
                headers,
                body,
            };
            // Need to return the remaining input as String, which is awkward.
            // This workaround seems problematic.
            // Let's stick to the original approach and rethink the inner parser structure.
            // REVERTING THIS CHANGE IDEA.
            // Returning error to indicate failure of this approach.
            Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))
        }
        Err(e) => {
             // Need to convert error from Error<&str> to Error<String>.
             Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))
             // Err(e.map_input(|_| input)) // This might work if error type allows map_input
        }
    }
    // request_parser_mapped(input)
} 

// Let's try the original mapped approach again, but fix the E0597 error using the compiler hint
// Revert the _inner function as well
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

/// Helper to parse headers and body
fn parse_headers_and_body(input: &str) -> IResult<&str, (Vec<Header>, Bytes), nom::error::Error<&str>> {
    map(
        tuple((
            terminated(many0(super::headers::header_parser), super::utils::crlf),
            rest
        )),
        |(headers, body_str)| (headers, Bytes::from(body_str))
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