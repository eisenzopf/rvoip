/*!
Error Handling

This module provides standardized error types and utilities
for the RVOIP stack. It includes:

- Common error types
- Error context utilities
- Error conversion traits
*/

pub mod types;
mod context;

pub use types::{Error, Result};
pub use context::{ErrorContext, ErrorExt}; 