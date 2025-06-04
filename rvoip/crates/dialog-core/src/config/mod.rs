//! Configuration module for dialog-core
//! 
//! This module provides configuration types for dialog management,
//! including both the legacy split configuration (for backward compatibility)
//! and the new unified configuration system.

pub mod unified;

// Re-export unified types for easy access
pub use unified::{
    DialogManagerConfig, 
    ClientBehavior, 
    ServerBehavior, 
    HybridBehavior,
    ClientConfigBuilder,
    ServerConfigBuilder, 
    HybridConfigBuilder,
}; 