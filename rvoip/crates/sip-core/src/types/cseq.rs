use crate::types::method::Method;
use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;

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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed_s = s.trim();
        all_consuming(parse_cseq)(trimmed_s.as_bytes())
            .map(|(_, cseq)| cseq)
            .map_err(Error::from)
    }
}

// TODO: Implement methods if needed 