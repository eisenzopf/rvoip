//! Example demonstrating the use of RtpMediaTransport
//!
//! This example shows how to set up an RTP session and use it with
//! the MediaTransport interface that media-core will consume.

use bytes::Bytes;
use std::time::Duration;
use tokio::time;
use tracing::{info, debug, error};
use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent, 
    MediaTransport, RtpMediaTransport, UdpRtpTransport
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting RTP MediaTransport example");
    
    // Create RTP sessions
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
    
    // Create RTP sessions
    let mut sender_session = RtpSession::new(sender_config).await?;
    let mut receiver_session = RtpSession::new(receiver_config).await?;
    
    // Get sender and receiver addresses
    let sender_addr = sender_session.local_addr()?;
    let receiver_addr = receiver_session.local_addr()?;
    
    info!("Sender bound to {}", sender_addr);
    info!("Receiver bound to {}", receiver_addr);
    
    // Set remote address in receiver session
    receiver_session.set_remote_addr(sender_addr).await;
    
    // Create MediaTransport for sender
    let transport = RtpMediaTransport::new(sender_session);
    
    // Set remote address directly on the transport
    transport.set_remote_addr(receiver_addr).await?;
    
    // Subscribe to events from receiver
    let mut event_receiver = receiver_session.subscribe();
    
    // Small delay to ensure address propagation
    time::sleep(Duration::from_millis(50)).await;
    
    // Spawn a task to handle received packets
    tokio::spawn(async move {
        while let Ok(event) = event_receiver.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    info!("Received packet: PT={}, SEQ={}, TS={}, size={}",
                        packet.header.payload_type,
                        packet.header.sequence_number,
                        packet.header.timestamp,
                        packet.payload.len());
                        
                    // Convert payload bytes to string if it's text
                    if let Ok(text) = String::from_utf8(packet.payload.to_vec()) {
                        info!("Payload content: {}", text);
                    }
                },
                RtpSessionEvent::Error(e) => {
                    error!("Receiver error: {}", e);
                },
                // Catch all other events
                _ => {}
            }
        }
    });
    
    // Send some test packets via the MediaTransport
    for i in 0..5 {
        let message = format!("Test packet {}", i);
        let payload = Bytes::from(message.as_bytes().to_vec());
        
        // Send packet
        let timestamp = i * 160; // 20ms worth of samples at 8kHz
        
        info!("Sending packet {} with timestamp {}", i, timestamp);
        transport.send_media(0, timestamp, payload, true).await?;
        
        // Wait a bit between packets
        time::sleep(Duration::from_millis(200)).await;
    }
    
    // Let the last packets arrive
    time::sleep(Duration::from_millis(500)).await;
    
    // Shut down
    info!("Shutting down");
    transport.close().await?;
    
    Ok(())
} 