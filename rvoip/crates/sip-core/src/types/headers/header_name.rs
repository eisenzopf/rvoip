use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

/// Common SIP header names
///
/// This enum represents all standard SIP header names defined in RFC 3261 and related
/// specifications, along with their compact forms and aliases. It also supports
/// custom header names through the `Other` variant.
///
/// Header names are case-insensitive in SIP, and this enum preserves the canonical
/// capitalization for standard headers while providing case-insensitive matching
/// during parsing.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Standard headers
/// let from = HeaderName::From;
/// assert_eq!(from.as_str(), "From");
///
/// // From compact form
/// let from_compact = HeaderName::from_str("f").unwrap();
/// assert_eq!(from_compact, HeaderName::From);
///
/// // Custom header
/// let custom = HeaderName::from_str("X-Custom-Header").unwrap();
/// assert_eq!(custom, HeaderName::Other("X-Custom-Header".to_string()));
/// ```
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
    /// Server: Server header
    Server,
    /// Unsupported: Features not supported by the UA
    Unsupported,
    /// Path: Path header (RFC 3327) for registration routing
    Path,
    /// Reason: Provides reasons for specific events (RFC 3326)
    Reason,
    /// Session-Expires: Session expiration information (RFC 4028)
    SessionExpires,
    /// MinSE: Minimum Session Expires header
    MinSE,
    /// RSeq: Response sequence number for reliable provisional responses (RFC 3262)
    RSeq,
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
            HeaderName::Server => "Server",
            HeaderName::Unsupported => "Unsupported",
            HeaderName::Path => "Path",
            HeaderName::Reason => "Reason",
            HeaderName::SessionExpires => "Session-Expires",
            HeaderName::Other(s) => s,
            HeaderName::MinSE => "Min-SE",
            HeaderName::RSeq => "RSeq",
        }
    }
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for HeaderName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Return an error for empty header names
        if s.is_empty() {
            return Err(Error::ParseError("Empty header name is not allowed".to_string()));
        }
        
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
            "content-disposition" | "d" => Ok(HeaderName::ContentDisposition),
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
            "alert-info" | "alert" => Ok(HeaderName::AlertInfo),
            "call-info" => Ok(HeaderName::CallInfo),
            "error-info" | "error" => Ok(HeaderName::ErrorInfo),
            "proxy-require" => Ok(HeaderName::ProxyRequire),
            "unsupported" | "u" => Ok(HeaderName::Unsupported),
            "path" => Ok(HeaderName::Path),
            "reason" => Ok(HeaderName::Reason),
            "session-expires" | "x" => Ok(HeaderName::SessionExpires),
            "min-se" => Ok(HeaderName::MinSE),
            "rseq" => Ok(HeaderName::RSeq),
            _ => Ok(HeaderName::Other(s.to_string())),
        }
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

        assert_eq!(HeaderName::from_str("Session-Expires").unwrap(), HeaderName::SessionExpires);
        assert_eq!(HeaderName::from_str("session-expires").unwrap(), HeaderName::SessionExpires);
        assert_eq!(HeaderName::from_str("x").unwrap(), HeaderName::SessionExpires);
    }
} 