//! Client security implementation
//!
//! This module contains the implementation of the client-specific security functionality.

mod client_security_impl;

// Re-export the implementation
pub use client_security_impl::DefaultClientSecurityContext; 