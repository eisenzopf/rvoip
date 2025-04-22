use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{all_consuming, map, map_res, opt, value},
    sequence::tuple,
    IResult,
};
use std::str;

use crate::error::{Error, Result};
use crate::types::param::Param;
use crate::uri::{Host, Scheme, Uri};
use crate::parser::ParseResult;

// Import parsers from sibling modules within uri/
use super::host::hostport;
use super::userinfo::userinfo;
use super::params::uri_parameters;
use super::headers::uri_headers;

/// Helper function to construct a Uri object from parsed components.
fn build_uri(
    scheme: Scheme,
    userinfo_opt: Option<(&[u8], Option<&[u8]>)>, // (user_bytes, password_bytes_opt)
    host_port: (Host, Option<u16>), // (host, port_opt)
    params: Vec<Param>,
    headers_opt: Option<Vec<(String, String)>>,
) -> Result<Uri> {
    // userinfo parser already gives &[u8], convert to String
    // TODO: Proper URI unescaping should happen here or within userinfo/params/headers parsers.
    let user = userinfo_opt
        .map(|(u, _)| str::from_utf8(u).map(|s| s.to_string()))
        .transpose()?;
    let password = userinfo_opt
        .and_then(|(_, p)| p)
        .map(|pw| str::from_utf8(pw).map(|s| s.to_string()))
        .transpose()?;

    let (host, port_opt) = host_port;

    let mut uri = Uri::new(scheme, host);
    uri.user = user;
    uri.password = password;
    uri.port = port_opt;
    uri.parameters = params;
    uri.headers = headers_opt.unwrap_or_default();
    Ok(uri)
}

// SIP-URI = "sip:" [ userinfo ] hostport uri-parameters [ headers ]
fn sip_uri(input: &[u8]) -> ParseResult<Uri> {
    map_res(
        tuple((
            value(Scheme::Sip, tag_no_case(b"sip:")), // Parse scheme tag and return Scheme enum
            opt(userinfo), // userinfo is optional
            hostport,
            uri_parameters,
            opt(uri_headers), // headers are optional
        )),
        |(scheme, userinfo_opt, host_port, params, headers_opt)| {
            build_uri(scheme, userinfo_opt, host_port, params, headers_opt)
        }
    )(input)
}

// SIPS-URI = "sips:" [ userinfo ] hostport uri-parameters [ headers ]
fn sips_uri(input: &[u8]) -> ParseResult<Uri> {
    map_res(
        tuple((
            value(Scheme::Sips, tag_no_case(b"sips:")),
            opt(userinfo),
            hostport,
            uri_parameters,
            opt(uri_headers),
        )),
        |(scheme, userinfo_opt, host_port, params, headers_opt)| {
            build_uri(scheme, userinfo_opt, host_port, params, headers_opt)
        }
    )(input)
}

// Request-URI = SIP-URI / SIPS-URI / absoluteURI
// absoluteURI parsing is omitted for now
fn request_uri(input: &[u8]) -> ParseResult<Uri> {
    // TODO: Add absoluteURI parser if needed
    alt((sip_uri, sips_uri))(input)
}

/// Parse a URI byte slice into a Uri object
pub fn parse_uri(input: &[u8]) -> Result<Uri> {
    match all_consuming(request_uri)(input) {
        Ok((_, uri)) => Ok(uri),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            let offset = input.len() - e.input.len();
            Err(Error::ParsingError { 
                message: format!("Failed to parse URI near offset {}: {:?}", offset, e.code), 
                source: None 
            })
        },
        Err(nom::Err::Incomplete(_)) => Err(Error::ParsingError{ 
            message: "Incomplete URI input".to_string(), 
            source: None 
        }),
    }
} 