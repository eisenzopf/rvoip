//! DTLS message types
//!
//! This module contains the different message types used in DTLS.

pub mod handshake;
pub mod content;
pub mod extension;

// Re-export common types for convenience
pub use handshake::HandshakeMessage;
pub use content::ContentMessage;
pub use extension::Extension; 