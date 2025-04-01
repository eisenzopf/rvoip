//! Core SIP protocol implementation for rvoip
//!
//! This crate provides the fundamental SIP message types, parsing,
//! and serialization functionality for the rvoip stack.

mod error;
mod header;
mod message;
mod method;
mod parser;
mod uri;
mod version;

pub use error::{Error, Result};
pub use header::{Header, HeaderName, HeaderValue};
pub use message::{Message, Request, Response, StatusCode};
pub use method::Method;
pub use parser::parse_message;
pub use uri::Uri;
pub use version::Version;

/// Re-export of common types and functions
pub mod prelude {
    pub use super::{Error, Header, HeaderName, HeaderValue, Message, Method, Request, Response, Result, StatusCode, Uri, Version, parse_message};
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
} 