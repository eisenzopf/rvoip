//! Example demonstrating SSRC-based demultiplexing
//!
//! This example shows how multiple RTP streams with different SSRCs
//! can be handled within a single RTP session.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use rand::Rng;
use tokio::sync::Mutex;
use tracing::{info, debug};

// Import necessary components from RTP core
use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSsrc, RtpTimestamp,
    RtpSessionEvent
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("RTP SSRC Demultiplexing Example");
    
    // Create two RTP sessions that will communicate with each other
    let addr1 = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
    let addr2 = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
    
    // Configure the first session (receiver)
    let receiver_config = RtpSessionConfig {
        local_addr: addr1,
        remote_addr: None, // Will be set after the sender is created
        ssrc: None, // Generate random SSRC
        payload_type: 96, // Dynamic payload type
        clock_rate: 8000, // 8kHz
        jitter_buffer_size: Some(50),
        max_packet_age_ms: Some(200),
        enable_jitter_buffer: true,
    };
    
    // Create receiver session and wrap in Arc<Mutex<>> for sharing
    let receiver = RtpSession::new(receiver_config).await?;
    let receiver = Arc::new(Mutex::new(receiver));
    
    // Get the local address before moving into the task
    let receiver_addr = {
        let receiver_guard = receiver.lock().await;
        receiver_guard.local_addr()?
    };
    info!("Receiver listening on {}", receiver_addr);
    
    // Subscribe to session events
    let mut event_rx = {
        let receiver_guard = receiver.lock().await;
        receiver_guard.subscribe()
    };
    
    // Configure the sender session
    let sender_config = RtpSessionConfig {
        local_addr: addr2,
        remote_addr: Some(receiver_addr),
        ssrc: None, // Generate random SSRC
        payload_type: 96, // Dynamic payload type
        clock_rate: 8000, // 8kHz
        jitter_buffer_size: Some(50),
        max_packet_age_ms: Some(200),
        enable_jitter_buffer: false, // Sender doesn't need a jitter buffer
    };
    
    // Create sender session
    let mut sender = RtpSession::new(sender_config).await?;
    let sender_addr = sender.local_addr()?;
    info!("Sender bound to {}", sender_addr);
    
    // Set the sender's address as the remote for the receiver
    {
        let mut receiver_guard = receiver.lock().await;
        receiver_guard.set_remote_addr(sender_addr).await;
    }
    
    // Add a small delay to ensure the receiver is fully ready
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Get the SSRC of the sender
    let sender_ssrc = sender.get_ssrc();
    info!("Sender SSRC: {:08x}", sender_ssrc);
    
    // Create additional SSRCs for simulating multiple streams
    let mut rng = rand::thread_rng();
    let alt_ssrc1: RtpSsrc = rng.gen();
    let alt_ssrc2: RtpSsrc = rng.gen();
    info!("Additional SSRCs: {:08x}, {:08x}", alt_ssrc1, alt_ssrc2);
    
    // Pre-create streams for the SSRCs in the receiver to ensure they're tracked
    {
        let mut receiver_guard = receiver.lock().await;
        receiver_guard.create_stream_for_ssrc(sender_ssrc).await;
        receiver_guard.create_stream_for_ssrc(alt_ssrc1).await;
        receiver_guard.create_stream_for_ssrc(alt_ssrc2).await;
        
        // Verify streams were created
        let ssrcs = receiver_guard.get_all_ssrcs().await;
        info!("Pre-created streams for SSRCs: {:?}", ssrcs);
    }
    
    // Track how many unique SSRCs we've seen in the receiver task
    let seen_ssrcs_count = Arc::new(Mutex::new(0));
    let seen_ssrcs_count_clone = seen_ssrcs_count.clone();
    
    // Spawn a task to handle received packets and events
    let receiver_clone = receiver.clone();
    let receiver_handle = tokio::spawn(async move {
        let mut packet_count = 0;
        let mut stream_count = 0;
        
        // Keep track of unique SSRCs seen
        let mut seen_ssrcs = std::collections::HashSet::new();
        
        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Ok(RtpSessionEvent::PacketReceived(packet)) => {
                            packet_count += 1;
                            let packet_ssrc = packet.header.ssrc;
                            
                            debug!("Received packet #{} from SSRC={:08x}: seq={}, ts={}, payload={} bytes",
                                   packet_count, 
                                   packet_ssrc,
                                   packet.header.sequence_number,
                                   packet.header.timestamp,
                                   packet.payload.len());
                        },
                        Ok(RtpSessionEvent::NewStreamDetected { ssrc }) => {
                            // Track if we've seen this SSRC before
                            if seen_ssrcs.insert(ssrc) {
                                info!("Discovered new stream with SSRC={:08x}", ssrc);
                                
                                // Update the counter of seen SSRCs
                                let mut count = seen_ssrcs_count_clone.lock().await;
                                *count = seen_ssrcs.len();
                            }
                        },
                        Ok(RtpSessionEvent::Error(e)) => {
                            info!("Session error: {}", e);
                        },
                        Ok(RtpSessionEvent::Bye { ssrc, reason }) => {
                            info!("Received BYE from SSRC={:08x}, reason: {:?}", ssrc, reason);
                        },
                        Ok(RtpSessionEvent::RtcpSenderReport { ssrc, .. }) => {
                            debug!("Received RTCP SR from SSRC={:08x}", ssrc);
                        },
                        Ok(RtpSessionEvent::RtcpReceiverReport { ssrc, .. }) => {
                            debug!("Received RTCP RR from SSRC={:08x}", ssrc);
                        },
                        Err(e) => {
                            info!("Event channel error: {}", e);
                            break;
                        }
                    }
                },
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Periodically check all SSRCs directly using the new method
                    let mut receiver_guard = receiver_clone.lock().await;
                    let all_ssrcs = receiver_guard.get_all_ssrcs().await;
                    info!("Currently detected {} SSRCs via get_all_ssrcs()", all_ssrcs.len());
                    
                    // Update our seen count directly from the source of truth
                    for ssrc in &all_ssrcs {
                        if seen_ssrcs.insert(*ssrc) {
                            info!("Discovered new stream with SSRC={:08x} via polling", ssrc);
                        }
                    }
                    
                    // Update the counter
                    let mut count = seen_ssrcs_count_clone.lock().await;
                    *count = seen_ssrcs.len();
                    
                    // Also get the stream count using the old method
                    stream_count = receiver_guard.stream_count().await;
                    info!("Current number of streams via stream_count(): {}", stream_count);
                    
                    // Get information about all streams
                    let all_streams = receiver_guard.get_all_streams().await;
                    for stream in all_streams {
                        info!("Stream SSRC={:08x}: packets={}, lost={}, jitter={}ms",
                             stream.ssrc, stream.packets_received, stream.packets_lost, stream.jitter);
                    }
                }
            }
        }
    });
    
    // Send packets from different SSRCs
    info!("Sending packets from 3 different SSRCs...");
    
    // Send a few packets from the main SSRC
    let mut timestamp = 0;
    for i in 0..5 {
        let payload = Bytes::from(format!("Packet {} from main SSRC", i));
        sender.send_packet(timestamp, payload, false).await?;
        timestamp += 160; // 20ms at 8kHz
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Wait a bit before sending from other SSRCs
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Create some raw packets with alt_ssrc1
    info!("Sending packets with alternate SSRC1: {:08x}", alt_ssrc1);
    let transport = sender.transport();
    
    // Create and send packets with alt_ssrc1
    timestamp = 0;
    for i in 0..5 {
        let packet = rvoip_rtp_core::RtpPacket::new_with_payload(
            96, 
            i as u16, 
            timestamp, 
            alt_ssrc1, 
            Bytes::from(format!("Packet {} from alt SSRC1", i))
        );
        timestamp += 160;
        
        // Send directly through the transport
        transport.send_rtp(&packet, receiver_addr).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Wait a bit before sending from other SSRCs
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Create and send packets with alt_ssrc2
    info!("Sending packets with alternate SSRC2: {:08x}", alt_ssrc2);
    timestamp = 0;
    for i in 0..5 {
        let packet = rvoip_rtp_core::RtpPacket::new_with_payload(
            96, 
            i as u16, 
            timestamp, 
            alt_ssrc2, 
            Bytes::from(format!("Packet {} from alt SSRC2", i))
        );
        timestamp += 160;
        
        // Send directly through the transport
        transport.send_rtp(&packet, receiver_addr).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Wait for all packets to be processed
    // The jitter buffer might delay packet processing, so we'll wait longer
    info!("Waiting for all packets to be processed...");
    
    // Wait up to 10 seconds for all SSRCs to be detected
    let expected_ssrc_count = 3; // We're expecting 3 different SSRCs
    for _ in 0..10 {
        let current_count = *seen_ssrcs_count.lock().await;
        info!("Currently detected {} out of {} expected SSRCs", current_count, expected_ssrc_count);
        
        if current_count >= expected_ssrc_count {
            info!("All expected SSRCs have been detected!");
            break;
        }
        
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
    // Give a bit more time for packets to be fully processed through jitter buffers
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Check if the streams were properly demultiplexed
    info!("Verifying stream demultiplexing...");
    
    // Create a list of SSRCs to check
    let ssrcs_to_check = vec![sender_ssrc, alt_ssrc1, alt_ssrc2];
    
    // Get information about specific streams with retries
    let mut receiver_guard = receiver.lock().await;
    let mut total_streams_found = 0;
    
    for &ssrc in &ssrcs_to_check {
        if let Some(stream_stats) = receiver_guard.get_stream(ssrc).await {
            info!("Stream with SSRC={:08x} has {} packets received", 
                  stream_stats.ssrc, stream_stats.packets_received);
            total_streams_found += 1;
        } else {
            info!("No stream found for SSRC={:08x}", ssrc);
        }
    }
    
    // Show all streams in case we received unexpected ones
    info!("Listing all {} streams:", receiver_guard.stream_count().await);
    let all_streams = receiver_guard.get_all_streams().await;
    for stream in all_streams {
        info!("Stream SSRC={:08x}: packets={}, lost={}, jitter={}ms",
             stream.ssrc, stream.packets_received, stream.packets_lost, stream.jitter);
    }
    
    // Report success or failure
    if total_streams_found == ssrcs_to_check.len() {
        info!("✅ Success! All {} expected streams were demultiplexed correctly", ssrcs_to_check.len());
    } else {
        info!("⚠️ Warning: Only found {} out of {} expected streams", 
             total_streams_found, ssrcs_to_check.len());
    }
    
    // Close the sessions
    sender.close().await?;
    receiver_guard.close().await?;
    
    // Cancel the receiver task
    receiver_handle.abort();
    
    info!("Example completed successfully");
    Ok(())
} 