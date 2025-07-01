//! # SIP Via Header
//! 
//! This module provides an implementation of the SIP Via header as defined in
//! [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
//!
//! The Via header is one of the most important headers in SIP, serving multiple purposes:
//!
//! - It traces the path taken by a request so responses can be routed back
//! - It detects and prevents routing loops
//! - It provides information about the protocol used for transmission
//! - It allows for network address translation (NAT) traversal with `received` and `rport` parameters
//!
//! Each Via header can contain multiple comma-separated entries, with each entry having
//! its own protocol information, host, port, and parameters.
//!
//! ## Structure of a Via header
//!
//! ```text
//! Via: SIP/2.0/UDP pc33.atlanta.com:5060;branch=z9hG4bK776asdhds
//! ```
//!
//! The structure consists of:
//! - Protocol name/version (SIP/2.0)
//! - Transport protocol (UDP)
//! - Host (pc33.atlanta.com)
//! - Optional port (5060)
//! - Parameters (branch=z9hG4bK776asdhds)
//!
//! ## Common parameters
//!
//! - `branch`: Transaction identifier (must start with "z9hG4bK" for RFC 3261 compliance)
//! - `received`: IP address where the request was received (added by servers)
//! - `rport`: Used for symmetric response routing through NAT
//! - `maddr`: Multicast address for the request
//! - `ttl`: Time-to-live for multicast messages
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Via header programmatically
//! let via = Via::new(
//!     "SIP", "2.0", "UDP",
//!     "192.168.1.1", Some(5060),
//!     vec![Param::branch("z9hG4bK776asdhds")]
//! ).unwrap();
//!
//! // Get the branch parameter
//! assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
//!
//! // Add rport parameter (for NAT traversal)
//! let mut via = via;
//! via.set_rport(None);  // Flag parameter, will be "rport" without value
//!
//! // Add received parameter (used by servers)
//! use std::net::IpAddr;
//! let addr = IpAddr::from_str("203.0.113.1").unwrap();
//! via.set_received(addr);
//! ```

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
use crate::types::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};

/// A structured representation of a SIP Via header
///
/// The Via header is used to record the path taken by a request so that
/// responses can be routed back along the same path. It can contain multiple
/// Via entries (hop-by-hop), each with its own protocol, host, port, and parameters.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a basic Via header
/// let via = Via::new(
///     "SIP", "2.0", "UDP",
///     "192.168.1.1", Some(5060),
///     vec![Param::branch("z9hG4bK776asdhds")]
/// ).unwrap();
///
/// // Access components
/// assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
/// assert_eq!(via.headers()[0].transport(), "UDP");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Via(pub Vec<ViaHeader>);

impl Via {
    /// Create a new Via header with a single Via entry.
    ///
    /// # Parameters
    ///
    /// - `protocol_name`: Protocol name (usually "SIP")
    /// - `protocol_version`: Protocol version (usually "2.0")
    /// - `transport`: Transport protocol (e.g., "UDP", "TCP", "TLS")
    /// - `host`: Host or IP address
    /// - `port`: Optional port number
    /// - `params`: Vector of parameters (e.g., branch, received, etc.)
    ///
    /// # Returns
    ///
    /// A Result containing the new Via header, or an error if the host is invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", Some(5060),
    ///     vec![Param::branch("z9hG4bK776asdhds")]
    /// ).unwrap();
    /// ```
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

    /// Create a new Via header with simplified parameters.
    ///
    /// This is a convenience constructor that creates a SIP/2.0 Via header
    /// with the given transport, host, and port.
    ///
    /// # Parameters
    ///
    /// - `protocol_name`: Protocol name (usually "SIP")
    /// - `protocol_version`: Protocol version (usually "2.0") 
    /// - `transport`: Transport protocol (e.g., "UDP", "TCP", "TLS")
    /// - `host`: Host or IP address
    /// - `port`: Optional port number
    /// - `params`: Optional additional parameters
    ///
    /// # Returns
    ///
    /// A Result containing the new Via header, or an error if the host is invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new_simple("SIP", "2.0", "UDP", "example.com", None, vec![]).expect("Failed to create Via");
    /// ```
    pub fn new_simple(
        protocol_name: impl Into<String>,
        protocol_version: impl Into<String>,
        transport: impl Into<String>,
        host: impl Into<String>,
        port: Option<u16>,
        params: Vec<Param>,
    ) -> Result<Self> {
        // Create a basic branch parameter with a random ID if none is provided
        let mut all_params = params;
        if !all_params.iter().any(|p| matches!(p, Param::Branch(_))) {
            let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
            all_params.push(Param::branch(branch));
        }
        
        // Use the full constructor and propagate any errors
        Self::new(
            protocol_name, protocol_version, transport,
            host, port,
            all_params
        )
    }

    /// Get the branch parameter value from the first Via entry.
    ///
    /// The branch parameter uniquely identifies a transaction in SIP
    /// and must be globally unique. In RFC 3261, it must start with
    /// the magic cookie "z9hG4bK" to distinguish it from RFC 2543 
    /// implementations.
    ///
    /// # Returns
    ///
    /// The branch parameter value as a string slice, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![Param::branch("z9hG4bK776asdhds")]
    /// ).unwrap();
    ///
    /// assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
    /// ```
    pub fn branch(&self) -> Option<&str> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Branch(val) => Some(val.as_str()),
                _ => None,
            })
        })
    }

    /// Set or replace the branch parameter in all Via entries.
    ///
    /// # Parameters
    ///
    /// - `branch`: The branch value to set (should start with "z9hG4bK" for RFC 3261 compliance)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// via.set_branch("z9hG4bK776asdhds");
    /// assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
    /// ```
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
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to look for
    ///
    /// # Returns
    ///
    /// - `Some(Some(String))` if the parameter exists and has a value
    /// - `Some(None)` if the parameter exists but has no value (flag parameter)
    /// - `None` if the parameter doesn't exist
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![
    ///         Param::branch("z9hG4bK776asdhds"),
    ///         Param::Lr,  // Flag parameter with no value
    ///         Param::Other("custom".to_string(), Some("value".into()))
    ///     ]
    /// ).unwrap();
    ///
    /// assert_eq!(via.get("branch"), Some(Some("z9hG4bK776asdhds".to_string())));
    /// assert_eq!(via.get("lr"), Some(None));  // Flag parameter
    /// assert_eq!(via.get("custom"), Some(Some("value".to_string())));
    /// assert_eq!(via.get("nonexistent"), None);
    /// ```
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

    /// Set or replace a parameter in all Via entries.
    ///
    /// This method allows setting any parameter on all Via entries. For standard parameters
    /// like `branch`, `received`, `maddr`, etc., it will use the appropriate `Param` variant.
    /// For non-standard parameters, it will use `Param::Other`.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to set
    /// - `value`: The parameter value to set, or None to add a flag parameter
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// // Set a standard parameter
    /// via.set("branch", Some("z9hG4bK776asdhds"));
    /// assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
    ///
    /// // Set a custom parameter
    /// via.set("custom", Some("value"));
    /// assert_eq!(via.get("custom"), Some(Some("value".to_string())));
    ///
    /// // Set a flag parameter
    /// via.set("lr", None::<String>);
    /// assert_eq!(via.get("lr"), Some(None));
    /// ```
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
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to check for
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![
    ///         Param::branch("z9hG4bK776asdhds"),
    ///         Param::Lr
    ///     ]
    /// ).unwrap();
    ///
    /// assert!(via.contains("branch"));
    /// assert!(via.contains("lr"));
    /// assert!(!via.contains("nonexistent"));
    /// ```
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
    ///
    /// # Returns
    ///
    /// A slice containing all the Via header entries
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![Param::branch("z9hG4bK776asdhds")]
    /// ).unwrap();
    ///
    /// assert_eq!(via.headers().len(), 1);
    /// ```
    pub fn headers(&self) -> &[ViaHeader] {
        &self.0
    }

    /// Get the received parameter value as IpAddr from the first Via entry.
    ///
    /// The received parameter is added by a server to indicate the source
    /// IP address from which the request was received, used for NAT traversal.
    ///
    /// # Returns
    ///
    /// The received parameter as an IpAddr, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::net::IpAddr;
    /// use std::str::FromStr;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// let addr = IpAddr::from_str("192.0.2.1").unwrap();
    /// via.set_received(addr);
    ///
    /// assert_eq!(via.received(), Some(addr));
    /// ```
    pub fn received(&self) -> Option<IpAddr> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Received(ip) => Some(*ip),
                _ => None,
            })
        })
    }

    /// Sets or replaces the received parameter in all Via entries.
    ///
    /// # Parameters
    ///
    /// - `addr`: The IP address to set as the received parameter
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::net::IpAddr;
    /// use std::str::FromStr;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// let addr = IpAddr::from_str("192.0.2.1").unwrap();
    /// via.set_received(addr);
    ///
    /// assert_eq!(via.received(), Some(addr));
    /// ```
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
    ///
    /// The maddr parameter specifies the multicast address for the request.
    /// It's used in the context of multicast SIP transmissions.
    ///
    /// # Returns
    ///
    /// The maddr parameter value as a string slice, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// via.set_maddr("224.0.1.75");
    /// assert_eq!(via.maddr(), Some("224.0.1.75"));
    /// ```
    pub fn maddr(&self) -> Option<&str> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Maddr(val) => Some(val.as_str()),
                _ => None,
            })
        })
    }

    /// Sets or replaces the maddr parameter in all Via entries.
    ///
    /// # Parameters
    ///
    /// - `maddr`: The multicast address to set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// via.set_maddr("224.0.1.75");
    /// assert_eq!(via.maddr(), Some("224.0.1.75"));
    /// ```
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
    ///
    /// The ttl (time-to-live) parameter specifies the number of hops a request
    /// can travel before being discarded. It's primarily used for multicast
    /// SIP messages.
    ///
    /// # Returns
    ///
    /// The ttl parameter value as a u8, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// via.set_ttl(5);
    /// assert_eq!(via.ttl(), Some(5));
    /// ```
    pub fn ttl(&self) -> Option<u8> {
        self.0.first().and_then(|v| {
            v.params.iter().find_map(|p| match p {
                Param::Ttl(val) => Some(*val),
                _ => None,
            })
        })
    }

    /// Sets or replaces the ttl parameter in all Via entries.
    ///
    /// # Parameters
    ///
    /// - `ttl`: The time-to-live value to set (0-255)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// via.set_ttl(5);
    /// assert_eq!(via.ttl(), Some(5));
    /// ```
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
    ///
    /// The rport parameter is used for symmetric response routing through NAT.
    /// In requests, it's typically a flag parameter with no value.
    /// In responses, servers add the source port value to this parameter.
    ///
    /// # Returns
    ///
    /// - `None` if rport is not present
    /// - `Some(None)` if rport is a flag with no value
    /// - `Some(Some(port))` if rport has a port value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Via with rport as a flag parameter
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![Param::Rport(None)]
    /// ).unwrap();
    ///
    /// assert_eq!(via.rport(), Some(None));
    ///
    /// // Set rport with a value (as a server would do)
    /// via.set_rport(Some(12345));
    /// assert_eq!(via.rport(), Some(Some(12345)));
    /// ```
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
    ///
    /// # Parameters
    ///
    /// - `port`: The port value to set, or None to add rport as a flag parameter
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", None,
    ///     vec![]
    /// ).unwrap();
    ///
    /// // Add rport as a flag parameter (as a client would do in a request)
    /// via.set_rport(None);
    /// assert_eq!(via.rport(), Some(None));
    ///
    /// // Set rport with a value (as a server would do in a response)
    /// via.set_rport(Some(12345));
    /// assert_eq!(via.rport(), Some(Some(12345)));
    /// ```
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
///
/// The sent-protocol part of a Via header contains the protocol name,
/// protocol version, and transport protocol used for the SIP message.
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::types::via::SentProtocol;
///
/// let protocol = SentProtocol {
///     name: "SIP".to_string(),
///     version: "2.0".to_string(),
///     transport: "UDP".to_string(),
/// };
///
/// assert_eq!(protocol.to_string(), "SIP/2.0/UDP");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentProtocol {
    /// Protocol name (usually "SIP")
    pub name: String,
    /// Protocol version (usually "2.0")
    pub version: String,
    /// Transport protocol (e.g., "UDP", "TCP", "TLS", "SCTP")
    pub transport: String,
}

impl fmt::Display for SentProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.name, self.version, self.transport)
    }
}

/// Represents a single Via header entry.
///
/// Each Via header can contain multiple entries, each representing a hop
/// in the request path. This struct represents one such entry, with its
/// protocol information, host, port, and parameters.
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
/// use rvoip_sip_core::types::uri::Host;
/// use rvoip_sip_core::types::Param;
///
/// let via_header = ViaHeader {
///     sent_protocol: SentProtocol {
///         name: "SIP".to_string(),
///         version: "2.0".to_string(),
///         transport: "UDP".to_string(),
///     },
///     sent_by_host: Host::domain("example.com"),
///     sent_by_port: Some(5060),
///     params: vec![Param::branch("z9hG4bK776asdhds")],
/// };
///
/// assert_eq!(via_header.to_string(), "SIP/2.0/UDP example.com:5060;branch=z9hG4bK776asdhds");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViaHeader {
    /// Protocol information (name, version, transport)
    pub sent_protocol: SentProtocol,
    /// Host name or IP address
    pub sent_by_host: Host,
    /// Optional port number
    pub sent_by_port: Option<u16>,
    /// Parameters (branch, received, rport, etc.)
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
    ///
    /// # Returns
    ///
    /// A string containing the protocol name and version (e.g., "SIP/2.0")
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![],
    /// };
    ///
    /// assert_eq!(via_header.protocol(), "SIP/2.0");
    /// ```
    pub fn protocol(&self) -> String {
        format!("{}/{}", self.sent_protocol.name, self.sent_protocol.version)
    }
    
    /// Returns the transport protocol (e.g., "UDP", "TCP")
    ///
    /// # Returns
    ///
    /// A string slice containing the transport protocol
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "TCP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![],
    /// };
    ///
    /// assert_eq!(via_header.transport(), "TCP");
    /// ```
    pub fn transport(&self) -> &str {
        &self.sent_protocol.transport
    }
    
    /// Returns the host part of the Via header
    ///
    /// # Returns
    ///
    /// A reference to the Host enum
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![],
    /// };
    ///
    /// assert_eq!(via_header.host().to_string(), "example.com");
    /// ```
    pub fn host(&self) -> &Host {
        &self.sent_by_host
    }
    
    /// Returns the port in the Via header, if present
    ///
    /// # Returns
    ///
    /// An Option containing the port number, or None if not specified
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: Some(5060),
    ///     params: vec![],
    /// };
    ///
    /// assert_eq!(via_header.port(), Some(5060));
    /// ```
    pub fn port(&self) -> Option<u16> {
        self.sent_by_port
    }
    
    /// Retrieves the branch parameter value, if present
    ///
    /// # Returns
    ///
    /// An Option containing the branch value as a string slice, or None if not present
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    /// use rvoip_sip_core::types::Param;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![Param::branch("z9hG4bK776asdhds")],
    /// };
    ///
    /// assert_eq!(via_header.branch(), Some("z9hG4bK776asdhds"));
    /// ```
    pub fn branch(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Branch(val) => Some(val.as_str()),
            _ => None,
        })
    }
    
    /// Retrieves the received parameter as a string, if present
    ///
    /// # Returns
    ///
    /// An Option containing the received IP address as a string, or None if not present
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    /// use rvoip_sip_core::types::Param;
    /// use std::net::IpAddr;
    /// use std::str::FromStr;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![Param::Received(IpAddr::from_str("192.0.2.1").unwrap())],
    /// };
    ///
    /// assert_eq!(via_header.received(), Some("192.0.2.1".to_string()));
    /// ```
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
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to look for
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    /// use rvoip_sip_core::types::Param;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![Param::branch("z9hG4bK776asdhds")],
    /// };
    ///
    /// assert!(via_header.contains("branch"));
    /// assert!(!via_header.contains("nonexistent"));
    /// ```
    pub fn contains(&self, name: &str) -> bool {
        self.params.iter().any(|p| match p {
            Param::Other(key, _) => key.eq_ignore_ascii_case(name),
            Param::Branch(_) if name.eq_ignore_ascii_case("branch") => true,
            Param::Received(_) if name.eq_ignore_ascii_case("received") => true,
            Param::Maddr(_) if name.eq_ignore_ascii_case("maddr") => true,
            Param::Ttl(_) if name.eq_ignore_ascii_case("ttl") => true,
            Param::Lr if name.eq_ignore_ascii_case("lr") => true,
            _ => false,
        })
    }

    /// Alias for contains() method to maintain backward compatibility
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to look for
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    pub fn has_param(&self, name: &str) -> bool {
        self.contains(name)
    }

    /// Returns the value of a parameter, if present
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to look for
    ///
    /// # Returns
    ///
    /// - `Some(Some(value))` if the parameter exists and has a value
    /// - `Some(None)` if the parameter exists but has no value (flag parameter)
    /// - `None` if the parameter doesn't exist
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::via::{SentProtocol, ViaHeader};
    /// use rvoip_sip_core::types::uri::Host;
    /// use rvoip_sip_core::types::Param;
    ///
    /// let via_header = ViaHeader {
    ///     sent_protocol: SentProtocol {
    ///         name: "SIP".to_string(),
    ///         version: "2.0".to_string(),
    ///         transport: "UDP".to_string(),
    ///     },
    ///     sent_by_host: Host::domain("example.com"),
    ///     sent_by_port: None,
    ///     params: vec![
    ///         Param::branch("z9hG4bK776asdhds"),
    ///         Param::Lr,
    ///         Param::Other("custom".to_string(), Some("value".into()))
    ///     ],
    /// };
    ///
    /// assert_eq!(via_header.param_value("branch"), Some(Some("z9hG4bK776asdhds".to_string())));
    /// assert_eq!(via_header.param_value("lr"), Some(None));
    /// assert_eq!(via_header.param_value("custom"), Some(Some("value".to_string())));
    /// assert_eq!(via_header.param_value("nonexistent"), None);
    /// ```
    pub fn param_value(&self, name: &str) -> Option<Option<String>> {
        self.params.iter().find_map(|p| match p {
            Param::Other(key, value) if key.eq_ignore_ascii_case(name) => {
                Some(value.as_ref().and_then(|gv| gv.as_str().map(String::from)))
            },
            Param::Branch(val) if name.eq_ignore_ascii_case("branch") => Some(Some(val.to_string())),
            Param::Received(val) if name.eq_ignore_ascii_case("received") => Some(Some(val.to_string())),
            Param::Maddr(val) if name.eq_ignore_ascii_case("maddr") => Some(Some(val.to_string())),
            Param::Ttl(val) if name.eq_ignore_ascii_case("ttl") => Some(Some(val.to_string())),
            Param::Rport(val) if name.eq_ignore_ascii_case("rport") => 
                Some(val.map(|v| v.to_string())),
            Param::Lr if name.eq_ignore_ascii_case("lr") => Some(None),
            _ => None,
        })
    }
}

// Add TypedHeaderTrait implementation
impl TypedHeaderTrait for Via {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Via` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Via
    }

    /// Converts this Via header into a generic Header.
    ///
    /// Creates a Header instance from this Via header, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A Header instance representing this Via header
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Via(self.0.clone()))
    }

    /// Creates a Via header from a generic Header.
    ///
    /// Attempts to parse and convert a generic Header into a Via header.
    /// This will succeed if the header is a valid Via header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Via header if successful, or an error otherwise
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != HeaderName::Via {
            return Err(Error::InvalidHeader(format!(
                "Expected Via header, got {:?}", header.name
            )));
        }

        // Try to use the pre-parsed value if available
        if let HeaderValue::Via(values) = &header.value {
            return Ok(Via(values.clone()));
        }

        // Otherwise parse from raw value
        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    s.parse::<Via>()
                } else {
                    Err(Error::ParseError("Invalid UTF-8 in Via header".to_string()))
                }
            },
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected value type for Via header: {:?}", header.value
            ))),
        }
    }
}

// Implement FromStr for Via
impl FromStr for Via {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match crate::parser::headers::via::parse_via_params_public(s.as_bytes()) {
            Ok((_, headers)) => Ok(Via(headers)),
            Err(e) => Err(crate::error::Error::ParseError(format!("Failed to parse Via header: {:?}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_via_typed_header_trait() {
        // Create a Via header
        let via_str = "SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK776asdhds";
        let via = Via::from_str(via_str).unwrap();

        // Test header_name()
        assert_eq!(Via::header_name(), HeaderName::Via);

        // Test to_header()
        let header = via.to_header();
        assert_eq!(header.name, HeaderName::Via);

        // Test from_header()
        let round_trip = Via::from_header(&header).unwrap();
        assert_eq!(round_trip, via);
    }
}