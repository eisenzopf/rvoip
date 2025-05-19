//! Server security implementation
//!
//! This module contains the implementation of the server-specific security functionality.

mod server_security_impl;

// Re-export the implementation
pub use server_security_impl::DefaultServerSecurityContext; 