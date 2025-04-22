// Declare URI sub-modules
pub mod host;
pub mod userinfo;
pub mod params;
pub mod headers;
pub mod absolute;

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
    bytes::complete as bytes,
    combinator::{map, map_res, opt, pair},
    sequence::tuple,
    IResult,
    error::ErrorKind,
    error::Error as NomError,
    combinator::recognize,
    error::{ErrorKind, Error as NomError, ParseError},
};
use std::collections::HashMap;
use std::str;

use crate::types::uri::{Host, Uri};
use crate::types::param::Param;
use crate::parser::ParseResult;

// SIP-URI = "sip:" [ userinfo ] hostport uri-parameters [ headers ]
pub(crate) fn parse_sip_uri(input: &[u8]) -> ParseResult<Uri> {
    map_res(
        recognize(
            tuple((
                bytes::tag(b"sip:".as_slice()),
                opt(pair(userinfo, at)),
                hostport,
                opt(uri_parameters),
                opt(uri_headers),
            ))
        ),
        |bytes| {
            let (rem, (_, user_opt, (host, port), params_opt, headers_opt)) = tuple((
                bytes::tag(b"sip:".as_slice()),
                opt(pair(userinfo, at)),
                hostport,
                opt(uri_parameters),
                opt(uri_headers),
            ))(bytes)?;

            if !rem.is_empty() {
                 return Err(NomError::new(rem, ErrorKind::Verify));
            }
            
            let (user, password) = user_opt
                .map(|(u, p_opt, _)| (Some(u), p_opt))
                .unwrap_or((None, None));

            Ok(Uri {
                scheme: Scheme::Sip,
                user: user.map(String::from),
                password: password.map(String::from),
                host,
                port,
                parameters: params_opt.unwrap_or_default(),
                headers: headers_opt.unwrap_or_default(),
            })
        }
    )(input)
}

// SIPS-URI = "sips:" [ userinfo ] hostport uri-parameters [ headers ]
pub(crate) fn parse_sips_uri(input: &[u8]) -> ParseResult<Uri> {
     map_res(
        recognize(
            tuple((
                bytes::tag(b"sips:".as_slice()),
                opt(pair(userinfo, at)),
                hostport,
                opt(uri_parameters),
                opt(uri_headers),
            ))
        ),
        |bytes| {
             let (rem, (_, user_opt, (host, port), params_opt, headers_opt)) = tuple((
                bytes::tag(b"sips:".as_slice()),
                opt(pair(userinfo, at)),
                hostport,
                opt(uri_parameters),
                opt(uri_headers),
            ))(bytes)?;

            if !rem.is_empty() {
                 return Err(NomError::new(rem, ErrorKind::Verify));
            }

            let (user, password) = user_opt
                .map(|(u, p_opt, _)| (Some(u), p_opt))
                .unwrap_or((None, None));
                
            Ok(Uri {
                scheme: Scheme::Sips,
                user: user.map(String::from),
                password: password.map(String::from),
                host,
                port,
                parameters: params_opt.unwrap_or_default(),
                headers: headers_opt.unwrap_or_default(),
            })
        }
    )(input)
}

/// Public entry point for parsing a SIP or SIPS URI
/// Can be re-exported by the main parser mod.rs
pub fn parse_uri(input: &[u8]) -> ParseResult<Uri> {
    alt((parse_sip_uri, parse_sips_uri))(input)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::ParamValue;
    use crate::types::uri::Host;
    use std::net::Ipv4Addr;
    use nom::error::ErrorKind;

    #[test]
    fn test_parse_simple_sip_uri() {
        let uri_bytes = b"sip:user@example.com";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, "sip");
        assert_eq!(uri.userinfo, Some(("user".to_string(), None)));
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert!(uri.parameters.is_empty());
        assert_eq!(uri.headers, None);
    }
    
    #[test]
    fn test_parse_sips_uri_with_port() {
        let uri_bytes = b"sips:alice@atlanta.com:5061";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, "sips");
        assert_eq!(uri.userinfo, Some(("alice".to_string(), None)));
        assert!(matches!(uri.host, Host::Domain(d) if d == "atlanta.com"));
        assert_eq!(uri.port, Some(5061));
        assert!(uri.parameters.is_empty());
        assert_eq!(uri.headers, None);
    }

    #[test]
    fn test_parse_sip_uri_ipv4() {
        let uri_bytes = b"sip:192.168.0.1:8080";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, "sip");
        assert_eq!(uri.userinfo, None);
        assert!(matches!(uri.host, Host::Address(addr) if addr == Ipv4Addr::new(192, 168, 0, 1).into()));
        assert_eq!(uri.port, Some(8080));
        assert!(uri.parameters.is_empty());
        assert_eq!(uri.headers, None);
    }

     #[test]
    fn test_parse_sip_uri_with_params() {
        let uri_bytes = b"sip:user@example.com;transport=tcp;lr";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, "sip");
        assert_eq!(uri.userinfo, Some(("user".to_string(), None)));
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert_eq!(uri.parameters.len(), 2);
        assert!(uri.parameters.contains(&Param::Transport("tcp".to_string())));
        assert!(uri.parameters.contains(&Param::Lr));
        assert_eq!(uri.headers, None);
    }

     #[test]
    fn test_parse_sip_uri_with_headers() {
        let uri_bytes = b"sip:user@example.com?Subject=Urgent&Priority=High";
        let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, "sip");
        assert_eq!(uri.userinfo, Some(("user".to_string(), None)));
        assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
        assert_eq!(uri.port, None);
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_some());
        let headers = uri.headers.unwrap();
        assert_eq!(headers.get("Subject"), Some(&"Urgent".to_string()));
        assert_eq!(headers.get("Priority"), Some(&"High".to_string()));
    }
    
    #[test]
    fn test_parse_sip_uri_complex() {
         let uri_bytes = b"sips:bob:password@[fe80::1]:5061;transport=tls;maddr=192.0.2.1?Subject=Hello";
         let (rem, uri) = parse_uri(uri_bytes).expect("Parsing failed");
         assert!(rem.is_empty());
         assert_eq!(uri.scheme, "sips");
         assert_eq!(uri.userinfo, Some(("bob".to_string(), Some("password".to_string()))));
         assert!(matches!(uri.host, Host::Address(_))); // Simplified check for IPv6
         assert_eq!(uri.port, Some(5061));
         assert_eq!(uri.parameters.len(), 2);
         assert!(uri.parameters.contains(&Param::Transport("tls".to_string())));
         assert!(uri.parameters.iter().any(|p| matches!(p, Param::Maddr(Host::Address(_)))));
         assert!(uri.headers.is_some());
         assert_eq!(uri.headers.unwrap().get("Subject"), Some(&"Hello".to_string()));
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
        assert!(result.is_err()); // Fails in hostport parser
    }
} 