//! Performance metrics and benchmarking for media processing
//!
//! This module provides tools to measure and compare the performance
//! of different audio processing strategies.

use std::time::{Duration, Instant};
use std::collections::VecDeque;
use crate::types::AudioFrame;
use crate::performance::zero_copy::ZeroCopyAudioFrame;
use crate::performance::pool::{AudioFramePool, PoolConfig, PooledAudioFrame};

/// Performance measurement results
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// Total processing time
    pub total_time: Duration,
    /// Average time per operation
    pub avg_time: Duration,
    /// Minimum time recorded
    pub min_time: Duration,
    /// Maximum time recorded
    pub max_time: Duration,
    /// Number of operations measured
    pub operation_count: u64,
    /// Memory allocations (estimated)
    pub allocation_count: u64,
    /// Total memory allocated (bytes)
    pub memory_allocated: u64,
}

impl PerformanceMetrics {
    /// Create empty metrics
    pub fn new() -> Self {
        Self {
            total_time: Duration::ZERO,
            avg_time: Duration::ZERO,
            min_time: Duration::MAX,
            max_time: Duration::ZERO,
            operation_count: 0,
            allocation_count: 0,
            memory_allocated: 0,
        }
    }
    
    /// Create disabled metrics collector (no-op for performance)
    pub fn disabled() -> Self {
        // Same as new() for now - could be optimized to be a no-op struct
        Self::new()
    }
    
    /// Add a timing measurement
    pub fn add_timing(&mut self, duration: Duration) {
        self.total_time += duration;
        self.operation_count += 1;
        self.min_time = self.min_time.min(duration);
        self.max_time = self.max_time.max(duration);
        self.avg_time = self.total_time / self.operation_count as u32;
    }
    
    /// Add memory allocation information
    pub fn add_allocation(&mut self, size_bytes: u64) {
        self.allocation_count += 1;
        self.memory_allocated += size_bytes;
    }
    
    /// Calculate performance ratio compared to baseline
    pub fn performance_ratio(&self, baseline: &PerformanceMetrics) -> f64 {
        if baseline.avg_time.is_zero() {
            return 1.0;
        }
        baseline.avg_time.as_nanos() as f64 / self.avg_time.as_nanos() as f64
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Comprehensive benchmark results comparing different approaches
#[derive(Debug, Clone)]
pub struct BenchmarkResults {
    /// Traditional AudioFrame performance
    pub traditional_metrics: PerformanceMetrics,
    /// Zero-copy AudioFrame performance
    pub zero_copy_metrics: PerformanceMetrics,
    /// Pooled AudioFrame performance
    pub pooled_metrics: PerformanceMetrics,
    /// Test parameters
    pub test_config: BenchmarkConfig,
}

/// Configuration for benchmarking
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Number of iterations
    pub iterations: usize,
    /// Frame size (samples per channel)
    pub frame_size: usize,
    /// Sample rate
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u8,
    /// Test name/description
    pub test_name: String,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            iterations: 1000,
            frame_size: 160, // 20ms at 8kHz
            sample_rate: 8000,
            channels: 1,
            test_name: "Audio Processing Benchmark".to_string(),
        }
    }
}

impl BenchmarkResults {
    /// Create a new benchmark results
    pub fn new(config: BenchmarkConfig) -> Self {
        Self {
            traditional_metrics: PerformanceMetrics::new(),
            zero_copy_metrics: PerformanceMetrics::new(),
            pooled_metrics: PerformanceMetrics::new(),
            test_config: config,
        }
    }
    
    /// Calculate performance improvements
    pub fn calculate_improvements(&self) -> BenchmarkSummary {
        BenchmarkSummary {
            zero_copy_speedup: self.zero_copy_metrics.performance_ratio(&self.traditional_metrics),
            pooled_speedup: self.pooled_metrics.performance_ratio(&self.traditional_metrics),
            traditional_avg_latency: self.traditional_metrics.avg_time,
            zero_copy_avg_latency: self.zero_copy_metrics.avg_time,
            pooled_avg_latency: self.pooled_metrics.avg_time,
        }
    }
    
    /// Print detailed results
    pub fn print_results(&self) {
        let summary = self.calculate_improvements();
        
        println!("\n🚀 {} Results", self.test_config.test_name);
        println!("================================================");
        println!("⏱️  Latency Results:");
        println!("  Traditional:  {:?}", summary.traditional_avg_latency);
        println!("  Zero-Copy:    {:?}", summary.zero_copy_avg_latency);
        println!("  Pooled:       {:?}", summary.pooled_avg_latency);
        
        println!("\n📈 Performance Improvements:");
        println!("  Zero-Copy Speedup:    {:.2}x", summary.zero_copy_speedup);
        println!("  Pooled Speedup:       {:.2}x", summary.pooled_speedup);
    }
}

/// Summary of benchmark improvements
#[derive(Debug, Clone)]
pub struct BenchmarkSummary {
    pub zero_copy_speedup: f64,
    pub pooled_speedup: f64,
    pub traditional_avg_latency: Duration,
    pub zero_copy_avg_latency: Duration,
    pub pooled_avg_latency: Duration,
}

/// Audio frame processing benchmark suite
pub struct AudioFrameBenchmark {
    config: BenchmarkConfig,
    pool: std::sync::Arc<AudioFramePool>,
}

impl AudioFrameBenchmark {
    /// Create a new benchmark suite
    pub fn new(config: BenchmarkConfig) -> Self {
        let pool_config = PoolConfig {
            initial_size: 32,
            max_size: 128,
            sample_rate: config.sample_rate,
            channels: config.channels,
            samples_per_frame: config.frame_size,
        };
        
        let pool = AudioFramePool::new(pool_config);
        
        Self { config, pool }
    }
    
    /// Run comprehensive benchmark comparing all approaches
    pub fn run_comprehensive_benchmark(&self) -> BenchmarkResults {
        let mut results = BenchmarkResults::new(self.config.clone());
        
        println!("🔬 Running comprehensive audio frame benchmark...");
        
        // Benchmark traditional AudioFrame processing
        println!("Testing traditional AudioFrame...");
        results.traditional_metrics = self.benchmark_traditional_frames();
        
        // Benchmark zero-copy processing  
        println!("Testing zero-copy AudioFrame...");
        results.zero_copy_metrics = self.benchmark_zero_copy_frames();
        
        // Benchmark pooled processing
        println!("Testing pooled AudioFrame...");
        results.pooled_metrics = self.benchmark_pooled_frames();
        
        results
    }
    
    /// Benchmark traditional AudioFrame operations
    fn benchmark_traditional_frames(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        let sample_size = self.config.frame_size * self.config.channels as usize;
        
        for _i in 0..self.config.iterations {
            let start = Instant::now();
            
            // Create frame (allocation)
            let samples = vec![0i16; sample_size];
            let frame = AudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);
            
            // Clone frame (copy)
            let _cloned_frame = frame.clone();
            
            // Modify frame (more copies)
            let mut modified_samples = frame.samples.clone();
            for sample in modified_samples.iter_mut() {
                *sample = (*sample).saturating_add(100);
            }
            let _modified_frame = AudioFrame::new(modified_samples, frame.sample_rate, frame.channels, frame.timestamp);
            
            let elapsed = start.elapsed();
            metrics.add_timing(elapsed);
            
            // Estimate memory allocations
            metrics.add_allocation((sample_size * std::mem::size_of::<i16>() * 3) as u64);
        }
        
        metrics
    }
    
    /// Benchmark zero-copy AudioFrame operations
    fn benchmark_zero_copy_frames(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        let sample_size = self.config.frame_size * self.config.channels as usize;
        
        for _i in 0..self.config.iterations {
            let start = Instant::now();
            
            // Create frame (single allocation)
            let samples = vec![0i16; sample_size];
            let frame = ZeroCopyAudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);
            
            // Clone frame (no copy - just Arc clone)
            let _cloned_frame = frame.clone();
            
            // Create slice (no copy - just new view)
            let _slice = frame.slice(0, self.config.frame_size / 2);
            
            let elapsed = start.elapsed();
            metrics.add_timing(elapsed);
            
            // Only one allocation for the initial samples
            metrics.add_allocation((sample_size * std::mem::size_of::<i16>()) as u64);
        }
        
        metrics
    }
    
    /// Benchmark pooled AudioFrame operations
    fn benchmark_pooled_frames(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        
        // Pre-warm the pool
        self.pool.prewarm(16);
        
        for _i in 0..self.config.iterations {
            let start = Instant::now();
            
            // Get frame from pool (likely no allocation)
            let frame = self.pool.get_frame();
            
            // Clone frame (no copy - just Arc clone)
            let _cloned_frame = frame.clone();
            
            // Frame automatically returns to pool on drop
            drop(frame);
            
            let elapsed = start.elapsed();
            metrics.add_timing(elapsed);
            
            // Pool reuse means minimal allocations
            metrics.add_allocation(0); // Pool hit = no allocation
        }
        
        // Account for pool misses in memory calculation
        let pool_stats = self.pool.get_stats();
        let missed_allocations = (pool_stats.pool_misses as u64) * 
            (self.config.frame_size as u64 * self.config.channels as u64 * std::mem::size_of::<i16>() as u64);
        metrics.memory_allocated = missed_allocations;
        metrics.allocation_count = pool_stats.pool_misses as u64;
        
        metrics
    }
    
    /// Benchmark codec processing pipeline
    pub fn benchmark_codec_pipeline(&self) -> BenchmarkResults {
        println!("🔬 Benchmarking codec processing pipeline...");
        
        let mut results = BenchmarkResults::new(self.config.clone());
        
        // Traditional pipeline
        results.traditional_metrics = self.benchmark_traditional_codec_pipeline();
        
        // Zero-copy pipeline  
        results.zero_copy_metrics = self.benchmark_zero_copy_codec_pipeline();
        
        // Pooled pipeline
        results.pooled_metrics = self.benchmark_pooled_codec_pipeline();
        
        results
    }
    
    fn benchmark_traditional_codec_pipeline(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        let sample_size = self.config.frame_size * self.config.channels as usize;
        
        for _i in 0..self.config.iterations {
            let start = Instant::now();
            
            // Simulate codec processing pipeline with copies
            let samples = vec![100i16; sample_size];
            let input_frame = AudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);
            
            // Stage 1: Pre-processing (copy)
            let preprocessed_samples = input_frame.samples.clone();
            let preprocessed_frame = AudioFrame::new(preprocessed_samples, input_frame.sample_rate, input_frame.channels, input_frame.timestamp);
            
            // Stage 2: Main processing (copy)
            let processed_samples = preprocessed_frame.samples.clone();
            let processed_frame = AudioFrame::new(processed_samples, preprocessed_frame.sample_rate, preprocessed_frame.channels, preprocessed_frame.timestamp);
            
            // Stage 3: Post-processing (copy)
            let final_samples = processed_frame.samples.clone();
            let _final_frame = AudioFrame::new(final_samples, processed_frame.sample_rate, processed_frame.channels, processed_frame.timestamp);
            
            let elapsed = start.elapsed();
            metrics.add_timing(elapsed);
            metrics.add_allocation((sample_size * std::mem::size_of::<i16>() * 4) as u64);
        }
        
        metrics
    }
    
    fn benchmark_zero_copy_codec_pipeline(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        let sample_size = self.config.frame_size * self.config.channels as usize;
        
        for _i in 0..self.config.iterations {
            let start = Instant::now();
            
            // Simulate codec processing pipeline with zero-copy
            let samples = vec![100i16; sample_size];
            let input_frame = ZeroCopyAudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);
            
            // Stage 1: Pre-processing (no copy - just reference)
            let preprocessed_frame = input_frame.clone();
            
            // Stage 2: Main processing (no copy - just reference)  
            let processed_frame = preprocessed_frame.clone();
            
            // Stage 3: Post-processing (no copy - just reference)
            let _final_frame = processed_frame.clone();
            
            let elapsed = start.elapsed();
            metrics.add_timing(elapsed);
            metrics.add_allocation((sample_size * std::mem::size_of::<i16>()) as u64); // Only initial allocation
        }
        
        metrics
    }
    
    fn benchmark_pooled_codec_pipeline(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        
        for _i in 0..self.config.iterations {
            let start = Instant::now();
            
            // Simulate codec processing pipeline with pooled frames
            let input_frame = self.pool.get_frame();
            
            // Stage 1: Pre-processing (no copy - just reference)
            let preprocessed_frame = input_frame.clone();
            
            // Stage 2: Main processing (no copy - just reference)
            let processed_frame = preprocessed_frame.clone();
            
            // Stage 3: Post-processing (no copy - just reference)
            let _final_frame = processed_frame.clone();
            
            let elapsed = start.elapsed();
            metrics.add_timing(elapsed);
            // Pool reuse means minimal allocations
        }
        
        // Account for pool allocations
        let pool_stats = self.pool.get_stats();
        metrics.allocation_count = pool_stats.pool_misses as u64;
        let missed_allocations = (pool_stats.pool_misses as u64) * 
            (self.config.frame_size as u64 * self.config.channels as u64 * std::mem::size_of::<i16>() as u64);
        metrics.memory_allocated = missed_allocations;
        
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_metrics() {
        let mut metrics = PerformanceMetrics::new();
        
        metrics.add_timing(Duration::from_micros(100));
        metrics.add_timing(Duration::from_micros(200));
        metrics.add_timing(Duration::from_micros(150));
        
        assert_eq!(metrics.operation_count, 3);
        assert_eq!(metrics.avg_time, Duration::from_micros(150));
        assert_eq!(metrics.min_time, Duration::from_micros(100));
        assert_eq!(metrics.max_time, Duration::from_micros(200));
    }
    
    #[test]
    fn test_performance_ratio() {
        let mut baseline = PerformanceMetrics::new();
        baseline.add_timing(Duration::from_micros(200));
        
        let mut improved = PerformanceMetrics::new();
        improved.add_timing(Duration::from_micros(100));
        
        let ratio = improved.performance_ratio(&baseline);
        assert_eq!(ratio, 2.0); // 2x faster
    }
    
    #[test]
    fn test_benchmark_config() {
        let config = BenchmarkConfig::default();
        assert_eq!(config.iterations, 1000);
        assert_eq!(config.frame_size, 160);
        assert_eq!(config.sample_rate, 8000);
    }
    
    #[test]
    fn test_audio_frame_benchmark_creation() {
        let config = BenchmarkConfig::default();
        let benchmark = AudioFrameBenchmark::new(config);
        
        // Should have created a pool
        let stats = benchmark.pool.get_stats();
        assert!(stats.pool_size > 0);
    }
    
    #[test] 
    fn test_benchmark_results() {
        let config = BenchmarkConfig {
            iterations: 10, // Small for test speed
            ..Default::default()
        };
        
        let benchmark = AudioFrameBenchmark::new(config);
        let results = benchmark.run_comprehensive_benchmark();
        
        // All metrics should have been populated
        assert!(results.traditional_metrics.operation_count > 0);
        assert!(results.zero_copy_metrics.operation_count > 0);
        assert!(results.pooled_metrics.operation_count > 0);
        
        // Zero-copy should typically be faster (though test data might be small)
        let summary = results.calculate_improvements();
        assert!(summary.zero_copy_speedup >= 0.5); // At least not much slower
        assert!(summary.pooled_speedup >= 0.5);
    }
} 