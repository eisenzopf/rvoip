use crate::types::method::Method;
use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::parser::headers::cseq::parse_cseq;
use serde::{Serialize, Deserialize};

/// Represents the CSeq header field (RFC 3261 Section 8.1.1.5).
/// Contains a sequence number and a method name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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