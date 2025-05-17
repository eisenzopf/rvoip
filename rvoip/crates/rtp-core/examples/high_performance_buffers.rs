//! Example demonstrating high-performance buffer management
//!
//! This example shows how to use the buffer management system
//! for high-scale deployments with tens of thousands of connections.

use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, debug, warn};

use rvoip_rtp_core::{
    buffer::{
        GlobalBufferManager, BufferLimits, BufferPool, SharedPools,
        AdaptiveJitterBuffer, JitterBufferConfig,
        TransmitBuffer, TransmitBufferConfig, PacketPriority
    },
    packet::{RtpHeader, RtpPacket},
    RtpSsrc
};

// Constants for the example
const NUM_STREAMS: usize = 500;
const PACKETS_PER_STREAM: usize = 1000;
const PACKET_SIZE: usize = 200;
const MAX_JITTER_MS: u64 = 50;

// Create simulated network jitter and packet loss
fn simulate_network_effects() -> tokio::time::Sleep {
    // Random delay between 0-MAX_JITTER_MS ms
    let jitter = rand::random::<u64>() % MAX_JITTER_MS;
    tokio::time::sleep(Duration::from_millis(jitter))
}

// Generate test packet
fn create_test_packet(seq: u16, ssrc: RtpSsrc, size: usize) -> RtpPacket {
    let header = RtpHeader::new(96, seq, (seq as u32) * 160, ssrc);
    let payload = Bytes::from(vec![0u8; size - 12]); // 12 bytes for RTP header
    RtpPacket::new(header, payload)
}

/// Simulates a single RTP stream with jitter buffer and transmit buffer
struct StreamSender {
    ssrc: RtpSsrc,
    transmit_buffer: TransmitBuffer,
    packet_tx: mpsc::Sender<RtpPacket>,
}

struct StreamReceiver {
    ssrc: RtpSsrc,
    jitter_buffer: AdaptiveJitterBuffer,
    packet_rx: mpsc::Receiver<RtpPacket>,
}

impl StreamSender {
    // Simulate sending packets through the transmit buffer
    async fn run_sender(&mut self) {
        for seq in 1..=PACKETS_PER_STREAM as u16 {
            // Create a packet
            let packet = create_test_packet(seq, self.ssrc, PACKET_SIZE);
            
            // Determine priority (make some packets high priority)
            let priority = if seq % 10 == 0 {
                PacketPriority::High
            } else {
                PacketPriority::Normal
            };
            
            // Queue in transmit buffer
            if self.transmit_buffer.queue_packet(packet, priority).await {
                // Get the next packet ready to send
                if let Some(packet) = self.transmit_buffer.get_next_packet().await {
                    // Simulate network effects
                    simulate_network_effects().await;
                    
                    // Send the packet
                    if let Err(e) = self.packet_tx.send(packet).await {
                        warn!("Failed to send packet: {}", e);
                    }
                    
                    // Simulate acknowledgment after a while
                    if seq > 10 {
                        self.transmit_buffer.acknowledge_packet(seq - 10);
                    }
                }
            }
            
            // Small delay between packets
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl StreamReceiver {
    // Simulate receiving packets through the jitter buffer
    async fn run_receiver(&mut self) {
        let mut packets_received = 0;
        let mut packets_played = 0;
        
        while let Some(packet) = self.packet_rx.recv().await {
            packets_received += 1;
            
            // Add to jitter buffer
            if self.jitter_buffer.add_packet(packet).await {
                // Try to get packets out of the jitter buffer
                while let Some(packet) = self.jitter_buffer.get_next_packet().await {
                    packets_played += 1;
                    
                    // Process the packet (in a real application)
                    if packets_played % 100 == 0 {
                        debug!("Stream {:08x}: Played {} packets", self.ssrc, packets_played);
                    }
                }
            }
            
            // Check for completion
            if packets_played >= PACKETS_PER_STREAM {
                break;
            }
        }
        
        // Report final stats
        let stats = self.jitter_buffer.get_stats();
        info!(
            "Stream {:08x} completed: received={}, played={}, jitter={:.2}ms, discontinuities={}",
            self.ssrc,
            packets_received,
            packets_played,
            stats.jitter_ms,
            stats.discontinuities
        );
    }
}

fn create_stream_pair(
    ssrc: RtpSsrc,
    buffer_manager: Arc<GlobalBufferManager>,
    packet_pool: Arc<BufferPool>
) -> (StreamSender, StreamReceiver) {
    let jitter_config = JitterBufferConfig {
        initial_size_ms: 50,
        min_size_ms: 20,
        max_size_ms: 200,
        clock_rate: 8000,
        adaptive: true,
        ..Default::default()
    };
    
    let transmit_config = TransmitBufferConfig {
        max_packets: 500,
        initial_cwnd: 32,
        congestion_control_enabled: true,
        ..Default::default()
    };
    
    let jitter_buffer = AdaptiveJitterBuffer::with_buffer_manager(jitter_config, buffer_manager.clone());
    let transmit_buffer = TransmitBuffer::with_buffer_manager(
        ssrc,
        transmit_config,
        buffer_manager,
        packet_pool
    );
    
    let (packet_tx, packet_rx) = mpsc::channel(100);
    
    let sender = StreamSender {
        ssrc,
        transmit_buffer,
        packet_tx,
    };
    
    let receiver = StreamReceiver {
        ssrc,
        jitter_buffer,
        packet_rx,
    };
    
    (sender, receiver)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("RTP High Performance Buffer Example");
    info!("Simulating {} streams with {} packets each", NUM_STREAMS, PACKETS_PER_STREAM);
    
    // Create global buffer manager with limits
    let buffer_limits = BufferLimits {
        max_packets_per_stream: 500,
        max_packet_size: 1500,
        max_memory: 1024 * 1024 * 100, // 100 MB
    };
    
    let buffer_manager = Arc::new(GlobalBufferManager::new(buffer_limits));
    
    // Create shared buffer pools
    let pools = SharedPools::new(10000);
    let packet_pool = Arc::new(pools.medium);
    
    // Create streams
    let mut stream_handles = Vec::new();
    
    // Spawn tasks for each stream
    for i in 0..NUM_STREAMS {
        let ssrc = rand::random::<u32>();
        let (mut sender, mut receiver) = create_stream_pair(
            ssrc,
            buffer_manager.clone(),
            packet_pool.clone()
        );
        
        // Spawn sender and receiver tasks
        let sender_handle = tokio::spawn(async move {
            sender.run_sender().await;
        });
        
        let receiver_handle = tokio::spawn(async move {
            receiver.run_receiver().await;
        });
        
        // Keep handles
        stream_handles.push((sender_handle, receiver_handle));
        
        // Small delay to stagger stream startup
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    
    // Wait for all streams to complete
    for (i, (sender, receiver)) in stream_handles.into_iter().enumerate() {
        if let Err(e) = sender.await {
            warn!("Stream {}: Sender task failed: {}", i, e);
        }
        
        if let Err(e) = receiver.await {
            warn!("Stream {}: Receiver task failed: {}", i, e);
        }
    }
    
    info!("All streams completed");
    
    Ok(())
} 