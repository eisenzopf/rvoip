//! Zero-copy RTP processing functionality
//!
//! This module provides zero-copy optimizations for RTP packet processing
//! to minimize memory allocations and improve performance.

use std::time::Instant;
use tracing::debug;
use bytes::Bytes;

use crate::error::Result;
use crate::performance::pool::PoolStats;
use rvoip_rtp_core as rtp_core;

use super::MediaSessionController;

impl MediaSessionController {
    /// Process RTP packet with zero-copy optimization
    /// 
    /// This method implements true zero-copy processing by:
    /// 1. Using pooled frames for audio processing (reuse)
    /// 2. Decoding directly into pooled buffer (zero-copy decode)
    /// 3. Processing in-place with SIMD (zero-copy processing)
    /// 4. Encoding to pre-allocated output buffer (zero-copy encode)
    /// 5. Creating RTP packet with buffer reference (zero-copy)
    pub async fn process_rtp_packet_zero_copy(&self, packet: &rtp_core::RtpPacket) -> Result<rtp_core::RtpPacket> {
        let start_time = Instant::now();
        
        // Step 1: Get pooled frame (reuses pre-allocated memory)
        let mut pooled_frame = self.frame_pool.get_frame_with_params(
            8000, // Sample rate
            1,    // Channels
            160,  // Frame size (20ms at 8kHz)
        );
        
        // Step 2: Decode RTP payload directly into pooled frame buffer (zero-copy)
        let payload_bytes: &[u8] = &packet.payload;
        {
            let mut codec = self.g711_codec.lock().await;
            codec.decode_to_buffer(payload_bytes, pooled_frame.samples_mut())?;
        }
        
        // Step 3: Apply SIMD processing in-place (zero-copy)
        self.simd_processor.apply_gain_in_place(pooled_frame.samples_mut(), 1.2);
        
        // Step 4: Encode from pooled buffer to pre-allocated output (zero-copy)
        let mut output_buffer = self.rtp_buffer_pool.get_buffer();
        let encoded_size = {
            let mut codec = self.g711_codec.lock().await;
            codec.encode_to_buffer(pooled_frame.samples(), output_buffer.as_mut())?
        };
        
        // Step 5: Create RTP packet with buffer reference (zero-copy)
        let new_payload = output_buffer.slice(encoded_size);
        let output_header = rtp_core::RtpHeader::new(
            packet.header.payload_type,
            packet.header.sequence_number + 1,
            packet.header.timestamp,
            packet.header.ssrc,
        );
        
        // Update performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            // Note: Zero allocations! We only track buffer reuse
            metrics.operation_count += 1;
        }
        
        debug!("Zero-copy RTP processing completed in {:?}", processing_time);
        Ok(rtp_core::RtpPacket::new(output_header, new_payload))
        // pooled_frame automatically returns to pool here
    }
    
    /// Process RTP packet with traditional approach (for comparison)
    /// 
    /// This method uses the traditional approach with allocations for comparison:
    /// 1. Extract payload to Vec<u8> (COPY)
    /// 2. Decode to Vec<i16> (COPY + ALLOCATION)
    /// 3. Create AudioFrame with Vec<i16> (COPY)
    /// 4. Process to Vec<i16> (COPY)
    /// 5. Encode to Vec<u8> (COPY + ALLOCATION)
    /// 6. Create RTP packet with Bytes (COPY)
    pub async fn process_rtp_packet_traditional(&self, packet: &rtp_core::RtpPacket) -> Result<rtp_core::RtpPacket> {
        let start_time = Instant::now();
        
        // Step 1: Extract payload → Vec<u8> (COPY)
        let payload_bytes = packet.payload.to_vec();
        
        // Step 2: Decode → Vec<i16> (COPY + ALLOCATION)
        let decoded_samples = {
            let mut codec = self.g711_codec.lock().await;
            let mut samples = vec![0i16; payload_bytes.len()];
            codec.decode_to_buffer(&payload_bytes, &mut samples)?;
            samples
        };
        let decoded_len = decoded_samples.len(); // Store length before move
        
        // Step 3: Create AudioFrame → Vec<i16> (COPY)
        let audio_frame = crate::types::AudioFrame::new(
            decoded_samples,
            8000,
            1,
            packet.header.timestamp,
        );
        
        // Step 4: Process → Vec<i16> (COPY)
        let mut processed_samples = audio_frame.samples.clone();
        self.simd_processor.apply_gain(&audio_frame.samples, 1.2, &mut processed_samples);
        
        // Step 5: Encode → Vec<u8> (COPY + ALLOCATION)
        let encoded_payload = {
            let mut codec = self.g711_codec.lock().await;
            let mut output = vec![0u8; processed_samples.len()];
            let encoded_size = codec.encode_to_buffer(&processed_samples, &mut output)?;
            output.truncate(encoded_size);
            output
        };
        
        // Step 6: Create RTP packet → Bytes (COPY)
        let new_payload = Bytes::from(encoded_payload);
        let output_header = rtp_core::RtpHeader::new(
            packet.header.payload_type,
            packet.header.sequence_number + 1,
            packet.header.timestamp,
            packet.header.ssrc,
        );
        
        // Update performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            metrics.add_allocation(payload_bytes.len() as u64);      // Allocation 1
            metrics.add_allocation(decoded_len as u64 * 2);          // Allocation 2 (i16) - use stored length
            metrics.add_allocation(processed_samples.len() as u64 * 2); // Allocation 3 (i16)
            metrics.add_allocation(new_payload.len() as u64);        // Allocation 4
            metrics.operation_count += 1;
        }
        
        debug!("Traditional RTP processing completed in {:?}", processing_time);
        Ok(rtp_core::RtpPacket::new(output_header, new_payload))
    }
    
    /// Get RTP buffer pool statistics
    pub fn get_rtp_buffer_pool_stats(&self) -> PoolStats {
        self.rtp_buffer_pool.get_stats()
    }
    
    /// Reset RTP buffer pool statistics
    pub fn reset_rtp_buffer_pool_stats(&self) {
        self.rtp_buffer_pool.reset_stats();
    }
} 