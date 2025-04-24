use crate::types::method::Method;
use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::parser::headers::cseq::{parse_cseq, full_parse_cseq};
use serde::{Serialize, Deserialize};

/// Represents the CSeq header field (RFC 3261 Section 8.1.1.5).
/// 
/// The CSeq header field serves as a way to identify and order transactions.
/// It consists of a sequence number and a method. The method name in the
/// CSeq header field MUST match the method name in the start-line, and the
/// sequence number value MUST be expressible as a 32-bit unsigned integer.
/// 
/// # Examples
/// ```
/// use crate::types::cseq::CSeq;
/// use crate::types::method::Method;
/// 
/// // Create from sequence number and Method enum
/// let cseq = CSeq::new(101, Method::Invite);
/// assert_eq!(cseq.to_string(), "101 INVITE");
/// 
/// // Create from string value
/// let cseq2: CSeq = "102 ACK".parse().unwrap();
/// assert_eq!(cseq2.sequence(), 102);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CSeq {
    pub seq: u32,
    pub method: Method,
}

impl CSeq {
    /// Creates a new CSeq header with the specified sequence number and method.
    pub fn new(seq: u32, method: Method) -> Self {
        Self { seq, method }
    }
    
    /// Creates a new CSeq header with the specified sequence number and method string.
    /// 
    /// # Errors
    /// Returns an error if the method string is not a valid SIP method.
    pub fn with_method_str(seq: u32, method_str: &str) -> Result<Self> {
        let method = Method::from_str(method_str)?;
        Ok(Self::new(seq, method))
    }
    
    /// Get the sequence number.
    pub fn sequence(&self) -> u32 {
        self.seq
    }
    
    /// Get the method.
    pub fn method(&self) -> &Method {
        &self.method
    }
    
    /// Increments the sequence number by 1 and returns a new CSeq with the same method.
    /// 
    /// # Panics
    /// Panics if the sequence number would overflow.
    pub fn increment(&self) -> Self {
        Self {
            seq: self.seq.checked_add(1).expect("CSeq sequence number overflow"),
            method: self.method.clone(),
        }
    }
    
    /// Increments the sequence number by 1 and changes the method.
    /// 
    /// # Panics
    /// Panics if the sequence number would overflow.
    pub fn increment_with_method(&self, method: Method) -> Self {
        Self {
            seq: self.seq.checked_add(1).expect("CSeq sequence number overflow"),
            method,
        }
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
    
    #[test]
    fn test_with_method_str() {
        // Standard method
        let cseq = CSeq::with_method_str(101, "INVITE").unwrap();
        assert_eq!(cseq.seq, 101);
        assert_eq!(cseq.method, Method::Invite);
        
        // Lowercase methods are treated as extensions
        let lowercase_result = CSeq::with_method_str(101, "invite").unwrap();
        assert_eq!(lowercase_result.method, Method::Extension("invite".into()),
                   "Lowercase methods are treated as extensions, not standard methods");
        
        // Custom/invalid method names are accepted as extensions
        let custom_result = CSeq::with_method_str(101, "INVALID-METHOD-NAME").unwrap();
        assert_eq!(custom_result.method, Method::Extension("INVALID-METHOD-NAME".into()),
                  "Custom method names are accepted as extensions");
        
        // Test with empty method (should fail)
        let empty_result = CSeq::with_method_str(101, "");
        assert!(empty_result.is_err(), "Empty method name should be rejected");
    }
    
    #[test]
    fn test_increment() {
        let cseq = CSeq::new(101, Method::Invite);
        let incremented = cseq.increment();
        assert_eq!(incremented.seq, 102);
        assert_eq!(incremented.method, Method::Invite);
        
        // Test method change
        let with_new_method = cseq.increment_with_method(Method::Bye);
        assert_eq!(with_new_method.seq, 102);
        assert_eq!(with_new_method.method, Method::Bye);
    }
} 