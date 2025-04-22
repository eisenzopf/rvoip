use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue}; // Basic header parts
use crate::types; // Import the types module itself
use crate::parser; // Import the parser module
use std::convert::TryFrom;
use nom::all_consuming;

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

    /// Represents an unknown or unparsed header.
    Other(HeaderName, HeaderValue),
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
            HeaderName::ContentLength => all_consuming(parser::headers::parse_content_length)(&value_bytes).map(|(_, v)| TypedHeader::ContentLength(v)),
            HeaderName::ContentDisposition => all_consuming(parser::headers::parse_content_disposition)(&value_bytes).map(|(_, v)| TypedHeader::ContentDisposition(v)),
            HeaderName::ContentEncoding => all_consuming(parser::headers::parse_content_encoding)(&value_bytes).map(|(_, v)| TypedHeader::ContentEncoding(v)), // Placeholder type
            HeaderName::ContentLanguage => all_consuming(parser::headers::parse_content_language)(&value_bytes).map(|(_, v)| TypedHeader::ContentLanguage(v)), // Placeholder type
            HeaderName::AcceptEncoding => all_consuming(parser::headers::parse_accept_encoding)(&value_bytes).map(|(_, v)| TypedHeader::AcceptEncoding(v)), // Placeholder type
            HeaderName::AcceptLanguage => all_consuming(parser::headers::parse_accept_language)(&value_bytes).map(|(_, v)| TypedHeader::AcceptLanguage(v)), // Placeholder type

            // Simple Value Headers
            HeaderName::MaxForwards => all_consuming(parser::headers::parse_max_forwards)(&value_bytes).map(|(_, v)| TypedHeader::MaxForwards(v)),
            HeaderName::Expires => all_consuming(parser::headers::parse_expires)(&value_bytes).map(|(_, v)| TypedHeader::Expires(v)),
            HeaderName::MinExpires => all_consuming(parser::headers::parse_min_expires)(&value_bytes).map(|(_, v)| TypedHeader::MinExpires(v)), // Placeholder type
            HeaderName::MimeVersion => all_consuming(parser::headers::parse_mime_version)(&value_bytes).map(|(_, v)| TypedHeader::MimeVersion(v)), // Placeholder type

            // Auth Headers
            HeaderName::WwwAuthenticate => all_consuming(parser::headers::parse_www_authenticate)(&value_bytes).map(|(_, v)| TypedHeader::WwwAuthenticate(v)),
            HeaderName::Authorization => all_consuming(parser::headers::parse_authorization)(&value_bytes).map(|(_, v)| TypedHeader::Authorization(v)),
            HeaderName::ProxyAuthenticate => all_consuming(parser::headers::parse_proxy_authenticate)(&value_bytes).map(|(_, v)| TypedHeader::ProxyAuthenticate(v)),
            HeaderName::ProxyAuthorization => all_consuming(parser::headers::parse_proxy_authorization)(&value_bytes).map(|(_, v)| TypedHeader::ProxyAuthorization(v)),
            HeaderName::AuthenticationInfo => all_consuming(parser::headers::parse_authentication_info)(&value_bytes).map(|(_, v)| TypedHeader::AuthenticationInfo(v)),

            // Token List Headers
            HeaderName::Allow => all_consuming(parser::headers::parse_allow)(&value_bytes).map(|(_, v)| TypedHeader::Allow(v)),
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
            HeaderName::InReplyTo => all_consuming(parser::headers::parse_in_reply_to)(&value_bytes).map(|(_, v)| TypedHeader::InReplyTo(v)), // Placeholder type
            HeaderName::Warning => all_consuming(parser::headers::parse_warning)(&value_bytes).map(|(_, v)| TypedHeader::Warning(v)),
            HeaderName::RetryAfter => all_consuming(parser::headers::parse_retry_after)(&value_bytes).map(|(_, v)| TypedHeader::RetryAfter(v)), // Placeholder type
            HeaderName::ErrorInfo => all_consuming(parser::headers::parse_error_info)(&value_bytes).map(|(_, v)| TypedHeader::ErrorInfo(v)), // Placeholder type
            HeaderName::AlertInfo => all_consuming(parser::headers::parse_alert_info)(&value_bytes).map(|(_, v)| TypedHeader::AlertInfo(v)), // Placeholder type
            HeaderName::CallInfo => all_consuming(parser::headers::parse_call_info)(&value_bytes).map(|(_, v)| TypedHeader::CallInfo(v)), // Placeholder type
            
            // Fallback for Other/Unimplemented
            _ => Ok(TypedHeader::Other(header.name.clone(), HeaderValue::Raw(value_bytes))), // Return Raw if unknown
        };
        
        // Map nom error to crate::Error
        parse_result.map_err(|e| {
            Error::ParsingError {
                message: format!("Failed to parse header '{:?}' value: {:?}", header.name, e),
                source: None,
            }
        })
    }
}

// TODO: Implement From implementations for each specific type to TypedHeader
// Example:
// impl From<types::Via> for TypedHeader {
//     fn from(via: types::Via) -> Self {
//         TypedHeader::Via(via)
//     }
// }

// TODO: Implement TryFrom<crate::header::Header> for TypedHeader
// This will be the core logic to convert a raw Header into a TypedHeader
// involving calling the appropriate parser based on HeaderName. 