use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Common SIP header names
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HeaderName {
    /// Call-ID: Unique identifier for this call
    CallId,
    /// Contact: Where subsequent requests should be sent
    Contact,
    /// Content-Length: Size of the message body
    ContentLength,
    /// Content-Type: Media type of the message body
    ContentType,
    /// CSeq: Command sequence number
    CSeq,
    /// From: Initiator of the request
    From,
    /// Max-Forwards: Limit on the number of proxies or gateways
    MaxForwards,
    /// To: Logical recipient of the request
    To,
    /// Via: Path taken by the request so far
    Via,
    /// Allow: Methods supported by the UA
    Allow,
    /// Authorization: Credentials provided by a UA
    Authorization,
    /// Expires: Expiration time for registration or subscription
    Expires,
    /// Record-Route: Record of proxies that want to stay in the path
    RecordRoute,
    /// Route: Forced route for a request
    Route,
    /// Supported: Features supported by the UA
    Supported,
    /// User-Agent: Product information
    UserAgent,
    /// Event: Event package for SUBSCRIBE/NOTIFY
    Event,
    /// Subscription-State: State of subscription in NOTIFY
    SubscriptionState,
    /// Refer-To: Target URI in REFER
    ReferTo,
    /// Referred-By: Identity of referrer in REFER
    ReferredBy,
    /// RAck: Acknowledge receipt of a reliable provisional response
    RAck,
    /// Custom header name
    Other(String),
}

impl HeaderName {
    /// Returns the canonical name of the header
    pub fn as_str(&self) -> &str {
        match self {
            HeaderName::CallId => "Call-ID",
            HeaderName::Contact => "Contact",
            HeaderName::ContentLength => "Content-Length",
            HeaderName::ContentType => "Content-Type",
            HeaderName::CSeq => "CSeq",
            HeaderName::From => "From",
            HeaderName::MaxForwards => "Max-Forwards",
            HeaderName::To => "To",
            HeaderName::Via => "Via",
            HeaderName::Allow => "Allow",
            HeaderName::Authorization => "Authorization",
            HeaderName::Expires => "Expires",
            HeaderName::RecordRoute => "Record-Route",
            HeaderName::Route => "Route",
            HeaderName::Supported => "Supported",
            HeaderName::UserAgent => "User-Agent",
            HeaderName::Event => "Event",
            HeaderName::SubscriptionState => "Subscription-State",
            HeaderName::ReferTo => "Refer-To",
            HeaderName::ReferredBy => "Referred-By",
            HeaderName::RAck => "RAck",
            HeaderName::Other(s) => s,
        }
    }
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HeaderName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "call-id" | "i" => Ok(HeaderName::CallId),
            "contact" | "m" => Ok(HeaderName::Contact),
            "content-length" | "l" => Ok(HeaderName::ContentLength),
            "content-type" | "c" => Ok(HeaderName::ContentType),
            "cseq" => Ok(HeaderName::CSeq),
            "from" | "f" => Ok(HeaderName::From),
            "max-forwards" => Ok(HeaderName::MaxForwards),
            "to" | "t" => Ok(HeaderName::To),
            "via" | "v" => Ok(HeaderName::Via),
            "allow" => Ok(HeaderName::Allow),
            "authorization" => Ok(HeaderName::Authorization),
            "expires" => Ok(HeaderName::Expires),
            "record-route" => Ok(HeaderName::RecordRoute),
            "route" => Ok(HeaderName::Route),
            "supported" | "k" => Ok(HeaderName::Supported),
            "user-agent" => Ok(HeaderName::UserAgent),
            "event" | "o" => Ok(HeaderName::Event),
            "subscription-state" => Ok(HeaderName::SubscriptionState),
            "refer-to" | "r" => Ok(HeaderName::ReferTo),
            "referred-by" | "b" => Ok(HeaderName::ReferredBy),
            "rack" => Ok(HeaderName::RAck),
            _ if !s.is_empty() => Ok(HeaderName::Other(s.to_string())),
            _ => Err(Error::InvalidHeader("Empty header name".to_string())),
        }
    }
}

/// Value of a SIP header, which can be one of several types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HeaderValue {
    /// Simple text value
    Text(String),
    /// Integer value
    Integer(i64),
    /// List of text values
    TextList(Vec<String>),
    /// Raw content (for complex headers we don't parse yet)
    Raw(String),
}

impl HeaderValue {
    /// Create a new text header value
    pub fn text(value: impl Into<String>) -> Self {
        HeaderValue::Text(value.into())
    }

    /// Create a new integer header value
    pub fn integer(value: i64) -> Self {
        HeaderValue::Integer(value)
    }

    /// Create a new list header value
    pub fn text_list(values: Vec<String>) -> Self {
        HeaderValue::TextList(values)
    }

    /// Create a new raw header value
    pub fn raw(value: impl Into<String>) -> Self {
        HeaderValue::Raw(value.into())
    }

    /// Try to get this value as text
    pub fn as_text(&self) -> Option<&str> {
        match self {
            HeaderValue::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Try to get this value as an integer
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            HeaderValue::Integer(int) => Some(*int),
            _ => None,
        }
    }

    /// Try to get this value as a list
    pub fn as_text_list(&self) -> Option<&[String]> {
        match self {
            HeaderValue::TextList(list) => Some(list),
            _ => None,
        }
    }

    /// Get this value as a string, regardless of its internal type
    pub fn to_string_value(&self) -> String {
        match self {
            HeaderValue::Text(text) => text.clone(),
            HeaderValue::Integer(int) => int.to_string(),
            HeaderValue::TextList(list) => list.join(", "),
            HeaderValue::Raw(raw) => raw.clone(),
        }
    }
}

impl fmt::Display for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderValue::Text(text) => write!(f, "{}", text),
            HeaderValue::Integer(int) => write!(f, "{}", int),
            HeaderValue::TextList(list) => {
                let mut first = true;
                for item in list {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                    first = false;
                }
                Ok(())
            }
            HeaderValue::Raw(raw) => write!(f, "{}", raw),
        }
    }
}

impl FromStr for HeaderValue {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // For now, we'll just use a basic parser
        // In a full implementation, we'd parse based on header type
        if let Ok(int) = s.parse::<i64>() {
            Ok(HeaderValue::Integer(int))
        } else if s.contains(',') {
            let items = s.split(',')
                .map(|item| item.trim().to_string())
                .collect();
            Ok(HeaderValue::TextList(items))
        } else {
            Ok(HeaderValue::Text(s.to_string()))
        }
    }
}

/// SIP header, consisting of a name and value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// Header name
    pub name: HeaderName,
    /// Header value
    pub value: HeaderValue,
}

impl Header {
    /// Create a new header
    pub fn new(name: HeaderName, value: HeaderValue) -> Self {
        Header { name, value }
    }

    /// Create a new text header
    pub fn text(name: HeaderName, value: impl Into<String>) -> Self {
        Header::new(name, HeaderValue::text(value))
    }

    /// Create a new integer header
    pub fn integer(name: HeaderName, value: i64) -> Self {
        Header::new(name, HeaderValue::integer(value))
    }

    /// Get the header as a formatted string, ready for wire transmission
    pub fn to_wire_format(&self) -> String {
        format!("{}: {}", self.name, self.value)
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_name_from_str() {
        assert_eq!(HeaderName::from_str("Via").unwrap(), HeaderName::Via);
        assert_eq!(HeaderName::from_str("v").unwrap(), HeaderName::Via);
        assert_eq!(HeaderName::from_str("To").unwrap(), HeaderName::To);
        assert_eq!(HeaderName::from_str("t").unwrap(), HeaderName::To);
        assert_eq!(HeaderName::from_str("cSeq").unwrap(), HeaderName::CSeq);
        
        // Extension header
        let custom = HeaderName::from_str("X-Custom").unwrap();
        assert!(matches!(custom, HeaderName::Other(s) if s == "X-Custom"));
        
        // Empty header name is invalid
        assert!(HeaderName::from_str("").is_err());
    }

    #[test]
    fn test_header_value_creation() {
        let text = HeaderValue::text("Hello");
        assert_eq!(text.as_text(), Some("Hello"));
        
        let int = HeaderValue::integer(42);
        assert_eq!(int.as_integer(), Some(42));
        
        let list = HeaderValue::text_list(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(list.as_text_list(), Some(&["a".to_string(), "b".to_string()][..]));
    }

    #[test]
    fn test_header_creation() {
        let h = Header::text(HeaderName::To, "sip:alice@example.com");
        assert_eq!(h.name, HeaderName::To);
        assert_eq!(h.value.as_text(), Some("sip:alice@example.com"));
        
        let h = Header::integer(HeaderName::ContentLength, 42);
        assert_eq!(h.name, HeaderName::ContentLength);
        assert_eq!(h.value.as_integer(), Some(42));
    }

    #[test]
    fn test_header_wire_format() {
        let h = Header::text(HeaderName::To, "sip:alice@example.com");
        assert_eq!(h.to_wire_format(), "To: sip:alice@example.com");
        
        let h = Header::integer(HeaderName::ContentLength, 42);
        assert_eq!(h.to_wire_format(), "Content-Length: 42");
        
        let h = Header::new(
            HeaderName::Via, 
            HeaderValue::text("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK776asdhds")
        );
        assert_eq!(
            h.to_wire_format(), 
            "Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK776asdhds"
        );
    }
} 