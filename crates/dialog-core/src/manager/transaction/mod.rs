//! Transaction Integration Modular Implementation
//!
//! This module provides a modular implementation of transaction integration between
//! dialog-core and transaction-core, organized into focused submodules.
//!
//! ## Submodules
//!
//! - [`traits`]: Core traits for transaction integration interfaces
//! - [`request_operations`]: Request sending operations and dialog-aware request building
//!
//! The original implementation has been partially modularized. Additional modules
//! for event processing and transaction management will be added as needed.

pub mod traits;
pub mod request_operations;

// Re-export the main types for external use
pub use traits::{TransactionIntegration, TransactionHelpers}; 