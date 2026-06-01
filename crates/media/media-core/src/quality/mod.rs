//! Quality Monitoring and Adaptation
//!
//! This module provides real-time quality monitoring, metrics collection,
//! and adaptive quality management for media sessions.

pub mod adaptation;
pub mod metrics;
pub mod monitor;

// Re-export main types
pub use adaptation::{AdaptationEngine, AdaptationStrategy, QualityAdjustment};
pub use metrics::{OverallMetrics, QualityMetrics, SessionMetrics};
pub use monitor::{QualityMonitor, QualityMonitorConfig};
