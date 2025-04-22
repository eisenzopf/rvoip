use crate::error::{Error, Result};
use crate::types; // Import the types module itself
use crate::parser; // Import the parser module
use std::convert::TryFrom;
use nom::combinator::all_consuming;
use ordered_float::NotNan;
use chrono; // Add use statement
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::param::Param;
use crate::types::uri::Uri; // Import Uri
use crate::types::contact::ContactValue as TypesContactValue; // Import type directly

use crate::parser::headers::{
    via::ViaEntry, 
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
};

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
    /// Server: Server header
    Server,
    /// Unsupported: Features not supported by the UA
    Unsupported,
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
            HeaderName::Other(s) => s,
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
    Contact(TypesContactValue), // Use imported type
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
            // Add handling for other variant types as needed
            _ => write!(f, "[Complex Value]"),
        }
    }
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

/// Represents any parsed SIP header in a strongly-typed way.
#[derive(Debug, Clone, PartialEq)] // Add necessary derives
pub enum TypedHeader {
    // Core Headers (Examples)
    Via(types::Via),
    From(types::From),
    To(types::to::To),
    Contact(types::Contact),
    CallId(types::CallId),
    CSeq(types::CSeq),
    Route(types::route::Route),
    RecordRoute(types::record_route::RecordRoute),
    MaxForwards(types::MaxForwards),
    ContentType(types::ContentType),
    ContentLength(types::ContentLength),
    Expires(types::Expires),

    // Auth Headers
    Authorization(types::auth::Authorization),
    WwwAuthenticate(types::auth::WwwAuthenticate),
    ProxyAuthenticate(types::auth::ProxyAuthenticate),
    ProxyAuthorization(types::auth::ProxyAuthorization),
    AuthenticationInfo(types::auth::AuthenticationInfo),

    // Add other typed headers here as they are defined...
    Accept(types::Accept),
    Allow(types::Allow),
    ReplyTo(types::reply_to::ReplyTo),
    Warning(types::Warning),
    ContentDisposition(types::ContentDisposition),

    // Placeholder Types (replace with actual types from types/*)
    ContentEncoding(Vec<String>),
    ContentLanguage(Vec<String>),
    AcceptEncoding(Vec<types::accept::EncodingInfo>),
    AcceptLanguage(Vec<types::accept::LanguageInfo>),
    MinExpires(u32),
    MimeVersion((u32, u32)),
    Require(Vec<String>),
    Supported(Vec<String>),
    Unsupported(Vec<String>),
    ProxyRequire(Vec<String>),
    Date(chrono::DateTime<chrono::FixedOffset>),
    Timestamp((ordered_float::NotNan<f32>, Option<ordered_float::NotNan<f32>>)),
    Organization(String),
    Priority(types::priority::PriorityValue),
    Subject(String),
    Server(Vec<types::server::ServerVal>),
    UserAgent(Vec<types::server::ServerVal>),
    InReplyTo(Vec<String>),
    RetryAfter((u32, Option<String>, Vec<types::retry_after::RetryParam>)),
    ErrorInfo(Vec<types::error_info::ErrorInfoValue>),
    AlertInfo(Vec<types::alert_info::AlertInfoValue>),
    CallInfo(Vec<types::call_info::CallInfoValue>),

    /// Represents an unknown or unparsed header.
    Other(HeaderName, HeaderValue),
}

impl TypedHeader {
    /// Returns the canonical name of this header.
    pub fn name(&self) -> HeaderName {
        match self {
            TypedHeader::Via(_) => HeaderName::Via,
            TypedHeader::From(_) => HeaderName::From,
            TypedHeader::To(_) => HeaderName::To,
            TypedHeader::Contact(_) => HeaderName::Contact,
            TypedHeader::CallId(_) => HeaderName::CallId,
            TypedHeader::CSeq(_) => HeaderName::CSeq,
            TypedHeader::Route(_) => HeaderName::Route,
            TypedHeader::RecordRoute(_) => HeaderName::RecordRoute,
            TypedHeader::MaxForwards(_) => HeaderName::MaxForwards,
            TypedHeader::ContentType(_) => HeaderName::ContentType,
            TypedHeader::ContentLength(_) => HeaderName::ContentLength,
            TypedHeader::Expires(_) => HeaderName::Expires,
            TypedHeader::Authorization(_) => HeaderName::Authorization,
            TypedHeader::WwwAuthenticate(_) => HeaderName::WwwAuthenticate,
            TypedHeader::ProxyAuthenticate(_) => HeaderName::ProxyAuthenticate,
            TypedHeader::ProxyAuthorization(_) => HeaderName::ProxyAuthorization,
            TypedHeader::AuthenticationInfo(_) => HeaderName::AuthenticationInfo,
            TypedHeader::Accept(_) => HeaderName::Accept,
            TypedHeader::Allow(_) => HeaderName::Allow,
            TypedHeader::ReplyTo(_) => HeaderName::ReplyTo,
            TypedHeader::Warning(_) => HeaderName::Warning,
            TypedHeader::ContentDisposition(_) => HeaderName::ContentDisposition,
            TypedHeader::ContentEncoding(_) => HeaderName::ContentEncoding,
            TypedHeader::ContentLanguage(_) => HeaderName::ContentLanguage,
            TypedHeader::AcceptEncoding(_) => HeaderName::AcceptEncoding,
            TypedHeader::AcceptLanguage(_) => HeaderName::AcceptLanguage,
            TypedHeader::MinExpires(_) => HeaderName::MinExpires,
            TypedHeader::MimeVersion(_) => HeaderName::MimeVersion,
            TypedHeader::Require(_) => HeaderName::Require,
            TypedHeader::Supported(_) => HeaderName::Supported,
            TypedHeader::Unsupported(_) => HeaderName::Unsupported,
            TypedHeader::ProxyRequire(_) => HeaderName::ProxyRequire,
            TypedHeader::Date(_) => HeaderName::Date,
            TypedHeader::Timestamp(_) => HeaderName::Timestamp,
            TypedHeader::Organization(_) => HeaderName::Organization,
            TypedHeader::Priority(_) => HeaderName::Priority,
            TypedHeader::Subject(_) => HeaderName::Subject,
            TypedHeader::Server(_) => HeaderName::Server,
            TypedHeader::UserAgent(_) => HeaderName::UserAgent,
            TypedHeader::InReplyTo(_) => HeaderName::InReplyTo,
            TypedHeader::RetryAfter(_) => HeaderName::RetryAfter,
            TypedHeader::ErrorInfo(_) => HeaderName::ErrorInfo,
            TypedHeader::AlertInfo(_) => HeaderName::AlertInfo,
            TypedHeader::CallInfo(_) => HeaderName::CallInfo,
            TypedHeader::Other(name, _) => name.clone(),
        }
    }
}

// Add Display implementation for TypedHeader
impl fmt::Display for TypedHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Format known typed headers by displaying their inner value
            // Assumes the inner types implement Display
            TypedHeader::Via(v) => write!(f, "{}: {}", HeaderName::Via, v),
            TypedHeader::From(v) => write!(f, "{}: {}", HeaderName::From, v),
            TypedHeader::To(v) => write!(f, "{}: {}", HeaderName::To, v),
            TypedHeader::Contact(v) => write!(f, "{}: {}", HeaderName::Contact, v),
            TypedHeader::CallId(v) => write!(f, "{}: {}", HeaderName::CallId, v),
            TypedHeader::CSeq(v) => write!(f, "{}: {}", HeaderName::CSeq, v),
            TypedHeader::Route(v) => write!(f, "{}: {}", HeaderName::Route, v),
            TypedHeader::RecordRoute(v) => write!(f, "{}: {}", HeaderName::RecordRoute, v),
            TypedHeader::MaxForwards(v) => write!(f, "{}: {}", HeaderName::MaxForwards, v),
            TypedHeader::ContentType(v) => write!(f, "{}: {}", HeaderName::ContentType, v),
            TypedHeader::ContentLength(v) => write!(f, "{}: {}", HeaderName::ContentLength, v),
            TypedHeader::Expires(v) => write!(f, "{}: {}", HeaderName::Expires, v),
            TypedHeader::Authorization(v) => write!(f, "{}: {}", HeaderName::Authorization, v),
            TypedHeader::WwwAuthenticate(v) => write!(f, "{}: {}", HeaderName::WwwAuthenticate, v),
            TypedHeader::ProxyAuthenticate(v) => write!(f, "{}: {}", HeaderName::ProxyAuthenticate, v),
            TypedHeader::ProxyAuthorization(v) => write!(f, "{}: {}", HeaderName::ProxyAuthorization, v),
            TypedHeader::AuthenticationInfo(v) => write!(f, "{}: {}", HeaderName::AuthenticationInfo, v),
            TypedHeader::Accept(v) => write!(f, "{}: {}", HeaderName::Accept, v),
            TypedHeader::Allow(v) => write!(f, "{}: {}", HeaderName::Allow, v),
            TypedHeader::ReplyTo(v) => write!(f, "{}: {}", HeaderName::ReplyTo, v),
            TypedHeader::Warning(v) => write!(f, "{}: {}", HeaderName::Warning, v),
            TypedHeader::ContentDisposition(v) => write!(f, "{}: {}", HeaderName::ContentDisposition, v),

            // Handle placeholder types (Vec<String>, tuples, etc.) - Requires Display impl for them
            // For now, using debug format as a placeholder if direct Display is complex
            TypedHeader::ContentEncoding(v) => write!(f, "{}: {:?}", HeaderName::ContentEncoding, v),
            TypedHeader::ContentLanguage(v) => write!(f, "{}: {:?}", HeaderName::ContentLanguage, v),
            TypedHeader::AcceptEncoding(v) => write!(f, "{}: {:?}", HeaderName::AcceptEncoding, v),
            TypedHeader::AcceptLanguage(v) => write!(f, "{}: {:?}", HeaderName::AcceptLanguage, v),
            TypedHeader::MinExpires(v) => write!(f, "{}: {}", HeaderName::MinExpires, v),
            TypedHeader::MimeVersion(v) => write!(f, "{}: {:?}", HeaderName::MimeVersion, v), // Assuming tuple doesn't have Display
            TypedHeader::Require(v) => write!(f, "{}: {:?}", HeaderName::Require, v),
            TypedHeader::Supported(v) => write!(f, "{}: {:?}", HeaderName::Supported, v),
            TypedHeader::Unsupported(v) => write!(f, "{}: {:?}", HeaderName::Unsupported, v),
            TypedHeader::ProxyRequire(v) => write!(f, "{}: {:?}", HeaderName::ProxyRequire, v),
            TypedHeader::Date(v) => write!(f, "{}: {}", HeaderName::Date, v), // chrono DateTime implements Display
            TypedHeader::Timestamp(v) => write!(f, "{}: {:?}", HeaderName::Timestamp, v), // tuple
            TypedHeader::Organization(v) => write!(f, "{}: {}", HeaderName::Organization, v),
            TypedHeader::Priority(v) => write!(f, "{}: {}", HeaderName::Priority, v), // Assuming PriorityValue implements Display
            TypedHeader::Subject(v) => write!(f, "{}: {}", HeaderName::Subject, v),
            TypedHeader::Server(v) => write!(f, "{}: {:?}", HeaderName::Server, v), // Vec<ServerVal>
            TypedHeader::UserAgent(v) => write!(f, "{}: {:?}", HeaderName::UserAgent, v), // Vec<ServerVal>
            TypedHeader::InReplyTo(v) => write!(f, "{}: {:?}", HeaderName::InReplyTo, v), // Vec<String>
            TypedHeader::RetryAfter(v) => write!(f, "{}: {:?}", HeaderName::RetryAfter, v), // tuple
            TypedHeader::ErrorInfo(v) => write!(f, "{}: {:?}", HeaderName::ErrorInfo, v), // Vec<...>
            TypedHeader::AlertInfo(v) => write!(f, "{}: {:?}", HeaderName::AlertInfo, v), // Vec<...>
            TypedHeader::CallInfo(v) => write!(f, "{}: {:?}", HeaderName::CallInfo, v), // Vec<...>

            // Format Other headers using the name and value
            TypedHeader::Other(name, value) => write!(f, "{}: {}", name, value), // Assumes HeaderValue implements Display
        }
    }
}

impl TryFrom<Header> for TypedHeader {
    type Error = Error;

    fn try_from(header: Header) -> Result<Self> {
        // We need the unfolded, raw value bytes here.
        // The message_header parser now puts Vec<u8> into HeaderValue::Raw.
        let value_bytes = match header.value {
            HeaderValue::Raw(bytes) => bytes, // Use the raw, unfolded bytes
            _ => return Ok(TypedHeader::Other(header.name.clone(), header.value.clone())), // Should not happen if message_header is used
        };
        
        // Use all_consuming to ensure the specific parser consumes the entire value
        let parse_result = match &header.name {
            // Address Headers
            HeaderName::From => all_consuming(parser::headers::parse_from)(&value_bytes).map(|(_, v)| TypedHeader::From(v)),
            HeaderName::To => all_consuming(parser::headers::parse_to)(&value_bytes).map(|(_, v)| TypedHeader::To(v)),
            HeaderName::Contact => all_consuming(parser::headers::parse_contact)(&value_bytes).map(|(_, v)| TypedHeader::Contact(v)), // Assuming ContactValue is handled
            HeaderName::ReplyTo => all_consuming(parser::headers::parse_reply_to)(&value_bytes).map(|(_, v)| TypedHeader::ReplyTo(v)),

            // Routing Headers
            HeaderName::Via => all_consuming(parser::headers::parse_via)(&value_bytes).map(|(_, v)| TypedHeader::Via(v)), // Assuming Via type wraps Vec<ViaHeader>
            HeaderName::Route => all_consuming(parser::headers::parse_route)(&value_bytes).map(|(_, v)| TypedHeader::Route(v)),
            HeaderName::RecordRoute => all_consuming(parser::headers::parse_record_route)(&value_bytes).map(|(_, v)| TypedHeader::RecordRoute(v)),

            // Dialog/Transaction IDs
            HeaderName::CallId => all_consuming(parser::headers::parse_call_id)(&value_bytes).map(|(_, v)| TypedHeader::CallId(v)),
            HeaderName::CSeq => all_consuming(parser::headers::parse_cseq)(&value_bytes).map(|(_, v)| TypedHeader::CSeq(v)),
            
            // Content Negotiation Headers
            HeaderName::Accept => all_consuming(parser::headers::parse_accept)(&value_bytes).map(|(_, v)| TypedHeader::Accept(v)),
            HeaderName::ContentType => all_consuming(parser::headers::parse_content_type)(&value_bytes).map(|(_, v)| TypedHeader::ContentType(v)),
            HeaderName::ContentLength => all_consuming(parser::headers::parse_content_length)(&value_bytes).map(|(_, v)| TypedHeader::ContentLength(types::ContentLength(v))),
            HeaderName::ContentDisposition => all_consuming(parser::headers::parse_content_disposition)(&value_bytes).map(|(_, v)| TypedHeader::ContentDisposition(v)),
            HeaderName::ContentEncoding => all_consuming(parser::headers::parse_content_encoding)(&value_bytes).map(|(_, v)| TypedHeader::ContentEncoding(v)), // Placeholder type
            HeaderName::ContentLanguage => all_consuming(parser::headers::parse_content_language)(&value_bytes).map(|(_, v)| TypedHeader::ContentLanguage(v)), // Placeholder type
            HeaderName::AcceptEncoding => all_consuming(parser::headers::parse_accept_encoding)(&value_bytes).map(|(_, v)| TypedHeader::AcceptEncoding(v)), // Placeholder type
            HeaderName::AcceptLanguage => all_consuming(parser::headers::parse_accept_language)(&value_bytes).map(|(_, v)| TypedHeader::AcceptLanguage(v)), // Placeholder type

            // Simple Value Headers
            HeaderName::MaxForwards => all_consuming(parser::headers::parse_max_forwards)(&value_bytes).map(|(_, v)| TypedHeader::MaxForwards(types::MaxForwards(v))),
            HeaderName::Expires => all_consuming(parser::headers::parse_expires)(&value_bytes).map(|(_, v)| TypedHeader::Expires(types::Expires(v))),
            HeaderName::MinExpires => all_consuming(parser::headers::parse_min_expires)(&value_bytes).map(|(_, v)| TypedHeader::MinExpires(v)), // Placeholder type
            HeaderName::MimeVersion => all_consuming(parser::headers::parse_mime_version)(&value_bytes).map(|(_, v)| TypedHeader::MimeVersion(v)), // Placeholder type

            // Auth Headers
            HeaderName::WwwAuthenticate => all_consuming(parser::headers::parse_www_authenticate)(&value_bytes).map(|(_, v)| TypedHeader::WwwAuthenticate(v)),
            HeaderName::Authorization => all_consuming(parser::headers::parse_authorization)(&value_bytes).map(|(_, v)| TypedHeader::Authorization(v)),
            HeaderName::ProxyAuthenticate => all_consuming(parser::headers::parse_proxy_authenticate)(&value_bytes).map(|(_, v)| TypedHeader::ProxyAuthenticate(v)),
            HeaderName::ProxyAuthorization => all_consuming(parser::headers::parse_proxy_authorization)(&value_bytes).map(|(_, v)| TypedHeader::ProxyAuthorization(v)),
            HeaderName::AuthenticationInfo => all_consuming(parser::headers::parse_authentication_info)(&value_bytes).map(|(_, v)| TypedHeader::AuthenticationInfo(v)),

            // Token List Headers
            HeaderName::Allow => all_consuming(parser::headers::parse_allow)(&value_bytes).map(|(_, v)| TypedHeader::Allow(types::Allow(v))),
            HeaderName::Require => all_consuming(parser::headers::parse_require)(&value_bytes).map(|(_, v)| TypedHeader::Require(v)), // Placeholder type
            HeaderName::Supported => all_consuming(parser::headers::parse_supported)(&value_bytes).map(|(_, v)| TypedHeader::Supported(v)), // Placeholder type
            HeaderName::Unsupported => all_consuming(parser::headers::parse_unsupported)(&value_bytes).map(|(_, v)| TypedHeader::Unsupported(v)), // Placeholder type
            HeaderName::ProxyRequire => all_consuming(parser::headers::parse_proxy_require)(&value_bytes).map(|(_, v)| TypedHeader::ProxyRequire(v)), // Placeholder type

            // Miscellaneous Headers
            HeaderName::Date => all_consuming(parser::headers::parse_date)(&value_bytes).map(|(_, v)| TypedHeader::Date(v)), // Placeholder type
            HeaderName::Timestamp => all_consuming(parser::headers::parse_timestamp)(&value_bytes).map(|(_, v)| TypedHeader::Timestamp(v)), // Placeholder type
            HeaderName::Organization => all_consuming(parser::headers::parse_organization)(&value_bytes).map(|(_, v)| TypedHeader::Organization(v)), // Placeholder type
            HeaderName::Priority => all_consuming(parser::headers::parse_priority)(&value_bytes).map(|(_, v)| TypedHeader::Priority(v)), // Placeholder type
            HeaderName::Subject => all_consuming(parser::headers::parse_subject)(&value_bytes).map(|(_, v)| TypedHeader::Subject(v)), // Placeholder type
            HeaderName::Server => all_consuming(parser::headers::parse_server)(&value_bytes).map(|(_, v)| TypedHeader::Server(v)), // Placeholder type
            HeaderName::UserAgent => all_consuming(parser::headers::parse_user_agent)(&value_bytes).map(|(_, v)| TypedHeader::UserAgent(v)), // Placeholder type
            HeaderName::InReplyTo => all_consuming(parser::headers::parse_in_reply_to)(&value_bytes).map(|(_, v)| TypedHeader::InReplyTo(v)), // Assuming InReplyTo type holds Vec<String>
            HeaderName::Warning => all_consuming(parser::headers::parse_warning)(&value_bytes).map(|(_, v)| {
                // Assuming parse_warning returns Ok((rest, (code, agent_uri_bytes, text_bytes)))
                // Need to parse agent_uri_bytes to Uri and text_bytes to String
                // THIS IS LIKELY INCORRECT AND NEEDS REFINEMENT.
                let (code, agent_uri_bytes, text_bytes) = v; // Placeholder destructuring
                let agent = Uri::from_str(std::str::from_utf8(agent_uri_bytes)?).map_err(|e| Error::ParseError(format!("Invalid agent URI in Warning: {}", e)))?;
                let text = String::from_utf8(text_bytes.to_vec())?;
                TypedHeader::Warning(types::Warning { code, agent, text })
            }),
            HeaderName::RetryAfter => all_consuming(parser::headers::parse_retry_after)(&value_bytes).map(|(_, v)| TypedHeader::RetryAfter(v)), // Placeholder type
            HeaderName::ErrorInfo => all_consuming(parser::headers::parse_error_info)(&value_bytes).map(|(_, v)| TypedHeader::ErrorInfo(v)), // Placeholder type
            HeaderName::AlertInfo => all_consuming(parser::headers::parse_alert_info)(&value_bytes).map(|(_, v)| TypedHeader::AlertInfo(v)), // Placeholder type
            HeaderName::CallInfo => all_consuming(parser::headers::parse_call_info)(&value_bytes).map(|(_, v)| TypedHeader::CallInfo(v)), // Placeholder type
            
            // Fallback for Other/Unimplemented
            _ => Ok(TypedHeader::Other(header.name.clone(), HeaderValue::Raw(value_bytes))), // Return Raw if unknown
        };
        
        // Map nom error to crate::Error
        parse_result.map_err(|e| {
            Error::ParseError(
                format!("Failed to parse header '{:?}' value: {:?}", header.name, e)
            )
        })
    }
}

/// Trait for typed headers
pub trait TypedHeaderTrait: Sized {
    /// Type of header name
    type Name: Into<HeaderName> + Copy;
    
    /// Header name
    fn header_name() -> Self::Name;
    
    /// Convert to an untyped Header
    fn to_header(&self) -> Header;
    
    /// Try to convert from an untyped Header
    fn from_header(header: &Header) -> Result<Self>;
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