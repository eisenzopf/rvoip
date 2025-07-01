//! SDP ICE Candidate Attribute Parser
//!
//! Implements parser for ICE candidate attributes as defined in RFC 8839.
//! Format: a=candidate:<foundation> <component-id> <transport> <priority> <conn-addr> <port> typ <cand-type> [raddr <raddr>] [rport <rport>] *(extensions)

use crate::error::{Error, Result};
use crate::types::sdp::{CandidateAttribute, ParsedAttribute};
use crate::sdp::attributes::common::{is_valid_ipv4, is_valid_ipv6, is_valid_hostname};

/// Parses candidate attribute based on RFC 8839
pub fn parse_candidate(value: &str) -> Result<ParsedAttribute> {
    let value = value.trim();
    let parts: Vec<&str> = value.split_whitespace().collect();
    
    // Check for minimum number of parts required
    if parts.len() < 8 {
        return Err(Error::SdpParsingError(format!(
            "Invalid candidate format, insufficient parts: {}", value
        )));
    }
    
    // Parse mandatory parts
    let foundation = parts[0].to_string();
    
    // Parse component ID (1-256)
    let component_id = match parts[1].parse::<u32>() {
        Ok(id) if id >= 1 && id <= 256 => id,
        _ => return Err(Error::SdpParsingError(format!(
            "Invalid component ID in candidate: {}", parts[1]
        ))),
    };
    
    // Parse transport (only UDP and TCP are valid)
    let transport = parts[2].to_string();
    if transport.to_uppercase() != "UDP" && transport.to_uppercase() != "TCP" {
        return Err(Error::SdpParsingError(format!(
            "Invalid transport in candidate: {}", transport
        )));
    }
    
    // Parse priority
    let priority = match parts[3].parse::<u32>() {
        Ok(priority) => priority,
        _ => return Err(Error::SdpParsingError(format!(
            "Invalid priority in candidate: {}", parts[3]
        ))),
    };
    
    // Parse connection address
    let connection_address = parts[4].to_string();
    if !is_valid_ipv4(&connection_address) && 
       !is_valid_ipv6(&connection_address) && 
       !is_valid_hostname(&connection_address) {
        return Err(Error::SdpParsingError(format!(
            "Invalid connection address in candidate: {}", connection_address
        )));
    }
    
    // Parse port
    let port = match parts[5].parse::<u16>() {
        Ok(port) => port,
        _ => return Err(Error::SdpParsingError(format!(
            "Invalid port in candidate: {}", parts[5]
        ))),
    };
    
    // Check 'typ'
    if parts[6] != "typ" {
        return Err(Error::SdpParsingError(format!(
            "Expected 'typ' keyword in candidate, found: {}", parts[6]
        )));
    }
    
    // Parse candidate type
    let candidate_type = parts[7].to_string();
    if !["host", "srflx", "prflx", "relay"].contains(&candidate_type.as_str()) {
        return Err(Error::SdpParsingError(format!(
            "Invalid candidate type: {}", candidate_type
        )));
    }
    
    // Parse optional parts
    let mut idx = 8;
    let mut related_address = None;
    let mut related_port = None;
    let mut extensions = Vec::new();
    
    while idx < parts.len() {
        match parts[idx] {
            "raddr" => {
                idx += 1;
                if idx >= parts.len() {
                    return Err(Error::SdpParsingError(
                        "raddr keyword without address".to_string()
                    ));
                }
                let addr = parts[idx].to_string();
                if !is_valid_ipv4(&addr) && !is_valid_ipv6(&addr) && !is_valid_hostname(&addr) {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid related address in candidate: {}", addr
                    )));
                }
                related_address = Some(addr);
            },
            "rport" => {
                idx += 1;
                if idx >= parts.len() {
                    return Err(Error::SdpParsingError(
                        "rport keyword without port".to_string()
                    ));
                }
                match parts[idx].parse::<u16>() {
                    Ok(port) => related_port = Some(port),
                    _ => return Err(Error::SdpParsingError(format!(
                        "Invalid related port in candidate: {}", parts[idx]
                    ))),
                }
            },
            _ => {
                // Handle extension
                let key = parts[idx].to_string();
                let mut value = None;
                
                // Check if there's a value for this extension
                if idx + 1 < parts.len() && !["raddr", "rport", "typ"].contains(&parts[idx + 1]) {
                    value = Some(parts[idx + 1].to_string());
                    idx += 1;
                }
                
                extensions.push((key, value));
            }
        }
        idx += 1;
    }
    
    // If related address is present, related port should also be present
    if related_address.is_some() && related_port.is_none() {
        return Err(Error::SdpParsingError(
            "Related address is present but related port is missing".to_string()
        ));
    }
    
    Ok(ParsedAttribute::Candidate(CandidateAttribute {
        foundation,
        component_id,
        transport,
        priority,
        connection_address,
        port,
        candidate_type,
        related_address,
        related_port,
        extensions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_candidate() {
        // Host candidate with IPv4 address
        // Example from RFC 8839 Section 5.1
        let value = "1 1 UDP 2130706431 10.0.1.1 8998 typ host";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "1");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 2130706431);
                assert_eq!(candidate.connection_address, "10.0.1.1");
                assert_eq!(candidate.port, 8998);
                assert_eq!(candidate.candidate_type, "host");
                assert_eq!(candidate.related_address, None);
                assert_eq!(candidate.related_port, None);
                assert!(candidate.extensions.is_empty());
            },
            _ => panic!("Expected Candidate attribute")
        }
    }

    #[test]
    fn test_parse_srflx_candidate() {
        // Server reflexive candidate with IPv4 address and related address/port
        let value = "2 1 UDP 1694498815 192.0.2.3 45664 typ srflx raddr 10.0.1.1 rport 8998";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "2");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 1694498815);
                assert_eq!(candidate.connection_address, "192.0.2.3");
                assert_eq!(candidate.port, 45664);
                assert_eq!(candidate.candidate_type, "srflx");
                assert_eq!(candidate.related_address.unwrap(), "10.0.1.1");
                assert_eq!(candidate.related_port.unwrap(), 8998);
                assert!(candidate.extensions.is_empty());
            },
            _ => panic!("Expected Candidate attribute")
        }
    }

    #[test]
    fn test_parse_relay_candidate() {
        // Relay candidate with IPv6 address
        let value = "3 1 UDP 16777215 2001:db8:1234::1 10000 typ relay raddr 2001:db8:1234::2 rport 9000";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "3");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 16777215);
                assert_eq!(candidate.connection_address, "2001:db8:1234::1");
                assert_eq!(candidate.port, 10000);
                assert_eq!(candidate.candidate_type, "relay");
                assert_eq!(candidate.related_address.unwrap(), "2001:db8:1234::2");
                assert_eq!(candidate.related_port.unwrap(), 9000);
                assert!(candidate.extensions.is_empty());
            },
            _ => panic!("Expected Candidate attribute")
        }
    }

    #[test]
    fn test_parse_tcp_candidate() {
        // TCP candidate with IPv4 address
        let value = "4 1 TCP 2128609279 192.168.2.1 9 typ host tcptype active";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "4");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "TCP");
                assert_eq!(candidate.priority, 2128609279);
                assert_eq!(candidate.connection_address, "192.168.2.1");
                assert_eq!(candidate.port, 9);
                assert_eq!(candidate.candidate_type, "host");
                assert_eq!(candidate.related_address, None);
                assert_eq!(candidate.related_port, None);
                
                // Check extension
                assert_eq!(candidate.extensions.len(), 1);
                let (key, value) = &candidate.extensions[0];
                assert_eq!(key, "tcptype");
                assert_eq!(value.as_ref().unwrap(), "active");
            },
            _ => panic!("Expected Candidate attribute")
        }
    }

    #[test]
    fn test_parse_candidate_with_hostname() {
        // Candidate with hostname instead of IP address
        let value = "5 1 UDP 2130706431 example.com 8998 typ host";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "5");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 2130706431);
                assert_eq!(candidate.connection_address, "example.com");
                assert_eq!(candidate.port, 8998);
                assert_eq!(candidate.candidate_type, "host");
            },
            _ => panic!("Expected Candidate attribute")
        }
    }

    #[test]
    fn test_parse_candidate_with_extensions() {
        // Candidate with multiple extensions
        let value = "6 1 UDP 2130706431 203.0.113.1 5000 typ host generation 0 network-id 1 network-cost 10";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "6");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 2130706431);
                assert_eq!(candidate.connection_address, "203.0.113.1");
                assert_eq!(candidate.port, 5000);
                assert_eq!(candidate.candidate_type, "host");
                
                // Check extensions
                assert_eq!(candidate.extensions.len(), 3);
                
                // First extension: generation 0
                assert_eq!(candidate.extensions[0].0, "generation");
                assert_eq!(candidate.extensions[0].1.as_ref().unwrap(), "0");
                
                // Second extension: network-id 1
                assert_eq!(candidate.extensions[1].0, "network-id");
                assert_eq!(candidate.extensions[1].1.as_ref().unwrap(), "1");
                
                // Third extension: network-cost 10
                assert_eq!(candidate.extensions[2].0, "network-cost");
                assert_eq!(candidate.extensions[2].1.as_ref().unwrap(), "10");
            },
            _ => panic!("Expected Candidate attribute")
        }
    }
    
    #[test]
    fn test_parse_complex_candidate() {
        // Complex candidate with all possible fields
        let value = "aL2X 2 UDP 1694498815 192.0.2.5 12200 typ srflx raddr 10.0.1.5 rport 36082 generation 0 ufrag 01Ab network-id 1 network-cost 50";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "aL2X");
                assert_eq!(candidate.component_id, 2);  // Component 2 (RTCP)
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 1694498815);
                assert_eq!(candidate.connection_address, "192.0.2.5");
                assert_eq!(candidate.port, 12200);
                assert_eq!(candidate.candidate_type, "srflx");
                assert_eq!(candidate.related_address.unwrap(), "10.0.1.5");
                assert_eq!(candidate.related_port.unwrap(), 36082);
                
                // Check extensions (4 of them)
                assert_eq!(candidate.extensions.len(), 4);
                
                // Check the ufrag extension
                let ufrag_ext = candidate.extensions.iter()
                    .find(|(key, _)| key == "ufrag")
                    .expect("ufrag extension not found");
                assert_eq!(ufrag_ext.1.as_ref().unwrap(), "01Ab");
            },
            _ => panic!("Expected Candidate attribute")
        }
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Candidate with extra whitespace
        let value = "  1    1   UDP   2130706431   10.0.1.1   8998   typ   host  ";
        
        match parse_candidate(value).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.foundation, "1");
                assert_eq!(candidate.component_id, 1);
                assert_eq!(candidate.transport, "UDP");
                assert_eq!(candidate.priority, 2130706431);
                assert_eq!(candidate.connection_address, "10.0.1.1");
                assert_eq!(candidate.port, 8998);
                assert_eq!(candidate.candidate_type, "host");
            },
            _ => panic!("Expected Candidate attribute")
        }
    }
    
    #[test]
    fn test_case_sensitivity() {
        // Test case sensitivity of transport and candidate type
        
        // Lower case UDP
        let value1 = "1 1 udp 2130706431 10.0.1.1 8998 typ host";
        match parse_candidate(value1).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.transport, "udp");
            },
            _ => panic!("Expected Candidate attribute")
        }
        
        // Lower case TCP
        let value2 = "1 1 tcp 2130706431 10.0.1.1 8998 typ host";
        match parse_candidate(value2).unwrap() {
            ParsedAttribute::Candidate(candidate) => {
                assert_eq!(candidate.transport, "tcp");
            },
            _ => panic!("Expected Candidate attribute")
        }
    }
    
    #[test]
    fn test_invalid_candidates() {
        // Missing foundation
        assert!(parse_candidate(" 1 UDP 2130706431 10.0.1.1 8998 typ host").is_err());
        
        // Invalid component ID (0 is invalid, must be 1-256)
        assert!(parse_candidate("1 0 UDP 2130706431 10.0.1.1 8998 typ host").is_err());
        assert!(parse_candidate("1 257 UDP 2130706431 10.0.1.1 8998 typ host").is_err());
        
        // Invalid transport protocol
        assert!(parse_candidate("1 1 SCTP 2130706431 10.0.1.1 8998 typ host").is_err());
        
        // Invalid connection address
        assert!(parse_candidate("1 1 UDP 2130706431 invalid$addr 8998 typ host").is_err());
        
        // Invalid port (too large)
        assert!(parse_candidate("1 1 UDP 2130706431 10.0.1.1 65536 typ host").is_err());
        
        // Invalid candidate type
        assert!(parse_candidate("1 1 UDP 2130706431 10.0.1.1 8998 typ unknown").is_err());
        
        // Missing required fields
        assert!(parse_candidate("1 1 UDP 2130706431 10.0.1.1").is_err());
        assert!(parse_candidate("1 1 UDP").is_err());
        assert!(parse_candidate("1").is_err());
        
        // Related address without related port
        assert!(parse_candidate("1 1 UDP 2130706431 10.0.1.1 8998 typ srflx raddr 192.168.1.1").is_err());
    }
    
    #[test]
    fn test_valid_ip_addresses() {
        // Test IPv4 address validation
        assert!(is_valid_ipv4("192.168.1.1"));
        assert!(is_valid_ipv4("10.0.0.1"));
        assert!(is_valid_ipv4("172.16.0.1"));
        assert!(is_valid_ipv4("203.0.113.1"));
        
        // Test IPv6 address validation
        assert!(is_valid_ipv6("2001:db8::1"));
        assert!(is_valid_ipv6("::1"));
        assert!(is_valid_ipv6("fe80::1234:5678:abcd"));
        
        // Test hostname validation
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("test-server.example.org"));
        assert!(is_valid_hostname("node1.local"));
    }
    
    #[test]
    fn test_invalid_ip_addresses() {
        // Invalid IPv4 addresses
        assert!(!is_valid_ipv4("256.0.0.1"));  // Octet too large
        assert!(!is_valid_ipv4("192.168.1"));  // Too few octets
        assert!(!is_valid_ipv4("192.168.1.1.5"));  // Too many octets
        assert!(!is_valid_ipv4("192.168..1"));  // Empty octet
        
        // Invalid IPv6 addresses - these should fail parsing
        assert!(!is_valid_ipv6("2001:db8:::1"));  // Too many colons
        assert!(!is_valid_ipv6("2001:db8:g:1"));  // Invalid character
        
        // Invalid hostnames
        assert!(!is_valid_hostname(".example.com"));  // Starts with dot
        assert!(!is_valid_hostname("example.com."));  // Ends with dot (in our implementation)
        assert!(!is_valid_hostname("example..com"));  // Consecutive dots
        assert!(!is_valid_hostname("test server.com"));  // Contains space
    }
} 