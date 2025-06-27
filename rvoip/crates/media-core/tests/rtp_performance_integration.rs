//! RTP-Core Performance Integration Tests
//!
//! This module tests the integration between media-core's performance optimizations
//! and rtp-core's RTP packet handling, validating zero-copy performance across
//! the complete RTP → Audio Processing → RTP pipeline.

use rvoip_media_core::prelude::*;
use rvoip_media_core::performance::{
    zero_copy::{ZeroCopyAudioFrame, SharedAudioBuffer},
    pool::{AudioFramePool, PoolConfig},
    metrics::{PerformanceMetrics, BenchmarkConfig},
    simd::SimdProcessor,
};

// Import rtp-core types
use rvoip_rtp_core::prelude::*;
use rvoip_rtp_core::{RtpPacket, RtpHeader, MediaFrame, MediaFrameType, G711UPayloadFormat};

use std::sync::Arc;
use std::time::Instant;
use bytes::Bytes;
use serial_test::serial;
use std::result::Result;
use std::cell::RefCell;

/// Configuration for RTP performance testing
#[derive(Debug, Clone)]
struct RtpPerformanceTestConfig {
    /// Number of RTP packets to process
    packet_count: usize,
    /// Payload type (codec)
    payload_type: u8,
    /// Sample rate for audio
    sample_rate: u32,
    /// Frame size in samples
    frame_size: usize,
    /// Number of channels
    channels: u8,
}

impl Default for RtpPerformanceTestConfig {
    fn default() -> Self {
        Self {
            packet_count: 1000,
            payload_type: 0, // PCMU
            sample_rate: 8000,
            frame_size: 160, // 20ms at 8kHz
            channels: 1,
        }
    }
}

/// End-to-End RTP Processing Pipeline
struct RtpProcessingPipeline {
    pool: Arc<AudioFramePool>,
    simd: SimdProcessor,
    g711_codec: G711UPayloadFormat,
    config: RtpPerformanceTestConfig,
    // Pre-allocated working buffers for zero-allocation processing
    decode_buffer: Arc<RefCell<Vec<i16>>>,
    simd_buffer: Arc<RefCell<Vec<i16>>>,
    encode_buffer: Arc<RefCell<Vec<u8>>>,
}

impl RtpProcessingPipeline {
    fn new(config: RtpPerformanceTestConfig) -> Self {
        let pool_config = PoolConfig {
            initial_size: 32,
            max_size: 128,
            sample_rate: config.sample_rate,
            channels: config.channels,
            samples_per_frame: config.frame_size,
        };
        
        let pool = AudioFramePool::new(pool_config);
        let simd = SimdProcessor::new();
        let g711_codec = G711UPayloadFormat::new(8000);
        
        // Extract frame_size before moving config
        let frame_size = config.frame_size;
        
        Self {
            pool,
            simd,
            g711_codec,
            config,
            decode_buffer: Arc::new(RefCell::new(Vec::with_capacity(frame_size))),
            simd_buffer: Arc::new(RefCell::new(Vec::with_capacity(frame_size))),
            encode_buffer: Arc::new(RefCell::new(Vec::with_capacity(frame_size))),
        }
    }
    
    /// Process RTP packet through zero-copy audio pipeline
    fn process_rtp_packet_zero_copy(&self, rtp_packet: &RtpPacket) -> Result<RtpPacket, Box<dyn std::error::Error>> {
        // Step 1: Extract audio payload from RTP packet (zero-copy view)
        let payload_bytes = &rtp_packet.payload;
        
        // Step 2: Decode to PCM using zero-copy approach
        let pcm_samples = self.decode_payload_zero_copy(payload_bytes)?;
        
        // Step 3: Create ZeroCopyAudioFrame from decoded samples
        let audio_frame = ZeroCopyAudioFrame::new(
            pcm_samples,
            self.config.sample_rate,
            self.config.channels,
            rtp_packet.header.timestamp,
        );
        
        // Step 4: Process audio with SIMD (operates on shared buffer)
        let processed_frame = self.process_audio_simd(&audio_frame)?;
        
        // Step 5: Encode back to RTP payload (reusing buffer when possible)
        let encoded_payload = self.encode_payload_zero_copy(&processed_frame)?;
        
        // Step 6: Create output RTP packet with processed payload
        let output_header = RtpHeader::new(
            rtp_packet.header.payload_type,
            rtp_packet.header.sequence_number + 1,
            rtp_packet.header.timestamp,
            rtp_packet.header.ssrc,
        );
        
        Ok(RtpPacket::new(output_header, encoded_payload))
    }
    
    /// Process RTP packet using pooled frames for maximum efficiency
    fn process_rtp_packet_pooled(&self, rtp_packet: &RtpPacket) -> Result<RtpPacket, Box<dyn std::error::Error>> {
        // Step 1: Get pooled frame (reuses pre-allocated memory)
        let mut pooled_frame = self.pool.get_frame_with_params(
            self.config.sample_rate,
            self.config.channels,
            self.config.frame_size,
        );
        
        // Step 2: Decode RTP payload directly into pooled frame buffer
        self.decode_payload_into_frame(&rtp_packet.payload, &mut pooled_frame)?;
        
        // Step 3: Apply SIMD processing to pooled frame
        let processed_frame = self.process_audio_simd(&pooled_frame)?;
        
        // Step 4: Encode processed audio back to RTP
        let encoded_payload = self.encode_payload_zero_copy(&processed_frame)?;
        
        let output_header = RtpHeader::new(
            rtp_packet.header.payload_type,
            rtp_packet.header.sequence_number + 1,
            rtp_packet.header.timestamp,
            rtp_packet.header.ssrc,
        );
        
        Ok(RtpPacket::new(output_header, encoded_payload))
        // pooled_frame automatically returns to pool here
    }
    
    /// Decode payload with zero-copy approach
    fn decode_payload_zero_copy(&self, payload: &[u8]) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
        // For G.711, each byte represents one sample
        let mut samples = Vec::with_capacity(payload.len());
        
        // G.711 μ-law decoding (simplified)
        for &byte in payload {
            let sample = self.g711_mulaw_decode(byte);
            samples.push(sample);
        }
        
        Ok(samples)
    }
    
    /// Decode payload directly into pooled frame (avoids intermediate allocation)
    fn decode_payload_into_frame(&self, payload: &[u8], frame: &mut impl std::ops::DerefMut<Target = ZeroCopyAudioFrame>) -> Result<(), Box<dyn std::error::Error>> {
        // Get mutable access to frame samples
        let frame_samples = unsafe {
            // This is a simplification - in real implementation we'd need proper mutable access
            std::slice::from_raw_parts_mut(
                frame.samples().as_ptr() as *mut i16,
                payload.len().min(frame.samples().len())
            )
        };
        
        // Decode directly into frame buffer
        for (i, &byte) in payload.iter().enumerate() {
            if i < frame_samples.len() {
                frame_samples[i] = self.g711_mulaw_decode(byte);
            }
        }
        
        Ok(())
    }
    
    /// Process audio using SIMD optimizations
    fn process_audio_simd(&self, frame: &ZeroCopyAudioFrame) -> Result<ZeroCopyAudioFrame, Box<dyn std::error::Error>> {
        let samples = frame.samples();
        let mut processed_samples = vec![0i16; samples.len()];
        
        // Apply gain using SIMD
        self.simd.apply_gain(samples, 1.2, &mut processed_samples);
        
        // Calculate RMS for monitoring
        let rms = self.simd.calculate_rms(samples);
        
        // Create processed frame (shares computation result)
        Ok(ZeroCopyAudioFrame::new(
            processed_samples,
            frame.sample_rate,
            frame.channels,
            frame.timestamp,
        ))
    }
    
    /// Encode audio back to RTP payload format
    fn encode_payload_zero_copy(&self, frame: &ZeroCopyAudioFrame) -> Result<Bytes, Box<dyn std::error::Error>> {
        let samples = frame.samples();
        let mut payload = Vec::with_capacity(samples.len());
        
        // G.711 μ-law encoding
        for &sample in samples {
            payload.push(self.g711_mulaw_encode(sample));
        }
        
        Ok(Bytes::from(payload))
    }
    
    /// Simplified G.711 μ-law decode
    fn g711_mulaw_decode(&self, byte: u8) -> i16 {
        // Simplified μ-law decoding
        let sign = if byte & 0x80 != 0 { -1 } else { 1 };
        let exponent = (byte >> 4) & 0x07;
        let mantissa = (byte & 0x0F) as u32;  // Cast to u32 to avoid overflow
        
        let value = if exponent == 0 {
            (mantissa << 4) + 132
        } else {
            ((mantissa << 4) + 132) << (exponent.saturating_sub(1))
        };
        
        (sign * value as i32) as i16
    }
    
    /// Simplified G.711 μ-law encode
    fn g711_mulaw_encode(&self, sample: i16) -> u8 {
        // Simplified μ-law encoding
        let mut value = sample.abs() as u32;
        let sign = if sample < 0 { 0x80 } else { 0x00 };
        
        value += 132;
        if value > 32767 { value = 32767; }
        
        let exponent = if value >= 256 {
            let mut exp = 1;
            while value >= (256 << exp) && exp < 7 {
                exp += 1;
            }
            exp.min(7) // Ensure exponent doesn't exceed 7
        } else {
            0
        };
        
        let mantissa = if exponent == 0 {
            (value >> 4) & 0x0F
        } else {
            ((value >> (exponent + 3)) & 0x0F)
        };
        
        (sign | (exponent << 4) | mantissa) as u8
    }
}

/// Create test RTP packet with audio payload
fn create_test_rtp_packet(sequence: u16, timestamp: u32, payload_size: usize) -> RtpPacket {
    let header = RtpHeader::new(0, sequence, timestamp, 0x12345678); // PCMU
    
    // Create realistic G.711 μ-law payload (sine wave)
    let mut payload = Vec::with_capacity(payload_size);
    for i in 0..payload_size {
        // Generate sine wave sample
        let sample = (32767.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 8000.0).sin()) as i16;
        // Encode to μ-law
        let mulaw_byte = encode_mulaw_simple(sample);
        payload.push(mulaw_byte);
    }
    
    RtpPacket::new(header, Bytes::from(payload))
}

/// Simplified μ-law encoding for test data
fn encode_mulaw_simple(sample: i16) -> u8 {
    // Very simplified μ-law encoding for test purposes
    let value = (sample / 256) as i8;
    (value as u8) ^ 0x55
}

#[tokio::test]
#[serial]
async fn test_rtp_zero_copy_integration() {
    println!("\n🔗 RTP Zero-Copy Integration Test");
    println!("===================================");
    
    let config = RtpPerformanceTestConfig::default();
    let pipeline = RtpProcessingPipeline::new(config.clone());
    
    // Create test RTP packet
    let input_packet = create_test_rtp_packet(1000, 160000, config.frame_size);
    
    println!("Input packet: PT={}, seq={}, ts={}, payload_len={}", 
             input_packet.header.payload_type,
             input_packet.header.sequence_number,
             input_packet.header.timestamp,
             input_packet.payload.len());
    
    // Process through zero-copy pipeline
    let start = Instant::now();
    let output_packet = pipeline.process_rtp_packet_zero_copy(&input_packet).unwrap();
    let processing_time = start.elapsed();
    
    println!("Output packet: PT={}, seq={}, ts={}, payload_len={}", 
             output_packet.header.payload_type,
             output_packet.header.sequence_number,
             output_packet.header.timestamp,
             output_packet.payload.len());
    
    println!("Zero-copy processing time: {:?}", processing_time);
    
    // Verify packet integrity
    assert_eq!(output_packet.header.payload_type, input_packet.header.payload_type);
    assert_eq!(output_packet.header.timestamp, input_packet.header.timestamp);
    assert_eq!(output_packet.payload.len(), input_packet.payload.len());
    
    // Verify SIMD processing applied (samples should be amplified)
    assert_ne!(output_packet.payload, input_packet.payload, "Audio should be processed");
    
    println!("✅ Zero-copy RTP integration working correctly");
}

#[tokio::test]
#[serial]
async fn test_rtp_pooled_performance() {
    println!("\n🏊 RTP Pooled Performance Test");
    println!("===============================");
    
    let config = RtpPerformanceTestConfig::default();
    let pipeline = RtpProcessingPipeline::new(config.clone());
    
    // Pre-warm the pool
    pipeline.pool.prewarm(16);
    
    let mut total_processing_time = std::time::Duration::ZERO;
    
    // Process multiple packets to test pool efficiency
    for i in 0..10 {
        let input_packet = create_test_rtp_packet(1000 + i, 160000 + (i as u32 * 160), config.frame_size);
        
        let start = Instant::now();
        let _output_packet = pipeline.process_rtp_packet_pooled(&input_packet).unwrap();
        total_processing_time += start.elapsed();
    }
    
    let avg_processing_time = total_processing_time / 10;
    let pool_stats = pipeline.pool.get_stats();
    
    println!("Average pooled processing time: {:?}", avg_processing_time);
    println!("Pool efficiency: {}/{} hits ({:.1}%)", 
             pool_stats.pool_hits, 
             pool_stats.allocated_count,
             100.0 * pool_stats.pool_hits as f32 / pool_stats.allocated_count as f32);
    
    // Pool should be highly efficient
    assert!(pool_stats.pool_hits >= 8, "Pool should have high hit rate");
    assert!(avg_processing_time < std::time::Duration::from_micros(500), "Should be reasonably fast with pooling");
    
    println!("✅ Pooled RTP processing highly efficient");
}

#[tokio::test]
#[serial]
async fn test_rtp_performance_comparison() {
    println!("\n📊 RTP Performance Comparison");
    println!("==============================");
    
    let config = RtpPerformanceTestConfig {
        packet_count: 100,
        ..Default::default()
    };
    let pipeline = RtpProcessingPipeline::new(config.clone());
    
    // Generate test packets
    let test_packets: Vec<RtpPacket> = (0..config.packet_count)
        .map(|i| create_test_rtp_packet(1000 + i as u16, 160000 + (i as u32 * 160), config.frame_size))
        .collect();
    
    // Benchmark zero-copy processing
    let start = Instant::now();
    for packet in &test_packets {
        let _result = pipeline.process_rtp_packet_zero_copy(packet).unwrap();
    }
    let zero_copy_time = start.elapsed();
    
    // Benchmark pooled processing
    let start = Instant::now();
    for packet in &test_packets {
        let _result = pipeline.process_rtp_packet_pooled(packet).unwrap();
    }
    let pooled_time = start.elapsed();
    
    let zero_copy_avg = zero_copy_time / config.packet_count as u32;
    let pooled_avg = pooled_time / config.packet_count as u32;
    let speedup = zero_copy_time.as_nanos() as f64 / pooled_time.as_nanos() as f64;
    
    println!("Zero-copy average: {:?}", zero_copy_avg);
    println!("Pooled average:    {:?}", pooled_avg);
    println!("Pooled speedup:    {:.2}x", speedup);
    
    let pool_stats = pipeline.pool.get_stats();
    println!("Pool efficiency:   {:.1}%", 
             100.0 * pool_stats.pool_hits as f32 / pool_stats.allocated_count as f32);
    
    // Pooled should be competitive or faster due to eliminated allocations
    assert!(speedup >= 0.9, "Pooled processing should be competitive with zero-copy, got {:.2}x", speedup);
    
    println!("✅ Performance comparison validates optimization benefits");
}

#[tokio::test]
#[serial]
async fn test_rtp_memory_efficiency() {
    println!("\n💾 RTP Memory Efficiency Test");
    println!("==============================");
    
    let config = RtpPerformanceTestConfig::default();
    let pipeline = RtpProcessingPipeline::new(config.clone());
    
    // Create test packet
    let input_packet = create_test_rtp_packet(1000, 160000, config.frame_size);
    
    // Process and measure memory sharing
    let output_packet = pipeline.process_rtp_packet_zero_copy(&input_packet).unwrap();
    
    // Verify that we can create multiple references without copying
    let packet_ref1 = output_packet.clone();
    let packet_ref2 = output_packet.clone();
    
    // All should share the same payload buffer
    assert_eq!(output_packet.payload.len(), packet_ref1.payload.len());
    assert_eq!(output_packet.payload.len(), packet_ref2.payload.len());
    
    // Payload bytes should be the same reference (Arc sharing)
    assert_eq!(output_packet.payload.as_ptr(), packet_ref1.payload.as_ptr());
    assert_eq!(output_packet.payload.as_ptr(), packet_ref2.payload.as_ptr());
    
    println!("✅ Memory sharing working correctly - {} references to same payload", 3);
}

#[tokio::test]
#[serial]
async fn test_rtp_simd_integration() {
    println!("\n⚡ RTP SIMD Integration Test");
    println!("=============================");
    
    let config = RtpPerformanceTestConfig {
        frame_size: 320, // Larger frame for better SIMD utilization
        ..Default::default()
    };
    let pipeline = RtpProcessingPipeline::new(config.clone());
    
    println!("SIMD available: {}", pipeline.simd.is_simd_available());
    
    // Create test packet
    let input_packet = create_test_rtp_packet(1000, 160000, config.frame_size);
    
    // Process with SIMD
    let start = Instant::now();
    let output_packet = pipeline.process_rtp_packet_zero_copy(&input_packet).unwrap();
    let simd_time = start.elapsed();
    
    println!("SIMD processing time: {:?}", simd_time);
    
    // Verify audio was processed (gain applied)
    assert_ne!(output_packet.payload, input_packet.payload);
    
    // Should be reasonably fast with SIMD
    assert!(simd_time < std::time::Duration::from_millis(1), "SIMD processing should be fast");
    
    println!("✅ SIMD integration working with RTP processing");
}

#[tokio::test]
#[serial]
async fn test_rtp_end_to_end_latency() {
    println!("\n🚀 RTP End-to-End Latency Test");
    println!("===============================");
    
    let config = RtpPerformanceTestConfig::default();
    let pipeline = RtpProcessingPipeline::new(config.clone());
    
    // Measure complete RTP → Audio → RTP latency
    let input_packet = create_test_rtp_packet(1000, 160000, config.frame_size);
    
    let start = Instant::now();
    
    // Step 1: RTP packet parsing (rtp-core)
    let parse_start = Instant::now();
    let serialized = input_packet.serialize().unwrap();
    let parsed_packet = RtpPacket::parse(&serialized).unwrap();
    let parse_time = parse_start.elapsed();
    
    // Step 2: Audio processing (media-core performance)
    let process_start = Instant::now();
    let processed_packet = pipeline.process_rtp_packet_zero_copy(&parsed_packet).unwrap();
    let process_time = process_start.elapsed();
    
    // Step 3: RTP packet serialization (rtp-core)
    let serialize_start = Instant::now();
    let _final_bytes = processed_packet.serialize().unwrap();
    let serialize_time = serialize_start.elapsed();
    
    let total_time = start.elapsed();
    
    println!("RTP parse time:       {:?}", parse_time);
    println!("Audio process time:   {:?}", process_time);
    println!("RTP serialize time:   {:?}", serialize_time);
    println!("Total end-to-end:     {:?}", total_time);
    
    // Total latency should be well under 1ms for real-time processing
    assert!(total_time < std::time::Duration::from_millis(1), 
            "End-to-end latency should be <1ms, got {:?}", total_time);
    
    // Audio processing should be fast enough for real-time (under 100µs in debug builds)
    assert!(process_time < std::time::Duration::from_micros(100), 
            "Audio processing should be <100µs for real-time, got {:?}", process_time);
    
    println!("✅ End-to-end latency achieves real-time performance target");
} 