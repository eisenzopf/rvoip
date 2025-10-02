use crate::error::{Error, Result};
use std::fmt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;
use nom::combinator::all_consuming;
use ordered_float::NotNan;
use chrono::{DateTime, FixedOffset};
use std::str::FromStr;
use log::debug;
use std::any::Any;

// Add missing imports needed for implementation
extern crate log;

// Import header components
use crate::types::headers::header_name::HeaderName;
use crate::types::headers::header_value::HeaderValue;
use crate::types::headers::header::Header;

// Import types used in the enum variants
use crate::types::via::{Via, ViaHeader};
use crate::types::from::From as FromHeaderValue; 
use crate::types::to::To as ToHeaderValue;
use crate::types::contact::Contact;
use crate::types::call_id::CallId;
use crate::types::cseq::CSeq;
use crate::types::route::Route;
use crate::types::record_route::RecordRoute;
use crate::types::max_forwards::MaxForwards;
use crate::types::content_type::ContentType;
use crate::types::content_length::ContentLength;
use crate::types::expires::Expires;
use crate::types::auth::{Authorization, WwwAuthenticate, ProxyAuthenticate, ProxyAuthorization, AuthenticationInfo};
use crate::types::accept::Accept;
use crate::types::allow::Allow;
use crate::types::reply_to::ReplyTo;
use crate::types::refer_to::ReferTo;
use crate::types::require::Require;
use crate::types::warning::{Warning, WarnAgent};
use crate::types::content_disposition::{ContentDisposition, DispositionType, DispositionParam, Handling};
use crate::types::subject::Subject;
use crate::types::content_encoding::ContentEncoding;
use crate::types::content_language::ContentLanguage;
use crate::types::accept_encoding::AcceptEncoding;
use crate::types::accept_language::AcceptLanguage;
use crate::types::supported::Supported;
use crate::types::unsupported::Unsupported;
use crate::types::retry_after::RetryAfter;
use crate::types::call_info::CallInfo;
use crate::types::param::Param;
use crate::types::address::Address;
use crate::types::uri::{Uri, Scheme};
use crate::prelude::GenericValue;
use crate::types::alert_info::{AlertInfo, AlertInfoHeader, AlertInfoList};
use crate::types::error_info::{ErrorInfo, ErrorInfoHeader, ErrorInfoList};
use crate::types::referred_by::ReferredBy;
use crate::types::session_expires::SessionExpires;
use crate::types::event::{Event as EventTypeData}; // Alias to avoid clash if Event struct is also used locally
use crate::types::subscription_state::SubscriptionState as SubscriptionStateType; // Full type from types::subscription_state
use crate::types::MimeVersion;
use crate::types::min_expires::MinExpires;
use crate::types::min_se::MinSE;
use crate::types::organization::Organization;
use crate::types::rseq::RSeq;

// Import parser components
use crate::parser;
use crate::parser::headers::route::RouteEntry;
use crate::parser::headers::reply_to::ReplyToValue;
use crate::parser::headers::accept::AcceptValue;
use crate::parser::headers::accept_encoding::EncodingInfo;
use crate::parser::headers::alert_info::AlertInfoValue;
use crate::parser::headers::error_info::ErrorInfoValue;
use crate::parser::headers::content_type::parse_content_type_value;
use crate::parser::headers::session_expires::parse_session_expires;

/// A strongly-typed representation of a SIP header.
///
/// This enum provides a type-safe way to work with parsed SIP headers. Each variant
/// corresponds to a specific header type with its appropriately typed value.
///
/// Unlike the more generic [`Header`], which stores headers as name-value pairs,
/// `TypedHeader` stores the header's value in a strongly-typed form, making it safer
/// and more convenient to work with known header types.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::convert::TryFrom;
///
/// // Create a typed Call-ID header
/// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
/// let header = TypedHeader::CallId(call_id);
///
/// // Access the header name
/// assert_eq!(header.name(), HeaderName::CallId);
///
/// // Convert to string representation
/// let header_str = header.to_string();
/// assert!(header_str.contains("Call-ID"));
/// assert!(header_str.contains("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    ReferredBy(ReferredBy), // Add ReferredBy variant
    Require(Require), // Use types::Require
    Warning(Vec<Warning>), // Use types::Warning
    ContentDisposition(ContentDisposition), // Use types::ContentDisposition
    Subject(Subject), // Use types::Subject instead of String
    
    // Add Event and SubscriptionState headers
    Event(EventTypeData), // ADDED Event variant, using alias
    SubscriptionState(SubscriptionStateType), // Proper type from types::subscription_state

    // Placeholder Types (replace with actual types from types/* where available)
    // These might still need Serialize/Deserialize if not using a types::* struct
    ContentEncoding(ContentEncoding),
    ContentLanguage(ContentLanguage),
    AcceptEncoding(AcceptEncoding), // Use our AcceptEncoding type
    AcceptLanguage(AcceptLanguage), // Use our new AcceptLanguage type instead of Vec<AcceptLanguageValue>
    MinExpires(MinExpires),
    MimeVersion(MimeVersion),
    Supported(Supported), // Use types::Supported instead of Vec<String>
    Unsupported(Unsupported), // Use types::Unsupported instead of Vec<String>
    ProxyRequire(crate::types::proxy_require::ProxyRequire),
    Date(DateTime<FixedOffset>), // Use imported chrono types
    Timestamp((NotNan<f32>, Option<NotNan<f32>>)), // Use imported NotNan
    Organization(Organization),
    Priority(crate::types::priority::Priority), // Use types::Priority
    Server(Vec<String>), // Replace with types::server::ServerVal when defined
    UserAgent(Vec<String>), // Replace with types::server::ServerVal when defined
    InReplyTo(crate::types::in_reply_to::InReplyTo),
    RetryAfter(RetryAfter), // Now using types::retry_after::RetryAfter
    ErrorInfo(ErrorInfoHeader), // Use our new ErrorInfoHeader type instead of Vec<ErrorInfoValue>
    AlertInfo(crate::types::alert_info::AlertInfoHeader), // Use our AlertInfoHeader type
    CallInfo(CallInfo), // Use our new CallInfo type
    Path(crate::types::path::Path), // Add Path header variant
    Reason(crate::types::reason::Reason), // Add Reason header variant
    SessionExpires(SessionExpires), // Added SessionExpires variant
    MinSE(MinSE),
    RSeq(crate::types::rseq::RSeq), // Added RSeq variant
    SipETag(crate::types::sip_etag::SipETag), // Added SIP-ETag variant
    SipIfMatch(crate::types::sip_if_match::SipIfMatch), // Added SIP-If-Match variant
    AllowEvents(crate::types::allow_events::AllowEvents), // Added Allow-Events variant

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
            TypedHeader::ReferTo(_) => HeaderName::ReferTo,
            TypedHeader::ReferredBy(_) => HeaderName::ReferredBy,
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
            TypedHeader::Event(_) => HeaderName::Event,
            TypedHeader::SubscriptionState(_) => HeaderName::SubscriptionState,
            TypedHeader::Path(_) => HeaderName::Path, // Add Path header case
            TypedHeader::Reason(_) => HeaderName::Reason, // Add Reason header case
            TypedHeader::SessionExpires(_) => HeaderName::SessionExpires, // Added SessionExpires case
            TypedHeader::MinSE(_) => HeaderName::MinSE,
            TypedHeader::RSeq(_) => HeaderName::RSeq, // Use proper HeaderName enum variant
            TypedHeader::SipETag(_) => HeaderName::SipETag,
            TypedHeader::SipIfMatch(_) => HeaderName::SipIfMatch,
            TypedHeader::AllowEvents(_) => HeaderName::AllowEvents,
            TypedHeader::Other(name, _) => name.clone(),
        }
    }

    /// Try to convert this TypedHeader to a reference of a specific header type
    /// 
    /// This method is used internally by the HeaderAccess trait implementations
    /// to provide type-safe access to headers.
    pub fn as_typed_ref<'a, T: TypedHeaderTrait + 'static>(&'a self) -> Option<&'a T> 
    where 
        <T as TypedHeaderTrait>::Name: std::fmt::Debug,
        T: std::fmt::Debug
    {
        if self.name() != T::header_name().into() {
            return None;
        }
        
        let type_id_t = std::any::TypeId::of::<T>();
        
        match self {
            TypedHeader::CallId(h) if type_id_t == std::any::TypeId::of::<crate::types::CallId>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::From(h) if type_id_t == std::any::TypeId::of::<crate::types::from::From>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::To(h) if type_id_t == std::any::TypeId::of::<crate::types::to::To>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Via(h) if type_id_t == std::any::TypeId::of::<crate::types::via::Via>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::CSeq(h) if type_id_t == std::any::TypeId::of::<crate::types::CSeq>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ContentLength(h) if type_id_t == std::any::TypeId::of::<crate::types::content_length::ContentLength>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::MaxForwards(h) if type_id_t == std::any::TypeId::of::<crate::types::MaxForwards>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Contact(h) if type_id_t == std::any::TypeId::of::<crate::types::contact::Contact>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ContentType(h) if type_id_t == std::any::TypeId::of::<crate::types::content_type::ContentType>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Require(h) if type_id_t == std::any::TypeId::of::<crate::types::require::Require>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Supported(h) if type_id_t == std::any::TypeId::of::<crate::types::supported::Supported>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ContentDisposition(h) if type_id_t == std::any::TypeId::of::<crate::types::content_disposition::ContentDisposition>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::InReplyTo(h) if type_id_t == std::any::TypeId::of::<crate::types::in_reply_to::InReplyTo>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ReplyTo(h) if type_id_t == std::any::TypeId::of::<crate::types::reply_to::ReplyTo>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Reason(h) if type_id_t == std::any::TypeId::of::<crate::types::reason::Reason>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ErrorInfo(h) if type_id_t == std::any::TypeId::of::<crate::types::error_info::ErrorInfoHeader>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::AlertInfo(h) if type_id_t == std::any::TypeId::of::<crate::types::alert_info::AlertInfoHeader>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::CallInfo(h) if type_id_t == std::any::TypeId::of::<crate::types::call_info::CallInfo>() => 
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Expires(h_inner) if type_id_t == std::any::TypeId::of::<crate::types::expires::Expires>() => {
                Some(unsafe { &*(h_inner as *const _ as *const T) })
            }
            TypedHeader::SessionExpires(h) if type_id_t == std::any::TypeId::of::<crate::types::session_expires::SessionExpires>() =>
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::Event(h) if type_id_t == std::any::TypeId::of::<crate::types::event::Event>() =>
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::MinSE(h) if type_id_t == std::any::TypeId::of::<crate::types::min_se::MinSE>() =>
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::RSeq(h) if type_id_t == std::any::TypeId::of::<crate::types::rseq::RSeq>() =>
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ReferTo(h) if type_id_t == std::any::TypeId::of::<crate::types::refer_to::ReferTo>() =>
                Some(unsafe { &*(h as *const _ as *const T) }),
            TypedHeader::ReferredBy(h) if type_id_t == std::any::TypeId::of::<crate::types::referred_by::ReferredBy>() =>
                Some(unsafe { &*(h as *const _ as *const T) }),
            _ => None,
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
            TypedHeader::ReferredBy(referred_by) => write!(f, "{}: {}", HeaderName::ReferredBy, referred_by),
            TypedHeader::Warning(warnings) => {
                write!(f, "{}: ", HeaderName::Warning)?;
                for (i, warning) in warnings.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", warning)?;
                }
                Ok(())
            },
            TypedHeader::ContentDisposition(cd) => write!(f, "{}: {}", HeaderName::ContentDisposition, cd),
            TypedHeader::ContentEncoding(content_encoding) => write!(f, "{}: {}", HeaderName::ContentEncoding, content_encoding),
            TypedHeader::ContentLanguage(content_language) => write!(f, "{}: {}", HeaderName::ContentLanguage, content_language),
            TypedHeader::AcceptEncoding(accept_encoding) => write!(f, "{}: {}", HeaderName::AcceptEncoding, accept_encoding),
            TypedHeader::AcceptLanguage(accept_language) => write!(f, "{}: {}", HeaderName::AcceptLanguage, accept_language),
            TypedHeader::MinExpires(min_expires) => write!(f, "{}: {}", HeaderName::MinExpires, min_expires),
            TypedHeader::MimeVersion(val) => write!(f, "{}: {}", HeaderName::MimeVersion, val),
            TypedHeader::Require(require) => {
                write!(f, "{}: {}", HeaderName::Require, require)
            },
            TypedHeader::Supported(supported) => {
                write!(f, "{}: {}", HeaderName::Supported, supported)
            },
            TypedHeader::Unsupported(unsupported) => {
                write!(f, "{}: {}", HeaderName::Unsupported, unsupported)
            },
            TypedHeader::ProxyRequire(proxy_require) => {
                write!(f, "{}: {}", HeaderName::ProxyRequire, proxy_require)
            },
            TypedHeader::Date(date) => write!(f, "{}: {}", HeaderName::Date, date),
            TypedHeader::Timestamp(timestamp) => {
                // Format the tuple using Display implementation for the (NotNan<f32>, Option<NotNan<f32>>) type
                let (value, delay) = timestamp;
                match delay {
                    Some(d) => write!(f, "{}: {} {}", HeaderName::Timestamp, value, d),
                    None => write!(f, "{}: {}", HeaderName::Timestamp, value)
                }
            },
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
                write!(f, "{}: {}", HeaderName::InReplyTo, in_reply_to)
            },
            TypedHeader::RetryAfter(retry_after) => write!(f, "{}: {}", HeaderName::RetryAfter, retry_after),
            TypedHeader::ErrorInfo(error_info) => {
                write!(f, "{}", error_info)
            },
            TypedHeader::AlertInfo(alert_info) => {
                write!(f, "{}: {}", HeaderName::AlertInfo, alert_info)
            },
            TypedHeader::CallInfo(call_info) => {
                write!(f, "{}", call_info)
            },
            TypedHeader::Event(event_data) => write!(f, "{}: {}", HeaderName::Event, event_data),
            TypedHeader::SubscriptionState(state) => write!(f, "{}: {}", HeaderName::SubscriptionState, state),
            TypedHeader::Path(path) => {
                write!(f, "{}: {}", HeaderName::Path, path)
            },
            TypedHeader::Reason(reason) => write!(f, "{}: {}", HeaderName::Reason, reason),
            TypedHeader::SessionExpires(session_expires) => write!(f, "{}: {}", HeaderName::SessionExpires, session_expires),
            TypedHeader::MinSE(val) => write!(f, "{}: {}", HeaderName::MinSE, val),
            TypedHeader::RSeq(val) => write!(f, "{}: {}", HeaderName::RSeq, val),
            TypedHeader::SipETag(val) => write!(f, "{}: {}", HeaderName::SipETag, val),
            TypedHeader::SipIfMatch(val) => write!(f, "{}: {}", HeaderName::SipIfMatch, val),
            TypedHeader::AllowEvents(val) => write!(f, "{}: {}", HeaderName::AllowEvents, val),
            TypedHeader::Other(name, value) => write!(f, "{}: {}", name, value),
        }
    }
}

/// Trait for header types that can be converted to/from the generic `Header` type.
///
/// This trait should be implemented by all strongly-typed header structs to allow
/// seamless conversion between generic `Header` instances and strongly-typed
/// representations.
///
/// By implementing this trait, a header type can be:
/// - Extracted from a generic `Header` using `from_header`
/// - Converted to a generic `Header` using `to_header`
/// - Identified by its canonical header name
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::convert::TryFrom;
///
/// // Implementing TypedHeaderTrait for a custom header
/// struct MyCustomHeader(String);
///
/// impl TypedHeaderTrait for MyCustomHeader {
///     type Name = HeaderName;
///
///     fn header_name() -> Self::Name {
///         HeaderName::Other("X-Custom".to_string())
///     }
///
///     fn to_header(&self) -> Header {
///         Header::text(Self::header_name(), &self.0)
///     }
///
///     fn from_header(header: &Header) -> Result<Self> {
///         if let HeaderValue::Raw(bytes) = &header.value {
///             if let Ok(s) = std::str::from_utf8(bytes) {
///                 return Ok(MyCustomHeader(s.to_string()));
///             }
///         }
///         Err(Error::InvalidHeader("Not a valid MyCustomHeader".to_string()))
///     }
/// }
///
/// // Using the trait
/// let header = Header::text(HeaderName::Other("X-Custom".to_string()), "test value");
/// let typed = MyCustomHeader::from_header(&header).unwrap();
/// assert_eq!(typed.0, "test value");
/// ```
pub trait TypedHeaderTrait: Sized {
    /// Type of header name
    type Name: Into<HeaderName> + Clone;
    
    /// Header name
    fn header_name() -> Self::Name;
    
    /// Convert to an untyped Header
    fn to_header(&self) -> Header;
    
    /// Try to convert from an untyped Header
    fn from_header(header: &Header) -> Result<Self>;
}

impl From<&TypedHeader> for HeaderName {
    fn from(header: &TypedHeader) -> HeaderName {
        match header {
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
            TypedHeader::Server(_) => HeaderName::Server,
            TypedHeader::UserAgent(_) => HeaderName::UserAgent,
            TypedHeader::InReplyTo(_) => HeaderName::InReplyTo,
            TypedHeader::ContentDisposition(_) => HeaderName::ContentDisposition,
            TypedHeader::Accept(_) => HeaderName::Accept,
            TypedHeader::AcceptLanguage(_) => HeaderName::AcceptLanguage,
            TypedHeader::AcceptEncoding(_) => HeaderName::AcceptEncoding,
            TypedHeader::ContentEncoding(_) => HeaderName::ContentEncoding,
            TypedHeader::ContentLanguage(_) => HeaderName::ContentLanguage,
            TypedHeader::Path(_) => HeaderName::Path,
            TypedHeader::MinSE(_) => HeaderName::MinSE,
            _ => header.name(),
        }
    }
}

// Add TryFrom<Header> implementation for TypedHeader
impl TryFrom<Header> for TypedHeader {
    type Error = Error;

    fn try_from(header: Header) -> Result<Self> {
        // Special case for pre-parsed HeaderValue variants
        match &header.value {
            HeaderValue::Authorization(auth) => {
                // Only process if the header name is correct
                if header.name != HeaderName::Authorization {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the Authorization directly without parsing
                return Ok(TypedHeader::Authorization(auth.clone()));
            },
            HeaderValue::WwwAuthenticate(www_auth) => {
                // Only process if the header name is correct
                if header.name != HeaderName::WwwAuthenticate {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the WwwAuthenticate directly without parsing
                return Ok(TypedHeader::WwwAuthenticate(www_auth.clone()));
            },
            HeaderValue::ProxyAuthenticate(proxy_auth) => {
                // Only process if the header name is correct
                if header.name != HeaderName::ProxyAuthenticate {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the ProxyAuthenticate directly without parsing
                return Ok(TypedHeader::ProxyAuthenticate(proxy_auth.clone()));
            },
            HeaderValue::ProxyAuthorization(proxy_auth) => {
                // Only process if the header name is correct
                if header.name != HeaderName::ProxyAuthorization {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the ProxyAuthorization directly without parsing
                return Ok(TypedHeader::ProxyAuthorization(proxy_auth.clone()));
            },
            HeaderValue::AuthenticationInfo(auth_info) => {
                // Only process if the header name is correct
                if header.name != HeaderName::AuthenticationInfo {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the AuthenticationInfo directly without parsing
                return Ok(TypedHeader::AuthenticationInfo(auth_info.clone()));
            },
            HeaderValue::ReferTo(refer_to) => {
                // Only process if the header name is correct
                if header.name != HeaderName::ReferTo {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the ReferTo directly without parsing
                return Ok(TypedHeader::ReferTo(refer_to.clone()));
            },
            HeaderValue::ReferredBy(referred_by) => {
                // Only process if the header name is correct
                if header.name != HeaderName::ReferredBy {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the ReferredBy directly without parsing
                return Ok(TypedHeader::ReferredBy(referred_by.clone()));
            },
            HeaderValue::ContentDisposition((disp_type_bytes, params_vec)) => {
                if header.name != HeaderName::ContentDisposition {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Convert bytes to string
                let disp_type_str = match std::str::from_utf8(disp_type_bytes) {
                    Ok(s) => s.to_string(),
                    Err(_) => return Ok(TypedHeader::Other(header.name.clone(), header.value.clone())),
                };
                
                // Parse disposition type
                let disposition_type = match disp_type_str.to_lowercase().as_str() {
                    "session" => DispositionType::Session,
                    "render" => DispositionType::Render,
                    "icon" => DispositionType::Icon,
                    "alert" => DispositionType::Alert,
                    _ => DispositionType::Other(disp_type_str),
                };
                
                // Convert params to HashMap
                let mut params = HashMap::new();
                for param in params_vec {
                    match param {
                        Param::Other(name, Some(GenericValue::Token(value))) => {
                            params.insert(name.clone(), value.clone());
                        },
                        Param::Other(name, Some(GenericValue::Quoted(value))) => {
                            params.insert(name.clone(), value.clone());
                        },
                        Param::Other(name, None) => {
                            // Flag parameter without value
                            params.insert(name.clone(), "".to_string());
                        },
                        _ => {} // Skip other parameter types
                    }
                }
                
                return Ok(TypedHeader::ContentDisposition(ContentDisposition {
                    disposition_type,
                    params,
                }));
            },
            HeaderValue::Accept(values) => {
                // Only process if the header name is correct
                if header.name != HeaderName::Accept {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Convert vector of AcceptValue to Accept
                return Ok(TypedHeader::Accept(Accept::from_media_types(values.clone())));
            },
            HeaderValue::ContentType(content_type) => {
                // Only process if the header name is correct
                if header.name != HeaderName::ContentType {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the ContentType directly without parsing
                return Ok(TypedHeader::ContentType(content_type.clone()));
            },
            HeaderValue::InReplyTo(in_reply_to) => {
                // Only process if the header name is correct
                if header.name != HeaderName::InReplyTo {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the InReplyTo directly without parsing
                return Ok(TypedHeader::InReplyTo(in_reply_to.clone()));
            },
            HeaderValue::ReplyTo(reply_to) => {
                // Only process if the header name is correct
                if header.name != HeaderName::ReplyTo {
                    return Ok(TypedHeader::Other(header.name.clone(), header.value.clone()));
                }
                
                // Use the ReplyTo directly without parsing
                return Ok(TypedHeader::ReplyTo(reply_to.clone()));
            },
            _ => {} // Continue with normal processing
        }
        
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
            HeaderName::Via => {
                // Try to parse as regular Via header first
                match all_consuming(parser::headers::parse_via)(value_bytes) {
                    Ok((_, v)) => Ok(TypedHeader::Via(Via(v))),
                    Err(e) => {
                        // If that fails, try to parse as just the via-params part (without "Via:" prefix)
                        match parser::headers::via::parse_via_params_public(value_bytes) {
                            Ok((_, v)) => Ok(TypedHeader::Via(Via(v))),
                            Err(_) => Err(Error::from(e.to_owned())) // Return the original error
                        }
                    }
                }
            }
            HeaderName::Route => all_consuming(parser::headers::parse_route)(value_bytes)
                .map(|(_, v)| TypedHeader::Route(v))
                .map_err(Error::from),
            HeaderName::RecordRoute => all_consuming(parser::headers::parse_record_route)(value_bytes)
                .map(|(_, v)| TypedHeader::RecordRoute(v))
                .map_err(Error::from),
            HeaderName::CallId => {
                // Convert the raw header value to a string instead of using the parser
                match std::str::from_utf8(value_bytes) {
                    Ok(s) => Ok(TypedHeader::CallId(CallId(s.trim().to_string()))),
                    Err(_) => {
                        // Try with the parser as a fallback
                        match all_consuming(parser::headers::parse_call_id)(value_bytes) {
                            Ok((_, call_id)) => Ok(TypedHeader::CallId(call_id)),
                            Err(e) => {
                                // For Call-ID, we'll be lenient - create it directly from bytes
                                debug!("Warning: CallId parse error: {:?}, using raw value", e);
                                match String::from_utf8(value_bytes.to_vec()) {
                                    Ok(s) => Ok(TypedHeader::CallId(CallId(s.trim().to_string()))),
                                    Err(_) => Err(Error::from(e.to_owned()))
                                }
                            }
                        }
                    }
                }
            }
            HeaderName::CSeq => all_consuming(parser::headers::parse_cseq)(value_bytes)
                .map(|(_, cseq_struct)| TypedHeader::CSeq(cseq_struct))
                .map_err(Error::from),
            HeaderName::Accept => {
                if let HeaderValue::Accept(values) = &header.value {
                    Ok(TypedHeader::Accept(Accept::from_media_types(values.clone())))
                } else if let HeaderValue::Raw(bytes) = &header.value {
                    let accept = Accept::from_str(std::str::from_utf8(bytes)?)?;
                    Ok(TypedHeader::Accept(accept))
                } else {
                    Err(Error::InvalidHeader(format!("Invalid {} header", HeaderName::Accept)))
                }
            },
            HeaderName::ContentType => {
                // The HeaderValue::ContentType case is already handled at the beginning of the function
                all_consuming(parse_content_type_value)(value_bytes)
                    .map(|(_, v)| TypedHeader::ContentType(ContentType(v)))
                    .map_err(Error::from)
            },
            HeaderName::ContentLength => all_consuming(parser::headers::parse_content_length)(value_bytes)
                .map_err(Error::from)
                .and_then(|(_, v_u64)| {
                    let length = v_u64.try_into().map_err(|_| Error::ParseError("Invalid Content-Length value (overflow)".into()))?;
                    Ok(TypedHeader::ContentLength(ContentLength(length)))
                }),
            HeaderName::ContentDisposition => {
                debug!("ContentDisposition header: {:?}", header);
                match all_consuming(parser::headers::content_disposition::parse_content_disposition)(value_bytes) {
                    Ok((_, (disp_type_str, params_vec))) => {
                        // Parse disposition type from string
                        let disposition_type = match disp_type_str.to_lowercase().as_str() {
                            "session" => DispositionType::Session,
                            "render" => DispositionType::Render,
                            "icon" => DispositionType::Icon,
                            "alert" => DispositionType::Alert,
                            _ => DispositionType::Other(disp_type_str),
                        };
                        
                        // Convert disposition params to HashMap
                        let mut params = HashMap::new();
                        for param in params_vec {
                            match param {
                                DispositionParam::Handling(handling) => {
                                    let value = match handling {
                                        Handling::Optional => "optional".to_string(),
                                        Handling::Required => "required".to_string(),
                                        Handling::Other(s) => s.clone(),
                                    };
                                    params.insert("handling".to_string(), value);
                                },
                                DispositionParam::Generic(Param::Other(name, Some(GenericValue::Token(value)))) => {
                                    params.insert(name, value);
                                },
                                DispositionParam::Generic(Param::Other(name, Some(GenericValue::Quoted(value)))) => {
                                    params.insert(name, value);
                                },
                                DispositionParam::Generic(Param::Other(name, None)) => {
                                    params.insert(name, "".to_string());
                                },
                                _ => {} // Skip other parameter types
                            }
                        }
                        
                        debug!("Created ContentDisposition: {:?}", ContentDisposition {
                            disposition_type: disposition_type.clone(),
                            params: params.clone(),
                        });
                        
                        Ok(TypedHeader::ContentDisposition(ContentDisposition {
                            disposition_type,
                            params,
                        }))
                    },
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            },
            HeaderName::ContentEncoding => {
                if let HeaderValue::ContentEncoding(values) = &header.value {
                    // Convert Vec<Vec<u8>> to Vec<String>
                    let encodings = values.iter()
                        .filter_map(|v| String::from_utf8(v.clone()).ok())
                        .collect::<Vec<_>>();
                    
                    Ok(TypedHeader::ContentEncoding(ContentEncoding::with_encodings(&encodings)))
                } else if let HeaderValue::Raw(bytes) = &header.value {
                    let content_encoding = ContentEncoding::from_str(std::str::from_utf8(bytes)?)?;
                    Ok(TypedHeader::ContentEncoding(content_encoding))
                } else {
                    Err(Error::InvalidHeader(format!("Invalid {} header", HeaderName::ContentEncoding)))
                }
            },
            HeaderName::ContentLanguage => {
                if let HeaderValue::ContentLanguage(values) = &header.value {
                    // Convert Vec<Vec<u8>> to Vec<String>
                    let languages = values.iter()
                        .filter_map(|v| String::from_utf8(v.clone()).ok())
                        .collect::<Vec<_>>();
                    
                    Ok(TypedHeader::ContentLanguage(ContentLanguage::with_languages(&languages)))
                } else if let HeaderValue::Raw(bytes) = &header.value {
                    let content_language = ContentLanguage::from_str(std::str::from_utf8(bytes)?)?;
                    Ok(TypedHeader::ContentLanguage(content_language))
                } else {
                    Err(Error::InvalidHeader(format!("Invalid {} header", HeaderName::ContentLanguage)))
                }
            },
            HeaderName::AcceptEncoding => {
                if let HeaderValue::AcceptEncoding(values) = &header.value {
                    let mut encoding_infos = Vec::new();
                    
                    // Convert raw Values to EncodingInfo, preserving q values
                    for value in values {
                        encoding_infos.push(value.clone());
                    }
                    
                    Ok(TypedHeader::AcceptEncoding(AcceptEncoding(encoding_infos)))
                } else if let HeaderValue::Raw(bytes) = &header.value {
                    let accept_encoding = AcceptEncoding::from_str(std::str::from_utf8(bytes)?)?;
                    Ok(TypedHeader::AcceptEncoding(accept_encoding))
                } else {
                    Err(Error::InvalidHeader(format!("Invalid {} header", HeaderName::AcceptEncoding)))
                }
            },
            HeaderName::AcceptLanguage => {
                if let HeaderValue::AcceptLanguage(values) = &header.value {
                    // Need to extract LanguageInfo objects from the Vec<AcceptLanguage>
                    // and create a single AcceptLanguage with all the language infos
                    let mut language_infos = Vec::new();
                    
                    // Flatten all language infos from all AcceptLanguage values
                    for accept_lang in values {
                        if let crate::types::accept_language::AcceptLanguage(langs) = accept_lang {
                            language_infos.extend_from_slice(&langs);
                        }
                    }
                    
                    Ok(TypedHeader::AcceptLanguage(AcceptLanguage(language_infos)))
                } else if let HeaderValue::Raw(bytes) = &header.value {
                    let accept_language = AcceptLanguage::from_str(std::str::from_utf8(bytes)?)?;
                    Ok(TypedHeader::AcceptLanguage(accept_language))
                } else {
                    Err(Error::InvalidHeader(format!("Invalid {} header", HeaderName::AcceptLanguage)))
                }
            },
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
                .map_err(Error::from)
                .and_then(|(_, v_u32)| {
                    MinExpires::from_str(&v_u32.to_string())
                        .map(TypedHeader::MinExpires)
                }),
            HeaderName::MimeVersion => all_consuming(parser::headers::parse_mime_version)(value_bytes)
                .map_err(Error::from)
                .map(|(_, parsed_version_u8)| {
                    TypedHeader::MimeVersion(
                        crate::types::mime_version::MimeVersion::new(
                            parsed_version_u8.major.into(),
                            parsed_version_u8.minor.into()
                        )
                    )
                }),
            HeaderName::WwwAuthenticate => {
                // Check if we're already dealing with a HeaderValue::WwwAuthenticate
                // This case should have been handled earlier in the special cases
                if let HeaderValue::WwwAuthenticate(_) = &header.value {
                    return Err(Error::InternalError("HeaderValue::WwwAuthenticate should have been handled in special cases".to_string()));
                }
                
                // Otherwise parse from raw bytes
                all_consuming(parser::headers::parse_www_authenticate)(value_bytes)
                    .map(|(_, v)| TypedHeader::WwwAuthenticate(WwwAuthenticate(v)))
                    .map_err(Error::from)
            },
            HeaderName::Authorization => {
                // Check if we're already dealing with a HeaderValue::Authorization
                // This case should have been handled earlier in the special cases
                if let HeaderValue::Authorization(_) = &header.value {
                    return Err(Error::InternalError("HeaderValue::Authorization should have been handled in special cases".to_string()));
                }
                
                // Otherwise parse from raw bytes
                all_consuming(parser::headers::parse_authorization)(value_bytes)
                    .map(|(_, v)| TypedHeader::Authorization(v))
                    .map_err(Error::from)
            },
            HeaderName::ProxyAuthenticate => all_consuming(parser::headers::parse_proxy_authenticate)(value_bytes)
                .map(|(_, v)| TypedHeader::ProxyAuthenticate(ProxyAuthenticate(v)))
                .map_err(Error::from),
            HeaderName::ProxyAuthorization => all_consuming(parser::headers::parse_proxy_authorization)(value_bytes)
                .map(|(_, v)| TypedHeader::ProxyAuthorization(ProxyAuthorization(v)))
                .map_err(Error::from),
            HeaderName::AuthenticationInfo => all_consuming(parser::headers::parse_authentication_info)(value_bytes)
                .map(|(_, v)| TypedHeader::AuthenticationInfo(AuthenticationInfo(v)))
                .map_err(Error::from),
            HeaderName::Allow => all_consuming(parser::headers::allow::parse_allow)(value_bytes)
                .map(|(_, allow)| TypedHeader::Allow(allow))
                .map_err(Error::from),
            HeaderName::Require => {
                // Use our new Require type
                Ok(TypedHeader::Require(Require::from_header(&header)?))
            },
            HeaderName::Supported => {
                // Use our new Supported type
                Ok(TypedHeader::Supported(Supported::from_header(&header)?))
            },
            HeaderName::Unsupported => all_consuming(parser::headers::unsupported::parse_unsupported)(value_bytes)
                .map(|(_, strings)| TypedHeader::Unsupported(Unsupported::with_tags(strings)))
                .map_err(Error::from),
            HeaderName::ProxyRequire => all_consuming(parser::headers::parse_proxy_require)(value_bytes)
                .map(|(_, strings)| TypedHeader::ProxyRequire(crate::types::proxy_require::ProxyRequire::from_strings(strings)))
                .map_err(Error::from),
            HeaderName::Date => all_consuming(parser::headers::parse_date)(value_bytes)
                .map(|(_, v)| TypedHeader::Date(v))
                .map_err(Error::from),
            HeaderName::Timestamp => all_consuming(parser::headers::parse_timestamp)(value_bytes)
                .map(|(_, v)| TypedHeader::Timestamp(v))
                .map_err(Error::from),
            HeaderName::Organization => all_consuming(parser::headers::parse_organization)(value_bytes)
                .map(|(_, org)| TypedHeader::Organization(org))
                .map_err(Error::from),
            HeaderName::Priority => all_consuming(parser::headers::parse_priority)(value_bytes)
                .map(|(_, priority)| TypedHeader::Priority(crate::types::priority::Priority::from_str(&priority.to_string()).unwrap_or(crate::types::priority::Priority::Normal)))
                .map_err(Error::from),
            HeaderName::Subject => {
                // Use our Subject type directly from the parser
                all_consuming(parser::headers::subject::parse_subject)(value_bytes)
                    .map(|(_, subject)| TypedHeader::Subject(subject))
                    .map_err(Error::from)
            },
            HeaderName::Server => all_consuming(parser::headers::parse_server)(value_bytes)
                 .map(|(_, server_vals)| TypedHeader::Server(server_vals.into_iter()
                     .map(|server_val| match server_val {
                         crate::types::server::ServerVal::Product(product) => {
                             format!("{}{}", product.name, product.version.map_or_else(String::new, |v| format!("/{}", v)))
                         },
                         crate::types::server::ServerVal::Comment(comment) => {
                             format!("({})", comment)
                         }
                     })
                     .collect::<Vec<String>>()))
                 .map_err(Error::from),
            HeaderName::UserAgent => all_consuming(parser::headers::parse_user_agent)(value_bytes)
                 .map(|(_, server_vals)| TypedHeader::UserAgent(server_vals.into_iter()
                     .map(|server_val| match server_val {
                         crate::types::server::ServerVal::Product(product) => {
                             format!("{}{}", product.name, product.version.map_or_else(String::new, |v| format!("/{}", v)))
                         },
                         crate::types::server::ServerVal::Comment(comment) => {
                             format!("({})", comment)
                         }
                     })
                     .collect::<Vec<String>>()))
                 .map_err(Error::from),
            HeaderName::InReplyTo => all_consuming(parser::headers::parse_in_reply_to)(value_bytes)
                .map(|(_, strings)| TypedHeader::InReplyTo(crate::types::in_reply_to::InReplyTo::with_multiple_strings(strings)))
                .map_err(Error::from),
             HeaderName::Warning => {
                 // First try to use WarningHeader TypedHeaderTrait implementation
                 if let Ok(s) = std::str::from_utf8(value_bytes) {
                     if let Ok(warning_header) = crate::types::warning::WarningHeader::from_str(s.trim()) {
                         return Ok(TypedHeader::Warning(warning_header.warnings));
                     }
                 }
                 
                 // Fallback to the original parsing logic
                 let parse_result = all_consuming(parser::headers::warning::parse_warning_value_list)(value_bytes);
                 match parse_result {
                     Ok((_, parsed_values)) => {
                         let mut typed_warnings = Vec::new();
                         for parsed_value in parsed_values {
                             let agent_uri = match parsed_value.agent {
                                 WarnAgent::HostPort(host, _port_opt) => {
                                     Uri::new(Scheme::Sip, host)
                                 },
                                 WarnAgent::Pseudonym(pseudonym_str) => {
                                     match crate::types::uri::Host::from_str(&pseudonym_str) {
                                          Ok(host) => Uri::new(Scheme::Sip, host),
                                          Err(_) => {
                                              return Err(Error::ParseError(format!("Cannot represent warning agent pseudonym '{}' as a valid host for Uri", pseudonym_str)));
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
                        
                        // Create a RetryAfter instance with the parsed values
                        let mut retry_after = RetryAfter::new(delay);
                        
                        // Set comment if present
                        if let Some(comment_text) = comment {
                            retry_after.comment = Some(comment_text);
                        }
                        
                        // Process parameters
                        for param in ra_value.params {
                            match param {
                                crate::parser::headers::retry_after::RetryParam::Duration(d) => {
                                    retry_after.duration = Some(d);
                                },
                                crate::parser::headers::retry_after::RetryParam::Generic(param) => {
                                    retry_after.parameters.push(param);
                                }
                            }
                        }
                        
                        Ok(TypedHeader::RetryAfter(retry_after))
                    })
            },
            HeaderName::ErrorInfo => {
                if let HeaderValue::Raw(bytes) = &header.value {
                    // Special handling for Error-Info with comments
                    if let Ok(s) = std::str::from_utf8(bytes) {
                        match ErrorInfoHeader::from_str(s.trim()) {
                            Ok(error_info) => {
                                return Ok(TypedHeader::ErrorInfo(error_info));
                            },
                            Err(_) => {
                                // Fall through to generic parser below
                            }
                        }
                    }
                    
                    // If direct FromStr parsing failed, try the standard parser
                    match all_consuming(parser::headers::error_info::parse_error_info)(bytes) {
                        Ok((_, error_info_values)) => {
                            let mut list = ErrorInfoList::new();
                            for value in error_info_values {
                                list.add(ErrorInfoHeader::from_error_info_value(&value));
                            }
                            
                            return Ok(TypedHeader::ErrorInfo(ErrorInfoHeader { error_info_list: list }));
                        },
                        Err(e) => {
                            return Err(Error::from(e.to_owned()));
                        }
                    }
                } else if let HeaderValue::ErrorInfo(values) = &header.value {
                    // Convert from parser values to our type
                    let mut list = ErrorInfoList::new();
                    for value in values {
                        list.add(ErrorInfoHeader::from_error_info_value(value));
                    }
                    return Ok(TypedHeader::ErrorInfo(ErrorInfoHeader { error_info_list: list }));
                } else {
                    return Err(Error::InvalidHeader(format!("Invalid {} header", HeaderName::ErrorInfo)));
                }
            },
            HeaderName::AlertInfo => all_consuming(parser::headers::parse_alert_info)(value_bytes)
                .map(|(_, alert_info_values)| {
                    // Convert parser values to our AlertInfo types
                    let mut alert_info_list = crate::types::alert_info::AlertInfoList::new();
                    for value in alert_info_values {
                        if let Ok(alert_info) = crate::types::alert_info::AlertInfoHeader::from_alert_info_value(&value) {
                            alert_info_list.add(alert_info);
                        }
                    }
                    TypedHeader::AlertInfo(crate::types::alert_info::AlertInfoHeader { alert_info_list })
                })
                .map_err(Error::from),
            HeaderName::CallInfo => all_consuming(parser::headers::parse_call_info)(value_bytes)
                .map(|(_, call_info_values)| TypedHeader::CallInfo(CallInfo(call_info_values)))
                .map_err(Error::from),
            HeaderName::ReferTo => {
                match all_consuming(parser::headers::parse_refer_to)(value_bytes) {
                    Ok((_, addr)) => Ok(TypedHeader::ReferTo(ReferTo::new(addr))),
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            },
            HeaderName::ReferredBy => {
                match all_consuming(parser::headers::parse_referred_by)(value_bytes) {
                    Ok((_, addr)) => Ok(TypedHeader::ReferredBy(ReferredBy::new(addr))),
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            },
            HeaderName::Path => {
                if let HeaderValue::Raw(bytes) = &header.value {
                    if let Ok(s) = std::str::from_utf8(bytes) {
                        let path = crate::types::path::Path::from_str(s.trim())?;
                        Ok(TypedHeader::Path(path))
                    } else {
                        Err(Error::InvalidHeader(format!("Invalid UTF-8 in Path header")))
                    }
                } else if let HeaderValue::Route(entries) = &header.value {
                    // Reuse Route header format for Path
                    Ok(TypedHeader::Path(crate::types::path::Path(entries.clone())))
                } else {
                    Err(Error::InvalidHeader(format!("Invalid Path header")))
                }
            },
            HeaderName::Reason => all_consuming(parser::headers::parse_reason)(value_bytes)
                .map(|(_, v)| TypedHeader::Reason(v))
                .map_err(Error::from),
            HeaderName::SessionExpires => all_consuming(parse_session_expires)(value_bytes)
                .map(|(_, (delta, refresher, params))| TypedHeader::SessionExpires(SessionExpires::new_with_params(delta, refresher, params)))
                .map_err(Error::from),
            HeaderName::Event => {
                // Assuming value_bytes is the string representation of the header value bytes
                let value_str = std::str::from_utf8(value_bytes)
                    .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in Event header value: {}", e)))?;
                Ok(TypedHeader::Event(EventTypeData::from_str(value_str)?))
            },
            HeaderName::SubscriptionState => {
                // Parse raw bytes to SubscriptionState using FromStr implementation
                let value_str = std::str::from_utf8(value_bytes)
                    .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in Subscription-State header value: {}", e)))?;
                Ok(TypedHeader::SubscriptionState(SubscriptionStateType::from_str(value_str)?))
            },
            HeaderName::MinSE => {
                let value_str = std::str::from_utf8(value_bytes)
                    .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in MinSE header value: {}", e)))?;
                Ok(TypedHeader::MinSE(MinSE::from_str(value_str)?))
            },
            HeaderName::RSeq => {
                let value_str = std::str::from_utf8(value_bytes)
                    .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in RSeq header value: {}", e)))?;
                Ok(TypedHeader::RSeq(crate::types::rseq::RSeq::from_str(value_str)?))
            },
            HeaderName::SipETag => {
                match all_consuming(parser::headers::parse_sip_etag)(value_bytes) {
                    Ok((_, etag)) => Ok(TypedHeader::SipETag(etag)),
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            },
            HeaderName::SipIfMatch => {
                match all_consuming(parser::headers::parse_sip_if_match)(value_bytes) {
                    Ok((_, if_match)) => Ok(TypedHeader::SipIfMatch(if_match)),
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            },
            HeaderName::AllowEvents => {
                match all_consuming(parser::headers::parse_allow_events)(value_bytes) {
                    Ok((_, allow)) => Ok(TypedHeader::AllowEvents(allow)),
                    Err(e) => Err(Error::from(e.to_owned())),
                }
            },
            _ => Ok(TypedHeader::Other(header.name.clone(), HeaderValue::Raw(value_bytes.to_vec()))),
        };
        
        parse_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_typed_header_name() {
        // Create a typed header
        let header = TypedHeader::CallId(CallId::new("test@example.com"));
        
        // Check that the name is correct
        assert_eq!(header.name(), HeaderName::CallId);
    }
} 