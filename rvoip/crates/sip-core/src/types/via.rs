use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::net::IpAddr;

use crate::error::{Error, Result};
use crate::types::Param;

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
    pub params: Vec<Param>,
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
            params: Vec::new(),
        }
    }
    
    /// Get the branch parameter value
    pub fn branch(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Branch(val) => Some(val.as_str()),
            _ => None,
        })
    }
    
    /// Set or replace the branch parameter.
    pub fn set_branch(&mut self, branch: impl Into<String>) {
        // Remove existing branch parameter(s)
        self.params.retain(|p| !matches!(p, Param::Branch(_)));
        // Add the new one
        self.params.push(Param::Branch(branch.into()));
    }
    
    /// Get the first value associated with a parameter name (case-insensitive for key).
    pub fn get(&self, name: &str) -> Option<Option<&str>> {
        self.params.iter().find_map(|p| match p {
            Param::Other(key, val) if key.eq_ignore_ascii_case(name) => Some(val.as_deref()),
            Param::Branch(val) if name.eq_ignore_ascii_case("branch") => Some(Some(val.as_str())),
            Param::Tag(val) if name.eq_ignore_ascii_case("tag") => Some(Some(val.as_str())),
            Param::Expires(val) if name.eq_ignore_ascii_case("expires") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
            Param::Received(val) if name.eq_ignore_ascii_case("received") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
            Param::Maddr(val) if name.eq_ignore_ascii_case("maddr") => Some(Some(val.as_str())),
            Param::Ttl(val) if name.eq_ignore_ascii_case("ttl") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
            Param::Q(val) if name.eq_ignore_ascii_case("q") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
            Param::Transport(val) if name.eq_ignore_ascii_case("transport") => Some(Some(val.as_str())),
            Param::User(val) if name.eq_ignore_ascii_case("user") => Some(Some(val.as_str())),
            Param::Method(val) if name.eq_ignore_ascii_case("method") => Some(Some(val.as_str())),
            Param::Lr if name.eq_ignore_ascii_case("lr") => Some(None),
            _ => None,
        })
    }
    
    /// Set or replace a parameter. Adds as Param::Other if the key isn't known.
    pub fn set(&mut self, name: impl Into<String>, value: Option<impl Into<String>>) {
        let key_string = name.into();
        let value_opt_string = value.map(|v| v.into());

        // Remove existing parameter(s) with the same name (case-insensitive)
        self.params.retain(|p| match p {
             Param::Other(k, _) => !k.eq_ignore_ascii_case(&key_string),
             Param::Branch(_) => !key_string.eq_ignore_ascii_case("branch"),
             Param::Tag(_) => !key_string.eq_ignore_ascii_case("tag"),
             Param::Expires(_) => !key_string.eq_ignore_ascii_case("expires"),
             Param::Received(_) => !key_string.eq_ignore_ascii_case("received"),
             Param::Maddr(_) => !key_string.eq_ignore_ascii_case("maddr"),
             Param::Ttl(_) => !key_string.eq_ignore_ascii_case("ttl"),
             Param::Lr => !key_string.eq_ignore_ascii_case("lr"),
             Param::Q(_) => !key_string.eq_ignore_ascii_case("q"),
             Param::Transport(_) => !key_string.eq_ignore_ascii_case("transport"),
             Param::User(_) => !key_string.eq_ignore_ascii_case("user"),
             Param::Method(_) => !key_string.eq_ignore_ascii_case("method"),
        });

        // Add the new parameter (attempt to use specific type if possible)
        let param = match key_string.to_ascii_lowercase().as_str() {
            "branch" => Param::Branch(value_opt_string.unwrap_or_default()),
            "tag" => Param::Tag(value_opt_string.unwrap_or_default()),
            "expires" => value_opt_string.and_then(|v| v.parse().ok()).map(Param::Expires).unwrap_or_else(|| Param::Other(key_string, value_opt_string)),
            "received" => value_opt_string.and_then(|v| v.parse().ok()).map(Param::Received).unwrap_or_else(|| Param::Other(key_string, value_opt_string)),
            "maddr" => Param::Maddr(value_opt_string.unwrap_or_default()),
            "ttl" => value_opt_string.and_then(|v| v.parse().ok()).map(Param::Ttl).unwrap_or_else(|| Param::Other(key_string, value_opt_string)),
            "lr" => Param::Lr,
            "q" => value_opt_string.and_then(|v| v.parse().ok()).map(Param::Q).unwrap_or_else(|| Param::Other(key_string, value_opt_string)),
            "transport" => Param::Transport(value_opt_string.unwrap_or_default()),
            "user" => Param::User(value_opt_string.unwrap_or_default()),
            "method" => Param::Method(value_opt_string.unwrap_or_default()),
            _ => Param::Other(key_string, value_opt_string),
        };
        self.params.push(param);
    }
    
    /// Check if a parameter exists (case-insensitive key).
    pub fn contains(&self, name: &str) -> bool {
        self.params.iter().any(|p| match p {
            Param::Other(key, _) => key.eq_ignore_ascii_case(name),
            Param::Branch(_) => name.eq_ignore_ascii_case("branch"),
            Param::Tag(_) => name.eq_ignore_ascii_case("tag"),
            Param::Expires(_) => name.eq_ignore_ascii_case("expires"),
            Param::Received(_) => name.eq_ignore_ascii_case("received"),
            Param::Maddr(_) => name.eq_ignore_ascii_case("maddr"),
            Param::Ttl(_) => name.eq_ignore_ascii_case("ttl"),
            Param::Lr => name.eq_ignore_ascii_case("lr"),
            Param::Q(_) => name.eq_ignore_ascii_case("q"),
            Param::Transport(_) => name.eq_ignore_ascii_case("transport"),
            Param::User(_) => name.eq_ignore_ascii_case("user"),
            Param::Method(_) => name.eq_ignore_ascii_case("method"),
        })
    }
    
    /// Get all parameters as a reference to the Vec<Param>
    pub fn params(&self) -> &Vec<Param> {
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
        for param in &self.params {
            result.push_str(&param.to_string());
        }
        
        result
    }

    /// Get the received parameter value as IpAddr, if present and valid.
    pub fn received(&self) -> Option<IpAddr> {
        self.params.iter().find_map(|p| match p {
            Param::Received(ip) => Some(*ip),
            // Also check Other in case it wasn't parsed specifically
            Param::Other(key, Some(val)) if key.eq_ignore_ascii_case("received") => {
                 IpAddr::from_str(val).ok()
            }
            _ => None,
        })
    }
    
    /// Sets or replaces the received parameter.
    pub fn set_received(&mut self, addr: IpAddr) {
        self.params.retain(|p| !matches!(p, Param::Received(_) | Param::Other(k, _) if k.eq_ignore_ascii_case("received")));
        self.params.push(Param::Received(addr));
    }
    
    /// Get the maddr parameter value, if present.
    pub fn maddr(&self) -> Option<&str> {
         self.params.iter().find_map(|p| match p {
            Param::Maddr(val) => Some(val.as_str()),
            Param::Other(key, Some(val)) if key.eq_ignore_ascii_case("maddr") => Some(val.as_str()),
            _ => None,
        })
    }
    
    /// Sets or replaces the maddr parameter.
    pub fn set_maddr(&mut self, maddr: impl Into<String>) {
        let maddr_string = maddr.into();
        self.params.retain(|p| !matches!(p, Param::Maddr(_) | Param::Other(k, _) if k.eq_ignore_ascii_case("maddr")));
        self.params.push(Param::Maddr(maddr_string));
    }

     /// Get the ttl parameter value, if present and valid.
    pub fn ttl(&self) -> Option<u8> {
        self.params.iter().find_map(|p| match p {
            Param::Ttl(val) => Some(*val),
             Param::Other(key, Some(val)) if key.eq_ignore_ascii_case("ttl") => {
                 val.parse::<u8>().ok()
            }
            _ => None,
        })
    }
    
    /// Sets or replaces the ttl parameter.
    pub fn set_ttl(&mut self, ttl: u8) {
        self.params.retain(|p| !matches!(p, Param::Ttl(_) | Param::Other(k, _) if k.eq_ignore_ascii_case("ttl")));
        self.params.push(Param::Ttl(ttl));
    }

    /// Check if the rport parameter (flag) is present.
    pub fn rport(&self) -> bool {
         self.params.iter().any(|p| matches!(p, Param::Other(k, None) if k.eq_ignore_ascii_case("rport")))
    }
    
    /// Sets or removes the rport parameter flag.
    pub fn set_rport(&mut self, present: bool) {
         self.params.retain(|p| !matches!(p, Param::Other(k, None) if k.eq_ignore_ascii_case("rport")));
         if present {
             self.params.push(Param::Other("rport".to_string(), None));
         }
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

// Note: FromStr implementation requires the parser, so it will live 
// with the parsing logic in parser/headers.rs 