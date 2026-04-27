#![doc = include_str!("builder.md")]

pub mod headers;
pub mod multipart;
mod request;
mod response;
pub use headers::*;
pub use multipart::MultipartBodyBuilder;
pub use request::SimpleRequestBuilder;
pub use response::SimpleResponseBuilder;

#[cfg(test)]
mod tests;
