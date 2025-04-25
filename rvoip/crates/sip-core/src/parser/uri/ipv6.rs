use nom::{
    bytes::complete::{tag, take_while1},
    combinator::{map_res, verify},
    sequence::{delimited, tuple},
    IResult, Err,
};
use std::net::IpAddr;
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;

// IPv6reference = "[" IPv6address "]"
// Improved IPv6address parser that properly validates both the content and ensures
// the closing bracket is present, otherwise returns an error.
pub fn ipv6_reference(input: &[u8]) -> ParseResult<Host> {
    // First check if there's an opening bracket but no closing bracket
    if input.starts_with(b"[") && !input.contains(&b']') {
        return Err(Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Char)));
    }
    
    map_res(
        delimited(
            tag(b"["),
            take_while1(|c: u8| c.is_ascii_hexdigit() || c == b':' || c == b'.' || c == b'%'),
            tag(b"]"),
        ),
        |bytes| {
            str::from_utf8(bytes)
                .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Char)))
                .and_then(|s| s.parse::<IpAddr>()
                    .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Verify)))) // Verify ensures valid IPv6 syntax
                .map(Host::Address)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv6Addr, Ipv4Addr};
    
    #[test]
    fn test_ipv6_reference_standard() {
        // Standard IPv6 addresses
        let (rem, host) = ipv6_reference(b"[2001:db8::1]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))));

        // Full form
        let (rem, host) = ipv6_reference(b"[2001:0db8:0000:0000:0000:0000:0000:0001]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))));
        
        // With trailing content
        let (rem, host) = ipv6_reference(b"[2001:db8::1]:5060").unwrap();
        assert_eq!(rem, b":5060");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))));
    }
    
    #[test]
    fn test_ipv6_reference_special_forms() {
        // Loopback address
        let (rem, host) = ipv6_reference(b"[::1]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))));
        
        // Unspecified address
        let (rem, host) = ipv6_reference(b"[::]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0))));
        
        // IPv4-mapped IPv6 address
        let (rem, host) = ipv6_reference(b"[::ffff:192.168.0.1]").unwrap();
        assert_eq!(rem, b"");
        let expected = Ipv6Addr::from(Ipv4Addr::new(192, 168, 0, 1).to_ipv6_mapped());
        assert_eq!(host, Host::Address(IpAddr::V6(expected)));
    }
    
    #[test]
    fn test_ipv6_reference_with_zone() {
        // IPv6 with zone identifier - not officially in RFC 3261, but in RFC 6874
        let zone_test = b"[fe80::1%eth0]";
        // Note: The standard Rust IpAddr parser doesn't support zone IDs, so if the implementation
        // is changed to support them, this test would need to be updated
        assert!(ipv6_reference(zone_test).is_err());
    }
    
    #[test]
    fn test_ipv6_reference_invalid() {
        // Missing brackets
        assert!(ipv6_reference(b"2001:db8::1").is_err());
        
        // Invalid characters
        assert!(ipv6_reference(b"[2001:db8::g]").is_err());
        
        // Too many segments
        assert!(ipv6_reference(b"[2001:db8:1:2:3:4:5:6:7]").is_err());
        
        // Invalid IPv4-mapped format
        assert!(ipv6_reference(b"[::ffff:192.168.0.300]").is_err());
        
        // Empty brackets
        assert!(ipv6_reference(b"[]").is_err());
        
        // Unclosed bracket
        assert!(ipv6_reference(b"[2001:db8::1").is_err());
    }
    
    #[test]
    fn test_ipv6_reference_rfc_examples() {
        // From RFC 3261 Section 19.1.1
        let (rem, host) = ipv6_reference(b"[2001:db8::9:1]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 9, 1))));
        
        // From RFC 4291 (IPv6 Address Architecture) Section 2.2
        // Link-local prefix
        let (rem, host) = ipv6_reference(b"[fe80::1]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))));
        
        // Site-local prefix (deprecated by RFC 3879, but still valid syntax)
        let (rem, host) = ipv6_reference(b"[fec0::1]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0xfec0, 0, 0, 0, 0, 0, 0, 1))));
        
        // From RFC 3986 Section 3.2.2 (URI Generic Syntax)
        let (rem, host) = ipv6_reference(b"[FEDC:BA98:7654:3210:FEDC:BA98:7654:3210]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0xFEDC, 0xBA98, 0x7654, 0x3210, 0xFEDC, 0xBA98, 0x7654, 0x3210))));
        
        let (rem, host) = ipv6_reference(b"[1080:0:0:0:8:800:200C:417A]").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V6(Ipv6Addr::new(0x1080, 0, 0, 0, 8, 0x800, 0x200C, 0x417A))));
    }
} 