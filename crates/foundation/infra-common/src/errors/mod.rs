/*!
Error Handling

This module provides standardized error types and utilities
for the RVOIP stack. It includes:

- Common error types
- Error context utilities
- Error conversion traits
*/

mod context;
pub mod types;

pub use context::{ErrorContext, ErrorExt};
pub use types::{Error, Result};
