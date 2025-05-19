//! Client transport implementation
//!
//! This module contains the implementation of the client-specific transport functionality.

mod client_transport_impl;

// Re-export the implementation
pub use client_transport_impl::DefaultMediaTransportClient; 