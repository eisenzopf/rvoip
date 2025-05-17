//! Example demonstrating CSRC management in mixed streams
//!
//! This example shows how to use the CSRC management capabilities in RTP
//! for conferencing scenarios where multiple sources are mixed together.

use bytes::Bytes;
use rand::{RngCore, Rng};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{info, debug, warn};

use rvoip_rtp_core::{
    RtpPacket, RtpHeader, 
    RtpSsrc, RtpCsrc, RtpSequenceNumber, RtpTimestamp,
    CsrcMapping, CsrcManager, MAX_CSRC_COUNT,
    packet::rtcp::{
        RtcpPacket, RtcpSourceDescription, RtcpSdesChunk, RtcpSdesItem
    }
};

// Simulates an audio sample buffer from a source
struct SourceBuffer {
    ssrc: RtpSsrc,
    participant_name: String,
    sequence: RtpSequenceNumber,
    timestamp: RtpTimestamp,
    samples: Vec<i16>,
    talking: bool,
}

impl SourceBuffer {
    fn new(ssrc: RtpSsrc, name: &str) -> Self {
        Self {
            ssrc,
            participant_name: name.to_string(),
            sequence: rand::random::<u16>(),
            timestamp: rand::random::<u32>(),
            samples: Vec::new(),
            talking: false,
        }
    }
    
    // Generate new audio samples (simulated)
    fn generate_samples(&mut self, sample_count: usize) {
        // 10% chance of changing talking state
        if rand::random::<f32>() < 0.1 {
            self.talking = !self.talking;
            info!("Participant {} is now {}", 
                 self.participant_name, 
                 if self.talking { "talking" } else { "silent" });
        }
        
        // Generate samples based on talking state
        self.samples.clear();
        if self.talking {
            // Generate "active" audio - just random values for simulation
            let mut rng = rand::thread_rng();
            for _ in 0..sample_count {
                // Generate values between -3000 and 3000 to simulate moderate volume speech
                self.samples.push(rng.gen_range(-3000..3000));
            }
        } else {
            // Generate silence (zeros)
            self.samples.resize(sample_count, 0);
        }
        
        // Update timestamp (assuming 8kHz sample rate)
        self.timestamp = self.timestamp.wrapping_add(sample_count as u32);
        self.sequence = self.sequence.wrapping_add(1);
    }
    
    // Create an RTP packet with the current samples
    fn create_packet(&self) -> RtpPacket {
        // For simplicity, we'll just pack raw PCM samples
        // In a real implementation, this would be encoded with the appropriate codec
        let mut payload_data = Vec::with_capacity(self.samples.len() * 2);
        
        for sample in &self.samples {
            // Convert to network byte order (big-endian)
            payload_data.push((*sample >> 8) as u8);
            payload_data.push(*sample as u8);
        }
        
        // Create RTP header
        let header = RtpHeader::new(
            0, // PCM audio
            self.sequence,
            self.timestamp,
            self.ssrc
        );
        
        RtpPacket::new(header, Bytes::from(payload_data))
    }
    
    // Check if the source is currently active (talking)
    fn is_active(&self) -> bool {
        self.talking
    }
}

// Simulates an RTP mixer that combines multiple sources
struct RtpMixer {
    // The SSRC of the mixed output stream
    mixer_ssrc: RtpSsrc,
    
    // Sequence number for the output stream
    sequence: RtpSequenceNumber,
    
    // Timestamp for the output stream
    timestamp: RtpTimestamp,
    
    // Source buffers by SSRC
    sources: HashMap<RtpSsrc, SourceBuffer>,
    
    // CSRC manager
    csrc_manager: CsrcManager,
}

impl RtpMixer {
    fn new() -> Self {
        Self {
            mixer_ssrc: rand::random::<u32>(),
            sequence: rand::random::<u16>(),
            timestamp: rand::random::<u32>(),
            sources: HashMap::new(),
            csrc_manager: CsrcManager::new(),
        }
    }
    
    // Add a new source to the mixer
    fn add_source(&mut self, source: SourceBuffer) {
        let ssrc = source.ssrc;
        let name = source.participant_name.clone();
        
        // Store the source
        self.sources.insert(ssrc, source);
        
        // Create a CSRC mapping for this source
        // We'll use the same value for both SSRC and CSRC for simplicity
        self.csrc_manager.add_mapping(CsrcMapping::with_names(
            ssrc, 
            ssrc, 
            format!("{}@example.com", name.to_lowercase()),
            name.clone()
        ));
        
        info!("Added source: SSRC={:08x}, Name={}", ssrc, name);
    }
    
    // Generate mixed output
    fn mix_output(&mut self, sample_count: usize) -> RtpPacket {
        // First, generate new samples for each source
        for source in self.sources.values_mut() {
            source.generate_samples(sample_count);
        }
        
        // Get list of active sources (those who are talking)
        let active_sources: Vec<&SourceBuffer> = self.sources.values()
            .filter(|s| s.is_active())
            .collect();
        
        info!("Mixing {} active sources out of {} total", 
             active_sources.len(), self.sources.len());
        
        // Mix the samples from active sources
        let mut mixed_samples = vec![0i32; sample_count];
        for source in &active_sources {
            for (i, &sample) in source.samples.iter().enumerate() {
                mixed_samples[i] += sample as i32;
            }
        }
        
        // Scale and clip the mixed samples to prevent overflow
        let mut payload_data = Vec::with_capacity(sample_count * 2);
        for sample in mixed_samples {
            // Simple scaling: divide by number of active sources if > 0
            let scaled = if active_sources.len() > 0 {
                sample / active_sources.len() as i32
            } else {
                sample
            };
            
            // Clip to i16 range
            let clipped = scaled.max(i16::MIN as i32).min(i16::MAX as i32) as i16;
            
            // Convert to network byte order (big-endian)
            payload_data.push((clipped >> 8) as u8);
            payload_data.push(clipped as u8);
        }
        
        // Create RTP header for the mixed output
        let mut header = RtpHeader::new(
            0, // PCM audio
            self.sequence,
            self.timestamp,
            self.mixer_ssrc
        );
        
        // Add CSRCs for active sources (limited to MAX_CSRC_COUNT)
        let active_ssrcs: Vec<RtpSsrc> = active_sources.iter()
            .map(|s| s.ssrc)
            .take(MAX_CSRC_COUNT as usize)
            .collect();
        
        // Get CSRC values from the manager
        let csrcs = self.csrc_manager.get_active_csrcs(&active_ssrcs);
        
        // Add CSRCs to the header
        header.add_csrcs(&csrcs);
        
        // Create the mixed packet
        let packet = RtpPacket::new(header, Bytes::from(payload_data));
        
        // Update sequence and timestamp for next packet
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(sample_count as u32);
        
        packet
    }
    
    // Create an RTCP SDES packet with information about all sources
    fn create_sdes_packet(&self) -> RtcpPacket {
        let mut sdes = RtcpSourceDescription::new();
        
        // Add chunk for the mixer itself
        let mut mixer_chunk = RtcpSdesChunk::new(self.mixer_ssrc);
        mixer_chunk.add_item(RtcpSdesItem::cname("mixer@example.com".to_string()));
        mixer_chunk.add_item(RtcpSdesItem::tool("rVOIP RTP Mixer".to_string()));
        sdes.add_chunk(mixer_chunk);
        
        // Add chunks for each source that has a mapping
        for mapping in self.csrc_manager.get_all_mappings() {
            let mut chunk = RtcpSdesChunk::new(mapping.csrc);
            
            // Add CNAME if available
            if let Some(cname) = &mapping.cname {
                chunk.add_item(RtcpSdesItem::cname(cname.clone()));
            }
            
            // Add NAME if available
            if let Some(name) = &mapping.display_name {
                chunk.add_item(RtcpSdesItem::name(name.clone()));
            }
            
            sdes.add_chunk(chunk);
        }
        
        RtcpPacket::SourceDescription(sdes)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("RTP CSRC Management Example");
    info!("---------------------------");
    info!("This example demonstrates a conference mixer that combines");
    info!("multiple audio sources and properly identifies contributors");
    info!("using CSRC identifiers in the RTP header.");
    
    // Create the mixer
    let mut mixer = RtpMixer::new();
    info!("Created mixer with SSRC={:08x}", mixer.mixer_ssrc);
    
    // Add some participants
    let participants = [
        ("Alice", rand::random::<u32>()),
        ("Bob", rand::random::<u32>()),
        ("Carol", rand::random::<u32>()),
        ("Dave", rand::random::<u32>()),
        ("Eve", rand::random::<u32>()),
    ];
    
    for &(name, ssrc) in &participants {
        mixer.add_source(SourceBuffer::new(ssrc, name));
    }
    
    info!("Added {} participants to the mixer", participants.len());
    
    // Create RTCP SDES packet with source descriptions
    let sdes_packet = mixer.create_sdes_packet();
    info!("Created RTCP SDES packet with source descriptions");
    
    // Print SDES information
    if let RtcpPacket::SourceDescription(sdes) = &sdes_packet {
        info!("SDES packet contains {} chunks", sdes.chunks.len());
        
        for (i, chunk) in sdes.chunks.iter().enumerate() {
            info!("Chunk {}: SSRC/CSRC={:08x}", i, chunk.ssrc);
            
            for item in &chunk.items {
                match item.item_type {
                    rvoip_rtp_core::packet::rtcp::RtcpSdesItemType::CName => {
                        info!("  CNAME: {}", item.value);
                    },
                    rvoip_rtp_core::packet::rtcp::RtcpSdesItemType::Name => {
                        info!("  NAME: {}", item.value);
                    },
                    rvoip_rtp_core::packet::rtcp::RtcpSdesItemType::Tool => {
                        info!("  TOOL: {}", item.value);
                    },
                    _ => {
                        info!("  {:?}: {}", item.item_type, item.value);
                    }
                }
            }
        }
    }
    
    // Simulation loop - generate mixed packets for 20 seconds
    info!("Starting mixer simulation...");
    
    for i in 1..=20 {
        // Generate a mixed packet with 160 samples (20ms at 8kHz)
        let mixed_packet = mixer.mix_output(160);
        
        // Get information about the packet
        let csrc_count = mixed_packet.header.cc;
        let csrcs = mixed_packet.header.csrc.clone();
        
        info!("Mixed packet {}: {} bytes, {} CSRCs: {:?}",
             i, mixed_packet.size(), csrc_count, csrcs);
        
        // If we have CSRCs, print the participant names
        if !csrcs.is_empty() {
            let active_names: Vec<String> = csrcs.iter()
                .filter_map(|&csrc| mixer.csrc_manager.get_by_csrc(csrc))
                .filter_map(|mapping| mapping.display_name.clone())
                .collect();
            
            info!("Active participants: {}", active_names.join(", "));
        } else {
            info!("No active participants in this packet");
        }
        
        // Wait 1 second in simulation time
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
    info!("CSRC management example completed successfully");
    
    Ok(())
} 