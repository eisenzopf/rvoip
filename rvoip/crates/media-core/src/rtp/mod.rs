//! RTP integration module for the media-core library
//!
//! This module provides integration with the rtp-core library for media transport,
//! including packetization, depacketization, and session management.

// Core RTP functionality
pub mod packetizer;
pub mod depacketizer;
pub mod session;

// Re-export common types
pub use packetizer::{Packetizer, PacketizerConfig};
pub use depacketizer::{Depacketizer, DepacketizerConfig, DepacketizerStats};
pub use session::{RtpSession, RtpSessionConfig, RtpSessionEvent, RtpSessionStats, MediaDirection};

use crate::error::Result;

/// Convert between media-core and rtp-core types
pub mod conversion {
    use crate::AudioBuffer;
    use crate::error::{Error, Result};
    use rvoip_rtp_core::packet::RtpPacket;
    
    /// Convert an RTP packet to an audio buffer
    /// 
    /// This is a convenience function that assumes raw PCM data.
    /// For encoded formats, use a Depacketizer.
    pub fn rtp_to_audio(
        packet: &RtpPacket, 
        sample_rate: crate::SampleRate,
        channels: u8,
        bit_depth: u8
    ) -> Result<AudioBuffer> {
        let format = crate::AudioFormat {
            sample_rate,
            channels,
            bit_depth,
        };
        
        Ok(AudioBuffer::new(
            packet.payload().clone(),
            format
        ))
    }
    
    /// Convert an audio buffer to an RTP packet
    ///
    /// This is a convenience function that assumes raw PCM data.
    /// For encoded formats, use a Packetizer.
    pub fn audio_to_rtp(
        buffer: &AudioBuffer,
        payload_type: u8,
        sequence: u16,
        timestamp: u32,
        ssrc: u32,
        marker: bool
    ) -> Result<RtpPacket> {
        use rvoip_rtp_core::packet::RtpPacketBuilder;
        use rvoip_rtp_core::payload::PayloadType;
        
        let builder = RtpPacketBuilder::new()
            .with_version(2)
            .with_padding(false)
            .with_extension(false)
            .with_marker(marker)
            .with_payload_type(PayloadType::new(payload_type))
            .with_sequence(sequence)
            .with_timestamp(timestamp)
            .with_ssrc(ssrc)
            .with_payload(buffer.data.clone());
        
        Ok(builder.build())
    }
}

/// Create a media session with a codec and RTP transport
///
/// This is a convenience function that creates an RTP session with a codec
/// and configures it for audio transport.
pub async fn create_audio_session(
    codec: std::sync::Arc<dyn crate::codec::Codec>,
    local_addr: std::net::SocketAddr,
    payload_type: u8,
    sample_rate: crate::SampleRate
) -> Result<(session::RtpSession, tokio::sync::mpsc::Receiver<session::RtpSessionEvent>)> {
    // Create RTP session config
    let config = session::RtpSessionConfig {
        local_addr,
        remote_addr: None,
        ssrc: rand::random::<u32>(),
        payload_type,
        clock_rate: sample_rate.as_hz(),
        audio_format: crate::AudioFormat {
            sample_rate,
            channels: 1,
            bit_depth: 16,
        },
        max_packet_size: 1200,
        reorder_packets: true,
        enable_rtcp: true,
    };
    
    // Create RTP session
    let (session, events) = session::RtpSession::new(config).await?;
    
    // Set codec
    session.set_codec(codec);
    
    Ok((session, events))
} 