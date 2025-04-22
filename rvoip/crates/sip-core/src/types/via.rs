use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::net::IpAddr;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::types::Param;
use crate::types::uri::Host;
use crate::types::param::Param;
use crate::types::param::GenericValue;
use std::net::{Ipv4Addr, Ipv6Addr};
use crate::parser::headers::via::ViaHeader;

/// A structured representation of a SIP Via header
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Via(pub Vec<ViaHeader>);

impl Via {
    /// Create a new Via header
    pub fn new(
        protocol: impl Into<String>,
        version: impl Into<String>,
        transport: impl Into<String>,
        host: impl Into<String>,
        port: Option<u16>,
    ) -> Self {
        Self(vec![ViaHeader::new(
            protocol.into(),
            version.into(),
            transport.into(),
            host.into(),
            port,
        )])
    }
    
    /// Get the branch parameter value
    pub fn branch(&self) -> Option<&str> {
        self.0.iter().find_map(|v| v.branch())
    }
    
    /// Set or replace the branch parameter.
    pub fn set_branch(&mut self, branch: impl Into<String>) {
        self.0.iter_mut().for_each(|v| v.set_branch(branch.clone()));
    }
    
    /// Get the first value associated with a parameter name (case-insensitive for key).
    pub fn get(&self, name: &str) -> Option<Option<&str>> {
        self.0.iter().find_map(|v| v.get(name))
    }
    
    /// Set or replace a parameter. Adds as Param::Other if the key isn't known.
    pub fn set(&mut self, name: impl Into<String>, value: Option<impl Into<String>>) {
        self.0.iter_mut().for_each(|v| v.set(name.clone(), value.clone()));
    }
    
    /// Check if a parameter exists (case-insensitive key).
    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|v| v.contains(name))
    }
    
    /// Get all parameters as a reference to the Vec<Param>
    pub fn params(&self) -> &Vec<ViaHeader> {
        &self.0
    }
    
    /// Get a formatted string representation for this Via header
    pub fn to_string(&self) -> String {
        self.0.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
    }

    /// Get the received parameter value as IpAddr, if present and valid.
    pub fn received(&self) -> Option<IpAddr> {
        self.0.iter().find_map(|v| v.received())
    }
    
    /// Sets or replaces the received parameter.
    pub fn set_received(&mut self, addr: IpAddr) {
        self.0.iter_mut().for_each(|v| v.set_received(addr));
    }
    
    /// Get the maddr parameter value, if present.
    pub fn maddr(&self) -> Option<&str> {
        self.0.iter().find_map(|v| v.maddr())
    }
    
    /// Sets or replaces the maddr parameter.
    pub fn set_maddr(&mut self, maddr: impl Into<String>) {
        self.0.iter_mut().for_each(|v| v.set_maddr(maddr.clone()));
    }

     /// Get the ttl parameter value, if present and valid.
    pub fn ttl(&self) -> Option<u8> {
        self.0.iter().find_map(|v| v.ttl())
    }
    
    /// Sets or replaces the ttl parameter.
    pub fn set_ttl(&mut self, ttl: u8) {
        self.0.iter_mut().for_each(|v| v.set_ttl(ttl));
    }

    /// Check if the rport parameter (flag) is present.
    pub fn rport(&self) -> bool {
        self.0.iter().any(|v| v.rport())
    }
    
    /// Sets or removes the rport parameter flag.
    pub fn set_rport(&mut self, present: bool) {
        self.0.iter_mut().for_each(|v| v.set_rport(present));
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

// Note: FromStr implementation requires the parser, so it will live 
// with the parsing logic in parser/headers.rs 