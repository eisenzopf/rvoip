//! Server transport implementation
//!
//! This module contains the implementation of the server-specific transport functionality.

mod server_transport_impl;

// Re-export the implementation
pub use server_transport_impl::DefaultMediaTransportServer; 