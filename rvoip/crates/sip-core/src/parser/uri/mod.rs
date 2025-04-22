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
    bytes::complete::tag,
    combinator::{map, map_res, opt},
    sequence::tuple,
    IResult,
};
use std::collections::HashMap;
use std::str;

use crate::types::uri::{Host, Uri};
use crate::types::param::Param;
use crate::parser::ParseResult;

// SIP-URI = "sip:" [ userinfo ] hostport uri-parameters [ headers ]
fn sip_uri(input: &[u8]) -> ParseResult<Uri> {
    map_res(
        tuple((
            tag("sip:"),
            opt(userinfo),
            hostport,       // Returns (Host, Option<u16>)
            uri_parameters, // Returns Vec<Param>
            opt(uri_headers), // Returns Option<HashMap<String, String>>
        )),
        |(_, userinfo_opt, (host_val, port_opt), params, headers_opt)| {
            // Convert userinfo bytes to Strings, handling potential UTF-8 errors
            let converted_userinfo = userinfo_opt
                .map(|(user_bytes, pass_opt_bytes)| {
                    let user_str = str::from_utf8(user_bytes)?.to_string();
                    let pass_str_opt = pass_opt_bytes
                        .map(|p| str::from_utf8(p).map(|s| s.to_string()))
                        .transpose()?;
                    Ok::<_, str::Utf8Error>((user_str, pass_str_opt))
                })
                .transpose()?;
            
            Ok(Uri {
                scheme: "sip".to_string(),
                userinfo: converted_userinfo,
                host: host_val,
                port: port_opt,
                parameters: params,
                headers: headers_opt,
            })
        },
    )(input)
}

// SIPS-URI = "sips:" [ userinfo ] hostport uri-parameters [ headers ]
fn sips_uri(input: &[u8]) -> ParseResult<Uri> {
     map_res(
        tuple((
            tag("sips:"),
            opt(userinfo),
            hostport,
            uri_parameters,
            opt(uri_headers),
        )),
        |(_, userinfo_opt, (host_val, port_opt), params, headers_opt)| {
             // Convert userinfo bytes to Strings
            let converted_userinfo = userinfo_opt
                .map(|(user_bytes, pass_opt_bytes)| {
                    let user_str = str::from_utf8(user_bytes)?.to_string();
                    let pass_str_opt = pass_opt_bytes
                        .map(|p| str::from_utf8(p).map(|s| s.to_string()))
                        .transpose()?;
                    Ok::<_, str::Utf8Error>((user_str, pass_str_opt))
                })
                .transpose()?;

            Ok(Uri {
                scheme: "sips".to_string(),
                userinfo: converted_userinfo,
                host: host_val,
                port: port_opt,
                parameters: params,
                headers: headers_opt,
            })
        },
    )(input)
}

/// Public entry point for parsing a SIP or SIPS URI
/// Can be re-exported by the main parser mod.rs
pub fn parse_uri(input: &[u8]) -> ParseResult<Uri> {
    alt((sip_uri, sips_uri))(input)
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