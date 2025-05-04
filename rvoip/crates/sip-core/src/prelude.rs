//! # Prelude
//!
//! The `rvoip_sip_core` prelude brings the most common types and traits into scope.
//!
//! This is typically imported with `use rvoip_sip_core::prelude::*;`

pub use crate::error::{Error, Result};
pub use crate::types::header::{Header, HeaderValue, TypedHeader, TypedHeaderTrait};
pub use crate::types::headers::HeaderName;
pub use crate::types::uri::{Uri, Host, Scheme};
pub use crate::types::Method;
pub use crate::types::StatusCode;
pub use crate::types::sip_message::Message;
pub use crate::types::sip_request::Request;
pub use crate::types::sip_response::Response;
pub use crate::types::Via;
pub use crate::types::Version;
pub use crate::parser::message::{MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE, ParseMode};
pub use crate::parser::parse_message;
pub use crate::parser::message::parse_message_with_mode;
pub use crate::types::multipart::{MultipartBody, MimePart, ParsedBody};
pub use crate::builder::{SimpleRequestBuilder as RequestBuilder, SimpleResponseBuilder as ResponseBuilder};
pub use crate::types::param::Param;
pub use crate::types::param::GenericValue;
pub use crate::types::warning::Warning;
pub use crate::types::warning::{WarnAgent, WarningValue};
pub use crate::types::address::Address;
pub use crate::types::from::From;
pub use crate::types::to::To;
pub use crate::types::call_id::CallId;
pub use crate::types::cseq::CSeq;
pub use crate::types::content_length::ContentLength;
pub use crate::types::content_type::ContentType;
pub use crate::parser::headers::content_type::ContentTypeValue;
pub use crate::types::max_forwards::MaxForwards;
pub use crate::types::contact::Contact;
pub use crate::types::contact::{ContactParamInfo, ContactValue};
pub use crate::types::supported::Supported;
pub use crate::types::unsupported::Unsupported;
pub use crate::types::content_disposition::{ContentDisposition, DispositionType, DispositionParam, Handling};
pub use crate::types::error_info::{ErrorInfo, ErrorInfoHeader, ErrorInfoList};
pub use crate::types::expires::Expires;
pub use crate::types::in_reply_to::InReplyTo;
pub use crate::types::MediaType;
pub use crate::types::organization::Organization;
pub use crate::types::priority::Priority;
pub use crate::types::record_route::RecordRoute;
pub use crate::types::record_route::RecordRouteEntry;
pub use crate::types::refer_to::ReferTo;
pub use crate::types::reply_to::ReplyTo;
pub use crate::types::require::Require;
pub use crate::types::retry_after::RetryAfter;
pub use crate::parser::headers::route::RouteEntry as ParserRouteValue;
pub use crate::types::route::Route;
pub use crate::types::path::Path;

// Server-related types needed for doc tests
pub use crate::types::server::{ServerInfo, ServerProduct, Product, ServerVal};

// Authentication-related types needed for doc tests
pub use crate::types::auth::{
    AuthParam, AuthenticationInfo, AuthenticationInfoParam, Authorization,
    Challenge, Credentials, DigestParam, ProxyAuthenticate, ProxyAuthorization,
    WwwAuthenticate, AuthScheme
};
pub use crate::types::{Algorithm, Qop};

// Additional header types previously missing
pub use crate::types::Allow;
pub use crate::types::Accept;
pub use crate::parser::headers::accept::AcceptValue;
pub use crate::types::Subject;
pub use crate::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
pub use crate::types::AcceptLanguage;
pub use crate::parser::headers::accept_language::LanguageInfo;

// Header builder extension traits
pub use crate::builder::headers::{
    HeaderSetter,
    AllowBuilderExt,
    AuthorizationExt,
    WwwAuthenticateExt,
    ProxyAuthenticateExt,
    ProxyAuthorizationExt,
    AuthenticationInfoExt,
    ContentEncodingExt,
    ContentLanguageExt,
    ContentDispositionExt,
    AcceptExt,
    AcceptEncodingExt,
    AcceptLanguageExt,
    RecordRouteBuilderExt,
    RouteBuilderExt,
    SupportedBuilderExt,
    UnsupportedBuilderExt,
    RequireBuilderExt,
    UserAgentBuilderExt,
    ServerBuilderExt,
    PathBuilderExt,
    ProxyRequireBuilderExt,
    ContentBuilderExt,
    CallIdBuilderExt,
    InReplyToBuilderExt
};

// Add ProxyRequire type
pub use crate::types::ProxyRequire;

// SDP Integration
#[cfg(feature = "sdp")]
pub use crate::sdp::{SdpBuilder, SdpSession};
#[cfg(feature = "sdp")]
pub use crate::builder::headers::ContentBuilderExt;
#[cfg(feature = "sdp")]
pub use crate::sdp::attributes::MediaDirection;

// JSON Support
pub use crate::json::SipJsonExt;
pub use crate::json::SipValue;

// Also add the macros
pub use crate::macros::*; 