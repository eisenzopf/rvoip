use nom::{
    bytes::complete::{tag, take_while_m_n},
    combinator::{map_res, recognize},
    error::ErrorKind,
    sequence::tuple,
    Err, IResult,
};
use std::net::{IpAddr, Ipv4Addr};
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;

// Helper function to parse a decimal octet value (1-255)
fn parse_decimal_octet(input: &[u8]) -> Result<u8, nom::Err<nom::error::Error<&[u8]>>> {
    // Ensure all characters are digits
    if !input.iter().all(|&c| c.is_ascii_digit()) {
        return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::Digit)));
    }
    
    // Safely convert to string
    let s = match str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::Char)))
    };
    
    // Parse as decimal (regardless of leading zeros)
    match s.parse::<u8>() {
        Ok(val) => Ok(val),
        Err(_) => Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::Verify)))
    }
}

// IPv4address = 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT
// Parse as decimal integer regardless of leading zeros
pub fn ipv4_address(input: &[u8]) -> ParseResult<Host> {
    // First try to match the pattern exactly with our constraints
    let (remaining, ip_bytes) = recognize(
        tuple((
            take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()), tag(b"."),
            take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()), tag(b"."),
            take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()), tag(b"."),
            take_while_m_n(1, 3, |c: u8| c.is_ascii_digit())
        ))
    )(input)?;
    
    // Check if there's another dot immediately after, which would make this an IPv4 with too many octets
    if !remaining.is_empty() && remaining[0] == b'.' {
        return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::TooLarge)));
    }
    
    // Now we need to ensure there are no more digits after what we've parsed
    // If the next character is a digit that's not part of a port/parameter/path, it means we have an octet with >3 digits
    if !remaining.is_empty() && remaining[0].is_ascii_digit() {
        // Check that we're not looking at a different part of the input (like a port)
        if remaining[0] != b':' && remaining[0] != b';' && remaining[0] != b'/' {
            return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::TooLarge)));
        }
    }
    
    // Split by dots to get the four octets and parse them as decimal
    let octets: Vec<&[u8]> = ip_bytes.split(|&c| c == b'.').collect();
    
    // Should be exactly 4 octets
    if octets.len() != 4 {
        return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::Count)));
    }
    
    // Parse each octet as a decimal value (handles leading zeros correctly)
    let o1 = parse_decimal_octet(octets[0])?;
    let o2 = parse_decimal_octet(octets[1])?;
    let o3 = parse_decimal_octet(octets[2])?;
    let o4 = parse_decimal_octet(octets[3])?;
    
    // Create the IPv4 address
    let ip_addr = IpAddr::V4(Ipv4Addr::new(o1, o2, o3, o4));
    
    Ok((remaining, Host::Address(ip_addr)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_ipv4_address_valid() {
        // Standard IPv4 addresses
        let (rem, host) = ipv4_address(b"192.168.1.1").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        
        // Loopback address
        let (rem, host) = ipv4_address(b"127.0.0.1").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        
        // Edge case - all zeros
        let (rem, host) = ipv4_address(b"0.0.0.0").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
        
        // Edge case - all max values
        let (rem, host) = ipv4_address(b"255.255.255.255").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255))));
        
        // Single digit octets
        let (rem, host) = ipv4_address(b"1.2.3.4").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))));
        
        // Mixed digit lengths
        let (rem, host) = ipv4_address(b"10.0.0.1").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        
        // Leading zeros (valid in SIP/HTTP URIs according to RFC 3986)
        // In decimal notation, leading zeros are allowed and should be treated as decimal
        let (rem, host) = ipv4_address(b"192.168.01.1").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn test_ipv4_address_with_remaining() {
        // With port
        let (rem, host) = ipv4_address(b"192.168.1.1:5060").unwrap();
        assert_eq!(rem, b":5060");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        
        // With parameters
        let (rem, host) = ipv4_address(b"192.168.1.1;transport=udp").unwrap();
        assert_eq!(rem, b";transport=udp");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        
        // With path
        let (rem, host) = ipv4_address(b"192.168.1.1/path").unwrap();
        assert_eq!(rem, b"/path");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn test_ipv4_address_invalid() {
        // Missing octet
        assert!(ipv4_address(b"192.168.1").is_err());
        
        // Extra octet
        assert!(ipv4_address(b"192.168.1.1.5").is_err());
        
        // Invalid separator
        assert!(ipv4_address(b"192,168,1,1").is_err());
        
        // No separator
        assert!(ipv4_address(b"192168101").is_err());
        
        // Empty octet
        assert!(ipv4_address(b"192.168..1").is_err());
        
        // Invalid octet - too large (>255)
        assert!(ipv4_address(b"192.168.1.256").is_err());
        
        // Invalid octet - more than 3 digits
        assert!(ipv4_address(b"192.168.1.1000").is_err());
        
        // Letters
        assert!(ipv4_address(b"192.168.1.abc").is_err());
        
        // Empty input
        assert!(ipv4_address(b"").is_err());
    }

    #[test]
    fn test_ipv4_address_rfc3261_examples() {
        // Examples from RFC 3261 Section 25.1 (Core Syntax) and RFC 3986 (URI)
        
        // From RFC 3261 Section 19.1.1 (SIP-URI Components)
        // "The host part equals an IP address or FQDN"
        let (rem, host) = ipv4_address(b"10.1.2.3").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
        
        // Example from RFC 3986 Section 3.2.2 (Host) for host component
        let (rem, host) = ipv4_address(b"192.0.2.16").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 16))));
    }

    #[test]
    fn test_leading_zeros_valid() {
        // According to RFC 3986 Section 3.2.2, leading zeros are permitted in IPv4 addresses
        // and are interpreted as decimal values
        let (rem, host) = ipv4_address(b"192.168.01.001").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        
        // More examples with different leading zero patterns
        let (rem, host) = ipv4_address(b"001.002.003.004").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(host, Host::Address(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))));
    }

    #[test]
    fn test_octet_validation() {
        // IPv4 address parsing should reject octets > 255
        assert!(ipv4_address(b"192.168.1.256").is_err());
        
        // Should reject octets with more than 3 digits
        assert!(ipv4_address(b"192.168.1.1000").is_err());
        
        // Should reject extra octets
        assert!(ipv4_address(b"192.168.1.1.5").is_err());
        
        // Should have exactly 4 octets
        assert!(ipv4_address(b"192.168.1").is_err());
    }
} 