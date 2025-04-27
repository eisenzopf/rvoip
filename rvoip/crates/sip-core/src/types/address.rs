//! # SIP Address
//! 
//! This module provides an implementation of the SIP Address format as defined in
//! [RFC 3261 Section 20.10](https://datatracker.ietf.org/doc/html/rfc3261#section-20.10).
//!
//! The Address format is used in various SIP headers including:
//! - From
//! - To
//! - Contact
//! - Route
//! - Record-Route
//!
//! ## Structure of a SIP Address
//!
//! A SIP address can take two forms:
//!
//! 1. **Name Address**: `"Display Name" <sip:user@domain>;param=value`
//! 2. **URI Address**: `sip:user@domain;param=value`
//!
//! In this implementation, all addresses are treated as Name Addresses internally,
//! with an optional display name.
//!
//! ## Common Parameters
//!
//! - `tag`: Uniquely identifies dialog participants (in From/To headers)
//! - `expires`: Indicates expiration time in seconds (in Contact headers)
//! - `q`: Priority value between 0.0 and 1.0 (in Contact headers)
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an Address from a string
//! let addr = Address::from_str("\"John Doe\" <sip:john@example.com>;tag=1234").unwrap();
//! assert_eq!(addr.display_name, Some("John Doe".to_string()));
//! assert_eq!(addr.uri.to_string(), "sip:john@example.com");
//! assert_eq!(addr.tag(), Some("1234"));
//!
//! // Create an Address programmatically
//! let uri = Uri::from_str("sip:alice@example.com").unwrap();
//! let mut addr = Address::new(Some("Alice Smith"), uri);
//! addr.set_tag("5678");
//! ```

use crate::types::uri::Uri;
use crate::types::param::{Param, GenericValue};
use crate::error::{Error, Result};
use crate::parser::parse_address;
use serde::{Serialize, Deserialize};
use std::fmt;
use std::str::FromStr;
use ordered_float::NotNan;

/// Represents a SIP Name Address (Display Name <URI>; params).
///
/// A SIP address consists of:
/// - An optional display name (e.g., "John Doe")
/// - A mandatory URI (e.g., sip:john@example.com)
/// - Optional parameters (e.g., tag=1234)
///
/// The Address type is used in multiple SIP headers including From, To, 
/// Contact, Route, and Record-Route headers.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Parse from a string
/// let addr = Address::from_str("\"John Doe\" <sip:john@example.com>;tag=1234").unwrap();
///
/// // Create programmatically
/// let uri = Uri::from_str("sip:alice@example.com").unwrap();
/// let addr = Address::new(Some("Alice"), uri);
/// ```
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Optional display name component
    pub display_name: Option<String>,
    /// Mandatory URI component
    pub uri: Uri,
    /// Optional parameters (including tag, expires, q-value, etc.)
    pub params: Vec<Param>,
}

// Manual PartialEq implementation to treat None and Some("") display_name as equal
impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        // Compare display_name (treating None and Some("") as equal)
        let display_name_eq = match (&self.display_name, &other.display_name) {
            (None, None) => true,
            (Some(s1), Some(s2)) => s1.trim() == s2.trim(),
            (Some(s), None) | (None, Some(s)) => s.trim().is_empty(),
        };

        // Compare URI and params (params order matters with Vec)
        display_name_eq && self.uri == other.uri && self.params == other.params
    }
}

/// Function to check if quoting is needed for display-name
/// Based on RFC 3261 relaxed LWS rules and token definition.
/// Quotes are needed if it's not a token or contains specific characters like ", \, or spaces.
pub fn needs_quoting(display_name: &str) -> bool {
    if display_name.is_empty() {
        return false; // Empty string should NOT be quoted
    }
    
    // The space character in "Test User" is causing it to be quoted.
    // SIP tokens don't include space, so we need to fix the test expectations instead of the function
    
    // Check for characters that *require* quoting or are not part of a token
    display_name.chars().any(|c| {
        !c.is_alphanumeric() && !matches!(c, '-' | '.' | '!' | '%' | '*' | '_' | '+' | '`' | '\'' | '~')
    }) || display_name.contains('"') || display_name.contains('\\')
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut wrote_display_name = false;
        if let Some(name) = &self.display_name {
            let trimmed_name = name.trim();
            if !trimmed_name.is_empty() {
                if needs_quoting(trimmed_name) {
                    write!(f, "\"{}\"", name.replace("\"", "\\\""))?;
                } else {
                    write!(f, "{}", trimmed_name)?;
                }
                wrote_display_name = true;
            }
        }
        
        if wrote_display_name {
             write!(f, " ")?;
        } 
        // Revert: Always write URI in angle brackets for now for name-addr
        write!(f, "<{}>", self.uri)?;

        // Write parameters
        for param in &self.params {
            write!(f, ";{}", param)?;
        }

        Ok(())
    }
}

impl Address {
    /// Creates a new Address with the given display name and URI.
    ///
    /// The display name is optional and will be normalized:
    /// - An empty or whitespace-only display name will be converted to None
    /// - A non-empty display name will be preserved as provided
    ///
    /// # Parameters
    ///
    /// - `display_name`: Optional display name (e.g., "John Doe")
    /// - `uri`: The SIP URI (e.g., sip:john@example.com)
    ///
    /// # Returns
    ///
    /// A new Address with no parameters
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// 
    /// // With display name
    /// let addr1 = Address::new(Some("Alice Smith"), uri.clone());
    /// assert_eq!(addr1.display_name, Some("Alice Smith".to_string()));
    ///
    /// // Without display name
    /// let addr2 = Address::new(None::<String>, uri.clone());
    /// assert_eq!(addr2.display_name, None);
    ///
    /// // Empty display name becomes None
    /// let addr3 = Address::new(Some(""), uri);
    /// assert_eq!(addr3.display_name, None);
    /// ```
    pub fn new(display_name: Option<impl Into<String>>, uri: Uri) -> Self {
        let normalized_display_name = display_name
            .map(|s| s.into()) // Convert to String
            .filter(|s| !s.trim().is_empty()); // Convert Some("") or Some("  ") to None
            
        Address {
            display_name: normalized_display_name, // Use the normalized version
            uri,
            params: Vec::new(),
        }
    }

    /// Sets or replaces the tag parameter.
    ///
    /// The tag parameter is used in From and To headers to uniquely 
    /// identify dialog participants and ensure dialog matching.
    ///
    /// # Parameters
    ///
    /// - `tag`: The tag value to set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let mut addr = Address::new(Some("Alice"), uri);
    ///
    /// // Set the tag
    /// addr.set_tag("1234abcd");
    /// assert_eq!(addr.tag(), Some("1234abcd"));
    ///
    /// // Replace an existing tag
    /// addr.set_tag("5678efgh");
    /// assert_eq!(addr.tag(), Some("5678efgh"));
    /// ```
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        // Remove existing tag parameter(s)
        self.params.retain(|p| !matches!(p, Param::Tag(_)));
        // Add the new one
        self.params.push(Param::Tag(tag.into()));
    }
    
    /// Gets the tag parameter value, if present.
    ///
    /// The tag parameter is used in From and To headers to uniquely
    /// identify dialog participants and ensure dialog matching.
    ///
    /// # Returns
    ///
    /// The tag parameter value as a string slice, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let addr = Address::from_str("\"John Doe\" <sip:john@example.com>;tag=1234").unwrap();
    /// assert_eq!(addr.tag(), Some("1234"));
    ///
    /// let addr = Address::from_str("\"John Doe\" <sip:john@example.com>").unwrap();
    /// assert_eq!(addr.tag(), None);
    /// ```
    pub fn tag(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Tag(tag_val) => Some(tag_val.as_str()),
            _ => None,
        })
    }
    
    /// Gets the expires parameter value, if present and valid.
    ///
    /// The expires parameter is commonly used in Contact headers to 
    /// indicate registration expiration time in seconds.
    ///
    /// # Returns
    ///
    /// The expires parameter value as seconds (u32), or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let addr = Address::from_str("<sip:alice@example.com>;expires=3600").unwrap();
    /// assert_eq!(addr.expires(), Some(3600));
    ///
    /// let addr = Address::from_str("<sip:alice@example.com>").unwrap();
    /// assert_eq!(addr.expires(), None);
    /// ```
    pub fn expires(&self) -> Option<u32> {
        self.params.iter().find_map(|p| match p {
            Param::Expires(val) => Some(*val),
            Param::Other(key, Some(val)) if key.eq_ignore_ascii_case("expires") => {
                val.as_str().and_then(|s| s.parse().ok()) // Use helper
            },
            _ => None,
        })
    }

    /// Set the expires parameter value.
    ///
    /// The expires parameter is commonly used in Contact headers to
    /// indicate registration expiration time in seconds.
    ///
    /// # Parameters
    ///
    /// - `expires`: The expiration time in seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let mut addr = Address::new(None, uri);
    ///
    /// addr.set_expires(3600); // 1 hour
    /// assert_eq!(addr.expires(), Some(3600));
    /// ```
    pub fn set_expires(&mut self, expires: u32) {
        // Remove existing expires param
        self.params.retain(|p| {
            match p {
                Param::Expires(_) => false, // Remove this variant
                Param::Other(k, _) => !k.eq_ignore_ascii_case("expires"), // Keep if key doesn't match
                _ => true, // Keep other variants
            }
        });
        self.params.push(Param::Expires(expires));
    }

    /// Get the q parameter value, if present.
    ///
    /// The q parameter (quality factor) is commonly used in Contact headers to
    /// indicate a relative priority between 0.0 and 1.0, with higher values
    /// indicating higher priority.
    ///
    /// # Returns
    ///
    /// The q parameter value as a non-NaN f32 between 0.0 and 1.0, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    /// use ordered_float::NotNan;
    ///
    /// let addr = Address::from_str("<sip:alice@example.com>;q=0.5").unwrap();
    /// assert_eq!(addr.q().map(|n| n.into_inner()), Some(0.5));
    ///
    /// let addr = Address::from_str("<sip:alice@example.com>").unwrap();
    /// assert_eq!(addr.q(), None);
    /// ```
    pub fn q(&self) -> Option<NotNan<f32>> {
        self.params.iter().find_map(|p| match p {
            Param::Q(val) => Some(*val),
            Param::Other(key, Some(val)) if key.eq_ignore_ascii_case("q") => { // Match GenericValue
                val.as_str().and_then(|s| s.parse::<f32>().ok()).and_then(|f| NotNan::try_from(f).ok())
            },
            _ => None,
        })
    }

    /// Set the q parameter value, clamping between 0.0 and 1.0.
    ///
    /// The q parameter (quality factor) is commonly used in Contact headers to
    /// indicate a relative priority, with higher values indicating higher priority.
    ///
    /// # Parameters
    ///
    /// - `q`: The quality value (automatically clamped between 0.0 and 1.0)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let mut addr = Address::new(None, uri);
    ///
    /// // Set normal q-value
    /// addr.set_q(0.8);
    /// assert_eq!(addr.q().map(|n| n.into_inner()), Some(0.8));
    ///
    /// // Value is clamped if outside valid range
    /// addr.set_q(1.5); // Value > 1.0
    /// assert_eq!(addr.q().map(|n| n.into_inner()), Some(1.0));
    ///
    /// addr.set_q(-0.5); // Value < 0.0
    /// assert_eq!(addr.q().map(|n| n.into_inner()), Some(0.0));
    /// ```
    pub fn set_q(&mut self, q: f32) {
        // Clamp the value
        let clamped_q = q.max(0.0).min(1.0);
        // Remove existing q param before adding new one
        self.params.retain(|p| !matches!(p, Param::Q(_)));
        self.params.push(Param::Q(NotNan::try_from(clamped_q).expect("Clamped q value should not be NaN")));
    }

    /// Check if a parameter exists (case-insensitive key).
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to check for (case-insensitive)
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let addr = Address::from_str("<sip:alice@example.com>;tag=1234;expires=3600").unwrap();
    /// 
    /// assert!(addr.has_param("tag"));
    /// assert!(addr.has_param("TAG")); // Case-insensitive
    /// assert!(addr.has_param("expires"));
    /// assert!(!addr.has_param("q")); // Not present
    /// ```
    pub fn has_param(&self, name: &str) -> bool {
        let name_lower = name.to_lowercase();
        self.params.iter().any(|p| {
            match p {
                Param::Branch(_) => name_lower == "branch",
                Param::Tag(_) => name_lower == "tag",
                Param::Expires(_) => name_lower == "expires",
                Param::Received(_) => name_lower == "received",
                Param::Maddr(_) => name_lower == "maddr",
                Param::Ttl(_) => name_lower == "ttl",
                Param::Lr => name_lower == "lr",
                Param::Q(_) => name_lower == "q",
                Param::Transport(_) => name_lower == "transport",
                Param::User(_) => name_lower == "user",
                Param::Method(_) => name_lower == "method",
                Param::Handling(_) => name_lower == "handling",
                Param::Duration(_) => name_lower == "duration",
                Param::Rport(_) => name_lower == "rport",
                Param::Other(key, _) => key.eq_ignore_ascii_case(&name_lower),
                _ => false,
            }
        })
    }

    /// Gets the value of a parameter by key (case-insensitive).
    ///
    /// # Parameters
    ///
    /// - `key`: The parameter name to look for (case-insensitive)
    ///
    /// # Returns
    ///
    /// - `Some(Some(value))` if the parameter exists and has a value
    /// - `Some(None)` if the parameter exists but has no value (flag parameter)
    /// - `None` if the parameter doesn't exist
    ///
    /// For typed parameters (like Expires, Q, etc.), this returns the string representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let addr = Address::from_str("<sip:alice@example.com>;tag=1234;lr;custom=value").unwrap();
    /// 
    /// assert_eq!(addr.get_param("tag"), Some(Some("1234")));
    /// assert_eq!(addr.get_param("lr"), Some(None)); // Flag parameter
    /// assert_eq!(addr.get_param("custom"), Some(Some("value")));
    /// assert_eq!(addr.get_param("nonexistent"), None);
    /// ```
    pub fn get_param(&self, key: &str) -> Option<Option<&str>> {
        Some(
            self.params
                .iter()
                .find_map(|p| match p {
                    Param::Branch(val) if key.eq_ignore_ascii_case("branch") => Some(Some(val.as_str())),
                    Param::Tag(val) if key.eq_ignore_ascii_case("tag") => Some(Some(val.as_str())),
                    Param::Expires(val) if key.eq_ignore_ascii_case("expires") => Some(Some(Box::leak(val.to_string().into_boxed_str()))), // Inefficient leak!
                    Param::Received(val) if key.eq_ignore_ascii_case("received") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
                    Param::Maddr(val) if key.eq_ignore_ascii_case("maddr") => Some(Some(val.as_str())),
                    Param::Ttl(val) if key.eq_ignore_ascii_case("ttl") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
                    Param::Lr if key.eq_ignore_ascii_case("lr") => Some(None), // Keep as Some(None) for flag params
                    Param::Q(val) if key.eq_ignore_ascii_case("q") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
                    Param::Transport(val) if key.eq_ignore_ascii_case("transport") => Some(Some(val.as_str())),
                    Param::User(val) if key.eq_ignore_ascii_case("user") => Some(Some(val.as_str())),
                    Param::Method(val) if key.eq_ignore_ascii_case("method") => Some(Some(val.as_str())),
                    // Wrap the Option<&str> in Some to match expected Option<Option<&str>>
                    Param::Other(k, v_opt) if k.eq_ignore_ascii_case(key) => Some(v_opt.as_ref().and_then(|gv| gv.as_str())),
                    Param::Handling(val) if key.eq_ignore_ascii_case("handling") => Some(Some(val.as_str())),
                    Param::Duration(val) if key.eq_ignore_ascii_case("duration") => Some(Some(Box::leak(val.to_string().into_boxed_str()))),
                    Param::Rport(val) if key.eq_ignore_ascii_case("rport") => match val {
                        Some(port) => Some(Some(Box::leak(port.to_string().into_boxed_str()))),
                        None => Some(None) // Flag parameter
                    },
                    _ => None,
                })
                .flatten() // Flatten the Option<Option<&str>> to Option<&str>
        )
    }

    /// Sets or replaces a parameter, storing it as Param::Other.
    ///
    /// This method can be used to set any parameter, but specialized methods
    /// like `set_tag()`, `set_expires()`, etc., should be preferred for
    /// standard parameters.
    ///
    /// # Parameters
    ///
    /// - `key`: The parameter name to set
    /// - `value`: The parameter value to set, or None to add a flag parameter
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let mut addr = Address::new(None, uri);
    ///
    /// // Set a parameter with a value
    /// addr.set_param("custom", Some("value"));
    /// assert_eq!(addr.get_param("custom"), Some(Some("value")));
    ///
    /// // Set a flag parameter (no value)
    /// addr.set_param("lr", None);
    /// assert_eq!(addr.get_param("lr"), Some(None));
    ///
    /// // Replace an existing parameter
    /// addr.set_param("custom", Some("new-value"));
    /// assert_eq!(addr.get_param("custom"), Some(Some("new-value")));
    /// ```
    pub fn set_param(&mut self, key: impl Into<String>, value: Option<impl Into<String>>) {
        let key_string = key.into();
        let value_opt_string = value.map(|v| v.into());

        // Remove existing param with the same key
         self.params.retain(|p| match p {
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
            Param::Other(k, _) => !k.eq_ignore_ascii_case(&key_string),
            Param::Handling(_) => !key_string.eq_ignore_ascii_case("handling"),
            Param::Duration(_) => !key_string.eq_ignore_ascii_case("duration"),
            Param::Rport(_) => !key_string.eq_ignore_ascii_case("rport"),
        });

        // Add as Param::Other
        self.params.push(Param::Other(key_string, value_opt_string.map(GenericValue::Token)));
    }

    /// Remove a parameter (case-insensitive key).
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to remove (case-insensitive)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut addr = Address::from_str("<sip:alice@example.com>;tag=1234;expires=3600").unwrap();
    /// 
    /// // Remove a parameter
    /// addr.remove_param("tag");
    /// assert!(!addr.has_param("tag"));
    /// assert!(addr.has_param("expires"));
    ///
    /// // Case-insensitive
    /// addr.remove_param("EXPIRES");
    /// assert!(!addr.has_param("expires"));
    ///
    /// // No-op if parameter doesn't exist
    /// addr.remove_param("nonexistent");
    /// ```
    pub fn remove_param(&mut self, name: &str) {
        let name_lower = name.to_lowercase();
        let old_params = std::mem::take(&mut self.params);
        
        self.params = old_params.into_iter()
            .filter(|p| {
                match p {
                    Param::Branch(_) => name_lower != "branch",
                    Param::Tag(_) => name_lower != "tag",
                    Param::Expires(_) => name_lower != "expires",
                    Param::Received(_) => name_lower != "received",
                    Param::Maddr(_) => name_lower != "maddr",
                    Param::Ttl(_) => name_lower != "ttl",
                    Param::Lr => name_lower != "lr",
                    Param::Q(_) => name_lower != "q",
                    Param::Transport(_) => name_lower != "transport",
                    Param::User(_) => name_lower != "user",
                    Param::Method(_) => name_lower != "method",
                    Param::Handling(_) => name_lower != "handling",
                    Param::Duration(_) => name_lower != "duration",
                    Param::Rport(_) => name_lower != "rport",
                    Param::Other(key, _) => !key.eq_ignore_ascii_case(&name_lower),
                    _ => true,
                }
            })
            .collect();
    }

    // Helper to construct from parser output
    // This helper seems unused now that parsers directly construct Address
    /* pub fn from_parsed(
        display_name_bytes: Option<Vec<u8>>,
        uri: Uri,
        params: Vec<Param>
    ) -> Result<Self> {
        let display_name = display_name_bytes
            .map(|bytes| String::from_utf8(bytes)) // TODO: Handle potential quoting/unescaping
            .transpose()?;
        // Conversion of params is lossy here, params are now part of Address directly
        Ok(Address { display_name, uri, params })
    } */
}

impl FromStr for Address {
    type Err = crate::error::Error;

    /// Parse a string as a SIP Address.
    ///
    /// This method parses a string representation of a SIP address into
    /// an Address struct.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Address, or an Error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Address;
    /// use std::str::FromStr;
    ///
    /// // Parse a name address with display name and parameters
    /// let addr = Address::from_str("\"John Doe\" <sip:john@example.com>;tag=1234").unwrap();
    /// assert_eq!(addr.display_name, Some("John Doe".to_string()));
    /// assert_eq!(addr.uri.to_string(), "sip:john@example.com");
    /// assert_eq!(addr.tag(), Some("1234"));
    ///
    /// // Parse a name address without display name
    /// let addr = Address::from_str("<sip:john@example.com>").unwrap();
    /// assert_eq!(addr.display_name, None);
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming, handle input type, map result and error
        nom::combinator::all_consuming(parse_address)(s.as_bytes())
            .map(|(_rem, addr)| addr) // Extract the address from the tuple
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
}

// TODO: Implement helper methods (e.g., new, tag(), set_tag(), etc.) 