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