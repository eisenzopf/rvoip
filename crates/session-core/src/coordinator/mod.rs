//! Top-level Session Coordinator
//!
//! This is the main orchestrator for the entire session-core system.
//! It coordinates between dialog, media, and other subsystems.

// Module declarations
mod coordinator;
mod event_handler;
mod session_ops;
mod bridge_ops;
mod sip_client;
mod server_manager;
pub mod transfer;
pub mod registry;
pub mod registrar_integration;

// Re-exports
pub use coordinator::{SessionCoordinator, CleanupTracker, CleanupLayer};
pub use transfer::TransferHandler;
pub use registrar_integration::RegistrarIntegration; 