use crate::types::method::Method;
use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::parser::headers::cseq::{parse_cseq, full_parse_cseq};
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
    
    /// Get the sequence number
    pub fn sequence(&self) -> u32 {
        self.seq
    }
    
    /// Get the method
    pub fn method(&self) -> &Method {
        &self.method
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
        
        // Try parsing as a full header first (with "CSeq:" prefix)
        let full_result = all_consuming(full_parse_cseq)(trimmed_s.as_bytes());
        if let Ok((_, cseq)) = full_result {
            return Ok(cseq);
        }
        
        // If that fails, try parsing just the value part
        all_consuming(parse_cseq)(trimmed_s.as_bytes())
            .map(|(_, cseq)| cseq)
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_from_str() {
        // Test parsing with just value
        let value = "101 INVITE";
        let cseq: CSeq = value.parse().unwrap();
        assert_eq!(cseq.seq, 101);
        assert_eq!(cseq.method, Method::Invite);
        
        // Test parsing with full header
        let header = "CSeq: 202 ACK";
        let cseq2: CSeq = header.parse().unwrap();
        assert_eq!(cseq2.seq, 202);
        assert_eq!(cseq2.method, Method::Ack);
        
        // Test with lowercase header name
        let header_lower = "cseq: 303 BYE";
        let cseq3: CSeq = header_lower.parse().unwrap();
        assert_eq!(cseq3.seq, 303);
        assert_eq!(cseq3.method, Method::Bye);
    }
    
    #[test]
    fn test_display() {
        let cseq = CSeq::new(101, Method::Invite);
        assert_eq!(cseq.to_string(), "101 INVITE");
    }
}

// TODO: Implement methods if needed 