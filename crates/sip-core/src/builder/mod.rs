#![doc = include_str!("builder.md")]

mod request;
mod response;
pub mod headers;
pub mod multipart;
pub use request::SimpleRequestBuilder;
pub use response::SimpleResponseBuilder;
pub use headers::*;
pub use multipart::MultipartBodyBuilder;

#[cfg(test)]
mod tests; 