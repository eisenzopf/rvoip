// Parser for generic parameters (generic-param)

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
    combinator::{map, map_res, opt, recognize},
    multi::many0,
    sequence::{pair, preceded},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;
use std::collections::HashMap;

// Import specific parsers from the new modules
use super::token::token;
use super::separators::{equal, semi};
use super::quoted::{quoted_string, quoted_pair};
use super::values::{qvalue, delta_seconds};
use super::uri::host::host;
use super::ParseResult;
use super::utils::unescape_uri_component;
use crate::types::param::{Param, GenericValue};
use crate::types::uri::Host;
use crate::error::Error;

use ordered_float::NotNan;

/// Helper to unquote a quoted string (represented as bytes).
/// Returns Ok(String) if successful, Err(Error) otherwise.
pub fn unquote_string(input: &[u8]) -> std::result::Result<String, Error> {
    unescape_uri_component(input)
        .map_err(|e| Error::ParseError(format!("Invalid escaped sequence in parameter: {}", e)))
}

// gen-value = token / host / quoted-string
fn gen_value(input: &[u8]) -> ParseResult<GenericValue> {
    alt((
        map(host, GenericValue::Host),
        map_res(quoted_string, |bytes| {
            unquote_string(bytes).map(GenericValue::Quoted)
        }),
        map_res(token, |bytes| {
            str::from_utf8(bytes)
                .map(|s| GenericValue::Token(s.to_string()))
                .map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Char)))
        }),
    ))(input)
}

// generic-param = token [ EQUAL gen-value ]
// Returns Param::Other(String, Option<GenericValue>)
pub fn generic_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        pair(token, opt(preceded(equal, gen_value))),
        |(name_b, val_opt)| {
            let name = str::from_utf8(name_b)
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(name_b, ErrorKind::Char)))?
                .to_string();
            Ok::<_, nom::Err<NomError<&[u8]>>>(Param::Other(name, val_opt))
        }
    )(input)
}

// accept-param = ("q" EQUAL qvalue) / generic-param
pub fn accept_param(input: &[u8]) -> ParseResult<Param> {
    alt((
        map(preceded(pair(tag_no_case(b"q"), equal), qvalue), Param::Q),
        generic_param,
    ))(input)
}

/// Parses zero or more semicolon-preceded generic parameters.
/// Input: b";name1=value1;name2;name3="value3""
/// Output: A Vec containing Param::Other variants.
pub fn semicolon_params0(input: &[u8]) -> ParseResult<Vec<Param>> {
    many0(
        preceded(
            semi, // Use the semi parser which handles surrounding SWS
            generic_param // Parse one generic parameter
        )
    )(input)
}

/// Parses one or more semicolon-preceded generic parameters.
pub fn semicolon_params1(input: &[u8]) -> ParseResult<Vec<Param>> {
    nom::multi::many1(
        preceded(
            semi, // Use the semi parser which handles surrounding SWS
            generic_param // Parse one generic parameter
        )
    )(input)
}

/// Parses zero or more semicolon-preceded parameters using a specific parameter parser function.
/// Useful for headers where parameters might not all be generic-param.
pub fn semicolon_separated_params0<'a, O, F>(param_parser: F) -> impl FnMut(&'a [u8]) -> ParseResult<Vec<O>> 
where
    F: FnMut(&'a [u8]) -> ParseResult<O> + Copy,
{
    many0(
        preceded(
            semi,
            param_parser,
        )
    )
}

/// Parses one or more semicolon-preceded parameters using a specific parameter parser function.
pub fn semicolon_separated_params1<'a, O, F>(param_parser: F) -> impl FnMut(&'a [u8]) -> ParseResult<Vec<O>> 
where
    F: FnMut(&'a [u8]) -> ParseResult<O> + Copy,
{
    nom::multi::many1(
        preceded(
            semi,
            param_parser,
        )
    )
}

// Helper to convert a Vec<Param> (like from semicolon_params0) to a HashMap for easier lookup.
// Note: This assumes parameter names are unique, last one wins if not.
pub fn params_to_hashmap(params: Vec<Param>) -> HashMap<String, Option<String>> {
    params.into_iter().fold(HashMap::new(), |mut acc, param| {
        if let Param::Other(name, value) = param {
            acc.insert(name, value);
        }
        // TODO: Handle other Param variants like Q if needed in the HashMap?
        acc
    })
}

// tag-param = "tag" EQUAL token
pub fn tag_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(tag_no_case(b"tag"), preceded(equal, token)),
        |tag_bytes| str::from_utf8(tag_bytes).map(|s| Param::Tag(s.to_string()))
    )(input)
}

// Specific param parser for From/To headers
// from-param / to-param = tag-param / generic-param
pub fn from_to_param(input: &[u8]) -> ParseResult<Param> {
    alt((tag_param, generic_param))(input)
}

// c-p-q = "q" EQUAL qvalue
fn cp_q(input: &[u8]) -> ParseResult<NotNan<f32>> {
    preceded(
        pair(tag_no_case(b"q"), equal),
        qvalue
    )(input)
}

// c-p-expires = "expires" EQUAL delta-seconds
fn cp_expires(input: &[u8]) -> ParseResult<u32> {
    preceded(
        pair(tag_no_case(b"expires"), equal),
        delta_seconds
    )(input)
}

// contact-params = c-p-q / c-p-expires / contact-extension
// contact-extension = generic-param
// Returns Param enum to capture the different types
pub fn contact_param_item(input: &[u8]) -> ParseResult<Param> {
    alt((
        map(cp_q, Param::Q),
        map(cp_expires, Param::Expires),
        generic_param // Fallback to generic
    ))(input)
}

// Function to parse a semicolon-separated list of key-value parameters into a HashMap<String, Option<String>>
pub fn hashmap_param_list<'a>(
    param_parser: impl Fn(&'a [u8]) -> ParseResult<'a, (String, Option<GenericValue>)>,
) -> impl FnMut(&'a [u8]) -> ParseResult<'a, HashMap<String, Option<String>>> {
    map(
        many0(preceded(semi, param_parser)),
        |params| {
            params.into_iter().map(|(name, value)| {
                // Convert Option<GenericValue> to Option<String>
                let string_value = value.map(|v| v.to_string());
                (name, string_value)
            }).collect()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Host;
    use std::net::Ipv4Addr;
    use ordered_float::NotNan;

    #[test]
    fn test_gen_value() {
        // Token
        let (rem_tok, val_tok) = gen_value(b"mytoken").unwrap();
        assert!(rem_tok.is_empty());
        assert!(matches!(val_tok, GenericValue::Token(s) if s == "mytoken"));

        // Host (Domain)
        let (rem_host, val_host) = gen_value(b"example.com").unwrap();
        assert!(rem_host.is_empty());
        assert!(matches!(val_host, GenericValue::Host(Host::Domain(d)) if d == "example.com"));

        // Host (IPv4)
        let (rem_ip, val_ip) = gen_value(b"192.0.2.1").unwrap();
        assert!(rem_ip.is_empty());
        assert!(matches!(val_ip, GenericValue::Host(Host::Address(a)) if a == Ipv4Addr::new(192,0,2,1).into()));

        // Quoted String
        let (rem_qs, val_qs) = gen_value(b"\"Quoted Value\"").unwrap();
        assert!(rem_qs.is_empty());
        assert!(matches!(val_qs, GenericValue::Quoted(s) if s == "Quoted Value"));

        // Quoted String with escaped chars
        let (rem_esc, val_esc) = gen_value(b"\"\\\\Quote\\\"\\\"\"").unwrap();
        assert!(rem_esc.is_empty());
        assert!(matches!(val_esc, GenericValue::Quoted(s) if s == "\\Quote\""));
    }

    #[test]
    fn test_generic_param_value() {
        let (rem, param) = generic_param(b"name=value").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Token(v))) if n == "name" && v == "value"));
    }

    #[test]
    fn test_generic_param_host() {
        let (rem, param) = generic_param(b"maddr=192.0.2.1").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Host(Host::Address(a)))) if n == "maddr" && a == Ipv4Addr::new(192,0,2,1).into()));
    }

    #[test]
    fn test_generic_param_quoted() {
        let (rem, param) = generic_param(b"display=\"Bob Smith\"").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Quoted(v))) if n == "display" && v == "Bob Smith"));
    }

    #[test]
    fn test_generic_param_no_value() {
        let (rem, param) = generic_param(b"flag").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, None) if n == "flag"));
    }

     #[test]
    fn test_generic_param_empty_quoted() {
        let (rem, param) = generic_param(b"empty=\"\"").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Quoted(v))) if n == "empty" && v == ""));
    }

    #[test]
    fn test_accept_param_q() {
        let (rem, param) = accept_param(b"q=0.5").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Q(q) if q == NotNan::new(0.5).unwrap()));
    }
    
    #[test]
    fn test_accept_param_generic() {
        let (rem, param) = accept_param(b"level=1").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Token(v))) if n == "level" && v == "1"));
    }

    #[test]
    fn test_contact_param_item() {
        // Q value
        let (rem_q, param_q) = contact_param_item(b"q=0.9").unwrap();
        assert!(rem_q.is_empty());
        assert!(matches!(param_q, Param::Q(q) if q == NotNan::new(0.9).unwrap()));

        // Expires
        let (rem_exp, param_exp) = contact_param_item(b"expires=3600").unwrap();
        assert!(rem_exp.is_empty());
        assert!(matches!(param_exp, Param::Expires(e) if e == 3600));

        // Generic
        let (rem_gen, param_gen) = contact_param_item(b"+sip.instance=\"urn:uuid:123\"").unwrap();
        assert!(rem_gen.is_empty());
        assert!(matches!(param_gen, Param::Other(n, Some(GenericValue::Quoted(v))) if n == "+sip.instance" && v == "urn:uuid:123"));
    }
} 