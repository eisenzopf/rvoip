//! Security components
//!
//! This module contains functionality related to media transport security:
//! - DTLS handshake management
//! - SRTP key negotiation
//! - Security context handling

// Re-export modules
pub mod client_security;

// Re-export important types and functions
pub use client_security::{
    initialize_security, is_secure, get_security_info,
    start_handshake, wait_for_handshake, is_handshake_complete,
    set_remote_fingerprint, close_security
}; 