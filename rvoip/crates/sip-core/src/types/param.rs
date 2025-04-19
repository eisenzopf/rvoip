use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use crate::error::{Error, Result};

// TODO: Add more specific parameter types (like rsip NewTypes) 
// e.g., Branch(String), Tag(String), Expires(u32), etc.

/// Represents a generic URI parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Param {
    /// The `branch` parameter, typically used in Via headers.
    Branch(String),
    /// The `tag` parameter, used in From/To headers for dialog identification.
    Tag(String),
    /// The `expires` parameter, used in Contact/Expires headers.
    Expires(u32),
    /// The `received` parameter, used in Via headers to indicate the source IP.
    Received(IpAddr),
    /// The `maddr` parameter, used in Via headers.
    Maddr(String), // Keep as string for now, could be Host
    /// The `ttl` parameter, used in Via headers.
    Ttl(u8),
    /// The `lr` parameter (loose routing), a flag parameter used in Via/Route.
    Lr,
    /// The `q` parameter (quality value), used in Contact headers.
    Q(f32),
    /// Transport parameter.
    Transport(String), // Consider using a Transport enum later
    /// User parameter.
    User(String),
    /// Method parameter (rarely used in URIs).
    Method(String), // Consider using types::Method later
    /// Generic parameter represented as key-value.
    Other(String, Option<String>),
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Param::Branch(val) => write!(f, ";branch={}", val),
            Param::Tag(val) => write!(f, ";tag={}", val),
            Param::Expires(val) => write!(f, ";expires={}", val),
            Param::Received(val) => write!(f, ";received={}", val),
            Param::Maddr(val) => write!(f, ";maddr={}", val),
            Param::Ttl(val) => write!(f, ";ttl={}", val),
            Param::Lr => write!(f, ";lr"),
            Param::Q(val) => write!(f, ";q={:.1}", val), // Format q value appropriately
            Param::Transport(val) => write!(f, ";transport={}", val),
            Param::User(val) => write!(f, ";user={}", val),
            Param::Method(val) => write!(f, ";method={}", val),
            Param::Other(key, Some(val)) => write!(f, ";{}={}", key, val),
            Param::Other(key, None) => write!(f, ";{}", key),
        }
    }
}

// Note: A FromStr or TryFrom implementation will be added 
// once the parser logic in parser/uri.rs is updated. 