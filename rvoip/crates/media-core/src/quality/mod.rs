//! Media quality monitoring module for the media-core library
//!
//! This module provides components for monitoring and adapting to media quality,
//! including metrics collection, quality estimation, and adaptive quality control.

// Quality metrics collection
pub mod metrics;
pub use metrics::{QualityMetrics, NetworkMetrics, AudioMetrics};

// Quality estimation (MOS, etc.)
pub mod estimation;
pub use estimation::{QualityEstimator, QualityScore, QualityLevel};

// Quality-based adaptation
pub mod adaptation;
pub use adaptation::{QualityAdapter, AdaptationAction, AdaptationConfig}; 