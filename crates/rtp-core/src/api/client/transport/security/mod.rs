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
    close_security, get_security_info, initialize_security, is_handshake_complete, is_secure,
    set_remote_fingerprint, start_handshake, wait_for_handshake,
};
