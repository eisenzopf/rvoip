use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::types::uri::Host; // Assuming Host type exists

// TODO: Add more specific parameter types (like rsip NewTypes) 
// e.g., Branch(String), Tag(String), Expires(u32), etc.

/// Represents the parsed value of a generic parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)] // Added Eq/Hash assuming Host is Eq/Hash
pub enum GenericValue {
    Token(String),
    Host(Host),
    Quoted(String),
}

// Implement Display for GenericValue for use in Param::Display
impl fmt::Display for GenericValue {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenericValue::Token(s) => write!(f, "{}", s),
            GenericValue::Host(h) => write!(f, "{}", h), // Assuming Host implements Display
            GenericValue::Quoted(s) => write!(f, "\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")), // Re-quote safely
        }
    }
}

/// Represents a generic URI parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    Q(NotNan<f32>),
    /// Transport parameter.
    Transport(String), // Consider using a Transport enum later
    /// User parameter.
    User(String),
    /// Method parameter (rarely used in URIs).
    Method(String), // Consider using types::Method later
    /// Handling parameter (added for Content-Disposition)
    Handling(String), // Added for Content-Disposition (Need Handling enum later)
    /// Duration parameter (added for Retry-After)
    Duration(u32), // Added for Retry-After
    /// Generic parameter represented as key-value.
    Other(String, Option<GenericValue>), // Changed value type
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
            Param::Q(val) => write!(f, ";q={:.1}", val.into_inner()),
            Param::Transport(val) => write!(f, ";transport={}", val),
            Param::User(val) => write!(f, ";user={}", val),
            Param::Method(val) => write!(f, ";method={}", val),
            Param::Handling(val) => write!(f, ";handling={}", val),
            Param::Duration(val) => write!(f, ";duration={}", val),
            Param::Other(key, Some(val)) => write!(f, ";{}={}", key, val), // Use GenericValue::Display
            Param::Other(key, None) => write!(f, ";{}", key),
        }
    }
}

// Note: A FromStr or TryFrom implementation will be added 
// once the parser logic in parser/uri.rs is updated. 