use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::net::IpAddr;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::types::Param;
use crate::types::uri::Host;
use crate::types::param::GenericValue;
use std::net::{Ipv4Addr, Ipv6Addr};

/// A structured representation of a SIP Via header
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Via(pub Vec<ViaHeader>);

impl Via {
    /// Create a new Via header with a single Via entry.
    pub fn new(
        protocol_name: impl Into<String>,
        protocol_version: impl Into<String>,
        transport: impl Into<String>,
        host: impl Into<String>,
        port: Option<u16>,
        params: Vec<Param>,
    ) -> Result<Self> {
        let sent_protocol = SentProtocol {
            name: protocol_name.into(),
            version: protocol_version.into(),
            transport: transport.into(),
        };
        // Attempt to parse the host string into the Host enum from types::uri
        let sent_by_host = Host::from_str(&host.into())?;

        let via_header = ViaHeader {
            sent_protocol,
            sent_by_host,
            sent_by_port: port,
            params,
        };
        Ok(Self(vec![via_header]))
    }

    /// Get the branch parameter value from the first Via entry.
    pub fn branch(&self) -> Option<&str> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Branch(val) => Some(val.as_str()),
                _ => None,
            })
        })
    }

    /// Set or replace the branch parameter in all Via entries.
    pub fn set_branch(&mut self, branch: impl Into<String> + Clone) {
        let branch_string = branch.into();
        for v in self.0.iter_mut() {
            if let Some(pos) = v.params.iter().position(|p| matches!(p, Param::Branch(_))) {
                v.params[pos] = Param::Branch(branch_string.clone());
            } else {
                v.params.push(Param::Branch(branch_string.clone()));
            }
        }
    }

    /// Get the first value associated with a parameter name from the first Via entry (case-insensitive for key).
    pub fn get(&self, name: &str) -> Option<Option<String>> {
        self.0.first().and_then(|v| {
             v.params.iter().find_map(|p| match p {
                 Param::Other(key, value) if key.eq_ignore_ascii_case(name) => {
                    Some(value.as_ref().and_then(|gv| gv.as_str().map(String::from)))
                 },
                 // Add cases for known params if needed (e.g., Branch, Tag, etc.)
                 Param::Branch(val) if "branch".eq_ignore_ascii_case(name) => Some(Some(val.to_string())),
                 Param::Received(val) if "received".eq_ignore_ascii_case(name) => Some(Some(val.to_string())), // Return owned String
                 Param::Maddr(val) if "maddr".eq_ignore_ascii_case(name) => Some(Some(val.to_string())),
                 Param::Ttl(val) if "ttl".eq_ignore_ascii_case(name) => Some(Some(val.to_string())), // Return owned String
                 Param::Lr if "lr".eq_ignore_ascii_case(name) => Some(None), // Flag parameter has no value
                 _ => None,
             })
        })
    }

    /// Set or replace a parameter in all Via entries. Adds as Param::Other if the key isn't known.
    /// Value is converted to GenericValue::Token if Some, or None if None.
    pub fn set(&mut self, name: impl Into<String> + Clone, value: Option<impl Into<String> + Clone>) {
        let name_string = name.into();
        let generic_value = value.map(|v| GenericValue::Token(v.into()));

        for v in self.0.iter_mut() {
            let name_lower = name_string.to_lowercase();
            let pos = v.params.iter().position(|p| match p {
                Param::Other(key, _) if key.eq_ignore_ascii_case(&name_lower) => true,
                Param::Branch(_) if name_lower == "branch" => true,
                Param::Received(_) if name_lower == "received" => true,
                Param::Maddr(_) if name_lower == "maddr" => true,
                Param::Ttl(_) if name_lower == "ttl" => true,
                Param::Lr if name_lower == "lr" => true,
                _ => false,
            });

            let new_param = match name_lower.as_str() {
                "branch" => Param::Branch(generic_value.as_ref().and_then(|gv| gv.as_str()).unwrap_or("").to_string()),
                "received" => Param::Received(generic_value.as_ref().and_then(|gv| gv.as_str()).unwrap_or("").parse().unwrap_or_else(|_| IpAddr::V4(Ipv4Addr::UNSPECIFIED))),
                "maddr" => Param::Maddr(generic_value.as_ref().and_then(|gv| gv.as_str()).unwrap_or("").to_string()),
                "ttl" => Param::Ttl(generic_value.as_ref().and_then(|gv| gv.as_str()).unwrap_or("0").parse().unwrap_or(0)),
                "lr" => Param::Lr,
                _ => Param::Other(name_string.clone(), generic_value.clone()),
            };

            if let Some(idx) = pos {
                 // Replace if value is Some, remove if None (unless it's a flag like lr)
                 if generic_value.is_some() || name_lower == "lr" {
                    v.params[idx] = new_param;
                 } else {
                    v.params.remove(idx);
                 }
            } else if generic_value.is_some() || name_lower == "lr" {
                // Add if not found and value is Some (or it's a flag)
                v.params.push(new_param);
            }
        }
    }

    /// Check if a parameter exists in the first Via entry (case-insensitive key).
    pub fn contains(&self, name: &str) -> bool {
        self.0.first().map_or(false, |v| {
            v.params.iter().any(|p| match p {
                Param::Other(key, _) => key.eq_ignore_ascii_case(name),
                Param::Branch(_) if name.eq_ignore_ascii_case("branch") => true,
                Param::Received(_) if name.eq_ignore_ascii_case("received") => true,
                Param::Maddr(_) if name.eq_ignore_ascii_case("maddr") => true,
                Param::Ttl(_) if name.eq_ignore_ascii_case("ttl") => true,
                Param::Lr if name.eq_ignore_ascii_case("lr") => true,
                 // Add more known params...
                _ => false,
            })
        })
    }

    /// Get all Via headers as a slice.
    pub fn headers(&self) -> &[ViaHeader] {
        &self.0
    }

    /// Get the received parameter value as IpAddr from the first Via entry.
    pub fn received(&self) -> Option<IpAddr> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Received(ip) => Some(*ip),
                _ => None,
            })
        })
    }

    /// Sets or replaces the received parameter in all Via entries.
    pub fn set_received(&mut self, addr: IpAddr) {
         for v in self.0.iter_mut() {
            if let Some(pos) = v.params.iter().position(|p| matches!(p, Param::Received(_))) {
                v.params[pos] = Param::Received(addr);
            } else {
                v.params.push(Param::Received(addr));
            }
        }
    }

    /// Get the maddr parameter value from the first Via entry.
    pub fn maddr(&self) -> Option<&str> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Maddr(val) => Some(val.as_str()),
                _ => None,
            })
        })
    }

    /// Sets or replaces the maddr parameter in all Via entries.
    pub fn set_maddr(&mut self, maddr: impl Into<String> + Clone) {
        let maddr_string = maddr.into();
         for v in self.0.iter_mut() {
            if let Some(pos) = v.params.iter().position(|p| matches!(p, Param::Maddr(_))) {
                v.params[pos] = Param::Maddr(maddr_string.clone());
            } else {
                v.params.push(Param::Maddr(maddr_string.clone()));
            }
        }
    }

     /// Get the ttl parameter value from the first Via entry.
    pub fn ttl(&self) -> Option<u8> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Ttl(val) => Some(*val),
                _ => None,
            })
        })
    }

    /// Sets or replaces the ttl parameter in all Via entries.
    pub fn set_ttl(&mut self, ttl: u8) {
         for v in self.0.iter_mut() {
            if let Some(pos) = v.params.iter().position(|p| matches!(p, Param::Ttl(_))) {
                v.params[pos] = Param::Ttl(ttl);
            } else {
                v.params.push(Param::Ttl(ttl));
            }
        }
    }

    /// Check if the rport parameter (flag) is present in the first Via entry.
    /// Returns None if rport is not present, Some(None) if rport is a flag,
    /// and Some(Some(port)) if rport has a value.
    pub fn rport(&self) -> Option<Option<u16>> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Rport(port) => Some(*port),
                Param::Other(key, value) if key.eq_ignore_ascii_case("rport") => {
                    // Handle as generic for backwards compatibility
                    if let Some(value) = value {
                        value.as_str().and_then(|s| s.parse::<u16>().ok()).map(Some)
                    } else {
                        Some(None)
                    }
                },
                _ => None,
            })
        })
    }

    /// Sets or replaces the rport parameter in all Via entries.
    /// - If port is None, adds rport as a flag parameter
    /// - If port is Some(value), adds rport with the specified value
    pub fn set_rport(&mut self, port: Option<u16>) {
        for v in self.0.iter_mut() {
            if let Some(pos) = v.params.iter().position(|p| 
                matches!(p, Param::Rport(_)) || 
                matches!(p, Param::Other(key, _) if key.eq_ignore_ascii_case("rport"))
            ) {
                v.params[pos] = Param::Rport(port);
            } else {
                v.params.push(Param::Rport(port));
            }
        }
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Join the Display representations of each ViaHeader with ", "
        let header_strings: Vec<String> = self.0.iter().map(|h| h.to_string()).collect();
        write!(f, "{}", header_strings.join(", "))
    }
}

/// Represents the protocol information in a Via header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentProtocol {
    pub name: String,
    pub version: String,
    pub transport: String,
}

impl fmt::Display for SentProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.name, self.version, self.transport)
    }
}

/// Represents a single Via header entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViaHeader {
    pub sent_protocol: SentProtocol,
    pub sent_by_host: Host,
    pub sent_by_port: Option<u16>,
    pub params: Vec<Param>,
}

// Implementation of Display trait for ViaHeader
impl fmt::Display for ViaHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.sent_protocol)?;
        
        // Format sent-by (host:port or host)
        write!(f, "{}", self.sent_by_host)?;
        if let Some(port) = self.sent_by_port {
            write!(f, ":{}", port)?;
        }
        
        // Format parameters
        for param in &self.params {
            write!(f, ";{}", param)?; // Add semicolon before each parameter
        }
        
        Ok(())
    }
}

impl ViaHeader {
    /// Returns the protocol as a string in "name/version" format
    pub fn protocol(&self) -> String {
        format!("{}/{}", self.sent_protocol.name, self.sent_protocol.version)
    }
    
    /// Returns the transport protocol (e.g., "UDP", "TCP")
    pub fn transport(&self) -> &str {
        &self.sent_protocol.transport
    }
    
    /// Returns the host part of the Via header
    pub fn host(&self) -> &Host {
        &self.sent_by_host
    }
    
    /// Returns the port in the Via header, if present
    pub fn port(&self) -> Option<u16> {
        self.sent_by_port
    }
    
    /// Retrieves the branch parameter value, if present
    pub fn branch(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Branch(val) => Some(val.as_str()),
            _ => None,
        })
    }
    
    /// Retrieves the received parameter as a string, if present
    pub fn received(&self) -> Option<String> {
        self.params.iter().find_map(|p| match p {
            Param::Received(ip) => Some(ip.to_string()),
            _ => None,
        })
    }
    
    /// Retrieves the ttl parameter value, if present
    pub fn ttl(&self) -> Option<u8> {
        self.params.iter().find_map(|p| match p {
            Param::Ttl(val) => Some(*val),
            _ => None,
        })
    }
    
    /// Retrieves the maddr parameter value, if present
    pub fn maddr(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Maddr(val) => Some(val.as_str()),
            _ => None,
        })
    }
    
    /// Checks if the header has a specific parameter
    pub fn has_param(&self, name: &str) -> bool {
        self.params.iter().any(|p| match p {
            Param::Branch(_) if name.eq_ignore_ascii_case("branch") => true,
            Param::Received(_) if name.eq_ignore_ascii_case("received") => true,
            Param::Maddr(_) if name.eq_ignore_ascii_case("maddr") => true,
            Param::Ttl(_) if name.eq_ignore_ascii_case("ttl") => true,
            Param::Rport(_) if name.eq_ignore_ascii_case("rport") => true,
            Param::Other(key, _) => key.eq_ignore_ascii_case(name),
            _ => false,
        })
    }
    
    /// Gets a parameter value as a string, if present
    pub fn param_value(&self, name: &str) -> Option<String> {
        self.params.iter().find_map(|p| match p {
            Param::Branch(val) if name.eq_ignore_ascii_case("branch") => Some(val.clone()),
            Param::Received(ip) if name.eq_ignore_ascii_case("received") => Some(ip.to_string()),
            Param::Maddr(val) if name.eq_ignore_ascii_case("maddr") => Some(val.clone()),
            Param::Ttl(val) if name.eq_ignore_ascii_case("ttl") => Some(val.to_string()),
            Param::Rport(Some(val)) if name.eq_ignore_ascii_case("rport") => Some(val.to_string()),
            Param::Rport(None) if name.eq_ignore_ascii_case("rport") => None,
            Param::Other(key, val) if key.eq_ignore_ascii_case(name) => 
                val.as_ref().map(|v| v.to_string()),
            _ => None,
        })
    }
}

// Note: FromStr implementation requires the parser, so it will live 
// with the parsing logic in parser/headers.rs 