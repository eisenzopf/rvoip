use std::convert::TryFrom;
use crate::types::header::{TypedHeaderTrait, Header};
use crate::types::TypedHeader;
use crate::types::headers::header_name::HeaderName;

/// Trait for setting headers on builders 
pub trait HeaderSetter {
    /// Set a header on the builder
    fn set_header<H: TypedHeaderTrait + 'static>(self, header: H) -> Self;
}

// Implementations for the builder types
impl HeaderSetter for crate::builder::SimpleRequestBuilder {
    fn set_header<H: TypedHeaderTrait + 'static>(self, header: H) -> Self {
        // Special case for WarningHeader
        if let Some(warning_header) = (&header as &dyn std::any::Any).downcast_ref::<crate::types::warning::WarningHeader>() {
            return self.header(crate::types::TypedHeader::Warning(warning_header.warnings.clone()));
        }
        
        let header_val = header.to_header();

        match TypedHeader::try_from(header_val) { // Removed .clone() as it's not needed without the error eprintfn
            Ok(typed_header) => {
                self.header(typed_header)
            },
            Err(_e) => { // Reverted _e to avoid unused variable warning if e was only for eprintfn
                // If try_from fails, the header is effectively ignored for now.
                // Consider logging this error in a real application.
                self // Return self to allow chaining even if a header fails to convert
            }
        }
    }
}

impl HeaderSetter for crate::builder::SimpleResponseBuilder {
    fn set_header<H: TypedHeaderTrait + 'static>(self, header: H) -> Self {
        // Special case for WarningHeader
        if let Some(warning_header) = (&header as &dyn std::any::Any).downcast_ref::<crate::types::warning::WarningHeader>() {
            return self.header(crate::types::TypedHeader::Warning(warning_header.warnings.clone()));
        }
        
        let header_val = header.to_header();
        match TypedHeader::try_from(header_val) {
            Ok(typed_header) => self.header(typed_header),
            Err(_) => self
        }
    }
}

// Header builder modules
pub mod authorization;
pub mod www_authenticate;
pub mod proxy_authenticate;
pub mod proxy_authorization;
pub mod authentication_info;
pub mod content_encoding;
pub mod content_language;
pub mod content_disposition;
pub mod accept;
pub mod accept_encoding;
pub mod accept_language;
pub mod record_route;
pub mod route;
pub mod allow;
pub mod supported;
pub mod unsupported;
pub mod require;
pub mod user_agent;
pub mod server;
pub mod path;
pub mod proxy_require;
pub mod content;
pub mod content_type;
pub mod call_id;
pub mod in_reply_to;
pub mod reason;
pub mod reply_to;
pub mod from;
pub mod to;
pub mod contact;
pub mod via;
pub mod cseq;
pub mod max_forwards;
pub mod call_info;
pub mod content_length;
pub mod mime_version;
pub mod expires;
pub mod organization;
pub mod alert_info;
pub mod error_info;
pub mod priority;
pub mod warning;
pub mod refer_to;
pub mod referred_by;
pub mod session_expires;
pub mod event;

// Re-export all header builders for convenient imports
pub use authorization::AuthorizationExt;
pub use www_authenticate::WwwAuthenticateExt;
pub use proxy_authenticate::ProxyAuthenticateExt;
pub use proxy_authorization::ProxyAuthorizationExt;
pub use authentication_info::AuthenticationInfoExt;
pub use content_encoding::ContentEncodingExt;
pub use content_language::ContentLanguageExt;
pub use content_disposition::ContentDispositionExt;
pub use accept::AcceptExt;
pub use accept_encoding::AcceptEncodingExt;
pub use accept_language::AcceptLanguageExt;
pub use record_route::RecordRouteBuilderExt;
pub use route::RouteBuilderExt;
pub use allow::AllowBuilderExt;
pub use supported::SupportedBuilderExt;
pub use unsupported::UnsupportedBuilderExt;
pub use require::RequireBuilderExt;
pub use user_agent::UserAgentBuilderExt;
pub use server::ServerBuilderExt;
pub use path::PathBuilderExt;
pub use proxy_require::ProxyRequireBuilderExt;
pub use content::ContentBuilderExt;
pub use content_type::ContentTypeBuilderExt;
pub use call_id::CallIdBuilderExt;
pub use in_reply_to::InReplyToBuilderExt;
pub use reason::ReasonBuilderExt;
pub use reply_to::ReplyToBuilderExt;
pub use from::FromBuilderExt;
pub use to::ToBuilderExt;
pub use contact::ContactBuilderExt;
pub use via::ViaBuilderExt;
pub use cseq::CSeqBuilderExt;
pub use max_forwards::MaxForwardsBuilderExt;
pub use call_info::CallInfoBuilderExt;
pub use content_length::ContentLengthBuilderExt;
pub use mime_version::MimeVersionBuilderExt;
pub use expires::ExpiresBuilderExt;
pub use organization::OrganizationBuilderExt;
pub use alert_info::AlertInfoBuilderExt;
pub use error_info::ErrorInfoBuilderExt;
pub use priority::PriorityBuilderExt;
pub use warning::WarningBuilderExt;
pub use refer_to::ReferToExt;
pub use referred_by::ReferredByExt;
pub use session_expires::SessionExpiresExt;
pub use event::EventBuilderExt;

