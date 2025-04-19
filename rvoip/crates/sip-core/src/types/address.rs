use crate::uri::Uri;
use crate::types::Param;
use crate::parser::headers::parse_address;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Represents a SIP Name Address (Display Name <URI>; params).
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct Address {
    pub display_name: Option<String>,
    pub uri: Uri,
    pub params: Vec<Param>,
}

// Function to check if quoting is needed for display-name
// Based on RFC 3261 relaxed LWS rules and token definition.
// Quotes are needed if it's not a token or contains specific characters like ", \, or spaces.
fn needs_quoting(display_name: &str) -> bool {
    if display_name.is_empty() {
        return true; // Empty string should be quoted ""
    }
    // Check for characters that *require* quoting or are not part of a token
    display_name.chars().any(|c| {
        !c.is_alphanumeric() && !matches!(c, '-' | '.' | '!' | '%' | '*' | '_' | '+' | '`' | '\\'' | '~')
    }) || display_name.contains('"') || display_name.contains('\\')
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.display_name {
            if name.is_empty() || needs_quoting(name) {
                // Escape quotes within the display name itself
                write!(f, "\"{}\"", name.replace("\"", "\"\"") )?;
            } else {
                write!(f, "{} ", name)?;
            }
        }
        // Always write URI in angle brackets for name-addr format
        write!(f, "<{}>", self.uri)?;

        // Write parameters
        for param in &self.params {
            write!(f, "{}", param)?; // Uses Param's Display impl
        }

        Ok(())
    }
}

impl Address {
    // Example helper method: get tag parameter
    pub fn tag(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Tag(tag_val) => Some(tag_val.as_str()),
            _ => None,
        })
    }

    /// Sets or replaces the tag parameter.
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        // Remove existing tag parameter(s)
        self.params.retain(|p| !matches!(p, Param::Tag(_)));
        // Add the new one
        self.params.push(Param::Tag(tag.into()));
    }
    
    // TODO: Add other helpers like expires, set_expires etc.
}

impl FromStr for Address {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_address(s)
    }
}

// TODO: Implement helper methods (e.g., new, tag(), set_tag(), etc.) 