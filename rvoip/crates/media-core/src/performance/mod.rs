//! Performance optimizations for media processing
//!
//! This module contains zero-copy implementations, object pooling, and
//! performance monitoring tools for high-performance media processing.

pub mod zero_copy;
pub mod pool;
pub mod metrics;
pub mod simd;

// Re-export main performance types
pub use zero_copy::{ZeroCopyAudioFrame, SharedAudioBuffer};
pub use pool::{AudioFramePool, PooledAudioFrame};
pub use metrics::{PerformanceMetrics, BenchmarkResults}; 