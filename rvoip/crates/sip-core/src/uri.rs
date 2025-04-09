use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

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

// Note: For now we'll just implement a simple parsing
// In a full implementation, we'd use the nom parser
impl FromStr for Uri {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Very basic implementation for now
        if let Some((scheme_str, rest)) = s.split_once(':') {
            let scheme = Scheme::from_str(scheme_str)?;
            
            let mut uri = Uri::new(scheme, "");
            
            // Check if we have user info
            let (authority, extra) = if let Some((authority, extra)) = rest.split_once(|c| c == ';' || c == '?') {
                (authority, Some(extra))
            } else {
                (rest, None)
            };
            
            // Parse user/host part
            if let Some((user_info, host_port)) = authority.split_once('@') {
                // User part exists
                if let Some((user, password)) = user_info.split_once(':') {
                    uri.user = Some(user.to_string());
                    uri.password = Some(password.to_string());
                } else {
                    uri.user = Some(user_info.to_string());
                }
                
                // Parse host:port
                if let Some((host, port_str)) = host_port.split_once(':') {
                    uri.host = host.to_string();
                    uri.port = Some(port_str.parse().map_err(|_| Error::InvalidUri("Invalid port".into()))?);
                } else {
                    uri.host = host_port.to_string();
                }
            } else {
                // No user part
                if let Some((host, port_str)) = authority.split_once(':') {
                    uri.host = host.to_string();
                    uri.port = Some(port_str.parse().map_err(|_| Error::InvalidUri("Invalid port".into()))?);
                } else {
                    uri.host = authority.to_string();
                }
            }
            
            // Handle parameters and headers
            if let Some(extra) = extra {
                if extra.contains('?') {
                    let (params_str, headers_str) = extra.split_once('?').unwrap();
                    
                    // Parse parameters
                    if !params_str.is_empty() {
                        for param in params_str.split(';') {
                            if !param.is_empty() {
                                if let Some((key, value)) = param.split_once('=') {
                                    uri.parameters.insert(key.to_string(), Some(value.to_string()));
                                } else {
                                    uri.parameters.insert(param.to_string(), None);
                                }
                            }
                        }
                    }
                    
                    // Parse headers
                    for header in headers_str.split('&') {
                        if !header.is_empty() {
                            if let Some((key, value)) = header.split_once('=') {
                                uri.headers.insert(key.to_string(), value.to_string());
                            }
                        }
                    }
                } else {
                    // Only parameters, no headers
                    for param in extra.split(';') {
                        if !param.is_empty() {
                            if let Some((key, value)) = param.split_once('=') {
                                uri.parameters.insert(key.to_string(), Some(value.to_string()));
                            } else {
                                uri.parameters.insert(param.to_string(), None);
                            }
                        }
                    }
                }
            }
            
            Ok(uri)
        } else {
            Err(Error::InvalidUri("Missing scheme".into()))
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