//! # rvoip-client-core
//!
//! SIP client coordination layer that leverages the rvoip infrastructure for client applications.
//!
//! This crate provides client-specific session management on top of the existing rvoip
//! infrastructure (transaction-core, media-core, rtp-core, etc.) to enable SIP client
//! applications.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    sip-client (Future)                      │
//! │              (Client Application Logic)                     │
//! ├─────────────────────────────────────────────────────────────┤
//! │                  client-core (This Crate)                   │
//! │             (Client Session Management)                     │
//! ├─────────────────────────────────────────────────────────────┤
//! │  transaction-core │  media-core  │  rtp-core   ← REUSE!    │
//! │  (SIP Protocol)   │  (Media)     │  (RTP)      ← REUSE!    │
//! ├─────────────────────────────────────────────────────────────┤
//! │              sip-transport │ UDP/TCP          ← REUSE!      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key Features
//!
//! - **Code Reuse**: 80% of infrastructure shared with server-side
//! - **Memory Safety**: Full Rust memory safety guarantees
//! - **Async Performance**: Built on tokio for high performance
//! - **Protocol Compliance**: Leverages same RFC-compliant SIP handling
//! - **Clean APIs**: Event-driven architecture for UI integration

pub mod client;
pub mod registration;
pub mod call;
pub mod events;
pub mod error;

// Re-export core types and traits
pub use client::{ClientManager, ClientConfig};
pub use registration::{RegistrationManager, RegistrationStatus, RegistrationConfig};
pub use call::{CallManager, CallState, CallInfo, CallId};
pub use events::{
    ClientEventHandler, ClientEvent, IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo,
    CallAction, Credentials, MediaEventType
};
pub use error::{ClientError, ClientResult};

// Re-export commonly used types from infrastructure
pub use rvoip_sip_core::{Uri, Request, Response};
pub use infra_common::EventBus;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_core_compiles() {
        // Basic compilation test
        assert!(true);
    }
} 