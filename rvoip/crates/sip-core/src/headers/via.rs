use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::header_parsers::parse_parameter;

/// Structured representation of a SIP Via header
#[derive(Debug, Clone, PartialEq)]
pub struct Via {
    /// Protocol (usually "SIP")
    pub protocol: String,
    /// Protocol version (usually "2.0")
    pub version: String,
    /// Transport protocol (UDP, TCP, etc.)
    pub transport: String,
    /// Host address
    pub host: String,
    /// Optional port
    pub port: Option<u16>,
    /// Header parameters
    pub params: ViaParams,
}

/// Parameters for a Via header
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViaParams {
    /// Raw parameters map
    params: HashMap<String, String>,
}

impl ViaParams {
    /// Create a new empty set of parameters
    pub fn new() -> Self {
        Self {
            params: HashMap::new(),
        }
    }

    /// Create from a parameters map
    pub fn from_map(params: HashMap<String, String>) -> Self {
        Self { params }
    }

    /// Get the branch parameter
    pub fn branch(&self) -> Option<&str> {
        self.params.get("branch").map(|s| s.as_str())
    }

    /// Set the branch parameter
    pub fn set_branch(&mut self, branch: impl Into<String>) {
        self.params.insert("branch".to_string(), branch.into());
    }

    /// Get a parameter by name
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(|s| s.as_str())
    }

    /// Set a parameter
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.params.insert(name.into(), value.into());
    }

    /// Check if a parameter exists
    pub fn contains(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }

    /// Get all parameters as a reference to the raw map
    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.params
    }
}

impl Via {
    /// Create a new Via header
    pub fn new(
        protocol: impl Into<String>,
        version: impl Into<String>,
        transport: impl Into<String>,
        host: impl Into<String>,
        port: Option<u16>,
    ) -> Self {
        Self {
            protocol: protocol.into(),
            version: version.into(),
            transport: transport.into(),
            host: host.into(),
            port,
            params: ViaParams::new(),
        }
    }

    /// Parse a Via header from a string
    pub fn parse(input: &str) -> Result<Self> {
        // Via header format: SIP/2.0/UDP host:port;branch=xxx;other=params
        let parts: Vec<&str> = input.split(';').collect();
        
        if parts.is_empty() {
            return Err(Error::InvalidHeader(format!("Invalid Via header: {}", input)));
        }
        
        // Parse protocol part: SIP/2.0/UDP
        let protocol_parts: Vec<&str> = parts[0].trim().split('/').collect();
        if protocol_parts.len() < 3 {
            return Err(Error::InvalidHeader(format!("Invalid Via protocol: {}", parts[0])));
        }
        
        let protocol = protocol_parts[0].to_string();
        let version = protocol_parts[1].to_string();
        
        // Extract transport and host:port
        let transport_and_host = protocol_parts[2].trim().split_whitespace().collect::<Vec<&str>>();
        
        if transport_and_host.is_empty() {
            return Err(Error::InvalidHeader(format!("Missing transport in Via: {}", parts[0])));
        }
        
        let transport = transport_and_host[0].to_string();
        
        // Default values
        let mut host = String::new();
        let mut port = None;
        
        // Extract sent-by (host:port)
        if transport_and_host.len() > 1 {
            let sent_by = transport_and_host[1];
            if sent_by.contains(':') {
                let host_port: Vec<&str> = sent_by.split(':').collect();
                host = host_port[0].to_string();
                if host_port.len() > 1 {
                    if let Ok(port_num) = host_port[1].parse::<u16>() {
                        port = Some(port_num);
                    }
                }
            } else {
                host = sent_by.to_string();
            }
        }
        
        // Create the Via object
        let mut via = Via::new(protocol, version, transport, host, port);
        
        // Parse parameters
        for i in 1..parts.len() {
            let param = parts[i].trim();
            let mut param_map = HashMap::new();
            parse_parameter(param, &mut param_map);
            
            // Add each parameter to our Via params
            for (name, value) in param_map {
                via.params.set(name, value);
            }
        }
        
        Ok(via)
    }

    /// Parse multiple Via headers
    pub fn parse_multiple(input: &str) -> Result<Vec<Self>> {
        use crate::header_parsers::parse_comma_separated_list;
        
        let via_parts = parse_comma_separated_list(input);
        
        let mut result = Vec::new();
        for part in via_parts {
            result.push(Self::parse(&part)?);
        }
        
        Ok(result)
    }

    /// Get a formatted string representation for this Via header
    pub fn to_string(&self) -> String {
        let mut result = format!("{}/{}/{} {}", 
            self.protocol, self.version, self.transport, self.host);
            
        if let Some(port) = self.port {
            result.push_str(&format!(":{}", port));
        }
        
        // Add parameters
        for (name, value) in self.params.as_map() {
            if value.is_empty() {
                result.push_str(&format!(";{}", name));
            } else {
                result.push_str(&format!(";{}={}", name, value));
            }
        }
        
        result
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl FromStr for Via {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Via::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, HeaderName, Message, Request, Method, Uri, Version};
    use std::str::FromStr;

    #[test]
    fn test_via_parse() {
        // Test basic Via header
        let via_str = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds";
        let via = Via::parse(via_str).unwrap();
        
        assert_eq!(via.protocol, "SIP");
        assert_eq!(via.version, "2.0");
        assert_eq!(via.transport, "UDP");
        assert_eq!(via.host, "pc33.example.com");
        assert_eq!(via.port, Some(5060));
        assert_eq!(via.params.branch(), Some("z9hG4bK776asdhds"));
        
        // Test with multiple parameters
        let via_str = "SIP/2.0/TCP 192.168.1.1;branch=z9hG4bK123;received=10.0.0.1;rport=5061";
        let via = Via::parse(via_str).unwrap();
        
        assert_eq!(via.transport, "TCP");
        assert_eq!(via.host, "192.168.1.1");
        assert_eq!(via.port, None);
        assert_eq!(via.params.branch(), Some("z9hG4bK123"));
        assert_eq!(via.params.get("received"), Some("10.0.0.1"));
        assert_eq!(via.params.get("rport"), Some("5061"));
        
        // Test with no parameters
        let via_str = "SIP/2.0/UDP example.com:5060";
        let via = Via::parse(via_str).unwrap();
        
        assert_eq!(via.transport, "UDP");
        assert_eq!(via.host, "example.com");
        assert_eq!(via.port, Some(5060));
        assert_eq!(via.params.branch(), None);
    }

    #[test]
    fn test_via_to_string() {
        let mut via = Via::new("SIP", "2.0", "UDP", "example.com", Some(5060));
        via.params.set_branch("z9hG4bK123");
        via.params.set("received", "10.0.0.1");
        
        // Get the string representation
        let via_str = via.to_string();
        
        // Since HashMap iteration order is not guaranteed, we'll check that all parts are present
        // rather than checking the exact string
        assert!(via_str.starts_with("SIP/2.0/UDP example.com:5060;"));
        assert!(via_str.contains("branch=z9hG4bK123"));
        assert!(via_str.contains("received=10.0.0.1"));
    }

    #[test]
    fn test_parse_multiple_vias() {
        let vias_str = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds, SIP/2.0/TCP proxy.example.com;branch=z9hG4bK123456";
        let vias = Via::parse_multiple(vias_str).unwrap();
        
        assert_eq!(vias.len(), 2);
        
        assert_eq!(vias[0].transport, "UDP");
        assert_eq!(vias[0].host, "pc33.example.com");
        assert_eq!(vias[0].params.branch(), Some("z9hG4bK776asdhds"));
        
        assert_eq!(vias[1].transport, "TCP");
        assert_eq!(vias[1].host, "proxy.example.com");
        assert_eq!(vias[1].params.branch(), Some("z9hG4bK123456"));
    }

    #[test]
    fn test_message_via_headers() {
        // Create a request with Via headers
        let mut request = Request::new(
            Method::Invite, 
            Uri::from_str("sip:bob@example.com").unwrap()
        );
        
        // Add Via headers
        request.headers.push(Header::text(
            HeaderName::Via,
            "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds"
        ));
        request.headers.push(Header::text(
            HeaderName::Via,
            "SIP/2.0/TCP proxy.example.com;branch=z9hG4bK123456"
        ));
        
        // Parse Via headers
        let vias = request.via_headers();
        
        assert_eq!(vias.len(), 2);
        
        // Check first Via
        assert_eq!(vias[0].protocol, "SIP");
        assert_eq!(vias[0].version, "2.0");
        assert_eq!(vias[0].transport, "UDP");
        assert_eq!(vias[0].host, "pc33.example.com");
        assert_eq!(vias[0].port, Some(5060));
        assert_eq!(vias[0].params.branch(), Some("z9hG4bK776asdhds"));
        
        // Check second Via
        assert_eq!(vias[1].protocol, "SIP");
        assert_eq!(vias[1].version, "2.0");
        assert_eq!(vias[1].transport, "TCP");
        assert_eq!(vias[1].host, "proxy.example.com");
        assert_eq!(vias[1].port, None);
        assert_eq!(vias[1].params.branch(), Some("z9hG4bK123456"));
        
        // Test first_via shortcut
        let first = request.first_via().unwrap();
        assert_eq!(first.transport, "UDP");
        assert_eq!(first.params.branch(), Some("z9hG4bK776asdhds"));
        
        // Test via Message variant
        let message = Message::Request(request);
        let vias_from_message = message.via_headers();
        
        assert_eq!(vias_from_message.len(), 2);
        assert_eq!(vias_from_message[0].params.branch(), Some("z9hG4bK776asdhds"));
    }

    #[test]
    fn test_creating_via_headers() {
        // Create a Via object
        let mut via = Via::new("SIP", "2.0", "UDP", "example.com", Some(5060));
        via.params.set_branch("z9hG4bK12345");
        via.params.set("received", "10.0.0.1");
        via.params.set("rport", "");
        
        // Convert to string and parse back
        let via_str = via.to_string();
        let parsed = Via::parse(&via_str).unwrap();
        
        // Verify parsed result
        assert_eq!(parsed.protocol, "SIP");
        assert_eq!(parsed.version, "2.0");
        assert_eq!(parsed.transport, "UDP");
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, Some(5060));
        assert_eq!(parsed.params.branch(), Some("z9hG4bK12345"));
        assert_eq!(parsed.params.get("received"), Some("10.0.0.1"));
        assert_eq!(parsed.params.get("rport"), Some(""));
    }

    #[test]
    fn test_multiple_via_parsing() {
        let vias_str = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds, SIP/2.0/TCP proxy.example.com;branch=z9hG4bK123456";
        let vias = Via::parse_multiple(vias_str).unwrap();
        
        assert_eq!(vias.len(), 2);
        
        assert_eq!(vias[0].transport, "UDP");
        assert_eq!(vias[0].host, "pc33.example.com");
        assert_eq!(vias[0].params.branch(), Some("z9hG4bK776asdhds"));
        
        assert_eq!(vias[1].transport, "TCP");
        assert_eq!(vias[1].host, "proxy.example.com");
        assert_eq!(vias[1].params.branch(), Some("z9hG4bK123456"));
    }
} 