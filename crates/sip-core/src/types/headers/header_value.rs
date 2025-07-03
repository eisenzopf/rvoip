use crate::error::{Error, Result};
use std::fmt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::types::param::Param;
use crate::types::contact::ContactValue as TypesContactValue;
use crate::types::from::From as FromHeaderValue;
use crate::types::to::To as ToHeaderValue;
use crate::parser::headers::route::RouteEntry;
use crate::types::record_route::RecordRouteEntry;
use crate::parser::headers::reply_to::ReplyToValue;
use crate::types::reply_to::ReplyTo;
use crate::types::refer_to::ReferTo;
use crate::types::referred_by::ReferredBy;
use crate::types::via::ViaHeader;
use crate::types::cseq::CSeq;
use crate::types::max_forwards::MaxForwards;
use crate::types::expires::Expires;
use crate::types::retry_after::RetryAfter;
use crate::types::warning::Warning;
use crate::parser::headers::accept::AcceptValue;
use crate::parser::headers::accept_encoding::EncodingInfo;
use crate::types::accept_language::AcceptLanguage;
use crate::types::content_length::ContentLength;
use crate::types::content_type::ContentType;
use crate::parser::headers::alert_info::AlertInfoValue;
use crate::types::call_info::CallInfoValue;
use crate::parser::headers::error_info::ErrorInfoValue;
use crate::types::in_reply_to::InReplyTo;
use crate::prelude::GenericValue;

/// Value of a SIP header, parsed into its specific structure.
///
/// This enum represents the value part of a SIP header, with variants for 
/// different header types. During parsing, header values are stored in the
/// appropriate variant based on the header name.
///
/// Most variants store partially parsed structured data (like addresses, 
/// parameters, etc.), while the `Raw` variant is used for unparsed or 
/// unknown header values.
///
/// This type is primarily used during the parsing process before converting
/// to more strongly-typed header representations.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a simple text value
/// let value = HeaderValue::text("Hello World");
/// assert_eq!(value.as_text(), Some("Hello World"));
///
/// // Create an integer value
/// let value = HeaderValue::integer(42);
/// assert_eq!(value.as_integer(), Some(42));
///
/// // Create a list value
/// let value = HeaderValue::text_list(vec!["foo".to_string(), "bar".to_string()]);
/// assert_eq!(value.as_text_list(), Some(vec!["foo", "bar"]));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HeaderValue {
    // === Address Headers ===
    Contact(TypesContactValue), // Use imported type
    From(FromHeaderValue),
    To(ToHeaderValue),
    Route(Vec<RouteEntry>),
    RecordRoute(Vec<RecordRouteEntry>),
    ReplyTo(ReplyTo),
    ReferTo(ReferTo),
    ReferredBy(ReferredBy),

    // === Request/Response Info ===
    Via(Vec<ViaHeader>), // ViaHeader would contain the parsed tuple
    CSeq(CSeq),
    MaxForwards(MaxForwards),
    CallId((Vec<u8>, Option<Vec<u8>>)), // (local_part, Option<host_part>)
    Expires(Expires),
    MinExpires(u32),
    RetryAfter(RetryAfter),
    Warning(Vec<Warning>),
    Timestamp((Vec<u8>, Option<Vec<u8>>), Option<(Vec<u8>, Option<Vec<u8>>)>), // (ts, delay_opt)
    Date(Vec<u8>),

    // === Content Negotiation ===
    Accept(Vec<AcceptValue>),
    AcceptEncoding(Vec<EncodingInfo>),
    AcceptLanguage(Vec<AcceptLanguage>),

    // === Body Info ===
    ContentLength(ContentLength),
    ContentType(ContentType),
    ContentEncoding(Vec<Vec<u8>>), // Vec<token>
    ContentLanguage(Vec<Vec<u8>>), // Vec<language-tag>
    ContentDisposition((Vec<u8>, Vec<Param>)), // (disp_type, params)
    MimeVersion((u8, u8)), // (major, minor)

    // === Capabilities/Options ===
    Allow(Vec<Vec<u8>>), // Vec<token>
    Require(Vec<Vec<u8>>), // Vec<token>
    Supported(Vec<Vec<u8>>), // Vec<token>
    Unsupported(Vec<Vec<u8>>), // Vec<token>
    ProxyRequire(Vec<Vec<u8>>), // Vec<token>

    // === Info Headers ===
    AlertInfo(Vec<AlertInfoValue>),
    CallInfo(Vec<CallInfoValue>), // Keep as Vec<CallInfoValue> for backward compatibility
    ErrorInfo(Vec<ErrorInfoValue>),

    // === Misc ===
    Organization(Option<Vec<u8>>),
    Priority(Vec<u8>),
    Subject(Option<Vec<u8>>),
    Server(Vec<(Option<(Vec<u8>, Option<Vec<u8>>)>, Option<Vec<u8>>)>), // Vec<(Product?, Comment?)>
    UserAgent(Vec<(Option<(Vec<u8>, Option<Vec<u8>>)>, Option<Vec<u8>>)>), // Vec<(Product?, Comment?)>
    InReplyTo(InReplyTo),

    // === Authentication (Placeholders) ===
    Authorization(crate::types::auth::Authorization), // Use proper type instead of Vec<u8>
    ProxyAuthorization(crate::types::auth::ProxyAuthorization), // Use proper type instead of Vec<u8>
    WwwAuthenticate(crate::types::auth::WwwAuthenticate), // Use proper type instead of Vec<u8>
    ProxyAuthenticate(crate::types::auth::ProxyAuthenticate), // Use proper type instead of Vec<u8>
    AuthenticationInfo(crate::types::auth::AuthenticationInfo), // Use proper type instead of Vec<u8>

    // === Other ===
    /// Raw value for unknown or unparsed headers
    Raw(Vec<u8>),
}

impl HeaderValue {
    pub fn text(value: impl Into<String>) -> Self {
        HeaderValue::Raw(value.into().into_bytes())
    }

    pub fn integer(value: i64) -> Self {
        HeaderValue::Raw(value.to_string().into_bytes())
    }

    pub fn text_list(values: Vec<String>) -> Self {
        HeaderValue::Raw(values.join(", ").into_bytes())
    }

    /// Creates a ContentType HeaderValue for SDP (application/sdp)
    pub fn content_type_sdp() -> Self {
        use crate::parser::headers::content_type::ContentTypeValue;
        use std::collections::HashMap;
        
        HeaderValue::ContentType(crate::types::content_type::ContentType(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: HashMap::new(),
        }))
    }

    /// Creates a ContentType HeaderValue for plain text (text/plain)
    pub fn content_type_text_plain() -> Self {
        use crate::parser::headers::content_type::ContentTypeValue;
        use std::collections::HashMap;
        
        HeaderValue::ContentType(crate::types::content_type::ContentType(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: HashMap::new(),
        }))
    }

    /// Creates a ContentType HeaderValue for JSON (application/json)
    pub fn content_type_json() -> Self {
        use crate::parser::headers::content_type::ContentTypeValue;
        use std::collections::HashMap;
        
        HeaderValue::ContentType(crate::types::content_type::ContentType(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: HashMap::new(),
        }))
    }

    /// Creates a ContentType HeaderValue for multipart/mixed
    pub fn content_type_multipart_mixed(boundary: impl Into<String>) -> Self {
        use crate::parser::headers::content_type::ContentTypeValue;
        use std::collections::HashMap;
        
        let mut parameters = HashMap::new();
        parameters.insert("boundary".to_string(), boundary.into());
        
        HeaderValue::ContentType(crate::types::content_type::ContentType(ContentTypeValue {
            m_type: "multipart".to_string(),
            m_subtype: "mixed".to_string(),
            parameters,
        }))
    }

    /// Creates a ContentLength HeaderValue
    pub fn content_length(length: usize) -> Self {
        HeaderValue::ContentLength(crate::types::content_length::ContentLength(length as u32))
    }

    /// Creates a MaxForwards HeaderValue
    pub fn max_forwards(value: u8) -> Self {
        HeaderValue::MaxForwards(crate::types::max_forwards::MaxForwards(value))
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            HeaderValue::Raw(bytes) => std::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        self.as_text().and_then(|s| s.parse().ok())
    }

    pub fn as_text_list(&self) -> Option<Vec<&str>> {
        self.as_text().map(|s| {
            s.split(',')
                .map(|part| part.trim())
                .filter(|part| !part.is_empty())
                .collect()
        })
    }
}

impl fmt::Display for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    write!(f, "{}", s)
                } else {
                    // Fall back to printing the raw bytes for non-UTF8 values
                    write!(f, "{:?}", bytes)
                }
            },
            HeaderValue::CallInfo(ref values) => {
                let mut first = true;
                for value in values {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "<{}>", value.uri)?;
                    for param in &value.params {
                        write!(f, ";{}", param)?;
                    }
                    first = false;
                }
                Ok(())
            },
            _ => write!(f, "[Complex Value]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_header_value_creation() {
        let text = HeaderValue::text("Hello");
        assert_eq!(text.as_text(), Some("Hello"));
        
        let int = HeaderValue::integer(42);
        assert_eq!(int.as_integer(), Some(42));
    }
} 