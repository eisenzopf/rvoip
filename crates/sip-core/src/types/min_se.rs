//! # Min-SE SIP Header
//!
//! This module defines the `Min-SE` header, as specified in [RFC 4028](https://datatracker.ietf.org/doc/html/rfc4028#section-3).
//!
//! The `Min-SE` header field is used in SIP (Session Initiation Protocol) to indicate the minimum
//! session expiration interval supported by a User Agent Client (UAC) or User Agent Server (UAS).
//! It plays a crucial role in the session timer mechanism, ensuring that sessions do not persist
//! indefinitely if one of the parties becomes unresponsive.
//!
//! ## Purpose
//!
//! When a UAC sends an INVITE request that includes a `Session-Expires` header, it may also
//! include a `Min-SE` header. This `Min-SE` value indicates the shortest session expiration
//! time the UAC is willing to accept. If the UAS receiving the request cannot honor a session
//! duration at least as long as the `Min-SE` value (or its own configured minimum, whichever is
//! higher), it must reject the request with a `422 Session Interval Too Small` response.
//! This response should include a `Min-SE` header field indicating the minimum interval it can support.
//!
//! The `Min-SE` header is also included in `422` responses by a UAS or proxy when the
//! `Session-Expires` interval proposed by the UAC is too short.
//!
//! ## Structure
//!
//! The `Min-SE` header field contains a single numeric value representing delta-seconds (an
//! integer number of seconds).
//!
//! ABNF from RFC 4028:
//! ```abnf
//! Min-SE  =  "Min-SE" HCOLON delta-seconds *(SEMI generic-param)
//! delta-seconds = 1*DIGIT
//! ```
//! While the ABNF allows for generic parameters, they are not commonly used with `Min-SE`.
//! This implementation focuses on the `delta-seconds` value. The default minimum value for
//! `Min-SE` is 90 seconds (RFC 4028, Section 7.1).
//!
//! ## Examples
//!
//! ```text
//! Min-SE: 120
//! ```
//! This indicates a minimum session timer of 120 seconds.
//!
//! In a `422` response:
//! ```text
//! SIP/2.0 422 Session Interval Too Small
//! Min-SE: 90
//! ```
//! This informs the UAC that the UAS requires a session timer of at least 90 seconds.

use crate::{
    Error, Result,
    parser::headers::parse_min_se_value,
    types::headers::{Header, HeaderName, HeaderValue, TypedHeaderTrait},
};
use std::{convert::TryFrom, fmt, str::FromStr};

/// Represents the `Min-SE` header value.
///
/// The primary component is the `delta_seconds`, indicating the minimum
/// acceptable session expiration interval in seconds.
///
/// See [RFC 4028, Section 3](https://datatracker.ietf.org/doc/html/rfc4028#section-3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MinSE {
    /// The minimum session expiration interval in seconds.
    pub delta_seconds: u32,
    // While ABNF allows generic-param, they are not standard for Min-SE.
    // If needed in the future, a `Params` field could be added here.
}

impl MinSE {
    /// Creates a new `MinSE` header value.
    ///
    /// # Arguments
    /// * `delta_seconds` - The minimum session interval in seconds.
    pub fn new(delta_seconds: u32) -> Self {
        MinSE { delta_seconds }
    }
}

impl TypedHeaderTrait for MinSE {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::MinSE
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Expected header '{}' but got '{}'",
                Self::header_name().as_str(),
                header.name.as_str()
            )));
        }

        match &header.value {
            HeaderValue::Raw(raw_value) => {
                let text_value = std::str::from_utf8(raw_value)
                    .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in MinSE raw value: {}", e)))?;
                MinSE::from_str(text_value.trim())
            }
            hv => Err(Error::Parser(format!(
                "Cannot parse Min-SE from HeaderValue variant: {:?}. Expected Raw for FromStr.", hv
            ))),
        }
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(self.to_string()))
    }
}

impl fmt::Display for MinSE {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.delta_seconds)
        // If generic parameters were supported, they would be appended here.
        // e.g., ";param=value"
    }
}

impl TryFrom<HeaderValue> for MinSE {
    type Error = Error;

    fn try_from(value: HeaderValue) -> Result<Self> {
        match value {
            HeaderValue::Raw(raw) => {
                let text = std::str::from_utf8(&raw)
                    .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in MinSE raw value for TryFrom: {}", e)))?;
                MinSE::from_str(text.trim())
            }
            hv => Err(Error::Parser(format!(
                "Cannot convert HeaderValue variant {:?} to MinSE. Expected Raw for FromStr.", hv
            ))),
        }
    }
}

impl std::str::FromStr for MinSE {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match parse_min_se_value::<nom::error::Error<&[u8]>>(s.as_bytes()) {
            Ok((remaining_input, min_se_val)) => {
                if !remaining_input.is_empty() {
                    Err(Error::Parser(format!(
                        "Unexpected trailing characters after Min-SE value: '{}'",
                        String::from_utf8_lossy(remaining_input)
                    )))
                } else {
                    Ok(min_se_val)
                }
            }
            Err(nom_err) => {
                Err(Error::Parser(format!(
                    "Failed to parse Min-SE value string '{}': {}",
                    s, nom_err.to_string()
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::headers::{Header, HeaderName, HeaderValue};
    use std::str::FromStr;

    #[test]
    fn test_min_se_new() {
        let min_se = MinSE::new(120);
        assert_eq!(min_se.delta_seconds, 120);
    }

    #[test]
    fn test_min_se_display() {
        let min_se = MinSE::new(90);
        assert_eq!(min_se.to_string(), "90");

        let min_se_large = MinSE::new(3600);
        assert_eq!(min_se_large.to_string(), "3600");
    }

    #[test]
    fn test_min_se_from_str_valid() {
        assert_eq!(MinSE::from_str("90").unwrap(), MinSE::new(90));
        assert_eq!(MinSE::from_str("1800").unwrap(), MinSE::new(1800));
    }

    #[test]
    fn test_min_se_from_str_invalid() {
        // Test cases that should fail FromStr parsing
        assert!(MinSE::from_str("abc").is_err(), "'abc' should be an error");
        assert!(MinSE::from_str("").is_err(), "Empty string should be an error");
        assert!(MinSE::from_str("90abc").is_err(), "'90abc' (trailing non-digits) should be an error");
        assert!(MinSE::from_str("90;").is_err(), "'90;' (trailing semicolon) should be an error, params not supported by current FromStr");
        assert!(MinSE::from_str("90;id=1").is_err(), "'90;id=1' (with param) should be an error, params not supported by current FromStr");
        // The parser for MinSE (parse_min_se_value) uses terminated(u32, tuple((multispace0, eof))).
        // This means it consumes optional whitespace before EOF.
        // Therefore, FromStr which calls this parser will also succeed for "90 ".
        assert!(MinSE::from_str("90 ").is_ok(), "'90 ' (with trailing space) should be OK due to parser consuming OWS before EOF");
        assert!(MinSE::from_str(" 90").is_err(), "' 90' (with leading space) should be an ERROR because FromStr does not trim before byte parsing");
    }
    
    #[test]
    fn test_min_se_from_str_with_params_not_supported_yet() {
        let result = MinSE::from_str("120;refresher=uac");
        assert!(result.is_err());
        if let Err(Error::Parser(msg)) = result {
            assert!(msg.contains("Unexpected trailing characters") || msg.contains("Failed to parse Min-SE value string"));
        } else {
            panic!("Expected a Parser error for Min-SE with parameters, got {:?}", result);
        }
    }


    #[test]
    fn test_typed_header_trait_for_min_se() {
        assert_eq!(MinSE::header_name(), HeaderName::MinSE);

        let min_se = MinSE::new(120);
        let header = min_se.to_header();
        assert_eq!(header.name, HeaderName::MinSE);
        assert_eq!(header.value.to_string(), "120");

        let parsed_min_se = MinSE::from_header(&header).unwrap();
        assert_eq!(parsed_min_se, min_se);
    }

    #[test]
    fn test_typed_header_trait_parse_error_wrong_name() {
        let header = Header::new(HeaderName::Expires, HeaderValue::text("90".to_string()));
        let result = MinSE::from_header(&header);
        assert!(result.is_err());
        if let Err(Error::InvalidHeader(msg)) = result {
            assert!(msg.contains("Expected header 'Min-SE' but got 'Expires'"));
        } else {
            panic!("Expected InvalidHeader error, got {:?}", result);
        }
    }

    #[test]
    fn test_typed_header_trait_parse_error_invalid_value() {
        let header = Header::new(HeaderName::MinSE, HeaderValue::text("invalid".to_string()));
        let result = MinSE::from_header(&header);
        assert!(result.is_err());
         if let Err(Error::Parser(msg)) = result {
            assert!(msg.contains("Failed to parse Min-SE value string 'invalid'"));
        } else {
            panic!("Expected Parser error, got {:?}", result);
        }
    }
    
    #[test]
    fn test_typed_header_trait_parse_from_raw_valid() {
        let raw_value = b"180".to_vec();
        let header = Header::new(HeaderName::MinSE, HeaderValue::Raw(raw_value));
        let parsed_min_se = MinSE::from_header(&header).unwrap();
        assert_eq!(parsed_min_se, MinSE::new(180));
    }

    #[test]
    fn test_typed_header_trait_parse_from_raw_invalid() {
        let raw_value = b"not-a-number".to_vec();
        let header = Header::new(HeaderName::MinSE, HeaderValue::Raw(raw_value));
        let result = MinSE::from_header(&header);
        assert!(result.is_err());
        if let Err(Error::Parser(msg)) = result {
            assert!(msg.contains("Failed to parse Min-SE value string 'not-a-number'"), "Actual message: {}", msg);
        } else {
            panic!("Expected Parser error for raw bytes, got {:?}", result);
        }
    }

    #[test]
    fn test_typed_header_trait_parse_from_raw_trailing_data() {
        let raw_value = b"90rubbish".to_vec();
        let header = Header::new(HeaderName::MinSE, HeaderValue::Raw(raw_value));
        let result = MinSE::from_header(&header);
        assert!(result.is_err());
        if let Err(Error::Parser(msg)) = result {
            assert!(msg.contains("Failed to parse Min-SE value string '90rubbish'"), "Actual message: {}", msg);
        } else {
            panic!("Expected Parser error for raw bytes with trailing data, got {:?}", result);
        }
    }

    #[test]
    fn test_try_from_header_value_text() {
        let header_value = HeaderValue::text("240".to_string());
        let min_se = MinSE::try_from(header_value).unwrap();
        assert_eq!(min_se, MinSE::new(240));
    }

    #[test]
    fn test_try_from_header_value_raw() {
        let header_value = HeaderValue::Raw(b"300".to_vec());
        let min_se = MinSE::try_from(header_value).unwrap();
        assert_eq!(min_se, MinSE::new(300));
    }

    #[test]
    fn test_try_from_header_value_invalid_type() {
        let header_value = HeaderValue::CSeq(crate::types::cseq::CSeq::new(1, crate::types::Method::Invite));
        let result = MinSE::try_from(header_value);
        assert!(result.is_err());
         match result {
            Err(Error::Parser(msg)) => {
                assert!(msg.contains("Cannot convert HeaderValue variant"));
            }
            _ => panic!("Expected Parser error for invalid HeaderValue conversion, got {:?}", result),
        }
    }
} 