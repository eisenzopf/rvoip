use std::convert::TryFrom;
use crate::types::header::{TypedHeaderTrait, Header};
use crate::types::TypedHeader;

/// Trait for setting headers on builders 
pub trait HeaderSetter {
    /// Set a header on the builder
    fn set_header<H: TypedHeaderTrait>(self, header: H) -> Self;
}

// Implementations for the builder types
impl HeaderSetter for crate::builder::SimpleRequestBuilder {
    fn set_header<H: TypedHeaderTrait>(self, header: H) -> Self {
        let header_val = header.to_header();
        eprintln!("Converting header to TypedHeader: {:?}", header_val);
        
        match TypedHeader::try_from(header_val) {
            Ok(typed_header) => {
                eprintln!("Successfully converted to TypedHeader: {:?}", typed_header);
                self.header(typed_header)
            },
            Err(e) => {
                eprintln!("Failed to convert to TypedHeader: {:?}", e);
                self
            }
        }
    }
}

impl HeaderSetter for crate::builder::SimpleResponseBuilder {
    fn set_header<H: TypedHeaderTrait>(self, header: H) -> Self {
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