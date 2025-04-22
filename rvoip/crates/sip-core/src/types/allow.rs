use crate::types::Method;
use crate::parser::headers::parse_allow;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};

/// Represents the Allow header field (RFC 3261 Section 20.5).
/// Lists the SIP methods supported by the User Agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allow(pub Vec<Method>);

impl Allow {
    /// Creates an empty Allow header.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Allow header with specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Allow header from an iterator of methods.
    pub fn from_methods<I>(methods: I) -> Self
    where
        I: IntoIterator<Item = Method>
    {
        Self(methods.into_iter().collect())
    }

    /// Checks if a specific method is allowed.
    pub fn allows(&self, method: &Method) -> bool {
        self.0.contains(method)
    }

    /// Adds a method if not already present.
    pub fn add_method(&mut self, method: Method) {
        if !self.allows(&method) {
            self.0.push(method);
        }
    }
}

impl fmt::Display for Allow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", method_strings.join(", "))
    }
}

impl FromStr for Allow {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::allow::parse_allow;

        let (_, methods_bytes) = all_consuming(parse_allow)(s.as_bytes()).map_err(Error::from)?;
        Ok(methods_bytes)
    }
}

// TODO: Implement methods (e.g., allows(Method)) 

// Implement IntoIterator for Allow
impl IntoIterator for Allow {
    type Item = Method;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

// Implement IntoIterator for &Allow
impl<'a> IntoIterator for &'a Allow {
    type Item = &'a Method;
    type IntoIter = std::slice::Iter<'a, Method>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

// Implement IntoIterator for &mut Allow
impl<'a> IntoIterator for &'a mut Allow {
    type Item = &'a mut Method;
    type IntoIter = std::slice::IterMut<'a, Method>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
} 