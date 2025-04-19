use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue}; // Basic header parts
use crate::types; // Import the types module itself
use crate::parser; // Import the parser module
use std::convert::TryFrom;

/// Represents any parsed SIP header in a strongly-typed way.
#[derive(Debug, Clone, PartialEq)] // Add necessary derives
pub enum TypedHeader {
    // Core Headers (Examples)
    Via(types::Via),
    From(types::From),
    To(types::To),
    Contact(types::Contact),
    CallId(types::CallId),
    CSeq(types::CSeq),
    Route(types::Route),
    RecordRoute(types::RecordRoute),
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
    ReplyTo(types::ReplyTo),
    Warning(types::Warning),
    ContentDisposition(types::ContentDisposition),

    /// Represents an unknown or unparsed header.
    Other(HeaderName, HeaderValue),
}

impl TryFrom<Header> for TypedHeader {
    type Error = Error;

    fn try_from(header: Header) -> Result<Self> {
        let value_text = match header.value.as_text() {
            Some(text) => text,
            None => return Ok(TypedHeader::Other(header.name.clone(), header.value)), // Clone needed here
        };

        let result = match header.name {
            // Core Headers
            HeaderName::Via => parser::headers::parse_via(value_text).map(TypedHeader::Via),
            HeaderName::From => parser::headers::parse_address(value_text).map(|addr| TypedHeader::From(types::From(addr))),
            HeaderName::To => parser::headers::parse_address(value_text).map(|addr| TypedHeader::To(types::To(addr))),
            HeaderName::Contact => parser::headers::parse_contact(value_text).map(|addrs| {
                addrs.into_iter().next().map(types::Contact).map(TypedHeader::Contact)
                     .unwrap_or_else(|| TypedHeader::Other(header.name.clone(), header.value.clone()))
            }),
            HeaderName::CSeq => parser::headers::parse_cseq(value_text).map(TypedHeader::CSeq),
            HeaderName::ContentType => parser::headers::parse_content_type(value_text).map(|mt| TypedHeader::ContentType(types::ContentType(mt))),
            HeaderName::Allow => parser::headers::parse_allow(value_text).map(TypedHeader::Allow),
            HeaderName::Accept => parser::headers::parse_accept(value_text).map(TypedHeader::Accept),
            HeaderName::ContentDisposition => parser::headers::parse_content_disposition(value_text).map(TypedHeader::ContentDisposition),
            HeaderName::Warning => parser::headers::parse_warning(value_text).map(TypedHeader::Warning),
            
            // Use dedicated parsers for simple types
            HeaderName::CallId => parser::headers::parse_call_id(value_text).map(TypedHeader::CallId),
            HeaderName::ContentLength => parser::headers::parse_content_length(value_text).map(TypedHeader::ContentLength),
            HeaderName::Expires => parser::headers::parse_expires(value_text).map(TypedHeader::Expires),
            HeaderName::MaxForwards => parser::headers::parse_max_forwards(value_text).map(TypedHeader::MaxForwards),
            
            // Auth Headers
            HeaderName::WwwAuthenticate => parser::headers::parse_www_authenticate(value_text).map(TypedHeader::WwwAuthenticate),
            HeaderName::Authorization => parser::headers::parse_authorization(value_text).map(TypedHeader::Authorization),
            HeaderName::ProxyAuthenticate => parser::headers::parse_proxy_authenticate(value_text).map(TypedHeader::ProxyAuthenticate),
            HeaderName::ProxyAuthorization => parser::headers::parse_proxy_authorization(value_text).map(TypedHeader::ProxyAuthorization),
            HeaderName::AuthenticationInfo => parser::headers::parse_authentication_info(value_text).map(TypedHeader::AuthenticationInfo),

            // Routing Headers
            HeaderName::Route => parser::headers::parse_route(value_text).map(TypedHeader::Route),
            HeaderName::RecordRoute => parser::headers::parse_record_route(value_text).map(TypedHeader::RecordRoute),
            HeaderName::ReplyTo => parser::headers::parse_reply_to(value_text).map(TypedHeader::ReplyTo),
            
            // TODO: Implement parsers for other headers as needed

            // Fallback for other/unimplemented headers
            _ => Ok(TypedHeader::Other(header.name, header.value)), // name and value are moved here
        };
        
        // Map specific parser errors to a more generic parsing error for this header
        result.map_err(|e| {
             if let TypedHeader::Other(n, v) = result.as_ref().unwrap_or(&TypedHeader::Other(header.name.clone(), header.value.clone())) {
                 // If it fell through to Other or was already an error, use original values
                  Error::Parser(format!(
                     "Failed to parse header '{:?}' value '{}': {}", 
                     n, 
                     value_text, 
                     e
                 ))
             } else {
                 // If parsing succeeded initially but mapping failed (shouldn't happen with current structure)
                  Error::Parser(format!(
                     "Failed to parse header '{:?}' value '{}': {}", 
                     header.name, 
                     value_text, 
                     e
                 ))
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