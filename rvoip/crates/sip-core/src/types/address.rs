use crate::types::uri::Uri;
use crate::types::param::{Param, GenericValue};
use crate::error::{Error, Result};
use crate::parser::parse_address;
use serde::{Serialize, Deserialize};
use std::fmt;
use std::str::FromStr;
use ordered_float::NotNan;

/// Represents a SIP Name Address (Display Name <URI>; params).
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct Address {
    pub display_name: Option<String>,
    pub uri: Uri,
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

// Function to check if quoting is needed for display-name
// Based on RFC 3261 relaxed LWS rules and token definition.
// Quotes are needed if it's not a token or contains specific characters like ", \, or spaces.
fn needs_quoting(display_name: &str) -> bool {
    if display_name.is_empty() {
        return false; // Empty string should NOT be quoted
    }
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
            write!(f, "{}", param)?;
        }

        Ok(())
    }
}

impl Address {
    /// Creates a new Address.
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
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        // Remove existing tag parameter(s)
        self.params.retain(|p| !matches!(p, Param::Tag(_)));
        // Add the new one
        self.params.push(Param::Tag(tag.into()));
    }
    
    /// Gets the tag parameter value.
    pub fn tag(&self) -> Option<&str> {
        self.params.iter().find_map(|p| match p {
            Param::Tag(tag_val) => Some(tag_val.as_str()),
            _ => None,
        })
    }
    
    /// Gets the expires parameter value, if present and valid.
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
    pub fn set_q(&mut self, q: f32) {
        // Clamp the value
        let clamped_q = q.max(0.0).min(1.0);
        // Remove existing q param before adding new one
        self.params.retain(|p| !matches!(p, Param::Q(_)));
        self.params.push(Param::Q(NotNan::try_from(clamped_q).expect("Clamped q value should not be NaN")));
    }

    /// Checks if a parameter with the given key exists (case-insensitive).
    pub fn has_param(&self, key: &str) -> bool {
        self.params.iter().any(|p| match p {
            Param::Branch(_) => key.eq_ignore_ascii_case("branch"),
            Param::Tag(_) => key.eq_ignore_ascii_case("tag"),
            Param::Expires(_) => key.eq_ignore_ascii_case("expires"),
            Param::Received(_) => key.eq_ignore_ascii_case("received"),
            Param::Maddr(_) => key.eq_ignore_ascii_case("maddr"),
            Param::Ttl(_) => key.eq_ignore_ascii_case("ttl"),
            Param::Lr => key.eq_ignore_ascii_case("lr"),
            Param::Q(_) => key.eq_ignore_ascii_case("q"),
            Param::Transport(_) => key.eq_ignore_ascii_case("transport"),
            Param::User(_) => key.eq_ignore_ascii_case("user"),
            Param::Method(_) => key.eq_ignore_ascii_case("method"),
            Param::Other(k, _) => k.eq_ignore_ascii_case(key),
            Param::Handling(_) => key.eq_ignore_ascii_case("handling"),
            Param::Duration(_) => key.eq_ignore_ascii_case("duration"),
        })
    }

    /// Gets the value of a parameter by key (case-insensitive).
    /// Returns Some(Some(value)) for key-value pairs, Some(None) for flags, None if not found.
    /// Note: For typed params like Expires, this returns the string representation.
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
                    _ => None,
                })
                .flatten() // Flatten the Option<Option<&str>> to Option<&str>
        )
    }

    /// Sets or replaces a parameter, storing it as Param::Other.
    /// Removes any existing parameter (typed or Other) with the same key (case-insensitive).
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
        });

        // Add as Param::Other
        self.params.push(Param::Other(key_string, value_opt_string.map(GenericValue::Token)));
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

    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming, handle input type, map result and error
        nom::combinator::all_consuming(parse_address)(s.as_bytes())
            .map(|(_rem, addr)| addr) // Extract the address from the tuple
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
}

// TODO: Implement helper methods (e.g., new, tag(), set_tag(), etc.) 