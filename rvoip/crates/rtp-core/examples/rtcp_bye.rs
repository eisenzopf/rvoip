//! Example demonstrating RTCP BYE packet handling
//!
//! This example shows how RTCP BYE packets are sent and received
//! when an RTP session is closed.

use bytes::Bytes;
use std::time::Duration;
use tokio::time;
use tracing::{info, debug, error, warn};
use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent, RtpEvent,
    packet::rtcp::{RtcpPacket, RtcpGoodbye},
    transport::RtpTransport,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting RTCP BYE packet example");
    
    // Create sender and receiver sessions
    let sender_config = RtpSessionConfig {
        local_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: None,
        ssrc: Some(0x12345678),
        payload_type: 0, // PCM µ-law
        clock_rate: 8000, // 8 kHz
        jitter_buffer_size: Some(5),
        max_packet_age_ms: Some(100),
        enable_jitter_buffer: true,
    };
    
    let receiver_config = RtpSessionConfig {
        local_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: None,
        ssrc: Some(0x87654321),
        payload_type: 0, // PCM µ-law
        clock_rate: 8000, // 8 kHz
        jitter_buffer_size: Some(5),
        max_packet_age_ms: Some(100),
        enable_jitter_buffer: true,
    };
    
    // Create the sessions
    let mut sender_session = RtpSession::new(sender_config).await?;
    let mut receiver_session = RtpSession::new(receiver_config).await?;
    
    // Get addresses
    let sender_addr = sender_session.local_addr()?;
    let receiver_addr = receiver_session.local_addr()?;
    
    info!("Sender bound to {}", sender_addr);
    info!("Receiver bound to {}", receiver_addr);
    
    // Set remote addresses
    sender_session.set_remote_addr(receiver_addr).await;
    receiver_session.set_remote_addr(sender_addr).await;
    
    // Subscribe to events from both sessions
    let mut sender_events = sender_session.subscribe();
    let mut receiver_events = receiver_session.subscribe();
    
    // Also subscribe to transport events from the receiver
    // This gives us direct access to RTCP packets
    let receiver_transport = receiver_session.transport();
    let mut receiver_transport_events = receiver_transport.subscribe();
    
    // Send a few test packets
    for i in 0..3 {
        let message = format!("Test packet {}", i);
        let payload = Bytes::from(message.as_bytes().to_vec());
        
        // Send packet
        let timestamp = i * 160; // 20ms worth of samples at 8kHz
        
        info!("Sending packet {} with timestamp {}", i, timestamp);
        sender_session.send_packet(timestamp, payload, true).await?;
        
        // Wait a bit between packets
        time::sleep(Duration::from_millis(50)).await;
    }
    
    // Create two tasks to listen for BYE events
    
    // Task 1: Listen for session-level events
    let bye_task1 = tokio::spawn(async move {
        info!("Receiver waiting for session-level BYE events");
        
        while let Ok(event) = receiver_events.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    info!("Receiver got packet: PT={}, SEQ={}, SSRC={:08x}", 
                          packet.header.payload_type,
                          packet.header.sequence_number,
                          packet.header.ssrc);
                },
                RtpSessionEvent::Error(e) => {
                    error!("Receiver error: {}", e);
                },
                RtpSessionEvent::Bye { ssrc, reason } => {
                    info!("Receiver got BYE event from SSRC={:08x}, reason: {:?}", ssrc, reason);
                    return true; // Signal that we got a BYE event
                }
            }
        }
        
        false // No BYE event received
    });
    
    // Task 2: Listen for transport-level RTCP packets
    let bye_task2 = tokio::spawn(async move {
        info!("Receiver waiting for transport-level RTCP packets");
        
        while let Ok(event) = receiver_transport_events.recv().await {
            match event {
                RtpEvent::RtcpReceived { data, source } => {
                    info!("Receiver got RTCP packet from {}", source);
                    
                    // Try to parse RTCP packet
                    match RtcpPacket::parse(&data) {
                        Ok(packet) => {
                            match packet {
                                RtcpPacket::Goodbye(bye) => {
                                    if !bye.sources.is_empty() {
                                        info!("Receiver got direct BYE packet from SSRC={:08x}, reason: {:?}", 
                                              bye.sources[0], bye.reason);
                                        return true; // Signal that we got a BYE packet
                                    }
                                }
                                _ => {
                                    debug!("Received non-BYE RTCP packet: {:?}", packet);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to parse RTCP packet: {}", e);
                        }
                    }
                }
                _ => {
                    // Ignore other events
                }
            }
        }
        
        false // No BYE packet received
    });
    
    // Create a task to listen for events on the sender side
    let sender_task = tokio::spawn(async move {
        while let Ok(event) = sender_events.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    info!("Sender got packet: PT={}, SEQ={}, SSRC={:08x}", 
                          packet.header.payload_type,
                          packet.header.sequence_number,
                          packet.header.ssrc);
                },
                RtpSessionEvent::Error(e) => {
                    error!("Sender error: {}", e);
                },
                RtpSessionEvent::Bye { ssrc, reason } => {
                    info!("Sender got BYE from SSRC={:08x}, reason: {:?}", ssrc, reason);
                }
            }
        }
    });
    
    // Wait a bit for packets to be exchanged
    time::sleep(Duration::from_millis(200)).await;
    
    // Close the sender session, which should send a BYE
    info!("Closing sender session (should send BYE)");
    sender_session.close().await?;
    
    // Wait for BYE to be processed
    time::sleep(Duration::from_millis(100)).await;
    
    // Wait for the BYE tasks to complete
    let (result1, result2) = tokio::join!(
        tokio::time::timeout(Duration::from_millis(500), bye_task1),
        tokio::time::timeout(Duration::from_millis(500), bye_task2)
    );
    
    let bye_event_received = match result1 {
        Ok(Ok(true)) => true,
        _ => false,
    };
    
    let bye_packet_received = match result2 {
        Ok(Ok(true)) => true,
        _ => false,
    };
    
    if bye_event_received || bye_packet_received {
        info!("BYE handling completed successfully");
        if bye_event_received {
            info!("- Session-level BYE event was received");
        }
        if bye_packet_received {
            info!("- Transport-level BYE packet was received");
        }
    } else {
        warn!("No BYE packet or event was detected!");
    }
    
    // Clean up receiver session
    receiver_session.close().await?;
    
    info!("Example completed");
    Ok(())
} 