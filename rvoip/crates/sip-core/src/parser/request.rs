use std::str;
use std::str::FromStr;
use nom::{
    branch::alt,
    bytes::complete::{take_till, take_while1, tag},
    character::complete::{line_ending, space1},
    combinator::{map_res, recognize},
    error::{Error as NomError, ErrorKind, ParseError},
    sequence::tuple,
    IResult,
};
use nom::character::complete::line_ending;
// Keep Result for FromStr impls if needed elsewhere
use crate::error::{Error, Result};
use crate::types::{Method, Version, Uri};
use crate::parser::uri::parse_uri;
use crate::parser::token::token;
use crate::parser::common::sip_version;
use crate::parser::whitespace::crlf;
use crate::parser::ParseResult;

/// Parser for a SIP request line
/// Changed signature to accept &[u8]
pub(crate) fn parse_request_line(input: &[u8]) -> ParseResult<(Method, Uri, Version)> {
    map_res(
        tuple((
            token,
            space1,
            parse_uri,
            space1,
            sip_version,
            crlf
        )),
        |(method_bytes, _, uri, _, version, _)| {
            str::from_utf8(method_bytes)?
                .parse::<Method>()
                .map(|m| (m, uri, version))
                .map_err(|_| "Invalid Method")
        }
    )(input)
}

// Removed request_parser, request_parser_nom, parse_headers_and_body functions. 

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Host;
    use std::net::Ipv4Addr;

    #[test]
    fn test_parse_valid_request_line() {
        let line = b"INVITE sip:user@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, (method, uri, version)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(method, Method::Invite);
        assert_eq!(uri.scheme, "sip");
        assert_eq!(uri.userinfo.unwrap().0, "user");
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(version, Version::new(2, 0));
    }

    #[test]
    fn test_parse_custom_method() {
        let line = b"PUBLISH sip:pres@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, (method, _, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(method, Method::Publish);
    }

    #[test]
    fn test_parse_request_line_sips() {
        let line = b"REGISTER sips:secure@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_ok());
        let (rem, (method, uri, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(method, Method::Register);
        assert_eq!(uri.scheme, "sips");
    }

    #[test]
    fn test_invalid_request_line_method() {
        let line = b"GET sip:user@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_invalid_request_line_uri() {
        let line = b"INVITE http://example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_request_line_version() {
        let line = b"INVITE sip:user@example.com HTTP/1.1\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_request_line_spacing() {
        let line = b"INVITEsip:user@example.com SIP/2.0\r\n";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_invalid_request_line_crlf() {
        let line = b"INVITE sip:user@example.com SIP/2.0";
        let result = parse_request_line(line);
        assert!(result.is_err());
    }
} 