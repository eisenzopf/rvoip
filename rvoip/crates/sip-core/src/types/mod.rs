// Placeholder for SIP type definitions 

pub mod via;
pub use via::Via;

pub mod method;
pub use method::Method;

pub mod status;
pub use status::StatusCode;

pub mod sip_message;
pub use sip_message::{Request, Response, Message};

pub mod param;
pub use param::Param;

// Add new URI module
pub mod uri;
pub use uri::{Uri, Host, Scheme};

// Add URI adapter module
pub mod uri_adapter;
pub use uri_adapter::UriAdapter;

// Add Version module
pub mod version;
pub use version::Version;

// SIP Core Types

pub mod accept;
pub mod accept_language;
pub mod address;
pub mod allow;
pub mod auth;
pub mod call_id;
pub mod call_info;
pub mod contact;
pub mod content_disposition;
pub mod content_length;
pub mod content_type;
pub mod media_type;
pub mod cseq;
pub mod expires;
pub mod from;
pub mod header;
pub use header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};
pub mod in_reply_to;
pub mod max_forwards;
pub mod organization;
pub mod record_route;
pub mod reply_to;
pub mod refer_to;
pub mod require;
pub mod route;
pub mod sdp;
pub mod subject;
pub mod to;
pub mod uri_with_params;
pub mod uri_with_params_list;
pub mod warning;
pub mod multipart;

// Modules missing re-exports - Add them
pub mod priority;
pub mod server;
pub mod retry_after;
pub mod error_info;

pub use accept::Accept;
pub use accept_language::AcceptLanguage;
pub use address::Address;
pub use allow::Allow;
pub use auth::*;
pub use call_id::CallId;
pub use call_info::{CallInfo, CallInfoValue, InfoPurpose};
pub use contact::Contact;
pub use content_disposition::*;
pub use content_length::ContentLength;
pub use content_type::ContentType;
pub use media_type::MediaType;
pub use cseq::CSeq;
pub use expires::Expires;
pub use from::From;
pub use in_reply_to::InReplyTo;
pub use max_forwards::MaxForwards;
pub use organization::Organization;
pub use require::Require;
pub use subject::Subject;
pub use warning::Warning;
pub use multipart::{MultipartBody, MimePart, ParsedBody};
pub use refer_to::ReferTo;

// Add missing pub use * directives
pub use priority::*;
pub use server::*;
pub use retry_after::*;
pub use error_info::*;