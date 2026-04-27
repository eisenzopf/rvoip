//! Media Engine - Central orchestrator for media processing
//!
//! This module contains the MediaEngine which coordinates all media processing
//! activities including codec management, session management, and integration
//! with other crates.

pub mod config;
pub mod lifecycle;
pub mod media_engine;

// Re-export main types for convenience
pub use config::{EngineCapabilities, MediaEngineConfig};
pub use lifecycle::{EngineState, LifecycleManager};
pub use media_engine::{MediaEngine, MediaSessionHandle, MediaSessionParams};
