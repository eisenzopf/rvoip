//! DTLS message types
//!
//! This module contains the different message types used in DTLS.

pub mod content;
pub mod extension;
pub mod handshake;

// Re-export common types for convenience
pub use content::ContentMessage;
pub use extension::Extension;
pub use handshake::HandshakeMessage;
