// Declare URI sub-modules
pub mod host;
pub mod userinfo;
pub mod params;
pub mod headers;
pub mod absolute;
pub mod authority;
pub mod path;
pub mod query;
pub mod scheme;

// Host sub-modules
pub mod hostname;
pub mod ipv4;
pub mod ipv6;

// Re-export relevant parsers from sub-modules
pub use host::{host, port, hostport};
pub use userinfo::userinfo;
pub use params::uri_parameters;
pub use headers::uri_headers;
pub use absolute::parse_absolute_uri;

// Add imports for combinators and types
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while},
    character::complete::{char, digit1, one_of},
    combinator::{map, map_res, opt, recognize, value, verify, all_consuming, eof},
    error::{Error as NomError, ErrorKind},
    multi::{many0, many1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::str;

use crate::types::uri::{Host, Uri};
use crate::types::param::Param;
use crate::parser::ParseResult;
use crate::Scheme;
use crate::error::Error;

use authority::parse_authority;

// SIP-URI = "sip:" [ userinfo ] hostport uri-parameters [ headers ]
pub fn parse_sip_uri(bytes: &[u8]) -> ParseResult<Uri> {
    let bytes_slice = bytes;
    let res = tuple((
        tag_no_case(b"sip:"),
        opt(userinfo),
        hostport,
        opt(uri_parameters),
        opt(uri_headers),
    ))(bytes_slice);

    match res {
        Ok((remaining, (_, user_info_opt, (host, port_opt), params_opt, headers_opt))) => {
            // user_info is already (String, Option<String>)
            let (user, password) = user_info_opt.unwrap_or((String::new(), None));
            
            let uri = Uri {
                scheme: Scheme::Sip,
                user: if user.is_empty() { None } else { Some(user) },
                password,
                host,
                port: port_opt,
                parameters: params_opt.unwrap_or_default(),
                headers: headers_opt.unwrap_or_default(),
            };
            
            Ok((remaining, uri))
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            Err(nom::Err::Error(e))
        }
        Err(e) => Err(e),
    }
}

// SIPS-URI = "sips:" [ userinfo ] hostport uri-parameters [ headers ]
pub fn parse_sips_uri(bytes: &[u8]) -> ParseResult<Uri> {
    let bytes_slice = bytes;
    let res = tuple((
        tag_no_case(b"sips:"),
        opt(userinfo),
        hostport,
        opt(uri_parameters),
        opt(uri_headers),
    ))(bytes_slice);

    match res {
        Ok((remaining, (_, user_info_opt, (host, port_opt), params_opt, headers_opt))) => {
            // user_info is already (String, Option<String>)
            let (user, password) = user_info_opt.unwrap_or((String::new(), None));
            
            let uri = Uri {
                scheme: Scheme::Sips,
                user: if user.is_empty() { None } else { Some(user) },
                password,
                host,
                port: port_opt,
                parameters: params_opt.unwrap_or_default(),
                headers: headers_opt.unwrap_or_default(),
            };
            
            Ok((remaining, uri))
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            Err(nom::Err::Error(e))
        }
        Err(e) => Err(e),
    }
}

/// Public entry point for parsing a SIP or SIPS URI
/// Can be re-exported by the main parser mod.rs
pub fn parse_uri(input: &[u8]) -> ParseResult<Uri> {
    alt((parse_sip_uri, parse_sips_uri))(input)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Scheme;
    use crate::types::uri::Host;
    use crate::types::param::Param;
    use crate::types::param::GenericValue;
    use std::net::{Ipv4Addr, IpAddr};
    use nom::error::ErrorKind;

    #[test]
    fn test_parse_simple_sip_uri() {
        let uri_bytes = b"sip:user@example.com";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("user".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
    }
    
    #[test]
    fn test_parse_sips_uri_with_port() {
        let uri_bytes = b"sips:alice@atlanta.com:5061";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sips);
        assert_eq!(uri.user, Some("alice".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "atlanta.com"));
        assert_eq!(uri.port, Some(5061));
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
    }

    #[test]
    fn test_parse_sip_uri_ipv4() {
        let uri_bytes = b"sip:192.168.0.1:8080";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, None);
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Address(addr) if addr == IpAddr::from(Ipv4Addr::new(192, 168, 0, 1))));
        assert_eq!(uri.port, Some(8080));
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
    }

     #[test]
    fn test_parse_sip_uri_with_params() {
        let uri_bytes = b"sip:user@example.com;transport=tcp;lr";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("user".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert_eq!(uri.parameters.len(), 2);
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(uri.headers.is_empty());
    }

     #[test]
    fn test_parse_sip_uri_with_headers() {
        let uri_bytes = b"sip:user@example.com?Subject=Urgent&Priority=High";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("user".to_string()));
        assert_eq!(uri.password, None);
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert!(uri.parameters.is_empty());
        assert!(!uri.headers.is_empty());
        let headers = uri.headers;
        assert_eq!(headers.get("Subject"), Some(&"Urgent".to_string()));
        assert_eq!(headers.get("Priority"), Some(&"High".to_string()));
    }
    
    #[test]
    fn test_parse_sip_uri_complex() {
         let uri_bytes = b"sips:bob:password@[fe80::1]:5061;transport=tls;maddr=192.0.2.1?Subject=Hello";
         let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
         assert!(rem.is_empty());
         assert_eq!(uri.scheme, Scheme::Sips);
         assert_eq!(uri.user, Some("bob".to_string()));
         assert_eq!(uri.password, Some("password".to_string()));
         assert!(matches!(uri.host, Host::Address(_)));
         assert_eq!(uri.port, Some(5061));
         assert_eq!(uri.parameters.len(), 2);
         assert!(uri.parameters.iter().any(|p| matches!(p, Param::Transport(s) if s == "tls")));
         assert!(uri.parameters.iter().any(|p| matches!(p, Param::Maddr(s) if s == "192.0.2.1")));
         assert!(!uri.headers.is_empty());
         let headers = uri.headers;
         assert_eq!(headers.get("Subject"), Some(&"Hello".to_string()));
    }

    #[test]
    fn test_invalid_uri_scheme() {
        let uri_bytes = b"http:user@example.com";
        let result = parse_uri(uri_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_uri_no_host() {
        let uri_bytes = b"sip:";
        let result = parse_uri(uri_bytes);
        assert!(result.is_err());
    }
} 