use crate::types::Method;
use crate::parser::headers::parse_cseq;
use crate::error::Result;
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
        use crate::parser::headers::cseq::parse_cseq;

        let trimmed_s = s.trim();
        match all_consuming(parse_cseq)(trimmed_s.as_bytes()) {
            Ok((_, value)) => {
                let method = Method::from_str(std::str::from_utf8(&value.method)?)?;
                Ok(CSeq { seq: value.seq, method })
            },
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse CSeq header: {:?}", e), 
                source: None 
            })
        }
    }
}

// TODO: Implement methods if needed 