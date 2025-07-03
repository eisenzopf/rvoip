//! # rvoip - A comprehensive VoIP library for Rust
//!
//! This crate provides a complete VoIP (Voice over IP) implementation in Rust,
//! including SIP, RTP, media processing, and call management capabilities.
//!
//! ## Overview
//!
//! The rvoip library is composed of several core components:
//!
//! - **SIP Core**: SIP protocol implementation and message parsing
//! - **SIP Transport**: Transport layer for SIP messages
//! - **Transaction Core**: SIP transaction management
//! - **Dialog Core**: SIP dialog state management
//! - **RTP Core**: Real-time Transport Protocol implementation
//! - **Media Core**: Audio/video processing and codec support
//! - **Session Core**: Session management and coordination
//! - **Client Core**: High-level client API
//! - **Call Engine**: Call routing and business logic
//! - **Infra Common**: Common infrastructure and utilities
//!
//! ## Quick Start
//!
//! ```rust
//! use rvoip::client_core::*;
//! use rvoip::session_core::*;
//! 
//! // Your VoIP application code here
//! ```
//!
//! ## Module Structure
//!
//! Each module corresponds to a specific aspect of VoIP functionality:
//!
//! - [`sip_core`]: Core SIP protocol implementation
//! - [`client_core`]: High-level client API
//! - [`session_core`]: Session management
//! - [`call_engine`]: Call routing and business logic
//! - [`rtp_core`]: RTP implementation
//! - [`media_core`]: Media processing
//! - [`dialog_core`]: Dialog state management
//! - [`transaction_core`]: Transaction management
//! - [`sip_transport`]: SIP transport layer

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

// Re-export all crates as modules
pub use rvoip_sip_core as sip_core;
pub use rvoip_sip_transport as sip_transport;
pub use rvoip_transaction_core as transaction_core;
pub use rvoip_dialog_core as dialog_core;
pub use rvoip_rtp_core as rtp_core;
pub use rvoip_media_core as media_core;
pub use rvoip_session_core as session_core;
pub use rvoip_call_engine as call_engine;
pub use rvoip_client_core as client_core;

// Re-export commonly used items for convenience
pub mod prelude {
    //! Common imports for rvoip applications
    //!
    //! This module provides convenient access to the most commonly used types
    //! from the rvoip ecosystem. Instead of importing everything with wildcards,
    //! users can import specific types they need from the individual crates.
    //!
    //! # Usage Examples
    //!
    //! ```rust
    //! // Import specific modules and types as needed
    //! use rvoip::sip_core::prelude::*;
    //! use rvoip::client_core::{Client, ClientBuilder};
    //! use rvoip::session_core::{SessionManager, SessionId};
    //!
    //! // Use the types normally
    //! // let client = ClientBuilder::new()...
    //! ```
    //!
    //! # Available Modules
    //!
    //! - [`crate::sip_core`] - Core SIP protocol types and parsing
    //! - [`crate::client_core`] - High-level client API
    //! - [`crate::session_core`] - Session management
    //! - [`crate::call_engine`] - Call routing and business logic
    //! - [`crate::rtp_core`] - RTP implementation
    //! - [`crate::media_core`] - Media processing
    //! - [`crate::dialog_core`] - Dialog state management
    //! - [`crate::transaction_core`] - Transaction management
    //! - [`crate::sip_transport`] - SIP transport layer
    
    // Note: We don't re-export specific types to avoid naming conflicts.
    // Users should import from the specific crates they need.
}

// Version information
/// The version of the rvoip library
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The description of the rvoip library
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION"); 