//! Quality Monitoring and Adaptation
//!
//! This module provides real-time quality monitoring, metrics collection,
//! and adaptive quality management for media sessions.

pub mod monitor;
pub mod metrics;
pub mod adaptation;

// Re-export main types
pub use monitor::{QualityMonitor, QualityMonitorConfig};
pub use metrics::{QualityMetrics, SessionMetrics, OverallMetrics};
pub use adaptation::{QualityAdjustment, AdaptationEngine, AdaptationStrategy}; 