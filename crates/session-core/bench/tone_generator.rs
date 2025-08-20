/// Audio tone generation for RTP streaming benchmarks
use std::f32::consts::PI;
use std::time::{Duration, Instant};
use rvoip_rtp_core::RtpPacket;
use bytes::Bytes;

/// Generate a sine wave tone at the specified frequency
pub fn generate_tone(frequency: f32, sample_rate: u32, duration: Duration) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration.as_secs_f32()) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * PI * frequency * t).sin();
        // Convert to 16-bit PCM (range: -32768 to 32767)
        // Use 0.3 amplitude to avoid clipping
        let pcm_sample = (sample * 0.3 * 32767.0) as i16;
        samples.push(pcm_sample);
    }
    
    samples
}

/// Create RTP packets from audio samples
pub fn create_rtp_packets(
    samples: &[i16],
    ssrc: u32,
    sample_rate: u32,
    payload_type: u8,
) -> Vec<RtpPacket> {
    // G.711 typically uses 160 samples per packet (20ms at 8kHz)
    const SAMPLES_PER_PACKET: usize = 160;
    let mut packets = Vec::new();
    let mut sequence_number = 0u16;
    let mut timestamp = 0u32;
    
    for chunk in samples.chunks(SAMPLES_PER_PACKET) {
        // Convert i16 samples to u-law or a-law (simplified: just convert to bytes)
        let payload: Vec<u8> = chunk.iter()
            .flat_map(|&sample| sample.to_be_bytes())
            .collect();
        
        let packet = RtpPacket::new_with_payload(
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            Bytes::from(payload),
        );
        
        packets.push(packet);
        
        sequence_number = sequence_number.wrapping_add(1);
        timestamp += SAMPLES_PER_PACKET as u32;
    }
    
    packets
}

/// Decode RTP payload back to PCM samples (for validation)
pub fn decode_rtp_payload(payload: &[u8]) -> Vec<i16> {
    // Simple decoding - assumes payload is raw PCM16 big-endian
    payload.chunks_exact(2)
        .map(|chunk| i16::from_be_bytes([chunk[0], chunk[1]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_tone_generation() {
        let samples = generate_tone(440.0, 8000, Duration::from_secs(1));
        assert_eq!(samples.len(), 8000);
        
        // Check that we have a non-silent signal
        let max_sample = samples.iter().max().unwrap();
        assert!(*max_sample > 5000); // Should have significant amplitude
    }
    
    #[test]
    fn test_rtp_packet_creation() {
        let samples = generate_tone(440.0, 8000, Duration::from_millis(100));
        let packets = create_rtp_packets(&samples, 12345, 8000, 0);
        
        // 100ms at 8000Hz = 800 samples
        // 160 samples per packet = 5 packets
        assert_eq!(packets.len(), 5);
        
        // Check sequence numbers
        for (i, packet) in packets.iter().enumerate() {
            assert_eq!(packet.header.sequence_number, i as u16);
        }
    }
}