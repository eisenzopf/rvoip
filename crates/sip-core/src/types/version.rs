//! # SIP Protocol Version
//!
//! This module provides an implementation of the SIP protocol version as defined in
//! [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261).
//!
//! The SIP protocol version is a required part of SIP messages, appearing in request lines
//! and status lines. Currently, the only version in use is "SIP/2.0", but the implementation
//! supports other versions for future compatibility.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create the standard SIP version (2.0)
//! let version = Version::sip_2_0();
//! assert_eq!(version.to_string(), "SIP/2.0");
//!
//! // Parse from a string
//! let version = Version::from_str("SIP/2.0").unwrap();
//! assert_eq!(version.major, 2);
//! assert_eq!(version.minor, 0);
//! ```

// Version implementation moved from root directory
// Implements SIP protocol version as defined in RFC 3261

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// SIP protocol version, as defined in RFC 3261.
///
/// The SIP protocol version identifies the protocol version being used in a SIP message.
/// It appears in the first line of both requests and responses:
///
/// - In requests: `METHOD URI SIP/2.0`
/// - In responses: `SIP/2.0 STATUS_CODE REASON_PHRASE`
///
/// Currently, "SIP/2.0" is the only version in use, but the implementation supports
/// other versions for future compatibility.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create the standard SIP version (2.0)
/// let version = Version::sip_2_0();
///
/// // Create a custom version (for future use)
/// let version = Version::new(3, 0);
/// assert_eq!(version.to_string(), "SIP/3.0");
/// ```
///
/// The current version is "SIP/2.0".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    /// Major version (currently 2)
    pub major: u8,
    /// Minor version (currently 0)
    pub minor: u8,
}

impl Version {
    /// Create a new SIP version with the given major and minor versions.
    ///
    /// This allows creating any arbitrary version, though in current SIP deployments,
    /// only version 2.0 is used.
    ///
    /// # Parameters
    ///
    /// - `major`: Major version number
    /// - `minor`: Minor version number
    ///
    /// # Returns
    ///
    /// A new `Version` instance with the specified version numbers
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Standard version
    /// let version = Version::new(2, 0);
    /// assert_eq!(version.to_string(), "SIP/2.0");
    ///
    /// // Future version (hypothetical)
    /// let version = Version::new(3, 1);
    /// assert_eq!(version.to_string(), "SIP/3.1");
    /// ```
    pub fn new(major: u8, minor: u8) -> Self {
        Version { major, minor }
    }

    /// Creates a SIP version with the standard version (2.0)
    ///
    /// This is a convenience method for creating the standard SIP version (2.0)
    /// that is currently used in all SIP deployments.
    ///
    /// # Returns
    ///
    /// A new `Version` instance with major version 2 and minor version 0
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let version = Version::sip_2_0();
    /// assert_eq!(version.major, 2);
    /// assert_eq!(version.minor, 0);
    /// assert_eq!(version.to_string(), "SIP/2.0");
    /// ```
    pub fn sip_2_0() -> Self {
        Version { major: 2, minor: 0 }
    }

    /// Returns the string representation of this version.
    ///
    /// Formats the version as "SIP/major.minor", which is the format
    /// used in SIP messages.
    ///
    /// # Returns
    ///
    /// A string in the format "SIP/major.minor"
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let version = Version::sip_2_0();
    /// assert_eq!(version.as_str(), "SIP/2.0");
    ///
    /// let version = Version::new(3, 1);
    /// assert_eq!(version.as_str(), "SIP/3.1");
    /// ```
    pub fn as_str(&self) -> String {
        format!("SIP/{}.{}", self.major, self.minor)
    }
}

impl Default for Version {
    /// Returns the default SIP version, which is SIP/2.0.
    ///
    /// # Returns
    ///
    /// The standard SIP version (2.0)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::default::Default;
    ///
    /// let version = Version::default();
    /// assert_eq!(version.major, 2);
    /// assert_eq!(version.minor, 0);
    /// assert_eq!(version, Version::sip_2_0());
    /// ```
    fn default() -> Self {
        Version::sip_2_0()
    }
}

impl fmt::Display for Version {
    /// Formats the version as a string in the format "SIP/major.minor".
    ///
    /// This is used to include the version in SIP messages.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let version = Version::sip_2_0();
    /// assert_eq!(version.to_string(), "SIP/2.0");
    ///
    /// let version = Version::new(3, 1);
    /// assert_eq!(version.to_string(), "SIP/3.1");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SIP/{}.{}", self.major, self.minor)
    }
}

impl FromStr for Version {
    type Err = Error;

    /// Parses a SIP version from a string.
    ///
    /// Currently, this only accepts "SIP/2.0" (case-insensitive). Other versions
    /// will return an error. This might be expanded in the future as new SIP versions
    /// are adopted.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse as a SIP version
    ///
    /// # Returns
    ///
    /// - `Ok(Version)`: If the string is a valid SIP version
    /// - `Err(Error::InvalidVersion)`: If the string is not a valid SIP version
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Standard version - case insensitive
    /// let version = Version::from_str("SIP/2.0").unwrap();
    /// assert_eq!(version, Version::sip_2_0());
    ///
    /// let version = Version::from_str("sip/2.0").unwrap();
    /// assert_eq!(version, Version::sip_2_0());
    ///
    /// // Invalid formats
    /// assert!(Version::from_str("2.0").is_err());
    /// assert!(Version::from_str("SIP/3.0").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Case-insensitive check for "SIP/2.0"
        if s.eq_ignore_ascii_case("SIP/2.0") {
            Ok(Version::sip_2_0())
        } else {
            // Consider adding more robust parsing if needed in the future
            Err(Error::InvalidVersion)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_from_str() {
        let v = Version::from_str("SIP/2.0").unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 0);
        
        // Case insensitive
        let v = Version::from_str("sip/2.0").unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 0);
        
        // Invalid format
        assert!(Version::from_str("2.0").is_err());
        assert!(Version::from_str("SIP/a.0").is_err());
        assert!(Version::from_str("SIP/2.b").is_err());
        assert!(Version::from_str("SIP/2").is_err());
    }

    #[test]
    fn test_version_display() {
        let v = Version::new(2, 0);
        assert_eq!(v.to_string(), "SIP/2.0");
        
        let v = Version::new(3, 1);
        assert_eq!(v.to_string(), "SIP/3.1");
    }

    #[test]
    fn test_version_default() {
        let v = Version::default();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 0);
        assert_eq!(v, Version::sip_2_0());
    }
} 