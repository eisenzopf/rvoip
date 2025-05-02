/// # SIP Builders
///
/// This module provides builder patterns for creating SIP messages with a fluent, chainable API.
/// The builders support all common SIP headers defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261)
/// and various extensions.
///
/// See the documentation for [`SimpleRequestBuilder`] and [`SimpleResponseBuilder`] for detailed 
/// examples of how to use the builders to create SIP messages.

mod request;
mod response;
pub mod headers;

pub use request::SimpleRequestBuilder;
pub use response::SimpleResponseBuilder;
pub use headers::*;

#[cfg(test)]
mod tests; 