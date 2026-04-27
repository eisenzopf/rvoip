//! # Prelude
//!
//! The `rvoip_sip_core` prelude brings the most common types and traits into scope.
//!
//! This is typically imported with `use rvoip_sip_core::prelude::*;`

pub use crate::builder::{
    SimpleRequestBuilder as RequestBuilder, SimpleResponseBuilder as ResponseBuilder,
};
pub use crate::error::{Error, Result};
pub use crate::parser::headers::content_type::ContentTypeValue;
pub use crate::parser::headers::route::RouteEntry as ParserRouteValue;
pub use crate::parser::headers::route::RouteEntry as PathEntry;
pub use crate::parser::message::parse_message_with_mode;
pub use crate::parser::message::{ParseMode, MAX_BODY_SIZE, MAX_HEADER_COUNT, MAX_LINE_LENGTH};
pub use crate::parser::parse_message;
pub use crate::types::address::Address;
pub use crate::types::call_id::CallId;
pub use crate::types::contact::Contact;
pub use crate::types::contact::{ContactParamInfo, ContactValue};
pub use crate::types::content_disposition::{
    ContentDisposition, DispositionParam, DispositionType, Handling,
};
pub use crate::types::content_length::ContentLength;
pub use crate::types::content_type::ContentType;
pub use crate::types::cseq::CSeq;
pub use crate::types::error_info::{ErrorInfo, ErrorInfoHeader, ErrorInfoList};
pub use crate::types::expires::Expires;
pub use crate::types::from::From;
pub use crate::types::header::{Header, HeaderValue, TypedHeader, TypedHeaderTrait};
pub use crate::types::headers::HeaderName;
pub use crate::types::in_reply_to::InReplyTo;
pub use crate::types::max_forwards::MaxForwards;
pub use crate::types::multipart::{MimePart, MultipartBody, ParsedBody};
pub use crate::types::organization::Organization;
pub use crate::types::param::GenericValue;
pub use crate::types::param::Param;
pub use crate::types::path::Path;
pub use crate::types::priority::Priority;
pub use crate::types::record_route::RecordRoute;
pub use crate::types::record_route::RecordRouteEntry;
pub use crate::types::refer_to::ReferTo;
pub use crate::types::referred_by::ReferredBy;
pub use crate::types::reply_to::ReplyTo;
pub use crate::types::require::Require;
pub use crate::types::retry_after::RetryAfter;
pub use crate::types::route::Route;
pub use crate::types::sip_message::Message;
pub use crate::types::sip_request::Request;
pub use crate::types::sip_response::Response;
pub use crate::types::supported::Supported;
pub use crate::types::to::To;
pub use crate::types::unsupported::Unsupported;
pub use crate::types::uri::{Host, Scheme, Uri};
pub use crate::types::warning::Warning;
pub use crate::types::warning::{WarnAgent, WarningHeader, WarningValue};
pub use crate::types::MediaType;
pub use crate::types::Method;
pub use crate::types::StatusCode;
pub use crate::types::Version;
pub use crate::types::Via;

// Server-related types needed for doc tests
pub use crate::types::server::{Product, ServerInfo, ServerProduct, ServerVal};

// Authentication-related types needed for doc tests
pub use crate::types::auth::{
    AuthParam, AuthScheme, AuthenticationInfo, AuthenticationInfoParam, Authorization, Challenge,
    Credentials, DigestParam, ProxyAuthenticate, ProxyAuthorization, WwwAuthenticate,
};
pub use crate::types::{Algorithm, Qop};

// Additional header types previously missing
pub use crate::parser::headers::accept::AcceptValue;
pub use crate::parser::headers::accept_language::LanguageInfo;
pub use crate::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
pub use crate::types::Accept;
pub use crate::types::AcceptLanguage;
pub use crate::types::Allow;
pub use crate::types::Subject;

// Header builder extension traits
pub use crate::builder::headers::{
    AcceptEncodingExt, AcceptExt, AcceptLanguageExt, AllowBuilderExt, AuthenticationInfoExt,
    AuthorizationExt, CallIdBuilderExt, ContentDispositionExt, ContentEncodingExt,
    ContentLanguageExt, HeaderSetter, InReplyToBuilderExt, PathBuilderExt, PriorityBuilderExt,
    ProxyAuthenticateExt, ProxyAuthorizationExt, ProxyRequireBuilderExt, RecordRouteBuilderExt,
    ReferredByExt, RequireBuilderExt, RouteBuilderExt, ServerBuilderExt, SupportedBuilderExt,
    UnsupportedBuilderExt, UserAgentBuilderExt, WarningBuilderExt, WwwAuthenticateExt,
};

// Add ProxyRequire type
pub use crate::types::ProxyRequire;

// SDP Integration
#[cfg(feature = "sdp")]
pub use crate::sdp::attributes::MediaDirection;
#[cfg(feature = "sdp")]
pub use crate::sdp::SdpBuilder;
#[cfg(feature = "sdp")]
pub use crate::types::sdp::SdpSession;

// JSON Support
pub use crate::json::SipJsonExt;
pub use crate::json::SipValue;

// Also add the macros
pub use crate::macros::*;
