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
};
use std::str;

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
use crate::types::via::{ViaHeader, SentProtocol};
use crate::uri::Host;
use crate::types::param::Param; // Use the main Param enum

// sent-protocol = protocol-name SLASH protocol-version SLASH transport
// protocol-name = "SIP" / token
// protocol-version = token
// transport = "UDP" / "TCP" / "TLS" / "SCTP" / other-transport
fn sent_protocol(input: &[u8]) -> ParseResult<SentProtocol> {
    map_res(
        tuple((
            alt((tag_no_case(b"SIP"), token)), // name
            preceded(slash, token), // version
            preceded(slash, token), // transport
        )),
        |(name_b, ver_b, tran_b)| {
            let name = str::from_utf8(name_b)?.to_string();
            let version = str::from_utf8(ver_b)?.to_string();
            let transport = str::from_utf8(tran_b)?.to_string();
            Ok(SentProtocol { name, version, transport })
        }
    )(input)
}

// sent-by = host [ COLON port ]
// Uses hostport parser from uri module
fn sent_by(input: &[u8]) -> ParseResult<(Host, Option<u16>)> {
    hostport(input)
}

// via-parm = sent-protocol LWS sent-by *( SEMI via-params )
fn via_param_parser(input: &[u8]) -> ParseResult<ViaHeader> {
    map(
        tuple((
            sent_protocol,
            preceded(lws, sent_by),
            semicolon_separated_params0(via_param_item) // Use list helper
        )),
        |(protocol, (host, port), params)| ViaHeader {
            sent_protocol: protocol,
            sent_by_host: host,
            sent_by_port: port,
            params,
        }
    )(input)
}

// Via = ( "Via" / "v" ) HCOLON via-parm *(COMMA via-parm)
pub(crate) fn parse_via(input: &[u8]) -> ParseResult<Vec<ViaHeader>> {
    preceded(
        pair(alt((tag_no_case(b"Via"), tag_no_case(b"v"))), hcolon),
        comma_separated_list1(via_param_parser) // Use the parser for a full via-parm
    )(input)
}

// ttl = 1*3DIGIT ; 0 to 255
fn ttl(input: &[u8]) -> ParseResult<u8> {
    map_res(
        take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()),
        |ttl_bytes| str::from_utf8(ttl_bytes)?.parse::<u8>() // u8 handles 0-255 range
    )(input)
}

// via-ttl = "ttl" EQUAL ttl
fn via_ttl(input: &[u8]) -> ParseResult<Param> { // Changed return type to Param
    map(
        preceded(pair(tag_no_case(b"ttl"), equal), ttl),
        Param::Ttl
    )(input)
}

// via-maddr = "maddr" EQUAL host
fn via_maddr(input: &[u8]) -> ParseResult<Param> { // Changed return type to Param
    map(
        preceded(
            pair(tag_no_case(b"maddr"), equal),
            // Map Host to String for Param::Maddr
            map_res(host, |h| Ok::<_, ()>(h.to_string())) 
        ),
        Param::Maddr
    )(input)
}

// IPv4address / IPv6address (for received param)
fn ip_address_only(input: &[u8]) -> ParseResult<std::net::IpAddr> { // Return IpAddr directly
     alt(( 
         map_res(ipv4_address, |h| if let Host::Address(ip) = h { Ok(ip) } else { Err("Expected IP Address") }), 
         map_res(ipv6_reference, |h| if let Host::Address(ip) = h { Ok(ip) } else { Err("Expected IP Address") })
     ))(input)
}

// via-received = "received" EQUAL (IPv4address / IPv6address)
fn via_received(input: &[u8]) -> ParseResult<Param> { // Changed return type to Param
    map(
        preceded(pair(tag_no_case(b"received"), equal), ip_address_only),
        Param::Received
    )(input)
}

// via-branch = "branch" EQUAL token
fn via_branch(input: &[u8]) -> ParseResult<Param> { // Changed return type to Param
    map(
        preceded(
            pair(tag_no_case(b"branch"), equal),
            map_res(token, |b| str::from_utf8(b).map(String::from))
        ),
        Param::Branch
    )(input)
}

// via-params = via-ttl / via-maddr / via-received / via-branch / via-extension
// via-extension = generic-param
fn via_params_item(input: &[u8]) -> ParseResult<Param> { // Returns Param
    alt((
        via_ttl,
        via_maddr,
        via_received,
        via_branch,
        generic_param, // generic_param already returns Param::Other
    ))(input)
}

// via-parm = sent-protocol LWS sent-by *( SEMI via-params )
// Returns ViaHeader struct
fn via_parm(input: &[u8]) -> ParseResult<ViaHeader> {
    map(
        tuple((
            sent_protocol, // (name, version, transport)
            preceded(lws, sent_by), // (Host, Option<u16>)
            many0(preceded(semi, via_params_item)) // Vec<Param>
        )),
        |((name, version, transport), (host, port), params_vec)| ViaHeader {
            protocol: name,
            version: version,
            transport: transport,
            host: host.to_string(), // Convert Host to String
            port: port,
            params: params_vec,
        }
    )(input)
}

// Via = ("Via" / "v") HCOLON via-parm *(COMMA via-parm)
// Note: HCOLON and compact form handled elsewhere.
pub(crate) fn parse_via(input: &[u8]) -> ParseResult<Vec<ViaHeader>> {
    comma_separated_list1(via_parm)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use crate::types::param::GenericValue;

    #[test]
    fn test_sent_protocol() {
        let (rem, sp) = sent_protocol(b"SIP/2.0/UDP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(sp.0, "SIP");
        assert_eq!(sp.1, "2.0");
        assert_eq!(sp.2, "UDP");
    }
    
     #[test]
    fn test_via_params() {
        let (rem_ttl, p_ttl) = via_params_item(b"ttl=10").unwrap();
        assert!(rem_ttl.is_empty());
        assert!(matches!(p_ttl, Param::Ttl(10)));

        let (rem_maddr, p_maddr) = via_params_item(b"maddr=192.0.2.1").unwrap();
        assert!(rem_maddr.is_empty());
        assert!(matches!(p_maddr, Param::Maddr(h) if h == "192.0.2.1"));

        let (rem_rec, p_rec) = via_params_item(b"received=1.2.3.4").unwrap();
        assert!(rem_rec.is_empty());
        assert!(matches!(p_rec, Param::Received(ip) if ip == Ipv4Addr::new(1,2,3,4)));

        let (rem_br, p_br) = via_params_item(b"branch=z9hG4bKabcdef").unwrap();
        assert!(rem_br.is_empty());
        assert!(matches!(p_br, Param::Branch(s) if s == "z9hG4bKabcdef"));

        let (rem_ext, p_ext) = via_params_item(b"custom=value").unwrap();
        assert!(rem_ext.is_empty());
        assert!(matches!(p_ext, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" && v == "value"));
    }
    
    #[test]
    fn test_via_parm_simple() {
        let input = b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        let result = via_parm(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap(); // Now returns ViaHeader
        assert!(rem.is_empty());
        assert_eq!(via.transport, "UDP");
        assert_eq!(via.host, "pc33.atlanta.com");
        assert_eq!(via.port, None);
        assert_eq!(via.params.len(), 1);
        assert!(matches!(via.params[0], Param::Branch(_)));
    }
    
    #[test]
    fn test_via_parm_complex() {
        let input = b"SIP/2.0/TCP client.biloxi.com:5060;branch=z9hG4bK74bf9;received=192.0.2.4;ttl=64";
         let result = via_parm(input);
        assert!(result.is_ok());
        let (rem, via) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(via.transport, "TCP");
        assert_eq!(via.port, Some(5060));
        assert_eq!(via.params.len(), 3);
        assert!(via.params.contains(&Param::Branch("z9hG4bK74bf9".to_string())));
        assert!(via.params.contains(&Param::Received(Ipv4Addr::new(192,0,2,4).into())));
        assert!(via.params.contains(&Param::Ttl(64)));
    }
    
    #[test]
    fn test_parse_via_multiple() {
        let input = b"SIP/2.0/UDP first.example.com:4000;branch=z9hG4bK776asdhds , SIP/2.0/UDP second.example.com:5060;branch=z9hG4bKnasd8;received=1.2.3.4";
        let result = parse_via(input);
        assert!(result.is_ok());
        let (rem, vias) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vias.len(), 2);
        assert_eq!(vias[0].port, Some(4000));
        assert_eq!(vias[1].params.len(), 2); 
    }
}