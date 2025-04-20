use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// SIP protocol version, as defined in RFC 3261.
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
    pub fn new(major: u8, minor: u8) -> Self {
        Version { major, minor }
    }

    /// Creates a SIP version with the standard version (2.0)
    pub fn sip_2_0() -> Self {
        Version { major: 2, minor: 0 }
    }

    /// Returns the string representation of this version.
    pub fn as_str(&self) -> String {
        format!("SIP/{}.{}", self.major, self.minor)
    }
}

impl Default for Version {
    fn default() -> Self {
        Version::sip_2_0()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SIP/{}.{}", self.major, self.minor)
    }
}

impl FromStr for Version {
    type Err = Error;

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