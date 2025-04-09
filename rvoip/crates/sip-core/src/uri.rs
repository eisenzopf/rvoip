use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

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
    pub host: String,
    /// Port (optional)
    pub port: Option<u16>,
    /// URI parameters (;key=value or ;key)
    pub parameters: HashMap<String, Option<String>>,
    /// URI headers (?key=value)
    pub headers: HashMap<String, String>,
}

impl Uri {
    /// Create a new URI with the minimum required fields
    pub fn new(scheme: Scheme, host: impl Into<String>) -> Self {
        Uri {
            scheme,
            user: None,
            password: None,
            host: host.into(),
            port: None,
            parameters: HashMap::new(),
            headers: HashMap::new(),
        }
    }

    /// Create a new SIP URI
    pub fn sip(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, host)
    }

    /// Create a new SIPS URI
    pub fn sips(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sips, host)
    }

    /// Create a new TEL URI
    pub fn tel(number: impl Into<String>) -> Self {
        Self::new(Scheme::Tel, number)
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
    pub fn with_parameter(mut self, key: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        self.parameters.insert(
            key.into(),
            value.map(|v| v.into()),
        );
        self
    }

    /// Add a header to the URI
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Returns the transport parameter if present
    pub fn transport(&self) -> Option<&str> {
        self.parameters.get("transport").and_then(|t| t.as_deref())
    }

    /// Returns the user=phone parameter if present
    pub fn is_phone_number(&self) -> bool {
        self.parameters.get("user")
            .and_then(|u| u.as_deref())
            .map(|u| u == "phone")
            .unwrap_or(false)
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;

        if let Some(user) = &self.user {
            write!(f, "{}", user)?;

            if let Some(password) = &self.password {
                write!(f, ":{}", password)?;
            }

            write!(f, "@")?;
        }

        write!(f, "{}", self.host)?;

        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }

        for (key, value) in &self.parameters {
            write!(f, ";{}", key)?;
            if let Some(val) = value {
                write!(f, "={}", val)?;
            }
        }

        if !self.headers.is_empty() {
            write!(f, "?")?;
            
            let mut first = true;
            for (key, value) in &self.headers {
                if !first {
                    write!(f, "&")?;
                }
                write!(f, "{}={}", key, value)?;
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
fn userinfo_parser(input: &str) -> IResult<&str, (Option<&str>, Option<&str>)> {
    match opt(terminated(
        pair(
            take_till(|c| c == ':' || c == '@'),
            opt(preceded(char(':'), take_till(|c| c == '@')))
        ),
        char('@')
    ))(input) {
        Ok((remaining, Some((user, password)))) => Ok((remaining, (Some(user), password))),
        Ok((remaining, None)) => Ok((remaining, (None, None))),
        Err(e) => Err(e),
    }
}

// Parse the host part
fn host_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '.' || c == '-' || c == '+')(input)
}

// Parse the port part
fn port_parser(input: &str) -> IResult<&str, u16> {
    map_res(
        preceded(char(':'), digit1),
        |s: &str| s.parse::<u16>()
    )(input)
}

// Parse a single parameter
fn parameter_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    preceded(
        char(';'),
        pair(
            map(take_till(|c| c == '=' || c == ';' || c == '?'), String::from),
            opt(preceded(
                char('='),
                map(take_till(|c| c == ';' || c == '?'), String::from)
            ))
        )
    )(input)
}

// Parse all parameters
fn parameters_parser(input: &str) -> IResult<&str, HashMap<String, Option<String>>> {
    map(
        many0(parameter_parser),
        |params| params.into_iter().collect()
    )(input)
}

// Parse a single header
fn header_parser(input: &str) -> IResult<&str, (String, String)> {
    separated_pair(
        map(take_till(|c| c == '=' || c == '&'), String::from),
        char('='),
        map(take_till(|c| c == '&'), String::from)
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
fn uri_parser(input: &str) -> IResult<&str, Uri> {
    let (input, scheme) = terminated(scheme_parser, char(':'))(input)?;
    let (input, (user, password)) = userinfo_parser(input)?;
    let (input, host) = host_parser(input)?;
    let (input, port) = opt(port_parser)(input)?;
    
    let (input, parameters) = opt(parameters_parser)(input)?;
    let (input, headers) = opt(headers_parser)(input)?;
    
    let mut uri = Uri::new(scheme, host);
    
    uri.user = user.map(String::from);
    uri.password = password.map(String::from);
    uri.port = port;
    
    if let Some(params) = parameters {
        uri.parameters = params;
    }
    
    if let Some(hdrs) = headers {
        uri.headers = hdrs;
    }
    
    Ok((input, uri))
}

impl FromStr for Uri {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match uri_parser(s) {
            Ok((_, uri)) => Ok(uri),
            Err(_) => Err(Error::InvalidUri(format!("Failed to parse URI: {s}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_uri() {
        let uri = Uri::sip("example.com");
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.host, "example.com");
        assert!(uri.user.is_none());
        assert!(uri.port.is_none());
        assert!(uri.parameters.is_empty());
        assert!(uri.headers.is_empty());
        
        assert_eq!(uri.to_string(), "sip:example.com");
    }

    #[test]
    fn test_complex_uri() {
        let uri = Uri::sip("example.com")
            .with_user("alice")
            .with_port(5060)
            .with_parameter("transport", Some("tcp"))
            .with_parameter("method", Some("INVITE"))
            .with_header("subject", "Project X");
            
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.host, "example.com");
        assert_eq!(uri.user, Some("alice".to_string()));
        assert_eq!(uri.port, Some(5060));
        assert_eq!(uri.parameters.get("transport").unwrap(), &Some("tcp".to_string()));
        assert_eq!(uri.headers.get("subject").unwrap(), "Project X");
        
        // The order of parameters and headers in the string representation may vary
        let s = uri.to_string();
        assert!(s.starts_with("sip:alice@example.com:5060;"));
        assert!(s.contains("transport=tcp"));
        assert!(s.contains("method=INVITE"));
        assert!(s.contains("?subject=Project X") || s.contains("&subject=Project X"));
    }

    #[test]
    fn test_parse_simple_uri() {
        let uri = Uri::from_str("sip:example.com").unwrap();
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.host, "example.com");
        assert!(uri.user.is_none());
        
        let uri = Uri::from_str("sips:secure.example.com:5061").unwrap();
        assert_eq!(uri.scheme, Scheme::Sips);
        assert_eq!(uri.host, "secure.example.com");
        assert_eq!(uri.port, Some(5061));
    }

    #[test]
    fn test_parse_complex_uri() {
        let uri = Uri::from_str("sip:alice@example.com;transport=tcp?subject=Meeting").unwrap();
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("alice".to_string()));
        assert_eq!(uri.host, "example.com");
        assert_eq!(uri.parameters.get("transport").unwrap(), &Some("tcp".to_string()));
        assert_eq!(uri.headers.get("subject").unwrap(), "Meeting");
    }

    #[test]
    fn test_tel_uri() {
        let uri = Uri::tel("+1-212-555-0123");
        assert_eq!(uri.scheme, Scheme::Tel);
        assert_eq!(uri.host, "+1-212-555-0123");
        
        let uri = Uri::from_str("tel:+1-212-555-0123").unwrap();
        assert_eq!(uri.scheme, Scheme::Tel);
        assert_eq!(uri.host, "+1-212-555-0123");
    }
} 