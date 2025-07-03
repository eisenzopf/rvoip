//! # SIP Event Header
//!
//! This module provides an implementation of the SIP Event header as defined in
//! [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665), which obsoletes RFC 3265.
//!
//! The Event header field is used to indicate a subscription to or notification of a
//! resource event. It is a crucial part of the SIP event notification framework,
//! allowing UAs to subscribe to changes in state (e.g., presence status, message waiting)
//! and receive notifications about these events.
//!
//! ## Structure
//!
//! The Event header typically contains an event type (which can be an event package
//! like "presence" or "message-summary", or a simple token) and an optional 'id'
//! parameter. Other generic parameters may also be present.
//!
//! Event Type:
//! - Can be a token (e.g., `dialog`)
//! - Or a package name enclosed in angle brackets (e.g., `<conference>`)
//!
//! Parameters:
//! - `id`: An optional parameter used to match subscriptions with notifications.
//! - Other generic parameters: Key-value pairs or flags.
//!
//! ## Format
//!
//! ```text
//! Event: presence;id=q876098
//! Event: <conference>;id=unique-call-id;param1=value1
//! Event: dialog;foo;bar=baz
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::event::{Event, EventType, ParamValue};
//! use rvoip_sip_core::types::header::TypedHeaderTrait; // For from_header/to_header if needed for testing
//! use rvoip_sip_core::types::Header; // For from_header/to_header if needed for testing
//! use std::str::FromStr;
//!
//! // Create an Event header for presence
//! let presence_event = Event::new(EventType::Token("presence".to_string()))
//!     .with_id("pres-123")
//!     .with_param("status", Some("online"));
//! assert_eq!(presence_event.to_string(), "presence;id=pres-123;status=online");
//!
//! // Create an Event header for a conference package
//! let conf_event = Event::new(EventType::Package("conference-info".to_string()))
//!     .with_id("conf-xyz")
//!     .with_param("version", Some("2"))
//!     .with_param("state", Some("full"));
//! assert_eq!(conf_event.to_string(), "<conference-info>;id=conf-xyz;state=full;version=2"); // Params sorted
//!
//! // Parse from a string (via TypedHeaderTrait if implemented, or direct for value)
//! // For direct value parsing, you'd typically use the parser in `parser::headers::event`
//! // This example assumes direct parsing of the value part for simplicity here.
//! // let parsed_event_val = "dialog;id=abc;custom-flag";
//! // // let event = Event::from_str(parsed_event_val) ... (if FromStr is on Event value)
//! ```

use std::fmt;
use std::str::FromStr;
use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};

use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::{Error, Result}; // Assuming Result is crate::error::Result

/// Represents the value of a generic parameter in an Event header.
/// It can either have a value (String) or be a valueless flag.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::event::ParamValue;
///
/// let p_value = ParamValue::Value("some_data".to_string());
/// let p_flag = ParamValue::None;
///
/// assert_eq!(p_value.to_string(), "some_data"); // If Display is implemented, or for debug
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParamValue {
    Value(String),
    None, // Represents a valueless parameter (a flag)
}

impl fmt::Display for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamValue::Value(val) => write!(f, "{}", val),
            ParamValue::None => write!(f, ""),
        }
    }
}

/// Type alias for a collection of generic parameters in an Event header.
/// Keys are parameter names (String) and values are `ParamValue`.
///
/// Uses `BTreeMap` to ensure parameters are stored and displayed in a canonical,
/// sorted order by key, as is common practice for SIP headers.
pub type Params = BTreeMap<String, ParamValue>;

/// Represents the type of event in an Event header (RFC 6665).
/// It can be an event package enclosed in angle brackets or a simple token.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::event::EventType;
///
/// let pkg_type = EventType::Package("presence".to_string());
/// let tok_type = EventType::Token("dialog".to_string());
///
/// assert_eq!(pkg_type.to_string(), "<presence>");
/// assert_eq!(tok_type.to_string(), "dialog");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// An event package, e.g., "`<presence>`"
    Package(String),
    /// A token representing an event type, e.g., "`dialog`"
    Token(String),
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventType::Package(pkg) => write!(f, "<{}>", pkg),
            EventType::Token(tok) => write!(f, "{}", tok),
        }
    }
}

/// Structure for the SIP Event header (RFC 6665).
///
/// Encapsulates an event type, an optional 'id' parameter, and other generic parameters.
/// The 'id' parameter is special and has a dedicated field, while other parameters
/// are stored in a `BTreeMap`.
///
/// See [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665) for more details.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::event::{Event, EventType, ParamValue};
///
/// // Basic construction
/// let event = Event::new(EventType::Token("presence".to_string()));
/// assert_eq!(event.to_string(), "presence");
///
/// // With ID and parameters
/// let event_full = Event::new(EventType::Package("conference".to_string()))
///     .with_id("conf-123")
///     .with_param("version", Some("3"))
///     .with_param("notify-only", None::<String>); // A flag parameter
/// assert_eq!(event_full.to_string(), "<conference>;id=conf-123;notify-only;version=3");
///
/// // Accessing fields
/// assert_eq!(event_full.event_type, EventType::Package("conference".to_string()));
/// assert_eq!(event_full.id, Some("conf-123".to_string()));
/// assert_eq!(event_full.params.get("version"), Some(&ParamValue::Value("3".to_string())));
/// assert_eq!(event_full.params.get("notify-only"), Some(&ParamValue::None));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Event {
    /// The type of event or event package.
    pub event_type: EventType,
    /// An optional 'id' parameter, used to match event subscriptions with notifications.
    /// The ABNF specifies `token` for the id value.
    pub id: Option<String>,
    /// Other generic parameters associated with the event.
    /// Stored in a `BTreeMap` for canonical ordering.
    pub params: Params,
}

impl Event {
    /// Creates a new `Event` header with the given event type.
    /// Initializes with no 'id' and no generic parameters.
    ///
    /// # Parameters
    /// - `event_type`: The [`EventType`] for this event.
    pub fn new(event_type: EventType) -> Self {
        Event {
            event_type,
            id: None,
            params: Params::new(),
        }
    }

    /// Sets or replaces the 'id' parameter for the `Event` header.
    ///
    /// # Parameters
    /// - `id`: The value for the 'id' parameter. Accepts any type that can be converted into a `String`.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Adds or updates a generic parameter in the `Event` header.
    /// If `value` is `None`, the parameter is added as a flag (e.g., ";param_name").
    /// If `value` is `Some`, it's added as "key=value" (e.g., ";param_name=param_value").
    ///
    /// # Parameters
    /// - `key`: The name of the parameter. Accepts any type that can be converted into a `String`.
    /// - `value`: An `Option` for the parameter's value. Accepts `Option<impl Into<String>>`.
    ///            If `None`, it's a flag. If `Some`, its content is the value.
    pub fn with_param(mut self, key: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        let param_val = match value.map(Into::into) {
            Some(v_str) => ParamValue::Value(v_str),
            None => ParamValue::None,
        };
        self.params.insert(key.into(), param_val);
        self
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.event_type)?;
        if let Some(id_val) = &self.id {
            // 'id' parameter name is conventionally lowercase.
            write!(f, ";id={}", id_val)?;
        }
        // BTreeMap iterates in sorted order of keys, ensuring canonical output for params.
        for (key, param_value) in &self.params {
            write!(f, ";{}", key)?;
            match param_value {
                ParamValue::Value(val_str) => write!(f, "={}", val_str)?,
                ParamValue::None => {} // Valueless params are just ";key"
            }
        }
        Ok(())
    }
}

// Implement FromStr for the value part of the Event header
// This allows parsing "presence;id=123" into an Event struct.
impl FromStr for Event {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // This will delegate to the nom parser for the header *value*
        // Requires the parser function to be accessible and to handle the whole value string.
        // crate::parser::headers::event::parse_event_header_value is the target.
        match crate::parser::headers::event::parse_event_header_value(s.as_bytes()) {
            Ok((_rem, event)) => { // _rem should be empty if parse_event_header_value used eof correctly
                // The `parse_event_header_value` already ensures full consumption via `terminated(..., eof)`.
                // So, if Ok, it means the whole string was validly parsed.
                Ok(event)
            }
            Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
                // Extracting the error code and the input slice that caused the error.
                // The `e.input` is the remainder of the input *at the point of error*.
                // The `e.code` is the nom error kind.
                Err(Error::Parser(format!(
                    "Failed to parse Event value string \'{}\': Parser error at \'{}\' (code: {:?})",
                    s,
                    String::from_utf8_lossy(e.input),
                    e.code
                )))
            }
            Err(nom::Err::Incomplete(needed)) => Err(Error::Parser(format!(
                "Failed to parse Event value string \'{}\': Incomplete input, needed: {:?}",
                s,
                needed
            ))),
        }
    }
}

impl TypedHeaderTrait for Event {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Event
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(self.to_string()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Mismatched header for Event: expected {}, got {}", 
                Self::header_name(), header.name
            )));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                // Use the FromStr implementation for Event which calls the parser
                // This requires `bytes` to be valid UTF-8 for `from_str`.
                let value_str = std::str::from_utf8(bytes).map_err(|e| 
                    Error::Parser(format!("Invalid UTF-8 in Event header value: {}", e))
                )?;
                Event::from_str(value_str) // This now calls the FromStr for Event above
            }
            // No HeaderValue::Event variant expected as per previous discussion
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected header value type for {}: expected Raw, got {:?}",
                Self::header_name(), header.value // Using Debug for header.value
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
    use crate::Error as SipError; 
    use std::str::FromStr;

    #[test]
    fn test_event_type_display() {
        assert_eq!(EventType::Package("presence".to_string()).to_string(), "<presence>");
        assert_eq!(EventType::Token("dialog".to_string()).to_string(), "dialog");
    }

    #[test]
    fn test_event_struct_creation_and_display() {
        let event1 = Event::new(EventType::Token("presence".to_string()))
            .with_id("123".to_string())
            .with_param("foo", Some("bar".to_string()))
            .with_param("flag", None::<String>);
        // Params are sorted by BTreeMap: flag, foo
        assert_eq!(event1.to_string(), "presence;id=123;flag;foo=bar");

        let event2 = Event::new(EventType::Package("conference-package".to_string()));
        assert_eq!(event2.to_string(), "<conference-package>");

        let event3 = Event::new(EventType::Token("custom".to_string()))
            .with_param("b", Some("2".to_string())) // Will be sorted
            .with_param("a", Some("1".to_string()));
        assert_eq!(event3.to_string(), "custom;a=1;b=2");

        let event4 = Event::new(EventType::Token("another".to_string()))
            .with_id("ID-XYZ".to_string())
            .with_param("UPPERCASE_PARAM", Some("Value".to_string()));
        assert_eq!(event4.to_string(), "another;id=ID-XYZ;UPPERCASE_PARAM=Value");
    }

    #[test]
    fn test_event_id_case_sensitivity_in_value() {
        let event = Event::new(EventType::Token("test".to_string())).with_id("CaseSensitiveID".to_string());
        assert_eq!(event.id, Some("CaseSensitiveID".to_string()));
        assert_eq!(event.to_string(), "test;id=CaseSensitiveID");
    }

    #[test]
    fn test_event_param_key_case_preservation() {
        let event = Event::new(EventType::Token("test".to_string()))
            .with_param("ParamKey", Some("value".to_string()));
        assert!(event.params.contains_key("ParamKey"));
        assert_eq!(event.to_string(), "test;ParamKey=value");
    }

    #[test]
    fn test_event_from_str_valid() {
        let event_str = "presence;id=123;flag;foo=bar";
        let event = Event::from_str(event_str).expect("Should parse valid event string");
        assert_eq!(event.event_type, EventType::Token("presence".to_string()));
        assert_eq!(event.id, Some("123".to_string()));
        assert_eq!(event.params.get("flag"), Some(&ParamValue::None));
        assert_eq!(event.params.get("foo"), Some(&ParamValue::Value("bar".to_string())));
    }

    #[test]
    fn test_event_from_str_empty_string() {
        let result = Event::from_str("");
        assert!(result.is_err());
        if let Err(SipError::Parser(msg)) = result {
            assert!(msg.contains("Failed to parse Event value string"));
        } else {
            panic!("Expected Parser error for empty string, got {:?}", result);
        }
    }

    #[test]
    fn test_event_from_str_incomplete() {
        let result = Event::from_str("<incomplete");
        assert!(result.is_err());
    }

    #[test]
    fn test_event_from_str_trailing_rubbish() {
        let result = Event::from_str("presence;id=123 then rubbish");
        assert!(result.is_err());
    }

    #[test]
    fn test_typed_header_trait_to_header() {
        let event = Event::new(EventType::Package("xyz".to_string()))
            .with_id("id1".to_string())
            .with_param("p1", Some("v1".to_string()))
            .with_param("p2", None::<String>);
        let header = event.to_header();

        assert_eq!(header.name, HeaderName::Event);
        let expected_value_str = "<xyz>;id=id1;p1=v1;p2";
        match header.value {
            HeaderValue::Raw(bytes) => {
                assert_eq!(String::from_utf8_lossy(&bytes), expected_value_str);
            }
            _ => panic!("Expected HeaderValue::Raw"),
        }
    }

    #[test]
    fn test_typed_header_trait_from_header_basic() {
        let header_value_str = "presence;id=abc-123;status=online";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_value_str.to_string()));
        
        let event = Event::from_header(&header).expect("Should parse successfully");

        assert_eq!(event.event_type, EventType::Token("presence".to_string()));
        assert_eq!(event.id, Some("abc-123".to_string()));
        assert_eq!(event.params.get("status"), Some(&ParamValue::Value("online".to_string())));
    }

    #[test]
    fn test_typed_header_trait_from_header_package_type() {
        let header_value_str = "<conference-info>;id=conf-xyz;version=2";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_value_str.to_string()));
        
        let event = Event::from_header(&header).expect("Should parse package type");

        assert_eq!(event.event_type, EventType::Package("conference-info".to_string()));
        assert_eq!(event.id, Some("conf-xyz".to_string()));
        assert_eq!(event.params.get("version"), Some(&ParamValue::Value("2".to_string())));
    }

    #[test]
    fn test_typed_header_trait_from_header_flag_param() {
        let header_value_str = "dialog;id=12345;notify-only";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_value_str.to_string()));

        let event = Event::from_header(&header).expect("Should parse flag param");
        assert_eq!(event.params.get("notify-only"), Some(&ParamValue::None));
    }
    
    #[test]
    fn test_typed_header_trait_from_header_no_id_no_params() {
        let header_value_str = "keep-alive";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_value_str.to_string()));
        
        let event = Event::from_header(&header).expect("Should parse event type only");
        assert_eq!(event.event_type, EventType::Token("keep-alive".to_string()));
        assert!(event.id.is_none());
        assert!(event.params.is_empty());
    }

    #[test]
    fn test_typed_header_trait_from_header_mismatched_name() {
        let header = Header::new(HeaderName::From, HeaderValue::text("presence;id=123".to_string()));
        let result = Event::from_header(&header);
        assert!(result.is_err());
        if let Err(SipError::InvalidHeader(msg)) = result {
            assert!(msg.contains("Mismatched header for Event"));
        } else {
            panic!("Expected InvalidHeader error, got {:?}", result);
        }
    }

    #[test]
    fn test_typed_header_trait_from_header_parser_error_incomplete() {
        let header = Header::new(HeaderName::Event, HeaderValue::text("<incomplete-package".to_string()));
        let result = Event::from_header(&header);
        assert!(result.is_err());
        if let Err(SipError::Parser(msg)) = result {
            assert!(msg.contains("Failed to parse Event value string"), "Error message: {}", msg);
        } else {
            panic!("Expected Parser error for incomplete input, got {:?}", result);
        }
    }

    #[test]
    fn test_typed_header_trait_from_header_parser_error_trailing_rubbish() {
        let header_value_str = "presence;id=123 extra-stuff";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_value_str.to_string()));
        let result = Event::from_header(&header);
        assert!(result.is_err());
         if let Err(SipError::Parser(msg)) = result {
            let expected_msg_part1 = "Failed to parse Event value string 'presence;id=123 extra-stuff': Parser error at '";
            let expected_rubbish_part = "extra-stuff"; // This is what e.input should be for the Eof error
            
            assert!(
                msg.starts_with(expected_msg_part1) && msg.contains(expected_rubbish_part) && msg.ends_with("(code: Eof)"),
                "Unexpected error message for trailing rubbish. Got: {}. Expected to start with '{}', contain '{}', and end with '(code: Eof)'",
                msg, expected_msg_part1, expected_rubbish_part
            );
        } else {
            panic!("Expected Parser error for trailing rubbish, got {:?}", result);
        }
    }
    
    #[test]
    fn test_typed_header_trait_from_header_invalid_utf8_in_raw() {
        let invalid_bytes = vec![0xC3, 0x28]; // Invalid UTF-8 sequence
        let header = Header::new(HeaderName::Event, HeaderValue::Raw(invalid_bytes));
        let result_syntax = Event::from_header(&header);
        assert!(result_syntax.is_err());
        if let Err(SipError::Parser(msg)) = result_syntax {
            assert!(msg.contains("Invalid UTF-8 in Event header value"), "Error message: {}", msg);
        } else {
            panic!("Expected Parser error for invalid UTF-8, got {:?}", result_syntax);
        }
    }

    #[test]
    fn test_event_header_round_trip() {
        let original_event = Event::new(EventType::Package("test-package".to_string()))
            .with_id("id-789".to_string())
            .with_param("flag1", None::<String>)
            .with_param("key2", Some("value2".to_string()));

        let header = original_event.to_header();
        let roundtripped_event = Event::from_header(&header).expect("Round trip should succeed");
        
        assert_eq!(original_event, roundtripped_event);
        assert_eq!(original_event.to_string(), roundtripped_event.to_string());
        assert_eq!(original_event.to_string(), "<test-package>;id=id-789;flag1;key2=value2");
    }
    
    #[test]
    fn test_event_header_with_special_chars_in_params_if_allowed_by_token() {
        let event_with_quotes = Event::new(EventType::Token("token-event".to_string()))
            .with_id("id-with-!%*".to_string())
            .with_param("param_quoted", Some(r#""a quoted string with spaces""#.to_string()));

        let header_str = r#"token-event;id=id-with-!%*;param_quoted="a quoted string with spaces""#;
        assert_eq!(event_with_quotes.to_string(), header_str);

        let header = event_with_quotes.to_header();
        let roundtripped = Event::from_header(&header).unwrap();
        assert_eq!(event_with_quotes, roundtripped);
    }

    #[test]
    fn test_multiple_id_params_behavior() {
        let header_val = "presence;id=first;ID=second;Id=third-flag";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_val.to_string()));
        let event = Event::from_header(&header).unwrap();

        assert_eq!(event.id, Some("first".to_string())); 
        assert_eq!(event.params.get("ID"), Some(&ParamValue::Value("second".to_string()))); 
        assert_eq!(event.params.get("Id"), Some(&ParamValue::Value("third-flag".to_string()))); 
    }

    #[test]
    fn test_id_param_as_flag_becomes_generic() {
        let header_val = "presence;id;foo=bar";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_val.to_string()));
        let event = Event::from_header(&header).unwrap();

        assert!(event.id.is_none()); 
        assert_eq!(event.params.get("id"), Some(&ParamValue::None)); 
        assert_eq!(event.params.get("foo"), Some(&ParamValue::Value("bar".to_string())));
    }

    // Ensure all .to_string() are added to EventType::Token initializations in existing tests
    #[test]
    fn test_event_struct_creation_and_display_corrected() {
        let event1 = Event::new(EventType::Token("presence".to_string()))
            .with_id("123".to_string())
            .with_param("foo", Some("bar".to_string()))
            .with_param("flag", None::<String>);
        assert_eq!(event1.to_string(), "presence;id=123;flag;foo=bar");

        let event2 = Event::new(EventType::Package("conference-package".to_string()));
        assert_eq!(event2.to_string(), "<conference-package>");

        let event3 = Event::new(EventType::Token("custom".to_string()))
            .with_param("b", Some("2".to_string()))
            .with_param("a", Some("1".to_string()));
        assert_eq!(event3.to_string(), "custom;a=1;b=2");

        let event4 = Event::new(EventType::Token("another".to_string()))
            .with_id("ID-XYZ".to_string())
            .with_param("UPPERCASE_PARAM", Some("Value".to_string()));
        assert_eq!(event4.to_string(), "another;id=ID-XYZ;UPPERCASE_PARAM=Value");
    }

    #[test]
    fn test_event_id_case_sensitivity_in_value_corrected() {
        let event = Event::new(EventType::Token("test".to_string())).with_id("CaseSensitiveID".to_string());
        assert_eq!(event.id, Some("CaseSensitiveID".to_string()));
        assert_eq!(event.to_string(), "test;id=CaseSensitiveID");
    }

    #[test]
    fn test_event_param_key_case_preservation_corrected() {
        let event = Event::new(EventType::Token("test".to_string()))
            .with_param("ParamKey", Some("value".to_string()));
        assert!(event.params.contains_key("ParamKey"));
        assert_eq!(event.to_string(), "test;ParamKey=value");
    }
} 