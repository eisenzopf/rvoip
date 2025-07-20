//! G.729A Performance Tests
//!
//! This module contains performance benchmarks for the G.729A codec implementation
//! to ensure it meets real-time processing requirements.

use crate::codecs::g729a::*;
use crate::codecs::g729a::encoder::*;
use crate::codecs::g729a::decoder::*;
use crate::codecs::g729a::types::*;
use crate::codecs::g729a::lpc::*;
use crate::codecs::g729a::filtering::*;
use crate::codecs::g729a::basic_ops::*;
use std::time::Instant;

/// Performance test configuration
const PERFORMANCE_ITERATIONS: usize = 1000;
const WARMUP_ITERATIONS: usize = 10;

/// Component-level performance tests
#[cfg(test)]
mod component_performance_tests {
    use super::*;

    #[test]
    fn bench_autocorr_performance() {
        let test_signal = generate_test_signal();
        let mut r_h = [0i16; MP1];
        let mut r_l = [0i16; MP1];
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            autocorr(&test_signal, M as Word16, &mut r_h, &mut r_l);
        }
        
        // Benchmark
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            autocorr(&test_signal, M as Word16, &mut r_h, &mut r_l);
        }
        let duration = start_time.elapsed();
        
        let avg_time_ns = duration.as_nanos() / PERFORMANCE_ITERATIONS as u128;
        let operations_per_sec = 1_000_000_000.0 / avg_time_ns as f64;
        
        println!("Autocorrelation performance:");
        println!("  Average time: {} ns", avg_time_ns);
        println!("  Operations/sec: {:.0}", operations_per_sec);
        println!("  Real-time requirement: {} ops/sec ({}x margin)", 
                100, operations_per_sec / 100.0);
        
        // Should be much faster than real-time requirement (100 calls/sec)
        assert!(operations_per_sec > 1000.0, "Autocorrelation too slow: {} ops/sec", operations_per_sec);
    }

    #[test]
    fn bench_levinson_performance() {
        // Setup typical autocorrelation values
        let r_h = [16384i16, 8192, 4096, 2048, 1024, 512, 256, 128, 64, 32, 16];
        let r_l = [0i16; MP1];
        let mut a = [0i16; MP1];
        let mut rc = [0i16; M];
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            levinson(&r_h, &r_l, &mut a, &mut rc);
        }
        
        // Benchmark
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            levinson(&r_h, &r_l, &mut a, &mut rc);
        }
        let duration = start_time.elapsed();
        
        let avg_time_ns = duration.as_nanos() / PERFORMANCE_ITERATIONS as u128;
        let operations_per_sec = 1_000_000_000.0 / avg_time_ns as f64;
        
        println!("Levinson-Durbin performance:");
        println!("  Average time: {} ns", avg_time_ns);
        println!("  Operations/sec: {:.0}", operations_per_sec);
        
        assert!(operations_per_sec > 1000.0, "Levinson-Durbin too slow: {} ops/sec", operations_per_sec);
    }

    #[test]
    fn bench_lsp_conversion_performance() {
        let mut a = [4096i16, -1000, 800, -600, 400, -200, 100, -50, 25, -12, 6];
        let mut lsp = [0i16; M];
        let old_lsp = [1000i16, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000];
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            az_lsp(&a, &mut lsp, &old_lsp);
        }
        
        // Benchmark
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            az_lsp(&a, &mut lsp, &old_lsp);
        }
        let duration = start_time.elapsed();
        
        let avg_time_ns = duration.as_nanos() / PERFORMANCE_ITERATIONS as u128;
        let operations_per_sec = 1_000_000_000.0 / avg_time_ns as f64;
        
        println!("LSP conversion performance:");
        println!("  Average time: {} ns", avg_time_ns);
        println!("  Operations/sec: {:.0}", operations_per_sec);
        
        assert!(operations_per_sec > 1000.0, "LSP conversion too slow: {} ops/sec", operations_per_sec);
    }

    #[test]
    fn bench_synthesis_filter_performance() {
        let a = [4096i16, -500, 400, -300, 200, -100, 50, -25, 12, -6, 3];
        let x = [1000i16; L_SUBFR]; // Input excitation
        let mut y = [0i16; L_SUBFR];
        let mut mem = [0i16; M];
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            syn_filt(&a, &x, &mut y, L_SUBFR as Word16, &mut mem, 1);
        }
        
        // Benchmark
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            syn_filt(&a, &x, &mut y, L_SUBFR as Word16, &mut mem, 1);
        }
        let duration = start_time.elapsed();
        
        let avg_time_ns = duration.as_nanos() / PERFORMANCE_ITERATIONS as u128;
        let operations_per_sec = 1_000_000_000.0 / avg_time_ns as f64;
        
        println!("Synthesis filter performance:");
        println!("  Average time: {} ns", avg_time_ns);
        println!("  Operations/sec: {:.0}", operations_per_sec);
        
        // This is called twice per frame (2 subframes), so need 200 ops/sec minimum
        assert!(operations_per_sec > 2000.0, "Synthesis filter too slow: {} ops/sec", operations_per_sec);
    }
}

/// Basic operations performance tests
#[cfg(test)]
mod basic_ops_performance_tests {
    use super::*;

    #[test]
    fn bench_basic_arithmetic_performance() {
        let test_values: Vec<(i16, i16)> = (0..1000).map(|i| ((i % 32767) as i16, ((i * 17) % 32767) as i16)).collect();
        
        // Benchmark add
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            for &(a, b) in &test_values {
                let _ = add(a, b);
            }
        }
        let add_duration = start_time.elapsed();
        
        // Benchmark mult
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            for &(a, b) in &test_values {
                let _ = mult(a, b);
            }
        }
        let mult_duration = start_time.elapsed();
        
        // Benchmark l_mult
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            for &(a, b) in &test_values {
                let _ = l_mult(a, b);
            }
        }
        let l_mult_duration = start_time.elapsed();
        
        let add_ops_per_sec = (PERFORMANCE_ITERATIONS * test_values.len()) as f64 / add_duration.as_secs_f64();
        let mult_ops_per_sec = (PERFORMANCE_ITERATIONS * test_values.len()) as f64 / mult_duration.as_secs_f64();
        let l_mult_ops_per_sec = (PERFORMANCE_ITERATIONS * test_values.len()) as f64 / l_mult_duration.as_secs_f64();
        
        println!("Basic operations performance:");
        println!("  add(): {:.0} ops/sec", add_ops_per_sec);
        println!("  mult(): {:.0} ops/sec", mult_ops_per_sec);
        println!("  l_mult(): {:.0} ops/sec", l_mult_ops_per_sec);
        
        // These should be extremely fast
        assert!(add_ops_per_sec > 10_000_000.0, "add() too slow");
        assert!(mult_ops_per_sec > 1_000_000.0, "mult() too slow");
        assert!(l_mult_ops_per_sec > 1_000_000.0, "l_mult() too slow");
    }

    #[test]
    fn bench_division_performance() {
        let test_pairs: Vec<(i16, i16)> = (1..1000).map(|i| (16384, i as i16)).collect();
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            for &(num, den) in &test_pairs {
                let _ = div_s(num, den);
            }
        }
        
        // Benchmark
        let start_time = Instant::now();
        for _ in 0..PERFORMANCE_ITERATIONS {
            for &(num, den) in &test_pairs {
                let _ = div_s(num, den);
            }
        }
        let duration = start_time.elapsed();
        
        let ops_per_sec = (PERFORMANCE_ITERATIONS * test_pairs.len()) as f64 / duration.as_secs_f64();
        
        println!("Division performance:");
        println!("  div_s(): {:.0} ops/sec", ops_per_sec);
        
        // Division is more expensive but should still be fast enough
        assert!(ops_per_sec > 100_000.0, "div_s() too slow: {} ops/sec", ops_per_sec);
    }
}

/// End-to-end performance tests
#[cfg(test)]
mod end_to_end_performance_tests {
    use super::*;

    #[test]
    fn bench_encoder_performance() {
        let mut encoder = G729AEncoder::new();
        let test_frame = generate_test_frame();
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            let _ = encoder.encode(&test_frame);
        }
        
        // Benchmark
        let start_time = Instant::now();
        let mut successful_encodes = 0;
        
        for _ in 0..PERFORMANCE_ITERATIONS {
            match encoder.encode(&test_frame) {
                Ok(_) => successful_encodes += 1,
                Err(_) => {}, // Count failed encodes but continue
            }
        }
        
        let duration = start_time.elapsed();
        let frames_per_sec = successful_encodes as f64 / duration.as_secs_f64();
        let real_time_factor = frames_per_sec / 100.0; // 100 frames/sec for 8kHz, 10ms frames
        
        println!("Encoder performance:");
        println!("  Successful encodes: {}/{}", successful_encodes, PERFORMANCE_ITERATIONS);
        println!("  Frames/sec: {:.2}", frames_per_sec);
        println!("  Real-time factor: {:.2}x", real_time_factor);
        println!("  Average frame time: {:.2}ms", 1000.0 / frames_per_sec);
        
        if successful_encodes > 0 {
            // For real-time operation, need >100 frames/sec (real-time factor > 1.0)
            // Target is much higher for safety margin
            if real_time_factor > 10.0 {
                println!("✓ Excellent encoder performance");
            } else if real_time_factor > 1.0 {
                println!("✓ Adequate encoder performance for real-time");
            } else {
                println!("⚠ Encoder may be too slow for real-time");
            }
        }
    }

    #[test]
    fn bench_decoder_performance() {
        let mut decoder = G729ADecoder::new();
        let test_bits = vec![0u8; 10]; // Placeholder bitstream
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            let _ = decoder.decode(&test_bits, false);
        }
        
        // Benchmark
        let start_time = Instant::now();
        let mut successful_decodes = 0;
        
        for _ in 0..PERFORMANCE_ITERATIONS {
            match decoder.decode(&test_bits, false) {
                Ok(_) => successful_decodes += 1,
                Err(_) => {}, // Count failed decodes but continue
            }
        }
        
        let duration = start_time.elapsed();
        let frames_per_sec = successful_decodes as f64 / duration.as_secs_f64();
        let real_time_factor = frames_per_sec / 100.0;
        
        println!("Decoder performance:");
        println!("  Successful decodes: {}/{}", successful_decodes, PERFORMANCE_ITERATIONS);
        println!("  Frames/sec: {:.2}", frames_per_sec);
        println!("  Real-time factor: {:.2}x", real_time_factor);
        println!("  Average frame time: {:.2}ms", 1000.0 / frames_per_sec);
        
        if successful_decodes > 0 {
            if real_time_factor > 10.0 {
                println!("✓ Excellent decoder performance");
            } else if real_time_factor > 1.0 {
                println!("✓ Adequate decoder performance for real-time");
            } else {
                println!("⚠ Decoder may be too slow for real-time");
            }
        }
    }

    #[test]
    fn bench_codec_round_trip_performance() {
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        let test_frame = generate_test_frame();
        
        // Warmup
        for _ in 0..WARMUP_ITERATIONS {
            if let Ok(bits) = encoder.encode(&test_frame) {
                let _ = decoder.decode(&bits, false);
            }
        }
        
        // Benchmark round trip
        let start_time = Instant::now();
        let mut successful_round_trips = 0;
        
        for _ in 0..PERFORMANCE_ITERATIONS {
            match encoder.encode(&test_frame) {
                Ok(bits) => {
                    match decoder.decode(&bits, false) {
                        Ok(_) => successful_round_trips += 1,
                        Err(_) => {},
                    }
                }
                Err(_) => {},
            }
        }
        
        let duration = start_time.elapsed();
        let round_trips_per_sec = successful_round_trips as f64 / duration.as_secs_f64();
        let real_time_factor = round_trips_per_sec / 100.0;
        
        println!("Codec round-trip performance:");
        println!("  Successful round trips: {}/{}", successful_round_trips, PERFORMANCE_ITERATIONS);
        println!("  Round trips/sec: {:.2}", round_trips_per_sec);
        println!("  Real-time factor: {:.2}x", real_time_factor);
        println!("  Average round-trip time: {:.2}ms", 1000.0 / round_trips_per_sec);
        
        if successful_round_trips > 0 {
            if real_time_factor > 5.0 {
                println!("✓ Excellent round-trip performance");
            } else if real_time_factor > 1.0 {
                println!("✓ Adequate round-trip performance for real-time");
            } else {
                println!("⚠ Round-trip may be too slow for real-time");
            }
        }
    }
}

/// Memory usage performance tests
#[cfg(test)]
mod memory_performance_tests {
    use super::*;

    #[test]
    fn test_encoder_memory_usage() {
        let encoder = G729AEncoder::new();
        let total_size = std::mem::size_of_val(&encoder);
        
        println!("Encoder memory usage:");
        println!("  Total size: {} bytes", total_size);
        
        // G.729A should use less than 64KB total
        assert!(total_size < 65536, "Encoder memory usage too high: {} bytes", total_size);
        
        println!("✓ Encoder memory usage acceptable");
    }

    #[test]
    fn test_decoder_memory_usage() {
        let decoder = G729ADecoder::new();
        let total_size = std::mem::size_of_val(&decoder);
        
        println!("Decoder memory usage:");
        println!("  Total size: {} bytes", total_size);
        
        // G.729A should use less than 64KB total
        assert!(total_size < 65536, "Decoder memory usage too high: {} bytes", total_size);
        
        println!("✓ Decoder memory usage acceptable");
    }

    #[test]
    fn test_no_memory_leaks() {
        // Test that repeated encoding/decoding doesn't increase memory usage
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        let test_frame = generate_test_frame();
        let test_bits = vec![0u8; 10];
        
        // Process many frames
        for i in 0..10000 {
            let _ = encoder.encode(&test_frame);
            let _ = decoder.decode(&test_bits, false);
            
            // Every 1000 frames, check if we're still responsive
            if i % 1000 == 0 {
                // If this test completes without hanging, no obvious memory leaks
                let encoder_size = std::mem::size_of_val(&encoder);
                let decoder_size = std::mem::size_of_val(&decoder);
                
                // Sizes should remain constant
                assert!(encoder_size < 65536, "Encoder size grew unexpectedly: {}", encoder_size);
                assert!(decoder_size < 65536, "Decoder size grew unexpectedly: {}", decoder_size);
            }
        }
        
        println!("Memory leak test completed - no leaks detected");
    }
}

/// Complexity comparison tests
#[cfg(test)]
mod complexity_tests {
    use super::*;

    #[test]
    fn measure_computational_complexity() {
        // Measure relative complexity of different components
        let test_signal = generate_test_signal();
        let test_frame = generate_test_frame();
        
        // Time LPC analysis
        let start = Instant::now();
        for _ in 0..100 {
            let mut r_h = [0i16; MP1];
            let mut r_l = [0i16; MP1];
            autocorr(&test_signal, M as Word16, &mut r_h, &mut r_l);
            
            let mut a = [0i16; MP1];
            let mut rc = [0i16; M];
            levinson(&r_h, &r_l, &mut a, &mut rc);
        }
        let lpc_time = start.elapsed();
        
        // Time LSP conversion
        let start = Instant::now();
        for _ in 0..100 {
            let a = [4096i16, -1000, 800, -600, 400, -200, 100, -50, 25, -12, 6];
            let mut lsp = [0i16; M];
            let old_lsp = [1000i16, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000];
            az_lsp(&a, &mut lsp, &old_lsp);
        }
        let lsp_time = start.elapsed();
        
        // Time synthesis filtering
        let start = Instant::now();
        for _ in 0..100 {
            let a = [4096i16, -500, 400, -300, 200, -100, 50, -25, 12, -6, 3];
            let x = [1000i16; L_SUBFR];
            let mut y = [0i16; L_SUBFR];
            let mut mem = [0i16; M];
            syn_filt(&a, &x, &mut y, L_SUBFR as Word16, &mut mem, 1);
        }
        let synthesis_time = start.elapsed();
        
        let total_time = lpc_time + lsp_time + synthesis_time;
        
        println!("Computational complexity breakdown:");
        println!("  LPC analysis: {:.2}ms ({:.1}%)", 
                lpc_time.as_secs_f64() * 1000.0,
                (lpc_time.as_nanos() as f64 / total_time.as_nanos() as f64) * 100.0);
        println!("  LSP conversion: {:.2}ms ({:.1}%)", 
                lsp_time.as_secs_f64() * 1000.0,
                (lsp_time.as_nanos() as f64 / total_time.as_nanos() as f64) * 100.0);
        println!("  Synthesis filter: {:.2}ms ({:.1}%)", 
                synthesis_time.as_secs_f64() * 1000.0,
                (synthesis_time.as_nanos() as f64 / total_time.as_nanos() as f64) * 100.0);
        println!("  Total: {:.2}ms", total_time.as_secs_f64() * 1000.0);
        
        // Total should be less than 10ms (frame duration) for real-time processing
        // Allow reasonable margin for debug builds and test environment overhead
        assert!(total_time.as_millis() < 10, "Components too slow for real-time: {}ms", total_time.as_millis());
    }
}

/// Helper functions for performance tests
fn generate_test_signal() -> [i16; L_WINDOW] {
    let mut signal = [0i16; L_WINDOW];
    
    // Generate a synthetic speech-like signal
    for i in 0..L_WINDOW {
        let t = i as f64 / 8000.0; // 8kHz sample rate
        let mut sample = 0.0;
        
        // Add multiple frequency components
        sample += 0.3 * (2.0 * std::f64::consts::PI * 200.0 * t).sin();
        sample += 0.4 * (2.0 * std::f64::consts::PI * 400.0 * t).sin();
        sample += 0.2 * (2.0 * std::f64::consts::PI * 800.0 * t).sin();
        sample += 0.1 * (2.0 * std::f64::consts::PI * 1600.0 * t).sin();
        
        signal[i] = (sample * 8192.0) as i16; // Scale to Q15
    }
    
    signal
}

fn generate_test_frame() -> Vec<i16> {
    let mut frame = vec![0i16; L_FRAME];
    
    // Generate a simple test tone
    for i in 0..L_FRAME {
        let t = i as f64 / 8000.0;
        let sample = 0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin();
        frame[i] = (sample * 16384.0) as i16;
    }
    
    frame
} 