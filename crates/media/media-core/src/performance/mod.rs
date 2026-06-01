//! Performance optimizations for media processing
//!
//! This module contains zero-copy implementations, object pooling, and
//! performance monitoring tools for high-performance media processing.

pub mod metrics;
pub mod pool;
pub mod simd;
pub mod zero_copy;

// Re-export main performance types
pub use metrics::{BenchmarkResults, PerformanceMetrics};
pub use pool::{AudioFramePool, PooledAudioFrame};
pub use zero_copy::{SharedAudioBuffer, ZeroCopyAudioFrame};
