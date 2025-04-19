use crate::types::Method;
use crate::parser::headers::parse_allow;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Typed Allow header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_allow(s)
    }
}

// TODO: Implement methods (e.g., allows(Method)) 