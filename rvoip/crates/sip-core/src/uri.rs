use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

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
use crate::types::Param; // Import the Param enum

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
        match s.to_lowercase().as_str() {
            "sip" => Ok(Scheme::Sip),
            "sips" => Ok(Scheme::Sips),
            "tel" => Ok(Scheme::Tel),
            _ => Err(Error::InvalidUri(format!("Invalid scheme: {s}"))),
        }
    }
}

/// Host type for SIP URIs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Host {
    /// Domain name (e.g., example.com)
    Domain(String),
    /// IPv4 address (e.g., 192.168.1.1)
    IPv4(String),
    /// IPv6 address (e.g., 2001:db8::1)
    IPv6(String),
}

impl Host {
    /// Create a new host from a domain name
    pub fn domain(domain: impl Into<String>) -> Self {
        Host::Domain(domain.into())
    }

    /// Create a new host from an IPv4 address
    pub fn ipv4(ip: impl Into<String>) -> Self {
        Host::IPv4(ip.into())
    }

    /// Create a new host from an IPv6 address
    pub fn ipv6(ip: impl Into<String>) -> Self {
        Host::IPv6(ip.into())
    }

    /// Returns this host as a string
    pub fn as_str(&self) -> &str {
        match self {
            Host::Domain(domain) => domain,
            Host::IPv4(ip) => ip,
            Host::IPv6(ip) => ip,
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(domain) => write!(f, "{}", domain),
            Host::IPv4(ip) => write!(f, "{}", ip),
            Host::IPv6(ip) => write!(f, "[{}]", ip),
        }
    }
}

impl FromStr for Host {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.starts_with('[') && s.ends_with(']') {
            // IPv6 address
            let ip = &s[1..s.len()-1];
            if !is_valid_ipv6(ip) {
                return Err(Error::MalformedUriComponent {
                    component: "host".to_string(),
                    message: format!("Invalid IPv6 address: {}", ip),
                });
            }
            Ok(Host::IPv6(ip.to_string()))
        } else if s.chars().all(|c| c.is_ascii_digit() || c == '.') && s.contains('.') {
            // Probably an IPv4 address
            if !is_valid_ipv4(s) {
                return Err(Error::MalformedUriComponent {
                    component: "host".to_string(),
                    message: format!("Invalid IPv4 address: {}", s),
                });
            }
            Ok(Host::IPv4(s.to_string()))
        } else {
            // Domain name
            Ok(Host::Domain(s.to_string()))
        }
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
    pub parameters: Vec<Param>, // Changed from HashMap<String, Option<String>>
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
            parameters: Vec::new(), // Initialize as Vec
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
        // TODO: Handle replacing existing parameters if needed?
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

// Parse the scheme of a URI (sip, sips, tel)
fn scheme_parser(input: &str) -> IResult<&str, Scheme> {
    map_res(
        alt((
            tag("sip"),
            tag("sips"),
            tag("tel"),
        )),
        |s: &str| Scheme::from_str(s)
    )(input)
}

// Parse the userinfo part (user:password@)
fn userinfo_parser(input: &str) -> IResult<&str, (Option<String>, Option<String>)> {
    match opt(terminated(
        pair(
            map(
                take_till(|c| c == ':' || c == '@'),
                |s: &str| unescape_user_info(s).unwrap_or_else(|_| s.to_string())
            ),
            opt(preceded(
                char(':'),
                map(
                    take_till(|c| c == '@'),
                    |s: &str| unescape_user_info(s).unwrap_or_else(|_| s.to_string())
                )
            ))
        ),
        char('@')
    ))(input) {
        Ok((remaining, Some((user, password)))) => Ok((remaining, (Some(user), password))),
        Ok((remaining, None)) => Ok((remaining, (None, None))),
        Err(e) => Err(e),
    }
}

// Parse IPv4 address
fn ipv4_parser(input: &str) -> IResult<&str, Host> {
    let ip_parser = verify(
        take_while1(|c: char| c.is_ascii_digit() || c == '.'),
        |s: &str| is_valid_ipv4(s)
    );

    map(ip_parser, |s: &str| Host::IPv4(s.to_string()))(input)
}

// Parse IPv6 address
fn ipv6_parser(input: &str) -> IResult<&str, Host> {
    let ip_parser = delimited(
        char('['),
        take_while1(|c: char| c.is_ascii_hexdigit() || c == ':' || c == '.'),
        char(']')
    );

    map(ip_parser, |s: &str| Host::IPv6(s.to_string()))(input)
}

// Parse domain name
fn domain_parser(input: &str) -> IResult<&str, Host> {
    let domain_parser = take_while1(|c: char| c.is_alphanumeric() || c == '.' || c == '-' || c == '+');

    map(domain_parser, |s: &str| Host::Domain(s.to_string()))(input)
}

// Parse the host part (either IPv4, IPv6, or domain)
fn host_parser(input: &str) -> IResult<&str, Host> {
    alt((
        ipv6_parser,
        ipv4_parser,
        domain_parser
    ))(input)
}

// Parse the port part
fn port_parser(input: &str) -> IResult<&str, u16> {
    map_res(
        preceded(char(':'), digit1),
        |s: &str| s.parse::<u16>()
    )(input)
}

// Note: parameter_parser and parameters_parser are now in parser/uri.rs
// We might need to import them or adjust the uri_parser below

// Parse a single header
fn header_parser(input: &str) -> IResult<&str, (String, String)> {
    separated_pair(
        map(
            take_till(|c| c == '=' || c == '&'),
            |s: &str| unescape_param(s).unwrap_or_else(|_| s.to_string())
        ),
        char('='),
        map(
            take_till(|c| c == '&'),
            |s: &str| unescape_param(s).unwrap_or_else(|_| s.to_string())
        )
    )(input)
}

// Parse all headers
fn headers_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    preceded(
        char('?'),
        map(
            separated_list0(char('&'), header_parser),
            |headers| headers.into_iter().collect()
        )
    )(input)
}

// Parser for a complete URI
// Note: Assumes parameter_parser and parameters_parser are available
// We need to import them from parser::uri
fn uri_parser(input: &str) -> IResult<&str, Uri> {
    use crate::parser::uri::parameters_parser; // Import the refactored parser

    let (input, scheme) = terminated(scheme_parser, char(':'))(input)?;
    let (input, (user, password)) = userinfo_parser(input)?;
    let (input, host) = host_parser(input)?;
    let (input, port) = opt(port_parser)(input)?;

    let (input, parameters_vec) = opt(parameters_parser)(input)?;
    let (input, headers_map) = opt(headers_parser)(input)?;

    let mut uri = Uri::new(scheme, host);

    uri.user = user;
    uri.password = password;
    uri.port = port;

    uri.parameters = parameters_vec.unwrap_or_default();

    if let Some(hdrs) = headers_map {
        uri.headers = hdrs;
    }

    Ok((input, uri))
}

impl FromStr for Uri {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        // Handle the special '*' URI case
        if trimmed == "*" {
            // Represent '*' as a URI with a special host or convention
            // Option 1: Use a placeholder host
            // return Ok(Uri::new(Scheme::Sip, Host::Domain("*".to_string()))); 
            
            // Option 2: Add a specific variant to Host or Uri? (Requires lib changes)
            // For now, let's return an error as the current structure doesn't fit '*'
             return Err(Error::InvalidUri("Parsing '*' URI is not currently supported by this structure".to_string()));
             // OR, if we decide '*' should map to a default SIP URI for the host:
             // return Ok(Uri::sip("")); // Needs careful consideration
        }
        
        // Proceed with normal parsing if not '*'
        match uri_parser(trimmed) {
            // Ensure the entire input was consumed
            Ok((rest, uri)) if rest.is_empty() => Ok(uri),
            Ok((rest, _)) => Err(Error::InvalidUri(format!("Unexpected trailing characters after URI: {}", rest))),
            Err(e) => Err(Error::InvalidUri(format!("Failed to parse URI: {} - Error: {:?}", s, e))),
        }
    }
}

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