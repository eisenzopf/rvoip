use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

use crate::parser::headers::{
    via::ViaEntry, // Assuming ViaEntry is the Vec element type
    contact::ContactValue,
    from::FromHeaderValue,
    to::ToHeaderValue,
    route::RouteEntry,
    record_route::RecordRouteEntry,
    cseq::CSeqValue,
    content_type::ContentTypeValue,
    accept::AcceptValue,
    accept_encoding::AcceptEncodingValue,
    accept_language::AcceptLanguageValue,
    content_disposition::ContentDispositionValue,
    alert_info::AlertInfoValue,
    call_info::CallInfoValue,
    error_info::ErrorInfoValue,
    warning::WarningValue,
    retry_after::RetryAfterValue,
    reply_to::ReplyToValue,
    // server_val is complex, use Vec<u8> for now
    // TODO: Add auth header types
};
use crate::uri::Uri;
use ordered_float::NotNan;
use std::collections::HashMap;
use crate::types::param::Param;

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
    /// Min-Expires: Minimum expiration time for registration or subscription
    MinExpires,
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
    /// WWW-Authenticate: Challenge for authentication
    WwwAuthenticate,
    /// Accept: Media types acceptable for the response
    Accept,
    /// Accept-Encoding: Acceptable content encodings
    AcceptEncoding,
    /// Accept-Language: Acceptable languages for the response
    AcceptLanguage,
    /// Content-Disposition: Presentation style for the message body
    ContentDisposition,
    /// Content-Encoding: Content encoding of the message body
    ContentEncoding,
    /// Content-Language: Language of the message body
    ContentLanguage,
    /// Warning: Additional information about the status of a response
    Warning,
    /// Proxy-Authenticate: Challenge for proxy authentication
    ProxyAuthenticate,
    /// Proxy-Authorization: Credentials for proxy authentication
    ProxyAuthorization,
    /// Authentication-Info: Information related to authentication
    AuthenticationInfo,
    /// Reply-To: Address for replies
    ReplyTo,
    /// Require: Required capabilities for the request
    Require,
    /// Retry-After: Recommended time to wait before retrying
    RetryAfter,
    /// Subject: Subject of the message
    Subject,
    /// Timestamp: Timestamp of the message
    Timestamp,
    /// Organization: Organization of the message
    Organization,
    /// Priority: Priority of the message
    Priority,
    /// Date: Date of the message
    Date,
    /// MIME-Version: MIME version of the message
    MimeVersion,
    /// In-Reply-To: In-Reply-To header
    InReplyTo,
    /// Alert-Info: Alert-Info header
    AlertInfo,
    /// Call-Info: Call-Info header
    CallInfo,
    /// Error-Info: Error-Info header
    ErrorInfo,
    /// Proxy-Require: Required capabilities for the proxy
    ProxyRequire,
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
            HeaderName::MinExpires => "Min-Expires",
            HeaderName::RecordRoute => "Record-Route",
            HeaderName::Route => "Route",
            HeaderName::Supported => "Supported",
            HeaderName::UserAgent => "User-Agent",
            HeaderName::Event => "Event",
            HeaderName::SubscriptionState => "Subscription-State",
            HeaderName::ReferTo => "Refer-To",
            HeaderName::ReferredBy => "Referred-By",
            HeaderName::RAck => "RAck",
            HeaderName::WwwAuthenticate => "WWW-Authenticate",
            HeaderName::Accept => "Accept",
            HeaderName::AcceptEncoding => "Accept-Encoding",
            HeaderName::AcceptLanguage => "Accept-Language",
            HeaderName::ContentDisposition => "Content-Disposition",
            HeaderName::ContentEncoding => "Content-Encoding",
            HeaderName::ContentLanguage => "Content-Language",
            HeaderName::Warning => "Warning",
            HeaderName::ProxyAuthenticate => "Proxy-Authenticate",
            HeaderName::ProxyAuthorization => "Proxy-Authorization",
            HeaderName::AuthenticationInfo => "Authentication-Info",
            HeaderName::ReplyTo => "Reply-To",
            HeaderName::Require => "Require",
            HeaderName::RetryAfter => "Retry-After",
            HeaderName::Subject => "Subject",
            HeaderName::Timestamp => "Timestamp",
            HeaderName::Organization => "Organization",
            HeaderName::Priority => "Priority",
            HeaderName::Date => "Date",
            HeaderName::MimeVersion => "MIME-Version",
            HeaderName::InReplyTo => "In-Reply-To",
            HeaderName::AlertInfo => "Alert-Info",
            HeaderName::CallInfo => "Call-Info",
            HeaderName::ErrorInfo => "Error-Info",
            HeaderName::ProxyRequire => "Proxy-Require",
            HeaderName::Other(s) => s,
        }
    }
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderName::ProxyRequire => write!(f, "Proxy-Require"),
            HeaderName::RecordRoute => write!(f, "Record-Route"),
            HeaderName::ReplyTo => write!(f, "Reply-To"),
            HeaderName::Require => write!(f, "Require"),
            HeaderName::RetryAfter => write!(f, "Retry-After"),
            HeaderName::Route => write!(f, "Route"),
            HeaderName::Server => write!(f, "Server"),
            HeaderName::Subject => write!(f, "Subject"),
            HeaderName::Supported => write!(f, "Supported"),
            HeaderName::Timestamp => write!(f, "Timestamp"),
            HeaderName::To => write!(f, "To"),
            HeaderName::Unsupported => write!(f, "Unsupported"),
            HeaderName::UserAgent => write!(f, "User-Agent"),
            HeaderName::Via => write!(f, "Via"),
            HeaderName::Warning => write!(f, "Warning"),
            HeaderName::WwwAuthenticate => write!(f, "WWW-Authenticate"),
            HeaderName::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for HeaderName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let lower_s = s.to_lowercase();
        match lower_s.as_str() {
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
            "min-expires" => Ok(HeaderName::MinExpires),
            "record-route" => Ok(HeaderName::RecordRoute),
            "route" => Ok(HeaderName::Route),
            "server" => Ok(HeaderName::Server),
            "supported" | "k" => Ok(HeaderName::Supported),
            "user-agent" => Ok(HeaderName::UserAgent),
            "event" | "o" => Ok(HeaderName::Event),
            "subscription-state" => Ok(HeaderName::SubscriptionState),
            "refer-to" | "r" => Ok(HeaderName::ReferTo),
            "referred-by" | "b" => Ok(HeaderName::ReferredBy),
            "rack" => Ok(HeaderName::RAck),
            "www-authenticate" => Ok(HeaderName::WwwAuthenticate),
            "accept" => Ok(HeaderName::Accept),
            "accept-encoding" => Ok(HeaderName::AcceptEncoding),
            "accept-language" => Ok(HeaderName::AcceptLanguage),
            "content-disposition" => Ok(HeaderName::ContentDisposition),
            "content-encoding" | "e" => Ok(HeaderName::ContentEncoding),
            "content-language" => Ok(HeaderName::ContentLanguage),
            "warning" => Ok(HeaderName::Warning),
            "proxy-authenticate" => Ok(HeaderName::ProxyAuthenticate),
            "proxy-authorization" => Ok(HeaderName::ProxyAuthorization),
            "authentication-info" => Ok(HeaderName::AuthenticationInfo),
            "reply-to" => Ok(HeaderName::ReplyTo),
            "require" => Ok(HeaderName::Require),
            "retry-after" => Ok(HeaderName::RetryAfter),
            "subject" | "s" => Ok(HeaderName::Subject),
            "timestamp" => Ok(HeaderName::Timestamp),
            "organization" => Ok(HeaderName::Organization),
            "priority" => Ok(HeaderName::Priority),
            "date" => Ok(HeaderName::Date),
            "mime-version" => Ok(HeaderName::MimeVersion),
            "in-reply-to" => Ok(HeaderName::InReplyTo),
            "alert-info" => Ok(HeaderName::AlertInfo),
            "call-info" => Ok(HeaderName::CallInfo),
            "error-info" => Ok(HeaderName::ErrorInfo),
            "proxy-require" => Ok(HeaderName::ProxyRequire),
            "unsupported" => Ok(HeaderName::Unsupported),
            _ if !s.is_empty() => Ok(HeaderName::Other(s.to_string())),
            _ => Err(Error::InvalidHeader("Empty header name".to_string())),
        }
    }
}

/// Value of a SIP header, parsed into its specific structure.
#[derive(Debug, Clone, PartialEq)]
pub enum HeaderValue {
    // === Address Headers ===
    Contact(ContactValue), // Can be Star or Addresses
    From(FromHeaderValue),
    To(ToHeaderValue),
    Route(Vec<RouteEntry>),
    RecordRoute(Vec<RecordRouteEntry>),
    ReplyTo(ReplyToValue),

    // === Request/Response Info ===
    Via(Vec<ViaEntry>), // ViaEntry would contain the parsed tuple
    CSeq(CSeqValue),
    MaxForwards(u8),
    CallId((Vec<u8>, Option<Vec<u8>>)), // (local_part, Option<host_part>)
    Expires(u32),
    MinExpires(u32),
    RetryAfter(RetryAfterValue),
    Warning(Vec<WarningValue>),
    Timestamp((Vec<u8>, Option<Vec<u8>>), Option<(Vec<u8>, Option<Vec<u8>>)>), // (ts, delay_opt)
    Date(Vec<u8>),

    // === Content Negotiation ===
    Accept(Vec<AcceptValue>),
    AcceptEncoding(Vec<AcceptEncodingValue>),
    AcceptLanguage(Vec<AcceptLanguageValue>),

    // === Body Info ===
    ContentLength(u64),
    ContentType(ContentTypeValue),
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
    CallInfo(Vec<CallInfoValue>),
    ErrorInfo(Vec<ErrorInfoValue>),

    // === Misc ===
    Organization(Option<Vec<u8>>),
    Priority(Vec<u8>),
    Subject(Option<Vec<u8>>),
    Server(Vec<(Option<(Vec<u8>, Option<Vec<u8>>)>, Option<Vec<u8>>)>), // Vec<(Product?, Comment?)>
    UserAgent(Vec<(Option<(Vec<u8>, Option<Vec<u8>>)>, Option<Vec<u8>>)>), // Vec<(Product?, Comment?)>
    InReplyTo(Vec<(Vec<u8>, Option<Vec<u8>>)>), // Vec<callid>

    // === Authentication (Placeholders) ===
    Authorization(Vec<u8>), // Placeholder
    ProxyAuthorization(Vec<u8>), // Placeholder
    WwwAuthenticate(Vec<u8>), // Placeholder
    ProxyAuthenticate(Vec<u8>), // Placeholder
    AuthenticationInfo(Vec<u8>), // Placeholder

    // === Other ===
    /// Raw value for unknown or unparsed headers
    Raw(Vec<u8>),
    // TODO: Add variants for other headers (Event, Subscription-State, etc.)
}

/// SIP header, consisting of a name and value
#[derive(Debug, Clone, PartialEq)]
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