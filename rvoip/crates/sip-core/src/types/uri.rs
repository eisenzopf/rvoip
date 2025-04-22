// URI implementation moved from root directory
// Implements URI types according to RFC 3261

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1, hex_digit1},
    combinator::{map, map_res, opt, recognize, verify},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::types::param::Param; // Updated import path

/// SIP URI schema types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    /// SIP URI (non-secure)
    Sip,
    /// SIPS URI (secure SIP)
    Sips,
    /// TEL URI (telephone number)
    Tel,
}

impl Scheme {
    /// Returns the string representation of the scheme
    pub fn as_str(&self) -> &str {
        match self {
            Scheme::Sip => "sip",
            Scheme::Sips => "sips",
            Scheme::Tel => "tel",
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scheme {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Check specifically for schemes followed by ':' implicitly
        // The nom parser `terminated(scheme_parser, char(':'))` ensures this, 
        // so direct FromStr should handle the base scheme string correctly.
        match s.to_lowercase().as_str() {
            "sip" => Ok(Scheme::Sip),
            "sips" => Ok(Scheme::Sips),
            "tel" => Ok(Scheme::Tel),
            _ => Err(Error::InvalidUri(format!("Invalid scheme token: {}", s))),
        }
    }
}

/// Represents the host part of a URI.
/// Can be a domain name or an IP address.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Host {
    /// A domain name (e.g., "example.com").
    Domain(String),
    /// An IP address (v4 or v6).
    Address(IpAddr),
}

impl Host {
    /// Create a new host from a domain name
    pub fn domain(domain: impl Into<String>) -> Self {
        Host::Domain(domain.into())
    }

    /// Create a new host from an IPv4 address
    pub fn ipv4(ip: impl Into<String>) -> Self {
        Host::Address(IpAddr::V4(Ipv4Addr::from_str(ip.into().as_str()).unwrap()))
    }

    /// Create a new host from an IPv6 address
    pub fn ipv6(ip: impl Into<String>) -> Self {
        Host::Address(IpAddr::V6(Ipv6Addr::from_str(ip.into().as_str()).unwrap()))
    }

    /// Returns this host as a string
    pub fn as_str(&self) -> &str {
        match self {
            Host::Domain(domain) => domain,
            Host::Address(addr) => addr.to_string().as_str(),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(domain) => write!(f, "{}", domain),
            Host::Address(addr) => write!(f, "{}", addr),
        }
    }
}

impl FromStr for Host {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Attempt to parse as IP address first
        if let Ok(addr) = IpAddr::from_str(s) {
            return Ok(Host::Address(addr));
        } else if s.starts_with('[') && s.ends_with(']') {
             // Maybe IPv6 literal without brackets parsed?
             if let Ok(addr) = Ipv6Addr::from_str(&s[1..s.len()-1]) {
                 return Ok(Host::Address(IpAddr::V6(addr)));
             }
             // If it looked like IPv6 but failed, treat as domain
             Ok(Host::Domain(s.to_string()))
        } else {
            // Assume domain name if not a valid IP
            // TODO: Add stricter domain name validation?
             if s.is_empty() {
                 Err(Error::ParseError("Host cannot be empty".to_string()))
             } else {
                Ok(Host::Domain(s.to_string()))
             }
        }
    }
}

impl From<IpAddr> for Host {
    fn from(addr: IpAddr) -> Self {
        Host::Address(addr)
    }
}

impl From<Ipv4Addr> for Host {
    fn from(addr: Ipv4Addr) -> Self {
        Host::Address(IpAddr::V4(addr))
    }
}

impl From<Ipv6Addr> for Host {
    fn from(addr: Ipv6Addr) -> Self {
        Host::Address(IpAddr::V6(addr))
    }
}

/// SIP URI components as defined in RFC 3261
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uri {
    /// URI scheme (sip, sips, tel)
    pub scheme: Scheme,
    /// User part (optional)
    pub user: Option<String>,
    /// Password (optional, deprecated)
    pub password: Option<String>,
    /// Host (required)
    pub host: Host,
    /// Port (optional)
    pub port: Option<u16>,
    /// URI parameters (;key=value or ;key)
    pub parameters: Vec<Param>,
    /// URI headers (?key=value)
    pub headers: HashMap<String, String>,
}

impl Uri {
    /// Create a new URI with the minimum required fields
    pub fn new(scheme: Scheme, host: Host) -> Self {
        Uri {
            scheme,
            user: None,
            password: None,
            host,
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
        }
    }

    /// Create a new SIP URI with a domain host
    pub fn sip(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, Host::domain(host))
    }

    /// Create a new SIP URI with an IPv4 host
    pub fn sip_ipv4(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, Host::ipv4(host))
    }

    /// Create a new SIP URI with an IPv6 host
    pub fn sip_ipv6(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, Host::ipv6(host))
    }

    /// Create a new SIPS URI
    pub fn sips(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sips, Host::domain(host))
    }

    /// Create a new TEL URI
    pub fn tel(number: impl Into<String>) -> Self {
        Self::new(Scheme::Tel, Host::domain(number))
    }

    /// Get the username part of the URI, if present
    pub fn username(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// Set the user part of the URI
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set the password part of the URI (deprecated in SIP)
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the port part of the URI
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Add a parameter to the URI
    pub fn with_parameter(mut self, param: Param) -> Self {
        self.parameters.push(param);
        self
    }

    /// Add a header to the URI
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Returns the transport parameter if present
    pub fn transport(&self) -> Option<&str> {
        self.parameters.iter().find_map(|p| match p {
            Param::Transport(val) => Some(val.as_str()),
            _ => None,
        })
    }

    /// Returns the user=phone parameter if present
    pub fn is_phone_number(&self) -> bool {
        self.parameters.iter().any(|p| match p {
            Param::User(val) if val == "phone" => true,
            _ => false,
        })
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;

        if let Some(user) = &self.user {
            write!(f, "{}", escape_user_info(user))?;

            if let Some(password) = &self.password {
                write!(f, ":{}", escape_user_info(password))?;
            }

            write!(f, "@")?;
        }

        write!(f, "{}", self.host)?;

        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }

        // Iterate over Vec<Param> for display
        for param in &self.parameters {
            write!(f, "{}", param)?;
        }

        if !self.headers.is_empty() {
            write!(f, "?")?;

            let mut first = true;
            for (key, value) in &self.headers {
                if !first {
                    write!(f, "&")?;
                }
                write!(f, "{}={}", escape_param(key), escape_param(value))?;
                first = false;
            }
        }

        Ok(())
    }
}

// Parser-related functions need to be reimplemented or imported from parser module
// For now we'll just include the basic utility functions used by the URI implementation

// --- Helper functions (escape/unescape, validation) ---

/// Escape URI user info component according to RFC 3261
fn escape_user_info(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3); // Worst case: all chars need escaping (×3)

    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' |
            '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' => {
                result.push(c);
            },
            _ => {
                // Escape all other characters
                for byte in c.to_string().bytes() {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
    }

    result
}

/// Unescape URI user info component
fn unescape_user_info(s: &str) -> Result<String> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            let mut hex = String::with_capacity(2);

            if let Some(h1) = chars.next() {
                hex.push(h1);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: "Incomplete percent encoding".to_string()
                });
            }

            if let Some(h2) = chars.next() {
                hex.push(h2);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: "Incomplete percent encoding".to_string()
                });
            }

            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: format!("Invalid percent encoding: %{}", hex)
                });
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Escape URI parameters and headers
fn escape_param(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);

    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' |
            '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' | '+' => {
                result.push(c);
            },
            _ => {
                // Escape all other characters
                for byte in c.to_string().bytes() {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
    }

    result
}

/// Unescape URI parameters and headers
fn unescape_param(s: &str) -> Result<String> {
    unescape_user_info(s) // Same algorithm
}

/// Check if a string is a valid IPv4 address
fn is_valid_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();

    if parts.len() != 4 {
        return false;
    }

    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => continue,
            Err(_) => return false,
        }
    }

    true
}

/// Check if a string is a valid IPv6 address (simplified validation)
fn is_valid_ipv6(s: &str) -> bool {
    // Check for basic IPv6 format
    let parts = s.split(':').collect::<Vec<&str>>();

    // IPv6 has 8 parts max, or fewer if contains ::
    if parts.len() > 8 {
        return false;
    }

    // Check for empty parts (::)
    let empty_parts = parts.iter().filter(|p| p.is_empty()).count();

    // Handle :: (consecutive colons)
    if empty_parts > 0 {
        if empty_parts > 2 || (empty_parts == 2 && !s.contains("::")) {
            return false;
        }
    }

    // Validate each part
    for part in parts {
        if part.is_empty() {
            continue; // Empty part due to ::
        }

        // Check if it's an IPv4 address in the last part (IPv4-mapped IPv6)
        if part.contains('.') {
            return is_valid_ipv4(part);
        }

        // Each part should be a valid hex number with at most 4 digits
        if part.len() > 4 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }

    true
} 