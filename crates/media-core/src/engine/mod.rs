//! Media Engine - Central orchestrator for media processing
//!
//! This module contains the MediaEngine which coordinates all media processing
//! activities including codec management, session management, and integration
//! with other crates.

pub mod media_engine;
pub mod config;
pub mod lifecycle;

// Re-export main types for convenience
pub use media_engine::{MediaEngine, MediaSessionParams, MediaSessionHandle};
pub use config::{MediaEngineConfig, EngineCapabilities};
pub use lifecycle::{EngineState, LifecycleManager}; 