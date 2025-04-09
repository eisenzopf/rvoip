use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_while, take_while1},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use super::utils::{
    parse_param_name, parse_param_value, parse_semicolon_params, 
    parse_comma_separated_values
};

/// A structured representation of a SIP Via header
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
    pub params: HashMap<String, String>,
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
            params: HashMap::new(),
        }
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
    pub fn params(&self) -> &HashMap<String, String> {
        &self.params
    }
    
    /// Get a formatted string representation for this Via header
    pub fn to_string(&self) -> String {
        let mut result = format!("{}/{}/{} {}", 
            self.protocol, self.version, self.transport, self.host);
            
        if let Some(port) = self.port {
            result.push_str(&format!(":{}", port));
        }
        
        // Add parameters
        for (name, value) in &self.params {
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
        parse_via(s)
    }
}

/// Parse a Via header using nom
pub fn parse_via(input: &str) -> Result<Via> {
    match via_parser(input) {
        Ok((_, via)) => Ok(via),
        Err(e) => Err(Error::Parser(format!("Failed to parse Via header: {:?}", e))),
    }
}

/// Parse multiple Via headers separated by commas
pub fn parse_multiple_vias(input: &str) -> Result<Vec<Via>> {
    match multiple_vias_parser(input) {
        Ok((_, vias)) => Ok(vias),
        Err(e) => Err(Error::Parser(format!("Failed to parse multiple Via headers: {:?}", e))),
    }
}

/// Parser for a Via header's protocol part (SIP/2.0/UDP)
fn protocol_parser(input: &str) -> IResult<&str, (String, String, String)> {
    tuple((
        // Protocol name (SIP)
        map(
            take_while1(|c: char| c.is_alphabetic()),
            |s: &str| s.to_string()
        ),
        tag("/"),
        // Version (2.0)
        map(
            take_while1(|c: char| c.is_ascii_digit() || c == '.'),
            |s: &str| s.to_string()
        ),
        tag("/"),
        // Transport (UDP, TCP, etc)
        map(
            take_while1(|c: char| c.is_alphabetic()),
            |s: &str| s.to_string()
        )
    ))(input).map(|(next, (protocol, _, version, _, transport))| {
        (next, (protocol, version, transport))
    })
}

/// Parser for host:port
fn host_port_parser(input: &str) -> IResult<&str, (String, Option<u16>)> {
    let (input, host_port) = take_till(|c| c == ';' || c == ',' || c == '\r' || c == '\n')(input)?;
    
    let host_port_parts: Vec<&str> = host_port.trim().split(':').collect();
    let host = host_port_parts[0].to_string();
    let port = if host_port_parts.len() > 1 {
        host_port_parts[1].parse::<u16>().ok()
    } else {
        None
    };
    
    Ok((input, (host, port)))
}

/// Parser for a single Via header parameter (name=value or just name)
fn via_param_parser(input: &str) -> IResult<&str, (String, String)> {
    preceded(
        char(';'),
        alt((
            separated_pair(
                map(parse_param_name, |s| s.to_string()),
                char('='),
                map(parse_param_value, |s| s.to_string())
            ),
            map(
                parse_param_name,
                |name| (name.to_string(), "".to_string())
            )
        ))
    )(input)
}

/// Parser for a complete Via header
fn via_parser(input: &str) -> IResult<&str, Via> {
    let (input, (protocol, version, transport)) = protocol_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, (host, port)) = host_port_parser(input)?;
    
    // Create a basic Via object
    let mut via = Via::new(protocol, version, transport, host, port);
    
    // Parse parameters if present
    let (input, params) = many0(via_param_parser)(input)?;
    
    // Add parameters to the Via object
    for (name, value) in params {
        via.set(name, value);
    }
    
    Ok((input, via))
}

/// Parser for multiple Via headers
fn multiple_vias_parser(input: &str) -> IResult<&str, Vec<Via>> {
    separated_list1(
        pair(char(','), space0),
        via_parser
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_via_parser() {
        let input = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds";
        let (_, via) = via_parser(input).unwrap();
        
        assert_eq!(via.protocol, "SIP");
        assert_eq!(via.version, "2.0");
        assert_eq!(via.transport, "UDP");
        assert_eq!(via.host, "pc33.example.com");
        assert_eq!(via.port, Some(5060));
        assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
    }
    
    #[test]
    fn test_multiple_vias_parser() {
        let input = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds, SIP/2.0/TCP proxy.example.com;branch=z9hG4bK123456";
        let (_, vias) = multiple_vias_parser(input).unwrap();
        
        assert_eq!(vias.len(), 2);
        
        assert_eq!(vias[0].transport, "UDP");
        assert_eq!(vias[0].host, "pc33.example.com");
        assert_eq!(vias[0].branch(), Some("z9hG4bK776asdhds"));
        
        assert_eq!(vias[1].transport, "TCP");
        assert_eq!(vias[1].host, "proxy.example.com");
        assert_eq!(vias[1].branch(), Some("z9hG4bK123456"));
    }
    
    #[test]
    fn test_via_display() {
        let mut via = Via::new("SIP", "2.0", "UDP", "example.com", Some(5060));
        via.set_branch("z9hG4bK123");
        via.set("received", "10.0.0.1");
        
        let via_str = via.to_string();
        
        // Since HashMap iteration order is not guaranteed, we check that all parts are present
        assert!(via_str.starts_with("SIP/2.0/UDP example.com:5060;"));
        assert!(via_str.contains("branch=z9hG4bK123"));
        assert!(via_str.contains("received=10.0.0.1"));
    }
} 