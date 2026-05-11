// Headers module
//
// This module contains all the SIP header implementations.
// Each header type is implemented in its own file.

use crate::error::{Error, Result};

// Define sub-modules
pub mod common;
pub mod header;
pub mod header_access;
pub mod header_name;
pub mod header_value;
pub mod typed_header;

// Tests
#[cfg(test)]
mod tests;

// Re-export common types for convenience
pub use common::*;
pub use header::Header;
pub use header_access::*;
pub use header_name::HeaderName;
pub use header_value::HeaderValue;
pub use typed_header::{TypedHeader, TypedHeaderTrait};
