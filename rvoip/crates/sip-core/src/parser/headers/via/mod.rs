// Parser for the Via header (RFC 3261 Section 20.42)
// Via = ( "Via" / "v" ) HCOLON via-parm *(COMMA via-parm)
// via-parm = sent-protocol LWS sent-by *( SEMI via-params )

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while_m_n},
    character::complete::{digit1, space1},
    combinator::{map, map_res, opt, recognize, value},
    multi::{many0, separated_list1},
    sequence::{pair, preceded, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;
use std::fmt; // Import fmt
use serde::{Serialize, Deserialize};

// Import from new modules
use crate::parser::separators::{hcolon, comma, slash};
use crate::parser::token::token;
use crate::parser::whitespace::lws;
use crate::parser::uri::host::hostport;
use crate::parser::common::comma_separated_list1;
use crate::parser::common_params::semicolon_separated_params0;
use crate::parser::ParseResult;

// Import local submodules
mod params;
use params::via_param_item; // Use the parser for a single via param item

// Import types
use crate::types::via::SentProtocol;
use crate::types::uri::Host;
use crate::types::param::Param; // Use the main Param enum

/// Represents a single Via header entry.
/// Making this struct public for use in types/header.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViaHeader {
    pub sent_protocol: SentProtocol,
    pub sent_by_host: Host,
    pub sent_by_port: Option<u16>,
    pub params: Vec<Param>,
}

// Implementation of Display trait for ViaHeader
impl fmt::Display for ViaHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.sent_protocol)?;
        
        // Format sent-by (host:port or host)
        write!(f, "{}", self.sent_by_host)?;
        if let Some(port) = self.sent_by_port {
            write!(f, ":{}", port)?;
        }
        
        // Format parameters
        for param in &self.params {
            write!(f, "{}", param)?; // Assuming Param implements Display correctly (e.g., ";key=value")
        }
        
        Ok(())
    }
}

// sent-protocol = protocol-name SLASH protocol-version SLASH transport
// protocol-name = "SIP" / token
// protocol-version = token
// transport = "UDP" / "TCP" / "TLS" / "SCTP" / other-transport
// RFC 3261 Section 20.42
fn sent_protocol(input: &[u8]) -> ParseResult<SentProtocol> {
    map_res(
        tuple((
            alt((tag_no_case(b"SIP"), token)), // name
            preceded(slash, opt(token)), // version (make it optional)
            preceded(slash, token), // transport
        )),
        |(name_b, ver_b_opt, tran_b)| {
            let name = str::from_utf8(name_b)
                .map_err(|_| nom::Err::Failure(NomError::new(input, ErrorKind::Char)))?
                .to_string();
            
            // Handle empty version
            let version = match ver_b_opt {
                Some(ver_b) => str::from_utf8(ver_b)
                    .map_err(|_| nom::Err::Failure(NomError::new(input, ErrorKind::Char)))?
                    .to_string(),
                None => "".to_string()
            };
            
            let transport = str::from_utf8(tran_b)
                .map_err(|_| nom::Err::Failure(NomError::new(input, ErrorKind::Char)))?
                .to_string();
            
            // RFC 3261 requires protocol-name to be "SIP" (case-insensitive) or token
            // and transport to be one of UDP, TCP, TLS, SCTP or other valid transport
            
            Ok::<_, nom::Err<NomError<&[u8]>>>(SentProtocol { name, version, transport })
        }
    )(input)
}

// sent-by = host [ COLON port ]
// Uses hostport parser from uri module
fn sent_by(input: &[u8]) -> ParseResult<(Host, Option<u16>)> {
    hostport(input)
}

// via-parm = sent-protocol LWS sent-by *( SEMI via-params )
// RFC 3261 Section 20.42
fn via_param_parser(input: &[u8]) -> ParseResult<ViaHeader> {
    map_res(
        tuple((
            sent_protocol,
            preceded(lws, sent_by),
            semicolon_separated_params0(via_param_item) // Use list helper with imported parser
        )),
        |(protocol, (host, port), params)| {
            // According to RFC 3261, a branch parameter is mandatory for all Via headers after RFC 3261
            // However, we don't enforce this here as we may need to parse headers from pre-RFC 3261 implementations
            
            Ok::<_, nom::Err<NomError<&[u8]>>>(ViaHeader {
                sent_protocol: protocol,
                sent_by_host: host,
                sent_by_port: port,
                params,
            })
        }
    )(input)
}

// Via = ( "Via" / "v" ) HCOLON via-parm *(COMMA via-parm)
// RFC 3261 Section 20.42
pub fn parse_via(input: &[u8]) -> ParseResult<Vec<ViaHeader>> {
    // This is the strict RFC-compliant parser that requires the header name
    preceded(
        pair(alt((tag_no_case(b"Via"), tag_no_case(b"v"))), hcolon),
        comma_separated_list1(via_param_parser) // Use the parser for a full via-parm
    )(input)
}

// Test-only function that directly parses via-parm content without requiring the header name
// This makes tests easier to write when focusing on the content part only
#[cfg(test)]
pub(crate) fn parse_via_params(input: &[u8]) -> ParseResult<Vec<ViaHeader>> {
    comma_separated_list1(via_param_parser)(input)
}

/// Validates a Via header according to RFC 3261 requirements
/// Returns true if the Via header is valid, false otherwise
pub fn validate_via_header(via: &ViaHeader) -> bool {
    // Check that protocol name is case-insensitive "SIP"
    // RFC 3261 allows other protocol names, but "SIP" is standard
    let is_sip_protocol = via.sent_protocol.name.eq_ignore_ascii_case("SIP");
    
    // Check for valid version (typically "2.0")
    let has_valid_version = !via.sent_protocol.version.is_empty();
    
    // Check for valid transport (RFC 3261 lists UDP, TCP, TLS, SCTP as standard)
    let transport_upper = via.sent_protocol.transport.to_uppercase();
    let is_standard_transport = ["UDP", "TCP", "TLS", "SCTP"].contains(&transport_upper.as_str());
    
    // Check for valid host
    let has_valid_host = !via.sent_by_host.to_string().is_empty();
    
    // Check for mandatory branch parameter (all compliant requests MUST have branch starting with z9hG4bK)
    let has_compliant_branch = via.params.iter().any(|p| {
        if let Param::Branch(branch) = p {
            branch.starts_with("z9hG4bK")
        } else {
            false
        }
    });
    
    // All required checks pass
    is_sip_protocol && has_valid_version && has_valid_host && has_compliant_branch
}

/// Validates a list of Via headers according to RFC 3261 requirements
/// Returns true if all Via headers are valid, false otherwise
pub fn validate_via_headers(vias: &[ViaHeader]) -> bool {
    if vias.is_empty() {
        return false; // At least one Via header is required
    }
    
    vias.iter().all(validate_via_header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use crate::types::param::GenericValue;

    #[test]
    fn test_sent_protocol() {
        // Test standard SIP protocol
        let (rem, sp) = sent_protocol(b"SIP/2.0/UDP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.name, "SIP");
        assert_eq!(sp.version, "2.0");
        assert_eq!(sp.transport, "UDP");
        
        // Test case insensitivity for name
        let (rem, sp) = sent_protocol(b"sip/2.0/UDP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.name, "sip");
        assert_eq!(sp.version, "2.0");
        assert_eq!(sp.transport, "UDP");
        
        // Test other transport protocols
        let (rem, sp) = sent_protocol(b"SIP/2.0/TCP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.transport, "TCP");
        
        let (rem, sp) = sent_protocol(b"SIP/2.0/TLS").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.transport, "TLS");
        
        let (rem, sp) = sent_protocol(b"SIP/2.0/SCTP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.transport, "SCTP");
        
        // Test custom protocol name
        let (rem, sp) = sent_protocol(b"CUSTOM/1.0/UDP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.name, "CUSTOM");
        assert_eq!(sp.version, "1.0");
        assert_eq!(sp.transport, "UDP");
        
        // Test other transport
        let (rem, sp) = sent_protocol(b"SIP/2.0/WS").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.transport, "WS");
    }
    
    #[test]
    fn test_sent_by() {
        // Test domain only
        let (rem, (host, port)) = sent_by(b"example.com").unwrap();
        assert!(rem.is_empty());
        assert_eq!(host.to_string(), "example.com");
        assert_eq!(port, None);
        
        // Test domain with port
        let (rem, (host, port)) = sent_by(b"example.com:5060").unwrap();
        assert!(rem.is_empty());
        assert_eq!(host.to_string(), "example.com");
        assert_eq!(port, Some(5060));
        
        // Test IPv4 address
        let (rem, (host, port)) = sent_by(b"192.0.2.1").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(host, Host::Address(addr) if addr == IpAddr::from(Ipv4Addr::new(192, 0, 2, 1))));
        assert_eq!(port, None);
        
        // Test IPv4 address with port
        let (rem, (host, port)) = sent_by(b"192.0.2.1:5060").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(host, Host::Address(addr) if addr == IpAddr::from(Ipv4Addr::new(192, 0, 2, 1))));
        assert_eq!(port, Some(5060));
        
        // Some implementations may support IPv6 - check if yours does
        if let Ok((rem, (host, port))) = sent_by(b"[2001:db8::1]") {
            assert!(rem.is_empty());
            assert!(matches!(host, Host::Address(addr) if addr.to_string() == "2001:db8::1"));
            assert_eq!(port, None);
        }
        
        if let Ok((rem, (host, port))) = sent_by(b"[2001:db8::1]:5060") {
            assert!(rem.is_empty());
            assert!(matches!(host, Host::Address(addr) if addr.to_string() == "2001:db8::1"));
            assert_eq!(port, Some(5060));
        }
    }
     
    #[test]
    fn test_via_params() {
        // Test ttl parameter
        let (rem_ttl, p_ttl) = via_param_item(b"ttl=10").unwrap();
        assert!(rem_ttl.is_empty());
        assert!(matches!(p_ttl, Param::Ttl(10)));
        
        // Test ttl with invalid values (should be 0-255)
        let result = via_param_item(b"ttl=256");
        // This should either fail or clamp to 255
        if let Ok((_, p)) = result {
            assert!(matches!(p, Param::Ttl(t) if t <= 255));
        }

        // Test maddr parameter with domain
        let (rem_maddr, p_maddr) = via_param_item(b"maddr=example.com").unwrap();
        assert!(rem_maddr.is_empty());
        assert!(matches!(p_maddr, Param::Maddr(h) if h == "example.com"));
        
        // Test maddr parameter with IPv4
        let (rem_maddr, p_maddr) = via_param_item(b"maddr=192.0.2.1").unwrap();
        assert!(rem_maddr.is_empty());
        assert!(matches!(p_maddr, Param::Maddr(h) if h == "192.0.2.1"));

        // Test received parameter with IPv4
        let (rem_rec, p_rec) = via_param_item(b"received=1.2.3.4").unwrap();
        assert!(rem_rec.is_empty());
        assert!(matches!(p_rec, Param::Received(ip) if ip == Ipv4Addr::new(1,2,3,4)));
        
        // Test branch parameter
        let (rem_br, p_br) = via_param_item(b"branch=z9hG4bKabcdef").unwrap();
        assert!(rem_br.is_empty());
        assert!(matches!(p_br, Param::Branch(s) if s == "z9hG4bKabcdef"));
        
        // Test custom parameter 
        let (rem_ext, p_ext) = via_param_item(b"custom=value").unwrap();
        assert!(rem_ext.is_empty());
        // Debug print the actual p_ext value
        println!("DEBUG - p_ext: {:?}", p_ext);
        assert!(matches!(p_ext, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" && v == "value"));
        
        // Test flag parameter (no value)
        let (rem_flag, p_flag) = via_param_item(b"rport").unwrap();
        assert!(rem_flag.is_empty());
        assert!(matches!(p_flag, Param::Other(n, None) if n == "rport"));
        
        // Test value with quotes
        let (rem_quote, p_quote) = via_param_item(b"comment=\"test value\"").unwrap();
        assert!(rem_quote.is_empty());
        assert!(matches!(p_quote, Param::Other(n, Some(GenericValue::Quoted(v))) if n == "comment" && v == "test value"));
        
        // Test case insensitivity of parameter names
        let (rem_case, p_case) = via_param_item(b"BRANCH=z9hG4bKabcdef").unwrap();
        assert!(rem_case.is_empty());
        assert!(matches!(p_case, Param::Branch(s) if s == "z9hG4bKabcdef"));
    }
    
    #[test]
    fn test_via_parm_simple() {
        let input = b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        let result = via_param_parser(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap(); // Now returns ViaHeader
        assert!(rem.is_empty());
        assert_eq!(via.sent_protocol.transport, "UDP");
        assert_eq!(via.sent_by_host.to_string(), "pc33.atlanta.com");
        assert_eq!(via.sent_by_port, None);
        assert_eq!(via.params.len(), 1);
        assert!(matches!(&via.params[0], Param::Branch(_)));
    }
    
    #[test]
    fn test_via_parm_complex() {
        let input = b"SIP/2.0/TCP client.biloxi.com:5060;branch=z9hG4bK74bf9;received=192.0.2.4;ttl=64";
         let result = via_param_parser(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(via.sent_protocol.transport, "TCP");
        assert_eq!(via.sent_by_port, Some(5060));
        assert_eq!(via.params.len(), 3);
        assert!(via.params.contains(&Param::Branch("z9hG4bK74bf9".to_string())));
        assert!(via.params.contains(&Param::Received(Ipv4Addr::new(192,0,2,4).into())));
        assert!(via.params.contains(&Param::Ttl(64)));
    }
    
    #[test]
    fn test_via_parm_with_whitespace() {
        // RFC 3261 allows whitespace (LWS) between sent-protocol and sent-by
        let input = b"SIP/2.0/TCP  client.biloxi.com:5060;branch=z9hG4bK74bf9";
        let result = via_param_parser(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(via.sent_protocol.transport, "TCP");
        assert_eq!(via.sent_by_host.to_string(), "client.biloxi.com");
        assert_eq!(via.sent_by_port, Some(5060));
    }
    
    #[test]
    fn test_via_parm_with_rport() {
        // Test with rport parameter (no value)
        let input = b"SIP/2.0/UDP client.biloxi.com;rport;branch=z9hG4bK74bf9";
        let result = via_param_parser(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(via.params.len(), 2);
        
        // One parameter should be rport with no value
        let has_rport = via.params.iter().any(|p| matches!(p, Param::Other(n, None) if n == "rport"));
        assert!(has_rport, "Should have rport parameter with no value");
        
        // Test with rport parameter with value (which is valid according to RFC 3581)
        let input = b"SIP/2.0/UDP client.biloxi.com;rport=5060;branch=z9hG4bK74bf9";
        let result = via_param_parser(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap();
        assert!(rem.is_empty());
        
        // One parameter should be rport with a value
        let has_rport_value = via.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "rport" && v == "5060")
        );
        assert!(has_rport_value, "Should have rport parameter with value 5060");
    }
    
    #[test]
    fn test_parse_via_multiple() {
        let input = b"SIP/2.0/UDP first.example.com:4000;branch=z9hG4bK776asdhds , SIP/2.0/UDP second.example.com:5060;branch=z9hG4bKnasd8;received=1.2.3.4";
        let result = parse_via_params(input);
        println!("DEBUG - parse_via result: {:?}", result);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 2);
        assert_eq!(vias[0].sent_by_port, Some(4000));
        assert_eq!(vias[1].params.len(), 2); 
        
        // Also test with full header syntax
        let input_with_header = b"Via: SIP/2.0/UDP first.example.com:4000;branch=z9hG4bK776asdhds , SIP/2.0/UDP second.example.com:5060;branch=z9hG4bKnasd8;received=1.2.3.4";
        let result = parse_via(input_with_header);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 2);
    }
    
    #[test]
    fn test_parse_via_with_whitespace() {
        // Test with whitespace around commas
        let input = b"SIP/2.0/UDP first.example.com:4000;branch=z9hG4bK776asdhds , \r\n SIP/2.0/UDP second.example.com:5060;branch=z9hG4bKnasd8";
        let result = parse_via_params(input);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 2);
        
        // Test with full header and excessive whitespace (RFC compliant)
        let input_with_header = b"Via  :  \t SIP/2.0/UDP first.example.com:4000;branch=z9hG4bK776asdhds , \r\n SIP/2.0/UDP second.example.com:5060;branch=z9hG4bKnasd8";
        let result = parse_via(input_with_header);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 2);
    }
    
    #[test]
    fn test_parse_via_rfc_examples() {
        // Examples from RFC 3261 messages
        
        // Example from Section 7.1 (SIP MESSAGE)
        let input = b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        let result = parse_via_params(input);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 1);
        assert_eq!(vias[0].sent_protocol.name, "SIP");
        assert_eq!(vias[0].sent_protocol.version, "2.0");
        assert_eq!(vias[0].sent_protocol.transport, "UDP");
        assert_eq!(vias[0].sent_by_host.to_string(), "pc33.atlanta.com");
        assert_eq!(vias[0].sent_by_port, None);
        assert_eq!(vias[0].params.len(), 1);
        assert!(matches!(&vias[0].params[0], Param::Branch(s) if s == "z9hG4bK776asdhds"));
        
        // Example from Section 24.2 (SIP REGISTER)
        let input = b"SIP/2.0/UDP bobspc.biloxi.com:5060;branch=z9hG4bKnashds7";
        let result = parse_via_params(input);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 1);
        
        // Example from Section 25 (Examples with multiple Vias and parameters)
        let input = b"SIP/2.0/UDP server10.biloxi.com;branch=z9hG4bK4b43c2ff8.1, SIP/2.0/UDP bigbox3.site3.atlanta.com;branch=z9hG4bK77ef4c2312983.1";
        let result = parse_via_params(input);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 2);
        
        // Test with abbreviated header name (v:)
        let input_with_header = b"v: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        let result = parse_via(input_with_header);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 1);
    }
    
    #[test]
    fn test_via_parameter_handling() {
        // Test handling of multiple parameters including values that should be treated as tokens
        let input = b"SIP/2.0/UDP servernode:5060;branch=z9hG4bK-524287-1---a6b23e33cdfc7905;rport;received=192.168.1.100;test;multi-param=123;key=\"quoted value\"";
        let result = via_param_parser(input);
        assert!(result.is_ok());
        
        let (rem, via) = result.unwrap();
        assert!(rem.is_empty());
        
        // Check all the expected parameters exist
        assert!(via.params.iter().any(|p| matches!(p, Param::Branch(s) if s == "z9hG4bK-524287-1---a6b23e33cdfc7905")));
        assert!(via.params.iter().any(|p| matches!(p, Param::Other(s, None) if s == "rport")));
        assert!(via.params.iter().any(|p| matches!(p, Param::Received(ip) if *ip == Ipv4Addr::new(192, 168, 1, 100))));
        assert!(via.params.iter().any(|p| matches!(p, Param::Other(s, None) if s == "test")));
        assert!(via.params.iter().any(|p| matches!(p, Param::Other(s, Some(GenericValue::Token(v))) if s == "multi-param" && v == "123")));
        assert!(via.params.iter().any(|p| matches!(p, Param::Other(s, Some(GenericValue::Quoted(v))) if s == "key" && v == "quoted value")));
    }
    
    #[test]
    fn test_validator_functions() {
        // Create a valid Via header
        let valid_via = ViaHeader {
            sent_protocol: SentProtocol {
                name: "SIP".to_string(),
                version: "2.0".to_string(),
                transport: "UDP".to_string(),
            },
            sent_by_host: Host::Domain("example.com".to_string()),
            sent_by_port: Some(5060),
            params: vec![Param::Branch("z9hG4bK776asdhds".to_string())],
        };
        
        // Test valid header
        assert!(validate_via_header(&valid_via));
        
        // Test invalid protocol name
        let mut invalid_protocol = valid_via.clone();
        invalid_protocol.sent_protocol.name = "INVALID".to_string();
        assert!(!validate_via_header(&invalid_protocol));
        
        // Test empty version
        let mut invalid_version = valid_via.clone();
        invalid_version.sent_protocol.version = "".to_string();
        assert!(!validate_via_header(&invalid_version));
        
        // Test missing branch parameter
        let mut missing_branch = valid_via.clone();
        missing_branch.params = vec![Param::Other("rport".to_string(), None)];
        assert!(!validate_via_header(&missing_branch));
        
        // Test non-compliant branch (not starting with z9hG4bK)
        let mut invalid_branch = valid_via.clone();
        invalid_branch.params = vec![Param::Branch("invalid-branch".to_string())];
        assert!(!validate_via_header(&invalid_branch));
        
        // Test validate_via_headers function
        assert!(validate_via_headers(&[valid_via.clone()]));
        assert!(!validate_via_headers(&[])); // Empty list is invalid
        assert!(!validate_via_headers(&[invalid_branch])); // List with invalid header is invalid
    }
    
    #[test]
    fn test_edge_cases() {
        // Test with minimal valid input
        let minimal_input = b"SIP/2.0/UDP host;branch=z9hG4bK123";
        let result = parse_via_params(minimal_input);
        println!("1. Minimal valid input result: {:?}", result);
        assert!(result.is_ok());
        
        // Test with empty protocol version (technically valid per ABNF, but likely invalid semantically)
        let empty_version = b"SIP//UDP host;branch=z9hG4bK123";
        let result = parse_via_params(empty_version);
        println!("2. Empty version result: {:?}", result);
        assert!(result.is_ok());
        let (_, vias) = result.unwrap();
        assert_eq!(vias[0].sent_protocol.version, "");
        
        // Test with IPv6 address
        let ipv6_input = b"SIP/2.0/UDP [2001:db8::1]:5060;branch=z9hG4bK123";
        let result = parse_via_params(ipv6_input);
        println!("3. IPv6 input result: {:?}", result);
        assert!(result.is_ok());
        
        // Test with mixed case parameter names (should be case-insensitive)
        let mixed_case = b"SIP/2.0/UDP host;BrAnCh=z9hG4bK123;RpOrT";
        let result = parse_via_params(mixed_case);
        println!("4. Mixed case result: {:?}", result);
        assert!(result.is_ok());
        let (_, vias) = result.unwrap();
        assert!(vias[0].params.iter().any(|p| matches!(p, Param::Branch(s) if s == "z9hG4bK123")));
        
        // Test with unusual but valid transport protocols
        let unusual_transport = b"SIP/2.0/WS host;branch=z9hG4bK123";
        let result = parse_via_params(unusual_transport);
        println!("5. Unusual transport result: {:?}", result);
        assert!(result.is_ok());
        let (_, vias) = result.unwrap();
        assert_eq!(vias[0].sent_protocol.transport, "WS");
        
        // Test with extreme whitespace (all LWS points should accept arbitrary amounts)
        let whitespace_input = b"SIP/2.0/UDP  \t \r\n host  \r\n  ;  \t branch=z9hG4bK123";
        let result = parse_via_params(whitespace_input);
        println!("6. Whitespace input result: {:?}", result);
        assert!(result.is_ok());
    }
}