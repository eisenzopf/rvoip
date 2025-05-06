#![doc = include_str!("builder.md")]

mod request;
mod response;
pub mod headers;
pub use request::SimpleRequestBuilder;
pub use response::SimpleResponseBuilder;
pub use headers::*;

#[cfg(test)]
mod tests; 