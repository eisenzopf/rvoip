use crate::types::Method;
use crate::parser::headers::parse_cseq;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Typed CSeq header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct CSeq {
    pub seq: u32,
    pub method: Method,
}

impl CSeq {
    /// Creates a new CSeq header.
    pub fn new(seq: u32, method: Method) -> Self {
        Self { seq, method }
    }
}

impl fmt::Display for CSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.seq, self.method) // Method already implements Display
    }
}

impl FromStr for CSeq {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_cseq(s)
    }
}

// TODO: Implement methods if needed 