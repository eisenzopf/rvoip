//! DialogServer Modular Implementation
//!
//! This module provides a modular implementation of the DialogServer, organized into
//! focused submodules for better maintainability and separation of concerns.
//!
//! ## Submodules
//!
//! - [`core`]: Core server struct, constructors, and configuration
//! - [`call_operations`]: Call lifecycle management (handle, accept, reject, terminate)
//! - [`dialog_operations`]: Dialog management operations (create, query, list, terminate)
//! - [`response_builder`]: Response building and sending functionality
//! - [`sip_methods`]: Specialized SIP method handlers (BYE, REFER, NOTIFY, etc.)

pub mod core;
pub mod call_operations;
pub mod dialog_operations;
pub mod response_builder;
pub mod sip_methods;

// Re-export the main types for external use
pub use core::{DialogServer, ServerStats}; 