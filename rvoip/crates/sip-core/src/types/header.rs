use crate::error::{Error, Result};
use crate::types; // Import the types module itself
use crate::parser; // Import the parser module
use std::convert::TryFrom;
use nom::combinator::all_consuming;
use ordered_float::NotNan;
use chrono::DateTime; // Import DateTime specifically
use chrono::FixedOffset; // Import FixedOffset
use std::fmt;
use std::str::FromStr;
use std::string::FromUtf8Error; // Import FromUtf8Error

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::param::Param;
use crate::types::uri::Uri; // Import Uri
use crate::types::uri::Scheme; // Import Scheme
use crate::types::address::Address; // Add explicit import for Address
use crate::types::contact::{Contact, ContactValue as TypesContactValue}; // Import Contact
use crate::types::from::From as FromHeaderValue; // Rename From to avoid conflict
use crate::types::to::To as ToHeaderValue; // Rename To to avoid conflict
use crate::types::route::Route;
use crate::parser::headers::route::RouteEntry; // Import RouteEntry from parser
use crate::types::record_route::RecordRoute;
use crate::parser::headers::record_route::RecordRouteEntry; // Import RecordRouteEntry from parser
use crate::types::via::Via;
use crate::parser::headers::via::ViaHeader as ViaEntry; // Use ViaHeader from parser as ViaEntry
use crate::types::cseq::CSeq;
use crate::types::call_id::CallId;
use crate::types::content_length::ContentLength;
use crate::types::content_type::ContentType;
use crate::parser::headers::content_type::ContentTypeValue; // Import directly from parser
use crate::types::expires::Expires;
use crate::types::max_forwards::MaxForwards;
use crate::types::allow::Allow;
use crate::types::accept::Accept;
use crate::parser::headers::accept::AcceptValue; // Import directly from parser
use crate::types::auth::{Authorization, WwwAuthenticate, ProxyAuthenticate, ProxyAuthorization, AuthenticationInfo};
use crate::types::reply_to::ReplyTo;
use crate::parser::headers::reply_to::ReplyToValue; // Import from parser
use crate::types::warning::Warning;
use crate::types::content_disposition::{ContentDisposition, DispositionType}; // Import ContentDisposition
use crate::types::method::Method; // Needed for Allow parsing
use crate::types::priority::Priority; // Import Priority type
use crate::parser::headers::content_type::parse_content_type_value;
use crate::types::retry_after::RetryAfter;
// CSeqValue doesn't seem to exist, CSeq struct is used directly
// use crate::types::cseq::CSeqValue;
use crate::parser::headers::accept_encoding::EncodingInfo as AcceptEncodingValue; // Use EncodingInfo from parser
use crate::parser::headers::accept_language::LanguageInfo as AcceptLanguageValue; // Use LanguageInfo from parser
use crate::parser::headers::alert_info::AlertInfoValue; // Keep parser type if no types::* yet
use crate::parser::headers::call_info::CallInfoValue; // Keep parser type if no types::* yet
use crate::parser::headers::error_info::ErrorInfoValue; // Keep parser type if no types::* yet
use crate::types::refer_to::ReferTo; // Add ReferTo import
use crate::parser::headers::refer_to::ReferToValue; // Import from parser

// Helper From implementation for Error
impl From<FromUtf8Error> for Error {
    fn from(err: FromUtf8Error) -> Self {
        Error::ParseError(format!("UTF-8 Error: {}", err))
    }
}

// Helper From implementation for Nom errors (if not already covered)
// Add necessary From impls if Error::from doesn't handle all Nom errors used
// impl<I> From<nom::Err<nom::error::Error<I>>> for Error where I: std::fmt::Debug {
//     fn from(err: nom::Err<nom::error::Error<I>>) -> Self {
//         Error::ParseError(format!("Nom Parse Error: {:?}", err))
//     }
// }

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HeaderValue {
    // === Address Headers ===
    Contact(TypesContactValue), // Use imported type
    From(FromHeaderValue),
    To(ToHeaderValue),
    Route(Vec<RouteEntry>),
    RecordRoute(Vec<RecordRouteEntry>),
    ReplyTo(ReplyToValue),
    ReferTo(ReferToValue), // Add ReferTo variant

    // === Request/Response Info ===
    Via(Vec<ViaEntry>), // ViaEntry would contain the parsed tuple
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
    AcceptEncoding(Vec<AcceptEncodingValue>),
    AcceptLanguage(Vec<AcceptLanguageValue>),

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
    CallInfo(Vec<CallInfoValue>),
    ErrorInfo(Vec<ErrorInfoValue>),

    // === Misc ===
    Organization(Option<Vec<u8>>),
    Priority(Vec<u8>),
    Subject(Option<Vec<u8>>),
    Server(Vec<(Option<(Vec<u8>, Option<Vec<u8>>)>, Option<Vec<u8>>)>), // Vec<(Product?, Comment?)>
    UserAgent(Vec<(Option<(Vec<u8>, Option<Vec<u8>>)>, Option<Vec<u8>>)>), // Vec<(Product?, Comment?)>
    InReplyTo(Vec<String>),

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] // Add Serialize, Deserialize
pub enum TypedHeader {
    // Core Headers (Examples)
    Via(Via), // Use types::Via
    From(FromHeaderValue), // Use types::From alias
    To(ToHeaderValue), // Use types::To alias
    Contact(Contact), // Use types::Contact
    CallId(CallId), // Use types::CallId
    CSeq(CSeq), // Use types::CSeq
    Route(Route), // Use types::Route
    RecordRoute(RecordRoute), // Use types::RecordRoute
    MaxForwards(MaxForwards), // Use types::MaxForwards
    ContentType(ContentType), // Use types::ContentType
    ContentLength(ContentLength), // Use types::ContentLength
    Expires(Expires), // Use types::Expires

    // Auth Headers
    Authorization(Authorization),
    WwwAuthenticate(WwwAuthenticate),
    ProxyAuthenticate(ProxyAuthenticate),
    ProxyAuthorization(ProxyAuthorization),
    AuthenticationInfo(AuthenticationInfo),

    // Add other typed headers here as they are defined...
    Accept(Accept), // Use types::Accept
    Allow(Allow), // Use types::Allow
    ReplyTo(ReplyTo), // Use types::ReplyTo
    ReferTo(ReferTo), // Add ReferTo variant
    Warning(Vec<Warning>), // Use types::Warning
    ContentDisposition(ContentDisposition), // Use types::ContentDisposition

    // Placeholder Types (replace with actual types from types/* where available)
    // These might still need Serialize/Deserialize if not using a types::* struct
    ContentEncoding(Vec<String>),
    ContentLanguage(Vec<String>),
    AcceptEncoding(Vec<AcceptEncodingValue>), // Use alias for parser type
    AcceptLanguage(Vec<AcceptLanguageValue>), // Use alias for parser type
    MinExpires(u32), // Assuming types::MinExpires doesn't exist yet
    MimeVersion((u32, u32)), // Keep tuple if no types::* yet
    Require(Vec<String>),
    Supported(Vec<String>),
    Unsupported(Vec<String>),
    ProxyRequire(Vec<String>),
    Date(DateTime<FixedOffset>), // Use imported chrono types
    Timestamp((NotNan<f32>, Option<NotNan<f32>>)), // Use imported NotNan
    Organization(String),
    Priority(String), // Replace with types::priority::PriorityValue when defined
    Subject(String),
    Server(Vec<String>), // Replace with types::server::ServerVal when defined
    UserAgent(Vec<String>), // Replace with types::server::ServerVal when defined
    InReplyTo(Vec<String>),
    RetryAfter(RetryAfter), // Now using types::retry_after::RetryAfter
    ErrorInfo(Vec<crate::parser::headers::error_info::ErrorInfoValue>), // Use imported parser type
    AlertInfo(Vec<crate::parser::headers::alert_info::AlertInfoValue>), // Use imported parser type
    CallInfo(Vec<crate::parser::headers::call_info::CallInfoValue>), // Use imported parser type

    /// Represents an unknown or unparsed header.
    Other(HeaderName, HeaderValue),
}

impl TypedHeader {
    /// Returns the name of the header
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
            TypedHeader::ReferTo(_) => HeaderName::ReferTo, // Add ReferTo variant
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
            TypedHeader::Via(via) => write!(f, "{}: {}", HeaderName::Via, via),
            TypedHeader::From(from) => write!(f, "{}: {}", HeaderName::From, from),
            TypedHeader::To(to) => write!(f, "{}: {}", HeaderName::To, to),
            TypedHeader::Contact(contact) => write!(f, "{}: {}", HeaderName::Contact, contact),
            TypedHeader::CallId(call_id) => write!(f, "{}: {}", HeaderName::CallId, call_id),
            TypedHeader::CSeq(cseq) => write!(f, "{}: {}", HeaderName::CSeq, cseq),
            TypedHeader::Route(route) => write!(f, "{}: {}", HeaderName::Route, route),
            TypedHeader::RecordRoute(record_route) => write!(f, "{}: {}", HeaderName::RecordRoute, record_route),
            TypedHeader::MaxForwards(max_forwards) => write!(f, "{}: {}", HeaderName::MaxForwards, max_forwards),
            TypedHeader::ContentType(content_type) => write!(f, "{}: {}", HeaderName::ContentType, content_type),
            TypedHeader::ContentLength(content_length) => write!(f, "{}: {}", HeaderName::ContentLength, content_length),
            TypedHeader::Expires(expires) => write!(f, "{}: {}", HeaderName::Expires, expires),
            TypedHeader::Authorization(auth) => write!(f, "{}: {}", HeaderName::Authorization, auth),
            TypedHeader::WwwAuthenticate(www_auth) => write!(f, "{}: {}", HeaderName::WwwAuthenticate, www_auth),
            TypedHeader::ProxyAuthenticate(proxy_auth) => write!(f, "{}: {}", HeaderName::ProxyAuthenticate, proxy_auth),
            TypedHeader::ProxyAuthorization(proxy_auth) => write!(f, "{}: {}", HeaderName::ProxyAuthorization, proxy_auth),
            TypedHeader::AuthenticationInfo(auth_info) => write!(f, "{}: {}", HeaderName::AuthenticationInfo, auth_info),
            TypedHeader::Accept(accept) => write!(f, "{}: {}", HeaderName::Accept, accept),
            TypedHeader::Allow(allow) => write!(f, "{}: {}", HeaderName::Allow, allow),
            TypedHeader::ReplyTo(reply_to) => write!(f, "{}: {}", HeaderName::ReplyTo, reply_to),
            TypedHeader::ReferTo(refer_to) => write!(f, "{}: {}", HeaderName::ReferTo, refer_to),
            TypedHeader::Warning(warnings) => {
                write!(f, "{}: ", HeaderName::Warning)?;
                for (i, warning) in warnings.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", warning)?;
                }
                Ok(())
            },
            TypedHeader::ContentDisposition(content_disposition) => write!(f, "{}: {}", HeaderName::ContentDisposition, content_disposition),
            TypedHeader::ContentEncoding(content_encoding) => {
                write!(f, "{}: ", HeaderName::ContentEncoding)?;
                for (i, encoding) in content_encoding.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", encoding)?;
                }
                Ok(())
            },
            TypedHeader::ContentLanguage(content_language) => {
                write!(f, "{}: ", HeaderName::ContentLanguage)?;
                for (i, language) in content_language.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", language)?;
                }
                Ok(())
            },
            TypedHeader::AcceptEncoding(accept_encoding) => {
                write!(f, "{}: ", HeaderName::AcceptEncoding)?;
                for (i, encoding) in accept_encoding.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", encoding)?;
                }
                Ok(())
            },
            TypedHeader::AcceptLanguage(accept_language) => {
                write!(f, "{}: ", HeaderName::AcceptLanguage)?;
                for (i, language) in accept_language.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", language)?;
                }
                Ok(())
            },
            TypedHeader::MinExpires(min_expires) => write!(f, "{}: {}", HeaderName::MinExpires, min_expires),
            TypedHeader::MimeVersion(mime_version) => write!(f, "{}: {:?}", HeaderName::MimeVersion, mime_version),
            TypedHeader::Require(require) => {
                write!(f, "{}: ", HeaderName::Require)?;
                for (i, requirement) in require.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", requirement)?;
                }
                Ok(())
            },
            TypedHeader::Supported(supported) => {
                write!(f, "{}: ", HeaderName::Supported)?;
                for (i, feature) in supported.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", feature)?;
                }
                Ok(())
            },
            TypedHeader::Unsupported(unsupported) => {
                write!(f, "{}: ", HeaderName::Unsupported)?;
                for (i, feature) in unsupported.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", feature)?;
                }
                Ok(())
            },
            TypedHeader::ProxyRequire(proxy_require) => {
                write!(f, "{}: ", HeaderName::ProxyRequire)?;
                for (i, requirement) in proxy_require.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", requirement)?;
                }
                Ok(())
            },
            TypedHeader::Date(date) => write!(f, "{}: {}", HeaderName::Date, date),
            TypedHeader::Timestamp(timestamp) => write!(f, "{}: {:?}", HeaderName::Timestamp, timestamp),
            TypedHeader::Organization(organization) => write!(f, "{}: {}", HeaderName::Organization, organization),
            TypedHeader::Priority(priority) => write!(f, "{}: {}", HeaderName::Priority, priority),
            TypedHeader::Subject(subject) => write!(f, "{}: {}", HeaderName::Subject, subject),
            TypedHeader::Server(server) => {
                write!(f, "{}: ", HeaderName::Server)?;
                for (i, server_val) in server.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", server_val)?;
                }
                Ok(())
            },
            TypedHeader::UserAgent(user_agent) => {
                write!(f, "{}: ", HeaderName::UserAgent)?;
                for (i, agent) in user_agent.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", agent)?;
                }
                Ok(())
            },
            TypedHeader::InReplyTo(in_reply_to) => {
                write!(f, "{}: ", HeaderName::InReplyTo)?;
                for (i, reply) in in_reply_to.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", reply)?;
                }
                Ok(())
            },
            TypedHeader::RetryAfter(retry_after) => write!(f, "{}: {:?}", HeaderName::RetryAfter, retry_after),
            TypedHeader::ErrorInfo(error_info) => {
                write!(f, "{}: ", HeaderName::ErrorInfo)?;
                for (i, info) in error_info.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", info)?;
                }
                Ok(())
            },
            TypedHeader::AlertInfo(alert_info) => {
                write!(f, "{}: ", HeaderName::AlertInfo)?;
                for (i, info) in alert_info.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", info)?;
                }
                Ok(())
            },
            TypedHeader::CallInfo(call_info) => {
                write!(f, "{}: ", HeaderName::CallInfo)?;
                for (i, info) in call_info.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", info)?;
                }
                Ok(())
            },
            TypedHeader::Other(name, value) => write!(f, "{}: {}", name, value),
        }
    }
}

impl TryFrom<Header> for TypedHeader {
    type Error = Error;

    fn try_from(header: Header) -> Result<Self> {
        // We need the unfolded, raw value bytes here.
        // The message_header parser now puts Vec<u8> into HeaderValue::Raw.
        let value_bytes = match &header.value { // Borrow header.value
            HeaderValue::Raw(bytes) => bytes, // Use the raw, unfolded bytes
            _ => return Ok(TypedHeader::Other(header.name.clone(), header.value.clone())), // Should not happen if message_header is used
        };
        
        // Use all_consuming to ensure the specific parser consumes the entire value
        let parse_result = match &header.name {
            // Address Headers
            HeaderName::From => all_consuming(parser::headers::parse_from)(value_bytes)
                .map_err(Error::from)
                .map(|(_, addr)| TypedHeader::From(addr)),
            HeaderName::To => all_consuming(parser::headers::parse_to)(value_bytes)
                .map_err(Error::from)
                .map(|(_, addr)| TypedHeader::To(addr)),
            HeaderName::Contact => all_consuming(parser::headers::parse_contact)(value_bytes)
                .map_err(Error::from)
                .map(|(_, v)| TypedHeader::Contact(Contact(vec![v]))),
            HeaderName::ReplyTo => {
                match all_consuming(parser::headers::reply_to::parse_reply_to)(value_bytes) {
                    Ok((_, addr)) => Ok(TypedHeader::ReplyTo(addr)),
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            }

            // Routing Headers
            HeaderName::Via => all_consuming(parser::headers::parse_via)(value_bytes)
                .map(|(_, v)| TypedHeader::Via(Via(v)))
                .map_err(Error::from),
            HeaderName::Route => all_consuming(parser::headers::parse_route)(value_bytes)
                .map(|(_, v)| TypedHeader::Route(v))
                .map_err(Error::from),
            HeaderName::RecordRoute => all_consuming(parser::headers::parse_record_route)(value_bytes)
                .map(|(_, v)| TypedHeader::RecordRoute(v))
                .map_err(Error::from),

            // Dialog/Transaction IDs
            HeaderName::CallId => {
                match all_consuming(parser::headers::parse_call_id)(value_bytes) {
                    Ok((_, call_id)) => Ok(TypedHeader::CallId(call_id)),
                    Err(e) => Err(Error::from(e)),
                }
            }

            HeaderName::CSeq => all_consuming(parser::headers::parse_cseq)(value_bytes)
                .map(|(_, cseq_struct)| TypedHeader::CSeq(cseq_struct))
                .map_err(Error::from),

            // Content Negotiation Headers
            HeaderName::Accept => all_consuming(parser::headers::accept::parse_accept)(value_bytes)
                .map(|(_, v)| TypedHeader::Accept(v))
                .map_err(Error::from),
            HeaderName::ContentType => all_consuming(parse_content_type_value)(value_bytes)
                .map(|(_, v)| TypedHeader::ContentType(ContentType(v)))
                .map_err(Error::from),
            HeaderName::ContentLength => all_consuming(parser::headers::parse_content_length)(value_bytes)
                .map_err(Error::from)
                .and_then(|(_, v_u64)| {
                    let length = v_u64.try_into().map_err(|_| Error::ParseError("Invalid Content-Length value (overflow)".into()))?;
                    Ok(TypedHeader::ContentLength(ContentLength(length)))
                }),
            HeaderName::ContentDisposition => {
                match all_consuming(parser::headers::content_disposition::parse_content_disposition)(value_bytes) {
                    Ok((_, (disp_type_bytes, params_vec))) => {
                        let disposition_type = DispositionType::from_str(&disp_type_bytes)?;
                        let params: HashMap<String, String> = HashMap::new(); // Simplified for now
                        Ok(TypedHeader::ContentDisposition(ContentDisposition { disposition_type, params }))
                    },
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            }
            HeaderName::ContentEncoding => all_consuming(parser::headers::parse_content_encoding)(value_bytes)
                .map(|(_, strings)| TypedHeader::ContentEncoding(strings))
                .map_err(Error::from),
            HeaderName::ContentLanguage => all_consuming(parser::headers::parse_content_language)(value_bytes)
                .map(|(_, strings)| TypedHeader::ContentLanguage(strings))
                .map_err(Error::from),
            HeaderName::AcceptEncoding => all_consuming(parser::headers::parse_accept_encoding)(value_bytes)
                .map(|(_, v)| TypedHeader::AcceptEncoding(v))
                .map_err(Error::from),
            HeaderName::AcceptLanguage => all_consuming(parser::headers::parse_accept_language)(value_bytes)
                .map(|(_, v)| TypedHeader::AcceptLanguage(v))
                .map_err(Error::from),

            // Simple Value Headers
            HeaderName::MaxForwards => all_consuming(parser::headers::parse_max_forwards)(value_bytes)
                 .map_err(Error::from)
                 .and_then(|(_, v_u32)| {
                    let forwards = v_u32.try_into().map_err(|_| Error::ParseError("Invalid Max-Forwards value (overflow)".into()))?;
                    Ok(TypedHeader::MaxForwards(MaxForwards(forwards)))
                 }),
            HeaderName::Expires => all_consuming(parser::headers::parse_expires)(value_bytes)
                .map(|(_, v)| TypedHeader::Expires(Expires(v)))
                .map_err(Error::from),
            HeaderName::MinExpires => all_consuming(parser::headers::parse_min_expires)(value_bytes)
                .map(|(_, v)| TypedHeader::MinExpires(v))
                .map_err(Error::from),
            HeaderName::MimeVersion => all_consuming(parser::headers::parse_mime_version)(value_bytes)
                .map(|(_, v)| TypedHeader::MimeVersion((v.major.into(), v.minor.into())))
                .map_err(Error::from),

            // Auth Headers (Assuming parsers return appropriate structs/values)
            HeaderName::WwwAuthenticate => all_consuming(parser::headers::parse_www_authenticate)(value_bytes)
                .map(|(_, v)| TypedHeader::WwwAuthenticate(WwwAuthenticate(v)))
                .map_err(Error::from),
            HeaderName::Authorization => all_consuming(parser::headers::parse_authorization)(value_bytes)
                .map(|(_, v)| TypedHeader::Authorization(v))
                .map_err(Error::from),
            HeaderName::ProxyAuthenticate => all_consuming(parser::headers::parse_proxy_authenticate)(value_bytes)
                .map(|(_, v)| TypedHeader::ProxyAuthenticate(ProxyAuthenticate(v)))
                .map_err(Error::from),
            HeaderName::ProxyAuthorization => all_consuming(parser::headers::parse_proxy_authorization)(value_bytes)
                .map(|(_, v)| TypedHeader::ProxyAuthorization(ProxyAuthorization(v)))
                .map_err(Error::from),
            HeaderName::AuthenticationInfo => all_consuming(parser::headers::parse_authentication_info)(value_bytes)
                .map(|(_, v)| TypedHeader::AuthenticationInfo(AuthenticationInfo(v)))
                .map_err(Error::from),

            // Token List Headers
            HeaderName::Allow => all_consuming(parser::headers::allow::parse_allow)(value_bytes)
                .map(|(_, allow)| TypedHeader::Allow(allow))
                .map_err(Error::from),
            HeaderName::Require => all_consuming(parser::headers::parse_require)(value_bytes)
                .map(|(_, strings)| TypedHeader::Require(strings))
                .map_err(Error::from),
            HeaderName::Supported => all_consuming(parser::headers::parse_supported)(value_bytes)
                .map(|(_, strings)| TypedHeader::Supported(strings))
                .map_err(Error::from),
            HeaderName::Unsupported => all_consuming(parser::headers::parse_unsupported)(value_bytes)
                .map(|(_, strings)| TypedHeader::Unsupported(strings))
                .map_err(Error::from),
            HeaderName::ProxyRequire => all_consuming(parser::headers::parse_proxy_require)(value_bytes)
                .map(|(_, strings)| TypedHeader::ProxyRequire(strings))
                .map_err(Error::from),

            // Miscellaneous Headers
            HeaderName::Date => all_consuming(parser::headers::parse_date)(value_bytes)
                .map(|(_, v)| TypedHeader::Date(v))
                .map_err(Error::from),
            HeaderName::Timestamp => all_consuming(parser::headers::parse_timestamp)(value_bytes)
                .map(|(_, v)| TypedHeader::Timestamp(v))
                .map_err(Error::from),
            HeaderName::Organization => all_consuming(parser::headers::parse_organization)(value_bytes)
                .map(|(_, string)| TypedHeader::Organization(string))
                .map_err(Error::from),
            HeaderName::Priority => all_consuming(parser::headers::parse_priority)(value_bytes)
                .map(|(_, priority)| TypedHeader::Priority(priority.to_string()))
                .map_err(Error::from),
            HeaderName::Subject => all_consuming(parser::headers::parse_subject)(value_bytes)
                .map(|(_, string)| TypedHeader::Subject(string))
                .map_err(Error::from),
            HeaderName::Server => all_consuming(parser::headers::parse_server)(value_bytes)
                 .map(|(_, server_vals)| TypedHeader::Server(server_vals.into_iter()
                     .map(|server_val| match server_val {
                         types::server::ServerVal::Product(product) => {
                             format!("{}{}", product.name, product.version.map_or_else(String::new, |v| format!("/{}", v)))
                         },
                         types::server::ServerVal::Comment(comment) => {
                             format!("({})", comment)
                         }
                     })
                     .collect::<Vec<String>>()))
                 .map_err(Error::from),
            HeaderName::UserAgent => all_consuming(parser::headers::parse_user_agent)(value_bytes)
                 .map(|(_, server_vals)| TypedHeader::UserAgent(server_vals.into_iter()
                     .map(|server_val| match server_val {
                         types::server::ServerVal::Product(product) => {
                             format!("{}{}", product.name, product.version.map_or_else(String::new, |v| format!("/{}", v)))
                         },
                         types::server::ServerVal::Comment(comment) => {
                             format!("({})", comment)
                         }
                     })
                     .collect::<Vec<String>>()))
                 .map_err(Error::from),
            HeaderName::InReplyTo => all_consuming(parser::headers::parse_in_reply_to)(value_bytes)
                .map(|(_, strings)| TypedHeader::InReplyTo(strings))
                .map_err(Error::from),
             HeaderName::Warning => {
                 let parse_result = all_consuming(parser::headers::warning::parse_warning_value_list)(value_bytes);
                 match parse_result {
                     Ok((_, parsed_values)) => {
                         let mut typed_warnings = Vec::new();
                         for parsed_value in parsed_values {
                             let agent_uri = match parsed_value.agent {
                                 parser::headers::warning::WarnAgent::HostPort(host, port_opt) => {
                                     Uri::new(Scheme::Sip, host)
                                 },
                                 parser::headers::warning::WarnAgent::Pseudonym(bytes) => {
                                     let host_str = String::from_utf8_lossy(&bytes);
                                     match types::uri::Host::from_str(&host_str) {
                                          Ok(host) => Uri::new(Scheme::Sip, host),
                                          Err(_) => {
                                              return Err(Error::ParseError(format!("Cannot represent warning agent pseudonym '{}' as a valid host for Uri", host_str)));
                                          }
                                     }
                                 }
                             };

                             let text_string = String::from_utf8(parsed_value.text.to_vec())
                                 .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in warning text: {}", e)))?;

                             typed_warnings.push(Warning::new(parsed_value.code, agent_uri, text_string));
                         }
                         Ok(TypedHeader::Warning(typed_warnings))
                     },
                     Err(e) => Err(Error::from(e.to_owned())),
                 }
             },
            HeaderName::RetryAfter => {
                // Parse the RetryAfter header
                parser::headers::retry_after::parse_retry_after(value_bytes)
                    .map_err(Error::from)
                    .and_then(|(_, ra_value)| {
                        let delay = ra_value.delay;
                        let comment = ra_value.comment;
                        
                        // Convert params from RetryParam to HashMap entries
                        let mut parameters = HashMap::new();
                        for param in ra_value.params {
                            match param {
                                crate::parser::headers::retry_after::RetryParam::Duration(d) => {
                                    parameters.insert("duration".to_string(), d.to_string());
                                },
                                crate::parser::headers::retry_after::RetryParam::Generic(Param::Other(name, Some(value))) => {
                                    parameters.insert(name, value.to_string());
                                },
                                _ => {} // Ignore other param types
                            }
                        }
                        
                        Ok(TypedHeader::RetryAfter(RetryAfter {
                            delay,
                            comment,
                            parameters,
                        }))
                    })
            },
            HeaderName::ErrorInfo => all_consuming(parser::headers::parse_error_info)(value_bytes)
                .map(|(_, error_info_values)| TypedHeader::ErrorInfo(error_info_values))
                .map_err(Error::from),
            HeaderName::AlertInfo => all_consuming(parser::headers::parse_alert_info)(value_bytes)
                .map(|(_, alert_info_values)| TypedHeader::AlertInfo(alert_info_values))
                .map_err(Error::from),
            HeaderName::CallInfo => all_consuming(parser::headers::parse_call_info)(value_bytes)
                .map(|(_, call_info_values)| TypedHeader::CallInfo(call_info_values))
                .map_err(Error::from),
            HeaderName::ReferTo => {
                match header.value {
                    HeaderValue::ReferTo(value) => {
                        // Extract parts needed to create the ReferTo type
                        let display_name = value.display_name;
                        let uri = value.uri;
                        let params = value.params;
                        
                        // Build Address and ReferTo
                        let address = crate::types::address::Address::new(display_name, uri);
                        Ok(TypedHeader::ReferTo(ReferTo(address)))
                    }
                    _ => Err(Error::ParseError(format!("Invalid header value for Refer-To header")))
                }
            }

            // Fallback for Other/Unimplemented
            _ => Ok(TypedHeader::Other(header.name.clone(), HeaderValue::Raw(value_bytes.to_vec()))),
        };

        parse_result
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