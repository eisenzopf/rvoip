//! Performance optimization tests
//!
//! This module tests the zero-copy and object pooling optimizations.
//!
//! Wall-clock measurements are printed as local diagnostics only. Comparative
//! timing belongs in the Criterion benchmarks (`cargo bench -p
//! rvoip-media-core`), because a single sample inside `cargo test` is sensitive
//! to scheduler contention on hosted CI.

use rvoip_media_core::performance::{
    metrics::{BenchmarkConfig, BenchmarkResults, PerformanceMetrics},
    pool::{AudioFramePool, PoolConfig},
    simd::SimdProcessor,
    zero_copy::ZeroCopyAudioFrame,
};
use rvoip_media_core::prelude::*;
use serial_test::serial;
use std::time::Instant;

/// Audio frame processing benchmark
struct AudioFrameBenchmark {
    config: BenchmarkConfig,
    pool: std::sync::Arc<AudioFramePool>,
    /// SIMD processor; held by the benchmark but accessed via the
    /// closure inside `run_*`, so the direct read isn't visible.
    #[allow(dead_code)]
    simd: SimdProcessor,
}

impl AudioFrameBenchmark {
    fn new(config: BenchmarkConfig) -> Self {
        let pool_config = PoolConfig {
            initial_size: 32,
            max_size: 128,
            sample_rate: config.sample_rate,
            channels: config.channels,
            samples_per_frame: config.frame_size,
        };

        let pool = AudioFramePool::new(pool_config);
        let simd = SimdProcessor::new();

        Self { config, pool, simd }
    }

    /// Benchmark traditional AudioFrame operations
    fn benchmark_traditional_frames(&self) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::new();
        let sample_size = self.config.frame_size * self.config.channels as usize;

        for _i in 0..self.config.iterations {
            let start = Instant::now();

            // Create frame (allocation)
            let samples = vec![100i16; sample_size];
            let frame = AudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);

            // Clone frame (copy)
            let _cloned_frame = frame.clone();

            // Modify frame (more copies)
            let mut modified_samples = frame.samples.clone();
            for sample in modified_samples.iter_mut() {
                *sample = (*sample).saturating_add(50);
            }
            let _modified_frame = AudioFrame::new(
                modified_samples,
                frame.sample_rate,
                frame.channels,
                frame.timestamp,
            );

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
            let samples = vec![100i16; sample_size];
            let frame =
                ZeroCopyAudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);

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
        }

        // Account for pool misses in memory calculation
        let pool_stats = self.pool.get_stats();
        metrics.memory_allocated = (pool_stats.pool_misses as u64)
            * (self.config.frame_size * self.config.channels as usize * std::mem::size_of::<i16>())
                as u64;
        metrics.allocation_count = pool_stats.pool_misses as u64;

        metrics
    }

    /// Run comprehensive benchmark
    fn run_comprehensive_benchmark(&self) -> BenchmarkResults {
        println!("🔬 Running comprehensive audio frame benchmark...");
        println!(
            "Configuration: {} iterations, {} samples, {} Hz, {} channels",
            self.config.iterations,
            self.config.frame_size,
            self.config.sample_rate,
            self.config.channels
        );

        let traditional_metrics = self.benchmark_traditional_frames();
        let zero_copy_metrics = self.benchmark_zero_copy_frames();
        let pooled_metrics = self.benchmark_pooled_frames();

        BenchmarkResults {
            traditional_metrics,
            zero_copy_metrics,
            pooled_metrics,
            test_config: self.config.clone(),
        }
    }
}

/// Validate the deterministic work and allocation invariants behind a benchmark
/// run. Timing is intentionally excluded: ordinary tests should prove that each
/// strategy did equivalent work and retained its allocation characteristics,
/// while Criterion owns comparative timing.
fn assert_benchmark_invariants(results: &BenchmarkResults) {
    let iterations = results.test_config.iterations as u64;
    let frame_bytes = results.test_config.frame_size as u64
        * u64::from(results.test_config.channels)
        * std::mem::size_of::<i16>() as u64;

    assert_eq!(results.traditional_metrics.operation_count, iterations);
    assert_eq!(results.zero_copy_metrics.operation_count, iterations);
    assert_eq!(results.pooled_metrics.operation_count, iterations);

    assert_eq!(results.traditional_metrics.allocation_count, iterations);
    assert_eq!(results.zero_copy_metrics.allocation_count, iterations);
    assert_eq!(results.pooled_metrics.allocation_count, 0);

    assert_eq!(
        results.traditional_metrics.memory_allocated,
        iterations * frame_bytes * 3
    );
    assert_eq!(
        results.zero_copy_metrics.memory_allocated,
        iterations * frame_bytes
    );
    assert_eq!(results.pooled_metrics.memory_allocated, 0);
}

#[tokio::test]
#[serial]
async fn test_zero_copy_audio_frame() {
    println!("\n🚀 Zero-Copy AudioFrame Test");
    println!("==============================");

    let samples = vec![100, 200, 300, 400, 500, 600];
    let frame = ZeroCopyAudioFrame::new(samples.clone(), 8000, 2, 1000);

    // Test basic properties
    assert_eq!(frame.samples(), &samples);
    assert_eq!(frame.sample_rate, 8000);
    assert_eq!(frame.channels, 2);
    assert_eq!(frame.samples_per_channel(), 3);

    // Test zero-copy cloning
    let cloned_frame = frame.clone();
    assert_eq!(frame.ref_count(), 2);
    assert_eq!(cloned_frame.ref_count(), 2);

    // Test zero-copy slicing
    let slice = frame.slice(1, 2).unwrap();
    assert_eq!(slice.samples(), &[300, 400, 500, 600]);
    assert_eq!(frame.ref_count(), 3); // Original + clone + slice

    println!(
        "✅ Zero-copy operations verified - {} references to same data",
        frame.ref_count()
    );
}

#[tokio::test]
#[serial]
async fn test_audio_frame_pool() {
    println!("\n🎱 AudioFrame Pool Test");
    println!("========================");

    let config = PoolConfig::default();
    let pool = AudioFramePool::new(config.clone());

    // Test pool creation
    let initial_stats = pool.get_stats();
    assert_eq!(initial_stats.pool_size, config.initial_size);
    println!("✅ Pool created with {} frames", initial_stats.pool_size);

    // Test frame allocation and return
    {
        let _frame1 = pool.get_frame();
        let _frame2 = pool.get_frame();

        let stats = pool.get_stats();
        assert_eq!(stats.pool_hits, 2);
        assert_eq!(stats.pool_size, config.initial_size - 2);
        println!("✅ Pool allocated 2 frames, {} remaining", stats.pool_size);

        // Frames will be returned automatically on drop
    }

    // Check frames returned
    let final_stats = pool.get_stats();
    assert_eq!(final_stats.returned_count, 2);
    assert_eq!(final_stats.pool_size, config.initial_size);
    println!("✅ Frames automatically returned to pool");
}

#[tokio::test]
#[serial]
async fn test_simd_processor() {
    println!("\n⚡ SIMD Processor Test");
    println!("======================");

    let processor = SimdProcessor::new();
    println!("SIMD available: {}", processor.is_simd_available());

    // Test gain application
    let input = vec![1000, -1000, 2000, -2000];
    let mut output = vec![0; 4];

    processor.apply_gain(&input, 0.5, &mut output);
    assert_eq!(output, vec![500, -500, 1000, -1000]);
    println!("✅ SIMD gain application working");

    // Test in-place gain application
    let mut samples = vec![1000, -1000, 2000, -2000];
    processor.apply_gain_in_place(&mut samples, 0.5);
    assert_eq!(samples, vec![500, -500, 1000, -1000]);
    println!("✅ SIMD in-place gain application working");

    // Test RMS calculation
    let samples = vec![1000, -1000, 1000, -1000];
    let rms = processor.calculate_rms(&samples);
    let expected = 1000.0;
    assert!((rms - expected).abs() < 1.0);
    println!("✅ SIMD RMS calculation working: {:.6}", rms);
}

#[tokio::test]
#[serial]
async fn test_performance_benchmark_small() {
    println!("\n📊 Performance Benchmark (Small)");
    println!("=================================");

    let config = BenchmarkConfig {
        iterations: 100, // Small for testing
        frame_size: 160,
        sample_rate: 8000,
        channels: 1,
        test_name: "Small Performance Test".to_string(),
    };

    let benchmark = AudioFrameBenchmark::new(config);
    let results = benchmark.run_comprehensive_benchmark();

    results.print_results();
    assert_benchmark_invariants(&results);

    println!("✅ Benchmark work and allocation invariants validated");
}

#[tokio::test]
#[serial]
async fn test_performance_benchmark_large() {
    println!("\n📊 Performance Benchmark (Large)");
    println!("=================================");

    let config = BenchmarkConfig {
        iterations: 1000,
        frame_size: 320, // 40ms at 8kHz
        sample_rate: 8000,
        channels: 2, // Stereo
        test_name: "Large Performance Test".to_string(),
    };

    let benchmark = AudioFrameBenchmark::new(config);
    let results = benchmark.run_comprehensive_benchmark();

    results.print_results();
    assert_benchmark_invariants(&results);

    println!("✅ Large-frame work and allocation invariants validated");
}

#[tokio::test]
#[serial]
async fn test_memory_efficiency() {
    println!("\n💾 Memory Efficiency Test");
    println!("==========================");

    let frame_size = 1600; // Large frame for memory testing
    let samples = vec![100i16; frame_size];

    // Traditional approach - multiple allocations
    let traditional_frame = AudioFrame::new(samples.clone(), 8000, 1, 0);
    let traditional_clone1 = traditional_frame.clone();
    let traditional_clone2 = traditional_frame.clone();

    // Zero-copy approach - single allocation
    let zero_copy_frame = ZeroCopyAudioFrame::new(samples, 8000, 1, 0);
    let zero_copy_clone1 = zero_copy_frame.clone();
    let zero_copy_clone2 = zero_copy_frame.clone();

    // Verify sharing
    assert_eq!(zero_copy_frame.ref_count(), 3);
    assert_eq!(zero_copy_clone1.ref_count(), 3);
    assert_eq!(zero_copy_clone2.ref_count(), 3);

    println!(
        "✅ Zero-copy sharing verified: {} references to same data",
        zero_copy_frame.ref_count()
    );

    // Test that all frames have same data
    assert_eq!(traditional_frame.samples, zero_copy_frame.samples());
    assert_eq!(traditional_clone1.samples, zero_copy_clone1.samples());
    assert_eq!(traditional_clone2.samples, zero_copy_clone2.samples());

    println!("✅ Data consistency verified across all clones");
}

#[tokio::test]
#[serial]
async fn test_audio_processing_pipeline_performance() {
    println!("\n🔄 Audio Processing Pipeline Performance");
    println!("=========================================");

    let iterations = 500;
    let frame_size = 160;
    let sample_rate = 8000;

    // Traditional pipeline timing. Retain the final output so this ordinary
    // test gates semantic equivalence instead of a scheduler-sensitive ratio.
    let mut traditional_output = None;
    let start = Instant::now();
    for _i in 0..iterations {
        let samples = vec![100i16; frame_size];
        let input_frame = AudioFrame::new(samples, sample_rate, 1, 0);

        // Simulate 3-stage processing pipeline with copies
        let stage1_samples = input_frame.samples.clone();
        let stage1_frame = AudioFrame::new(
            stage1_samples,
            input_frame.sample_rate,
            input_frame.channels,
            input_frame.timestamp,
        );

        let stage2_samples = stage1_frame.samples.clone();
        let stage2_frame = AudioFrame::new(
            stage2_samples,
            stage1_frame.sample_rate,
            stage1_frame.channels,
            stage1_frame.timestamp,
        );

        let final_samples = stage2_frame.samples.clone();
        let final_frame = AudioFrame::new(
            final_samples,
            stage2_frame.sample_rate,
            stage2_frame.channels,
            stage2_frame.timestamp,
        );
        traditional_output = Some(final_frame);
    }
    let traditional_time = start.elapsed();

    // Zero-copy pipeline timing
    let mut zero_copy_output = None;
    let start = Instant::now();
    for _i in 0..iterations {
        let samples = vec![100i16; frame_size];
        let input_frame = ZeroCopyAudioFrame::new(samples, sample_rate, 1, 0);

        // Simulate 3-stage processing pipeline with no copies
        let stage1_frame = input_frame.clone();
        let stage2_frame = stage1_frame.clone();
        let final_frame = stage2_frame.clone();
        zero_copy_output = Some(final_frame);
    }
    let zero_copy_time = start.elapsed();

    let speedup = traditional_time.as_nanos() as f64 / zero_copy_time.as_nanos() as f64;

    println!("Traditional pipeline: {:?}", traditional_time);
    println!("Zero-copy pipeline:   {:?}", zero_copy_time);
    println!("Speedup: {:.2}x", speedup);

    let traditional_output = traditional_output.expect("traditional pipeline produced a frame");
    let zero_copy_output = zero_copy_output.expect("zero-copy pipeline produced a frame");
    assert_eq!(traditional_output.samples, zero_copy_output.samples());
    assert_eq!(traditional_output.sample_rate, zero_copy_output.sample_rate);
    assert_eq!(traditional_output.channels, zero_copy_output.channels);
    assert_eq!(traditional_output.timestamp, zero_copy_output.timestamp);

    println!("✅ Pipeline output equivalence verified");
}

#[tokio::test]
#[serial]
async fn test_pool_vs_allocation_performance() {
    println!("\n🏊 Pool vs Allocation Performance");
    println!("==================================");

    let iterations = 1000;
    let pool_config = PoolConfig::default();
    let pool = AudioFramePool::new(pool_config);

    // Pre-warm pool
    pool.prewarm(50);
    let warmed_pool_size = pool.get_stats().pool_size;

    // Fresh allocation timing
    let mut allocated_sample_total = 0i64;
    let start = Instant::now();
    for _i in 0..iterations {
        let samples = vec![100i16; 160];
        let frame = ZeroCopyAudioFrame::new(samples, 8000, 1, 0);
        allocated_sample_total += i64::from(frame.samples()[0]);
    }
    let allocation_time = start.elapsed();

    // Pool reuse timing
    let mut pooled_sample_total = 0i64;
    let start = Instant::now();
    for _i in 0..iterations {
        let frame = pool.get_frame();
        pooled_sample_total += i64::from(frame.samples()[0]);
    }
    let pool_time = start.elapsed();

    let speedup = allocation_time.as_nanos() as f64 / pool_time.as_nanos() as f64;

    println!("Fresh allocation: {:?}", allocation_time);
    println!("Pool reuse:       {:?}", pool_time);
    println!("Speedup: {:.2}x", speedup);

    let pool_stats = pool.get_stats();
    println!(
        "Pool hits: {}, Pool misses: {}",
        pool_stats.pool_hits, pool_stats.pool_misses
    );

    assert_eq!(allocated_sample_total, iterations as i64 * 100);
    assert_eq!(pooled_sample_total, 0);
    assert_eq!(pool_stats.pool_hits, iterations);
    assert_eq!(pool_stats.pool_misses, 0);
    assert_eq!(pool_stats.allocated_count, iterations);
    assert_eq!(pool_stats.returned_count, iterations);
    assert_eq!(pool_stats.pool_size, warmed_pool_size);

    println!("✅ Pool reuse and accounting invariants verified");
}
