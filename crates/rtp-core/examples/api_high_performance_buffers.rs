//! High-Performance Buffer Configuration Example
//!
//! This example demonstrates how to use the high-performance buffer APIs
//! for controlling packet priority, adaptive transmission, and congestion control.

use std::time::Duration;
use tokio::time;
use tracing::{info, debug, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::process;

use rvoip_rtp_core::{
    api::{
        client::{
            transport::MediaTransportClient,
            config::ClientConfigBuilder,
        },
        server::{
            transport::MediaTransportServer,
            config::ServerConfigBuilder,
        },
        common::{
            frame::{MediaFrame, MediaFrameType},
            error::MediaTransportError,
        },
    },
    buffer::{
        TransmitBufferConfig, BufferLimits, PacketPriority,
    },
};

use rvoip_rtp_core::api::client::transport::DefaultMediaTransportClient;
use rvoip_rtp_core::api::server::transport::DefaultMediaTransportServer;

// Maximum duration for the example to run (in seconds)
const MAX_RUNTIME_SECONDS: u64 = 10; // Reduced to 10 seconds for faster testing

// Constants for our example
const PACKETS_TO_SEND: usize = 100;
const HIGH_PRIORITY_INTERVAL: usize = 5; // Every 5th packet is high priority

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    // Set a top-level timeout to ensure the program terminates no matter what
    // This backup ensures we don't have hanging processes
    std::thread::spawn(|| {
        // Total maximum runtime including any cleanup
        std::thread::sleep(Duration::from_secs(MAX_RUNTIME_SECONDS + 10));
        tracing::error!("Emergency timeout triggered after {} seconds - terminating process", MAX_RUNTIME_SECONDS + 10);
        process::exit(0);
    });
    
    info!("High-Performance Buffer Configuration Example");
    
    // Create a flag to signal when to stop
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    
    // Configure server with high-performance buffers
    let server_transmit_config = TransmitBufferConfig {
        max_packets: 1000,
        initial_cwnd: 32,
        congestion_control_enabled: true,
        ..Default::default()
    };
    
    let server_buffer_limits = BufferLimits {
        max_packets_per_stream: 500,
        max_packet_size: 1500,
        max_memory: 50 * 1024 * 1024, // 50 MB
    };
    
    let server_config = ServerConfigBuilder::new()
        .local_address("127.0.0.1:0".parse().unwrap()) // Dynamic port
        .rtcp_mux(true)
        .transmit_buffer_config(server_transmit_config)
        .buffer_limits(server_buffer_limits)
        .high_performance_buffers_enabled(true)
        .security_config(rvoip_rtp_core::api::server::security::ServerSecurityConfig { 
            security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, 
            ..Default::default() 
        })
        .build()?;
    
    // Create server
    let server = DefaultMediaTransportServer::new(server_config).await?;
    
    // Start server
    server.start().await?;
    
    // Get the server's bound address
    let server_addr = server.get_local_address().await?;
    info!("Server bound to {}", server_addr);
    
    // Configure client with high-performance buffers
    let client_transmit_config = TransmitBufferConfig {
        max_packets: 500,
        initial_cwnd: 16,
        congestion_control_enabled: true,
        ..Default::default()
    };
    
    let client_buffer_limits = BufferLimits {
        max_packets_per_stream: 250,
        max_packet_size: 1500,
        max_memory: 10 * 1024 * 1024, // 10 MB
    };
    
    let client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
        .rtcp_mux(true)
        .transmit_buffer_config(client_transmit_config)
        .buffer_limits(client_buffer_limits)
        .high_performance_buffers_enabled(true)
        .security_config(rvoip_rtp_core::api::client::security::ClientSecurityConfig { 
            security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, 
            ..Default::default() 
        })
        .build();
    
    // Create client
    let client = DefaultMediaTransportClient::new(client_config).await?;
    
    // Connect client to server
    info!("Connecting client to server");
    client.connect().await?;
    
    // Set priority threshold - when buffer is more than 80% full, only high priority packets will be sent
    info!("Setting priority threshold: when buffer is >80% full, only HIGH priority packets will be sent");
    client.set_priority_threshold(0.8, PacketPriority::High).await?;
    
    // Launch the timer task that will stop the example after MAX_RUNTIME_SECONDS
    let timeout_task = tokio::spawn(async move {
        info!("Timer started: example will run for max {} seconds", MAX_RUNTIME_SECONDS);
        
        // First timer for graceful shutdown
        time::sleep(Duration::from_secs(MAX_RUNTIME_SECONDS)).await;
        info!("Timer expired after {} seconds, signaling graceful shutdown", MAX_RUNTIME_SECONDS);
        running_clone.store(false, Ordering::SeqCst);
        
        // Second timer for forced exit if graceful shutdown doesn't complete
        time::sleep(Duration::from_secs(5)).await;
        warn!("Forced shutdown triggered - process will now exit");
        process::exit(0); // Force exit the entire process
    });
    
    // Send packets with different priorities
    info!("Sending packets with varying priorities");
    
    let mut i = 0;
    let mut stats_check_counter = 0;
    
    // Main sending loop with explicit exit condition
    while i < PACKETS_TO_SEND && running.load(Ordering::SeqCst) {
        // Create a frame
        let frame = MediaFrame {
            frame_type: MediaFrameType::Audio,
            data: format!("Frame {}", i).into_bytes(),
            timestamp: i as u32 * 160, // 20ms at 8kHz
            sequence: i as u16,
            marker: i == 0, // Mark the first packet
            payload_type: 0, // PCMU
            ssrc: 0x1234ABCD, // Fixed SSRC
            csrcs: Vec::new(),
        };
        
        // Determine priority - make every 5th packet high priority
        let priority = if i % HIGH_PRIORITY_INTERVAL == 0 {
            info!("Sending HIGH priority packet {}", i);
            PacketPriority::High
        } else {
            debug!("Sending normal priority packet {}", i);
            PacketPriority::Normal
        };
        
        // Send frame with priority if not stopped
        if running.load(Ordering::SeqCst) {
            match client.send_frame_with_priority(frame, priority).await {
                Ok(_) => {
                    stats_check_counter += 1;
                    if stats_check_counter >= 10 {
                        // Get transmit buffer stats every 10 packets
                        stats_check_counter = 0;
                        if let Ok(stats) = client.get_transmit_buffer_stats().await {
                            info!(
                                "Transmit buffer stats: packets_queued={}, packets_sent={}, buffer_fullness={:.2}%", 
                                stats.packets_queued,
                                stats.packets_sent,
                                stats.buffer_fullness * 100.0
                            );
                        }
                    }
                },
                Err(MediaTransportError::BufferFull(_)) => {
                    warn!("Buffer full, could not send packet {}", i);
                    // Wait a bit for buffer to clear
                    if running.load(Ordering::SeqCst) {
                        time::sleep(Duration::from_millis(50)).await;
                    }
                },
                Err(e) => {
                    warn!("Error sending packet: {}", e);
                    // Continue anyway instead of returning error
                }
            }
        }
        
        // Increment counter
        i += 1;
        
        // Simulate transmission rate
        if running.load(Ordering::SeqCst) {
            time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    // Indicate why we stopped
    if i >= PACKETS_TO_SEND {
        info!("Completed sending all {} packets", PACKETS_TO_SEND);
    } else {
        info!("Stopped sending after {} packets due to time limit", i);
    }
    
    // Abort the timeout task if we completed naturally
    if i >= PACKETS_TO_SEND {
        timeout_task.abort();
    }
    
    // Get final stats if possible
    if let Ok(stats) = client.get_transmit_buffer_stats().await {
        info!(
            "Final transmit buffer stats: packets_queued={}, packets_sent={}, drops={}, retransmits={}, buffer_fullness={:.2}%", 
            stats.packets_queued,
            stats.packets_sent,
            stats.packets_dropped,
            stats.packets_retransmitted,
            stats.buffer_fullness * 100.0
        );
    }
    
    // Minimal wait for cleanup to ensure the program exits
    info!("Cleaning up...");
    
    // Very short wait for cleanup
    time::sleep(Duration::from_millis(100)).await;
    
    // Disconnect client
    match client.disconnect().await {
        Ok(_) => info!("Client disconnected successfully"),
        Err(e) => warn!("Error disconnecting client: {}", e),
    }
    
    // Stop server
    match server.stop().await {
        Ok(_) => info!("Server stopped successfully"),
        Err(e) => warn!("Error stopping server: {}", e),
    }
    
    info!("Example completed successfully");
    Ok(())
} 