//! Example demonstrating RTCP APPLICATION packet handling
//!
//! This example shows how RTCP APP packets can be sent and received
//! between RTP sessions.

use bytes::Bytes;
use std::time::Duration;
use tokio::time;
use tracing::{info, debug, error, warn};
use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent, RtpEvent,
    packet::rtcp::{RtcpPacket, RtcpApplicationDefined},
    transport::RtpTransport,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting RTCP APP packet example");
    
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
    
    // Create a task to listen for RTCP events on the receiver side
    let receiver_transport = receiver_session.transport();
    let mut receiver_transport_events = receiver_transport.subscribe();
    
    let rtcp_task = tokio::spawn(async move {
        info!("Receiver waiting for RTCP packets...");
        
        while let Ok(event) = receiver_transport_events.recv().await {
            match event {
                RtpEvent::RtcpReceived { data, source } => {
                    match RtcpPacket::parse(&data) {
                        Ok(packet) => {
                            match packet {
                                RtcpPacket::ApplicationDefined(app) => {
                                    info!("Received APP packet:");
                                    info!("  SSRC: {:08x}", app.ssrc);
                                    info!("  Name: {}", app.name_str());
                                    info!("  Data length: {} bytes", app.data.len());
                                    
                                    if let Ok(text) = std::str::from_utf8(&app.data) {
                                        info!("  Data: {}", text);
                                    }
                                    
                                    break;
                                }
                                _ => {
                                    debug!("Received non-APP RTCP packet: {:?}", packet);
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
        
        info!("RTCP APP handler finished");
    });
    
    // Send a few RTP packets to establish the session
    for i in 0..2 {
        let message = format!("Test packet {}", i);
        let payload = Bytes::from(message.as_bytes().to_vec());
        
        // Send packet
        let timestamp = i * 160; // 20ms worth of samples at 8kHz
        
        info!("Sending RTP packet {} with timestamp {}", i, timestamp);
        sender_session.send_packet(timestamp, payload, true).await?;
        
        // Wait a bit between packets
        time::sleep(Duration::from_millis(50)).await;
    }
    
    // Wait to ensure RTP packets are processed
    time::sleep(Duration::from_millis(100)).await;
    
    // Create an APP packet
    let app_packet = RtcpApplicationDefined::new_with_name(
        sender_session.get_ssrc(),
        "TEST"
    )?;
    
    // Set some application data (just a test message)
    let app_data = Bytes::from("This is application-specific data".as_bytes().to_vec());
    let mut app_packet = app_packet;
    app_packet.set_data(app_data);
    
    // Create an RTCP packet with the APP packet
    let rtcp_packet = RtcpPacket::ApplicationDefined(app_packet);
    
    // Serialize and send
    let rtcp_data = rtcp_packet.serialize()?;
    
    // Send the RTCP APP packet directly to the receiver
    info!("Sending RTCP APP packet");
    sender_session.transport().send_rtcp_bytes(&rtcp_data, receiver_addr).await?;
    
    // Wait for the APP packet to be processed
    match tokio::time::timeout(Duration::from_millis(500), rtcp_task).await {
        Ok(_) => info!("APP packet processing completed"),
        Err(_) => warn!("Timeout waiting for APP packet processing to complete"),
    }
    
    // Clean up sessions
    sender_session.close().await?;
    receiver_session.close().await?;
    
    info!("Example completed");
    Ok(())
} 