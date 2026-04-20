//! # SIP RAck (Response Acknowledgement) Header
//!
//! Implementation of the SIP `RAck` header as defined in
//! [RFC 3262 §7.2](https://datatracker.ietf.org/doc/html/rfc3262#section-7.2).
//!
//! `RAck` appears in a PRACK request sent by a UAC to acknowledge a reliable
//! provisional response from a UAS. Its value carries three components:
//!
//! ```text
//! RAck: <response-num> <cseq-num> <method>
//! RAck: 776656 1 INVITE
//! ```
//!
//! where:
//! - **response-num** is the `RSeq` value from the reliable provisional being
//!   acknowledged (RFC 3262 §7.1).
//! - **cseq-num** and **method** come from the `CSeq` of the original request
//!   that triggered the reliable provisional (always INVITE in practice since
//!   only INVITE can generate reliable provisionals per §3).
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::types::RAck;
//! use std::str::FromStr;
//!
//! // Build: acknowledge RSeq=1 for CSeq=101 INVITE
//! let rack = RAck::new(1, 101, Method::Invite);
//! assert_eq!(rack.to_string(), "1 101 INVITE");
//!
//! // Parse from a header value string
//! let parsed = RAck::from_str("42 7 INVITE").unwrap();
//! assert_eq!(parsed.rseq, 42);
//! assert_eq!(parsed.cseq, 7);
//! assert_eq!(parsed.method, Method::Invite);
//! ```

use crate::error::{Error, Result};
use crate::types::method::Method;
use crate::types::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// `RAck` header field (RFC 3262 §7.2).
///
/// Carried in PRACK requests to correlate with a specific reliable provisional
/// response. All three fields are required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RAck {
    /// Response sequence number being acknowledged (matches the `RSeq` of
    /// the reliable provisional response).
    pub rseq: u32,
    /// CSeq number of the original request (typically the INVITE).
    pub cseq: u32,
    /// Method of the original request.
    pub method: Method,
}

impl RAck {
    /// Build a new `RAck` value.
    pub fn new(rseq: u32, cseq: u32, method: Method) -> Self {
        Self { rseq, cseq, method }
    }
}

impl fmt::Display for RAck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.rseq, self.cseq, self.method)
    }
}

impl FromStr for RAck {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // RFC 3262 §7.2 ABNF: RAck = "RAck" HCOLON response-num LWS CSeq-num LWS Method
        // Here we're parsing just the header value (no name), so split on
        // whitespace and expect exactly three components.
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(Error::ParseError(format!(
                "RAck expects 3 whitespace-separated fields (rseq cseq method), got {}: {:?}",
                parts.len(),
                s
            )));
        }
        let rseq = parts[0].parse::<u32>().map_err(|_| {
            Error::ParseError(format!("Invalid RAck rseq number: {:?}", parts[0]))
        })?;
        let cseq = parts[1].parse::<u32>().map_err(|_| {
            Error::ParseError(format!("Invalid RAck cseq number: {:?}", parts[1]))
        })?;
        let method = Method::from_str(parts[2]).map_err(|_| {
            Error::ParseError(format!("Invalid RAck method: {:?}", parts[2]))
        })?;
        Ok(RAck { rseq, cseq, method })
    }
}

impl TypedHeaderTrait for RAck {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::RAck
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Expected {} header, got {}",
                Self::header_name(),
                header.name
            )));
        }
        match &header.value {
            HeaderValue::Raw(bytes) => {
                let s = std::str::from_utf8(bytes).map_err(|_| {
                    Error::InvalidHeader(format!("Invalid UTF-8 in {} header", Self::header_name()))
                })?;
                RAck::from_str(s)
            }
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected HeaderValue variant for {}",
                Self::header_name()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_display() {
        let r = RAck::new(42, 7, Method::Invite);
        assert_eq!(r.rseq, 42);
        assert_eq!(r.cseq, 7);
        assert_eq!(r.method, Method::Invite);
        assert_eq!(r.to_string(), "42 7 INVITE");
    }

    #[test]
    fn parse_basic() {
        let r: RAck = "1 101 INVITE".parse().unwrap();
        assert_eq!(r.rseq, 1);
        assert_eq!(r.cseq, 101);
        assert_eq!(r.method, Method::Invite);
    }

    #[test]
    fn parse_tolerates_surrounding_whitespace() {
        let r: RAck = "  5 99 INVITE  ".parse().unwrap();
        assert_eq!(r.rseq, 5);
        assert_eq!(r.cseq, 99);
    }

    #[test]
    fn parse_rejects_wrong_field_count() {
        assert!("1 2".parse::<RAck>().is_err());
        assert!("1 2 INVITE extra".parse::<RAck>().is_err());
    }

    #[test]
    fn parse_rejects_non_numeric() {
        assert!("abc 2 INVITE".parse::<RAck>().is_err());
        assert!("1 xyz INVITE".parse::<RAck>().is_err());
    }

    #[test]
    fn roundtrip_via_header() {
        let orig = RAck::new(7, 101, Method::Invite);
        let header = orig.to_header();
        assert_eq!(header.name, HeaderName::RAck);
        let round = RAck::from_header(&header).unwrap();
        assert_eq!(orig, round);
    }

    #[test]
    fn from_header_rejects_wrong_name() {
        let other = Header::new(
            HeaderName::CSeq,
            HeaderValue::Raw(b"1 101 INVITE".to_vec()),
        );
        assert!(RAck::from_header(&other).is_err());
    }
}
