use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::digit1,
    combinator::{map_res, opt},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;
use crate::parser::common_chars::digit; // Keep digit if still used by port

// Import the specific host type parsers
use super::hostname::hostname;
use super::ipv4::ipv4_address;
use super::ipv6::ipv6_reference;

// host = hostname / IPv4address / IPv6reference
pub fn host(input: &[u8]) -> ParseResult<Host> {
    // Order is important: try hostname first as IP addresses might contain valid domain chars
    alt((hostname, ipv4_address, ipv6_reference))(input)
}

// port = 1*DIGIT
pub fn port(input: &[u8]) -> ParseResult<u16> {
    // Manual implementation that ensures strict numeric validation
    if input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit)));
    }
    
    // Find the position of the first non-digit character
    let mut pos = 0;
    while pos < input.len() && input[pos].is_ascii_digit() {
        pos += 1;
    }
    
    // Ensure we found at least one digit
    if pos == 0 {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit)));
    }
    
    // Extract the digit sequence
    let digits = &input[..pos];
    let remaining = &input[pos..];
    
    // Parse the port number
    let port_str = std::str::from_utf8(digits)
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?;
    
    // Convert to u32 first to check for overflow
    let port_num = port_str.parse::<u32>()
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?;
    
    // Verify port is within valid range (0-65535)
    if port_num > 65535 {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)));
    }
    
    Ok((remaining, port_num as u16))
}

// hostport = host [ ":" port ]
pub fn hostport(input: &[u8]) -> ParseResult<(Host, Option<u16>)> {
    pair(host, opt(preceded(tag(b":"), port)))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // === Host Type Selection Tests ===

    #[test]
    fn test_host_domain() {
        // Basic hostname
        let (rem, result) = host(b"example.com").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(result, Host::Domain(domain) if domain == "example.com"));

        // Hostname with trailing content
        let (rem, result) = host(b"example.com:5060").unwrap();
        assert_eq!(rem, b":5060");
        assert!(matches!(result, Host::Domain(domain) if domain == "example.com"));
    }

    #[test]
    fn test_host_ipv4() {
        // Basic IPv4 address
        let (rem, result) = host(b"192.168.1.1").unwrap();
        assert!(rem.is_empty());
        match result {
            Host::Address(addr) => {
                assert_eq!(addr, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
            },
            _ => panic!("Expected IPv4 address"),
        }

        // IPv4 with trailing content
        let (rem, result) = host(b"192.168.1.1:5060").unwrap();
        assert_eq!(rem, b":5060");
        match result {
            Host::Address(addr) => {
                assert_eq!(addr, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
            },
            _ => panic!("Expected IPv4 address"),
        }
    }

    #[test]
    fn test_host_ipv6() {
        // Basic IPv6 address
        let (rem, result) = host(b"[2001:db8::1]").unwrap();
        assert!(rem.is_empty());
        match result {
            Host::Address(addr) => {
                assert!(addr.is_ipv6());
                if let IpAddr::V6(ipv6) = addr {
                    assert_eq!(ipv6.segments(), [0x2001, 0xdb8, 0, 0, 0, 0, 0, 1]);
                }
            },
            _ => panic!("Expected IPv6 address"),
        }

        // IPv6 with trailing content
        let (rem, result) = host(b"[2001:db8::1]:5060").unwrap();
        assert_eq!(rem, b":5060");
        match result {
            Host::Address(addr) => {
                assert!(addr.is_ipv6());
                if let IpAddr::V6(ipv6) = addr {
                    assert_eq!(ipv6.segments(), [0x2001, 0xdb8, 0, 0, 0, 0, 0, 1]);
                }
            },
            _ => panic!("Expected IPv6 address"),
        }
    }

    // === Port Tests ===

    #[test]
    fn test_port_valid() {
        // Valid port numbers
        let (rem, result) = port(b"5060").unwrap();
        assert!(rem.is_empty());
        assert_eq!(result, 5060);

        let (rem, result) = port(b"1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(result, 1);

        let (rem, result) = port(b"65535").unwrap();
        assert!(rem.is_empty());
        assert_eq!(result, 65535);
    }

    #[test]
    fn test_port_with_trailing() {
        // Port with trailing content
        let (rem, result) = port(b"5060;transport=udp").unwrap();
        assert_eq!(rem, b";transport=udp");
        assert_eq!(result, 5060);
    }

    #[test]
    fn test_port_invalid() {
        // Non-numeric port
        assert!(port(b"abc").is_err());
        
        // Port exceeding u16 range
        let result = port(b"65536");
        assert!(result.is_err()); // Value exceeds u16 max
        
        // Empty port
        assert!(port(b"").is_err());
    }

    // === Hostport Tests ===

    #[test]
    fn test_hostport_with_port() {
        // Domain with port
        let (rem, (host_val, port_opt)) = hostport(b"example.com:5060").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(host_val, Host::Domain(domain) if domain == "example.com"));
        assert_eq!(port_opt, Some(5060));

        // IPv4 with port
        let (rem, (host_val, port_opt)) = hostport(b"192.168.1.1:8080").unwrap();
        assert!(rem.is_empty());
        match host_val {
            Host::Address(addr) => {
                assert_eq!(addr, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
            },
            _ => panic!("Expected IPv4 address"),
        }
        assert_eq!(port_opt, Some(8080));

        // IPv6 with port
        let (rem, (host_val, port_opt)) = hostport(b"[2001:db8::1]:5060").unwrap();
        assert!(rem.is_empty());
        match host_val {
            Host::Address(addr) => {
                assert!(addr.is_ipv6());
                if let IpAddr::V6(ipv6) = addr {
                    assert_eq!(ipv6.segments(), [0x2001, 0xdb8, 0, 0, 0, 0, 0, 1]);
                }
            },
            _ => panic!("Expected IPv6 address"),
        }
        assert_eq!(port_opt, Some(5060));
    }

    #[test]
    fn test_hostport_without_port() {
        // Domain without port
        let (rem, (host_val, port_opt)) = hostport(b"example.com").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(host_val, Host::Domain(domain) if domain == "example.com"));
        assert_eq!(port_opt, None);

        // IPv4 without port
        let (rem, (host_val, port_opt)) = hostport(b"192.168.1.1").unwrap();
        assert!(rem.is_empty());
        match host_val {
            Host::Address(addr) => {
                assert_eq!(addr, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
            },
            _ => panic!("Expected IPv4 address"),
        }
        assert_eq!(port_opt, None);

        // IPv6 without port
        let (rem, (host_val, port_opt)) = hostport(b"[2001:db8::1]").unwrap();
        assert!(rem.is_empty());
        match host_val {
            Host::Address(addr) => {
                assert!(addr.is_ipv6());
                if let IpAddr::V6(ipv6) = addr {
                    assert_eq!(ipv6.segments(), [0x2001, 0xdb8, 0, 0, 0, 0, 0, 1]);
                }
            },
            _ => panic!("Expected IPv6 address"),
        }
        assert_eq!(port_opt, None);
    }

    #[test]
    fn test_hostport_with_trailing() {
        // Hostport with trailing content
        let (rem, (host_val, port_opt)) = hostport(b"example.com;param=value").unwrap();
        assert_eq!(rem, b";param=value");
        assert!(matches!(host_val, Host::Domain(domain) if domain == "example.com"));
        assert_eq!(port_opt, None);

        // Test with parameters similar to our failing test case
        let (rem, (host_val, port_opt)) = hostport(b"example.com;transport=tcp;lr").unwrap();
        assert_eq!(rem, b";transport=tcp;lr");
        assert!(matches!(host_val, Host::Domain(domain) if domain == "example.com"));
        assert_eq!(port_opt, None);
    }

    // === RFC 3261 Specific Tests ===

    #[test]
    fn test_rfc3261_examples() {
        // Examples from RFC 3261
        let examples = [
            (b"atlanta.com".as_ref(), "atlanta.com", None),
            (b"biloxi.com:5060".as_ref(), "biloxi.com", Some(5060)),
            (b"192.0.2.4".as_ref(), "192.0.2.4", None),
            (b"192.0.2.4:5060".as_ref(), "192.0.2.4", Some(5060)),
            (b"[2001:db8::1]".as_ref(), "[2001:db8::1]", None),
            (b"[2001:db8::1]:5060".as_ref(), "[2001:db8::1]", Some(5060)),
        ];

        for (input, expected_host, expected_port) in examples {
            let (rem, (host_val, port_opt)) = hostport(input).unwrap();
            assert!(rem.is_empty());
            
            match &host_val {
                Host::Domain(domain) => {
                    assert_eq!(domain, expected_host);
                },
                Host::Address(addr) => {
                    if addr.is_ipv4() {
                        assert_eq!(addr.to_string(), expected_host);
                    } else if addr.is_ipv6() {
                        // For IPv6, the expected string should already include brackets
                        assert!(expected_host.starts_with("[") && expected_host.ends_with("]"));
                        assert_eq!(format!("{}", addr), expected_host[1..expected_host.len()-1]);
                    }
                }
            }
            
            assert_eq!(port_opt, expected_port);
        }
    }

    // === Edge Cases ===

    #[test]
    fn test_numeric_hostname() {
        // Numeric hostnames that are not valid IPv4 addresses
        let (rem, host_val) = host(b"999.888.777.666").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(host_val, Host::Domain(domain) if domain == "999.888.777.666"));
        
        // This looks like an IPv4 address but has numbers > 255
        let (rem, host_val) = host(b"333.444.555.666").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(host_val, Host::Domain(domain) if domain == "333.444.555.666"));
    }

    #[test]
    fn test_invalid_hosts() {
        // Invalid IPv6 reference (missing closing bracket)
        assert!(host(b"[2001:db8::1").is_err());
        
        // Invalid IPv4 address (extra dot)
        let result = host(b"192.168.1.1.");
        // This should either be an error or parse as a domain name
        if let Ok((_, result)) = result {
            assert!(matches!(result, Host::Domain(_)));
        }
        
        // Invalid hostname (consecutive dots)
        assert!(host(b"example..com").is_err());
    }
} 