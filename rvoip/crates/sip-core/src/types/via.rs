use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::net::IpAddr;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::types::Param;
use crate::types::uri::Host as UriHost;
use crate::types::param::Param;
use crate::types::param::GenericValue;
use std::net::{Ipv4Addr, Ipv6Addr};
use crate::parser::headers::via::ViaHeader;

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
        let sent_by_host = UriHost::from_str(&host.into())?;

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
    pub fn get(&self, name: &str) -> Option<Option<&str>> {
        self.0.first().and_then(|v| {
             v.params.iter().find_map(|p| match p {
                 Param::Other(key, value) if key.eq_ignore_ascii_case(name) => {
                    Some(value.as_ref().and_then(|gv| gv.as_str()))
                 },
                 // Add cases for known params if needed (e.g., Branch, Tag, etc.)
                 Param::Branch(val) if "branch".eq_ignore_ascii_case(name) => Some(Some(val.as_str())),
                 Param::Received(val) if "received".eq_ignore_ascii_case(name) => Some(Some(val.to_string().as_str())), // Needs conversion
                 Param::Maddr(val) if "maddr".eq_ignore_ascii_case(name) => Some(Some(val.as_str())),
                 Param::Ttl(val) if "ttl".eq_ignore_ascii_case(name) => Some(Some(val.to_string().as_str())), // Needs conversion
                 // Add more known params...
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
    pub fn rport(&self) -> bool {
         self.0.first().map_or(false, |v| {
            v.params.iter().any(|p| matches!(p, Param::Other(key, None) if key.eq_ignore_ascii_case("rport")))
         })
    }

    /// Sets or removes the rport parameter flag in all Via entries.
    pub fn set_rport(&mut self, present: bool) {
         for v in self.0.iter_mut() {
             let rport_pos = v.params.iter().position(|p| matches!(p, Param::Other(key, _) if key.eq_ignore_ascii_case("rport")));
            if present {
                if rport_pos.is_none() {
                     v.params.push(Param::Other("rport".to_string(), None));
                } else {
                    // Ensure value is None if already present
                    v.params[rport_pos.unwrap()] = Param::Other("rport".to_string(), None);
                }
            } else if let Some(pos) = rport_pos {
                 v.params.remove(pos);
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

// Note: FromStr implementation requires the parser, so it will live 
// with the parsing logic in parser/headers.rs 