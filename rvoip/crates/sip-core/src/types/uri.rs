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

    /// Get the host as a string slice (only works for domain names).
    /// For addresses, it converts to String.
    pub fn as_str(&self) -> String {
        match self {
            Host::Domain(domain) => domain.clone(),
            Host::Address(addr) => addr.to_string(),
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
    /// Raw URI string for custom schemes
    pub raw_uri: Option<String>,
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
            raw_uri: None,
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

    /// Create a new URI with a custom scheme by storing the entire URI string
    /// This is used for schemes that are not explicitly supported (like http, https)
    /// but need to be preserved in the Call-Info header
    pub fn custom(uri_string: impl Into<String>) -> Self {
        let uri_string = uri_string.into();
        
        // If it's a SIP URI, parse it normally
        if uri_string.starts_with("sip:") || uri_string.starts_with("sips:") || uri_string.starts_with("tel:") {
            return Self::from_str(&uri_string).unwrap_or_else(|_| {
                // Fallback to custom storage if parsing fails
                let mut uri = Self::new(Scheme::Sip, Host::domain("unknown.host"));
                uri.raw_uri = Some(uri_string);
                uri
            });
        }
        
        // For non-SIP URIs, store the raw string
        let mut uri = Self::new(Scheme::Sip, Host::domain("unknown.host"));
        uri.raw_uri = Some(uri_string);
        uri
    }

    /// Check if this URI has a custom scheme (non-SIP)
    pub fn is_custom(&self) -> bool {
        self.raw_uri.is_some()
    }

    /// Get the raw URI string if this is a custom URI
    pub fn as_raw_uri(&self) -> Option<&str> {
        self.raw_uri.as_deref()
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
        // If this is a custom URI, just output the raw string
        if let Some(raw_uri) = &self.raw_uri {
            return f.write_str(raw_uri);
        }

        // Normal URI formatting
        write!(f, "{}:", self.scheme)?;
        
        // User info (username and optional password)
        if let Some(ref user) = self.user {
            let escaped_user = escape_user_info(user);
            write!(f, "{}", escaped_user)?;
            
            if let Some(ref password) = self.password {
                let escaped_password = escape_user_info(password);
                write!(f, ":{}", escaped_password)?;
            }
            
            write!(f, "@")?;
        }
        
        // Host (domain or IP address)
        match &self.host {
            Host::Domain(domain) => write!(f, "{}", domain)?,
            Host::Address(IpAddr::V4(addr)) => write!(f, "{}", addr)?,
            Host::Address(IpAddr::V6(addr)) => write!(f, "[{}]", addr)?,
        }
        
        // Optional port (only if not 0)
        if let Some(port) = self.port {
            // Don't show port 0
            if port > 0 {
                write!(f, ":{}", port)?;
            }
        }
        
        // Parameters (;key=value or ;key)
        for param in &self.parameters {
            write!(f, ";{}", param)?;
        }
        
        // Headers (?key=value&key=value)
        if !self.headers.is_empty() {
            let mut first = true;
            for (key, value) in &self.headers {
                if first {
                    write!(f, "?")?;
                    first = false;
                } else {
                    write!(f, "&")?;
                }
                
                // URL-encode key and value
                write!(f, "{}={}", escape_param(key), escape_param(value))?;
            }
        }
        
        Ok(())
    }
}

impl FromStr for Uri {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Parse URI string
        // Format: scheme:user:password@host:port;params?headers
        
        // Extract scheme
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() < 2 {
            return Err(Error::InvalidUri(format!("Missing scheme in URI: {}", s)));
        }
        
        let scheme_str = parts[0];
        let scheme = Scheme::from_str(scheme_str)?;
        let remainder = parts[1];
        
        // Extract user info and host parts
        let (user_info, host_part) = if let Some(idx) = remainder.rfind('@') {
            // Has user info
            let (user_part, host_part) = remainder.split_at(idx);
            (Some(user_part), &host_part[1..]) // Skip the @ character
        } else {
            // No user info
            (None, remainder)
        };
        
        // Parse user and password
        let (user, password) = if let Some(user_info) = user_info {
            if let Some(idx) = user_info.find(':') {
                let (u, p) = user_info.split_at(idx);
                (Some(unescape_user_info(u)?), Some(unescape_user_info(&p[1..])?))
            } else {
                (Some(unescape_user_info(user_info)?), None)
            }
        } else {
            (None, None)
        };
        
        // Extract parameters and headers
        let (host_port, params_headers) = if let Some(idx) = host_part.find(|c| c == ';' || c == '?') {
            host_part.split_at(idx)
        } else {
            (host_part, "")
        };
        
        // Parse host and port
        let (host_str, port) = if let Some(idx) = host_port.rfind(':') {
            let (h, p) = host_port.split_at(idx);
            let port_str = &p[1..];
            match port_str.parse::<u16>() {
                Ok(port) => (h, Some(port)),
                Err(_) => (host_port, None), // Not a valid port, treat it as part of host
            }
        } else {
            (host_port, None)
        };
        
        // Create host
        let host = Host::from_str(host_str)?;
        
        // Initialize the URI
        let mut uri = Uri::new(scheme, host).with_port(port.unwrap_or(0));
        if let Some(u) = user {
            uri.user = Some(u);
        }
        if let Some(p) = password {
            uri.password = Some(p);
        }
        
        // Extract and parse parameters
        let mut param_part = params_headers;
        let mut header_part = "";
        
        if let Some(idx) = param_part.find('?') {
            let (p, h) = param_part.split_at(idx);
            param_part = p;
            header_part = &h[1..]; // Skip the ? character
        }
        
        // Parse parameters
        if !param_part.is_empty() {
            let params = param_part.trim_start_matches(';').split(';');
            for param in params {
                if let Some(idx) = param.find('=') {
                    let (name, value) = param.split_at(idx);
                    uri.parameters.push(Param::Other(
                        name.to_string(),
                        Some(crate::types::param::GenericValue::Token(value[1..].to_string()))
                    ));
                } else {
                    uri.parameters.push(Param::Other(
                        param.to_string(),
                        None
                    ));
                }
            }
        }
        
        // Parse headers
        if !header_part.is_empty() {
            let headers = header_part.split('&');
            for header in headers {
                if let Some(idx) = header.find('=') {
                    let (name, value) = header.split_at(idx);
                    uri.headers.insert(name.to_string(), value[1..].to_string());
                }
            }
        }
        
        Ok(uri)
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