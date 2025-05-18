//! Example demonstrating RTCP Sender Reports and Receiver Reports
//!
//! This example shows how RTCP SR and RR packets are created, transmitted,
//! and processed to calculate quality metrics like jitter, packet loss, and RTT.

use bytes::Bytes;
use std::time::Duration;
use tokio::time;
use tracing::{info, debug, error, warn};
use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent,
    packet::rtcp::{RtcpPacket, RtcpSenderReport, RtcpReceiverReport, NtpTimestamp},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting RTCP reports example");
    
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
    
    // Subscribe to session events
    let mut sender_events = sender_session.subscribe();
    let mut receiver_events = receiver_session.subscribe();
    
    // Spawn a task to monitor sender events
    let sender_monitor = tokio::spawn(async move {
        while let Ok(event) = sender_events.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    info!("Sender received packet: PT={}, SEQ={}, SSRC={:08x}", 
                          packet.header.payload_type,
                          packet.header.sequence_number,
                          packet.header.ssrc);
                },
                RtpSessionEvent::RtcpSenderReport { ssrc, ntp_timestamp, rtp_timestamp, packet_count, octet_count, report_blocks } => {
                    info!("Sender received SR from SSRC={:08x}", ssrc);
                    info!("  NTP timestamp: {:?}, RTP timestamp: {}", ntp_timestamp, rtp_timestamp);
                    info!("  Packets: {}, Octets: {}", packet_count, octet_count);
                    info!("  Report blocks: {}", report_blocks.len());
                    
                    for block in report_blocks {
                        if block.ssrc == 0x12345678 { // Our SSRC
                            let fraction_lost_pct = (block.fraction_lost as f64 / 256.0) * 100.0;
                            info!("  Report about us: Loss {}%, Jitter {}", 
                                  fraction_lost_pct, block.jitter);
                        }
                    }
                },
                RtpSessionEvent::RtcpReceiverReport { ssrc, report_blocks } => {
                    info!("Sender received RR from SSRC={:08x}", ssrc);
                    info!("  Report blocks: {}", report_blocks.len());
                    
                    for block in report_blocks {
                        if block.ssrc == 0x12345678 { // Our SSRC
                            let fraction_lost_pct = (block.fraction_lost as f64 / 256.0) * 100.0;
                            info!("  Report about us: Loss {}%, Jitter {}", 
                                  fraction_lost_pct, block.jitter);
                        }
                    }
                },
                _ => {} // Ignore other events
            }
        }
    });
    
    // Spawn a task to monitor receiver events
    let receiver_monitor = tokio::spawn(async move {
        while let Ok(event) = receiver_events.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    info!("Receiver received packet: PT={}, SEQ={}, SSRC={:08x}", 
                          packet.header.payload_type,
                          packet.header.sequence_number,
                          packet.header.ssrc);
                },
                RtpSessionEvent::RtcpSenderReport { ssrc, ntp_timestamp, rtp_timestamp, packet_count, octet_count, report_blocks } => {
                    info!("Receiver received SR from SSRC={:08x}", ssrc);
                    info!("  NTP timestamp: {:?}, RTP timestamp: {}", ntp_timestamp, rtp_timestamp);
                    info!("  Packets: {}, Octets: {}", packet_count, octet_count);
                    info!("  Report blocks: {}", report_blocks.len());
                    
                    for block in report_blocks {
                        if block.ssrc == 0x87654321 { // Our SSRC
                            let fraction_lost_pct = (block.fraction_lost as f64 / 256.0) * 100.0;
                            info!("  Report about us: Loss {}%, Jitter {}", 
                                  fraction_lost_pct, block.jitter);
                        }
                    }
                },
                RtpSessionEvent::RtcpReceiverReport { ssrc, report_blocks } => {
                    info!("Receiver received RR from SSRC={:08x}", ssrc);
                    info!("  Report blocks: {}", report_blocks.len());
                    
                    for block in report_blocks {
                        if block.ssrc == 0x87654321 { // Our SSRC
                            let fraction_lost_pct = (block.fraction_lost as f64 / 256.0) * 100.0;
                            info!("  Report about us: Loss {}%, Jitter {}", 
                                  fraction_lost_pct, block.jitter);
                        }
                    }
                },
                _ => {} // Ignore other events
            }
        }
    });
    
    // Send some RTP packets from sender to receiver
    info!("Sending RTP packets from sender to receiver...");
    for i in 0..10 {
        // Create dummy payload
        let payload = Bytes::from(format!("Packet {}", i));
        
        // Send packet
        sender_session.send_packet(1000 + i * 160, payload, i == 0).await?;
        
        // Wait a bit
        time::sleep(Duration::from_millis(20)).await;
    }
    
    // Wait to allow packets to be processed
    time::sleep(Duration::from_millis(100)).await;
    
    // Send a Sender Report from sender
    info!("Sending RTCP SR from sender...");
    sender_session.send_sender_report().await?;
    
    // Wait a bit
    time::sleep(Duration::from_millis(100)).await;
    
    // Send a Receiver Report from receiver
    info!("Sending RTCP RR from receiver...");
    receiver_session.send_receiver_report().await?;
    
    // Wait for reports to be processed
    time::sleep(Duration::from_millis(200)).await;
    
    // Send another Sender Report from sender
    info!("Sending second RTCP SR from sender...");
    sender_session.send_sender_report().await?;
    
    // Wait a bit
    time::sleep(Duration::from_millis(100)).await;
    
    // Let's see what happens to the stats
    let sender_stats = sender_session.get_stats();
    let receiver_stats = receiver_session.get_stats();
    
    info!("Sender stats: Sent={}, Received={}, Jitter={}ms", 
          sender_stats.packets_sent,
          sender_stats.packets_received,
          sender_stats.jitter_ms);
          
    info!("Receiver stats: Sent={}, Received={}, Jitter={}ms, Lost={}", 
          receiver_stats.packets_sent,
          receiver_stats.packets_received,
          receiver_stats.jitter_ms,
          receiver_stats.packets_lost);
    
    // Wait for streams to fully finish
    time::sleep(Duration::from_millis(100)).await;
    
    // Close sessions
    sender_session.close().await?;
    receiver_session.close().await?;
    
    // Abort monitoring tasks
    sender_monitor.abort();
    receiver_monitor.abort();
    
    info!("Test completed successfully");
    Ok(())
} 