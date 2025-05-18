//! RTCP Rate Limiting Example
//! 
//! This example demonstrates how RTCP packet transmission rate is limited
//! to 5% of the session bandwidth as per RFC 3550. It creates two RTP
//! sessions with different bandwidths and shows how the RTCP intervals
//! are calculated differently.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, debug, warn, error, Level};
use tracing_subscriber::FmtSubscriber;
use bytes::Bytes;

use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent,
};
use rvoip_rtp_core::session::RtpSessionSender;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with more detailed output
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    // Create session configs with different bandwidths
    // Session 1: Low bandwidth (64 kbps)
    // Session 2: High bandwidth (1000 kbps)
    let session1_bw = 64000;
    let session2_bw = 1000000;
    
    info!("Creating two RTP sessions with different bandwidths:");
    info!("Session 1: {} bits/second", session1_bw);
    info!("Session 2: {} bits/second", session2_bw);
    
    // Calculate RTCP bandwidth (5% of session bandwidth)
    let rtcp_bw1 = (session1_bw as f64 * 0.05) as u32;
    let rtcp_bw2 = (session2_bw as f64 * 0.05) as u32;
    
    info!("Session 1 RTCP bandwidth: {} bits/second (5%)", rtcp_bw1);
    info!("Session 2 RTCP bandwidth: {} bits/second (5%)", rtcp_bw2);
    
    // Create first session (low bandwidth)
    let config1 = RtpSessionConfig {
        local_addr: "127.0.0.1:0".parse()?,
        remote_addr: Some("127.0.0.1:40002".parse()?), // We'll send to session 2
        ssrc: Some(0x11111111),
        payload_type: 96,
        clock_rate: 8000,
        enable_jitter_buffer: false,
        ..Default::default()
    };
    
    let mut session1 = RtpSession::new(config1).await?;
    session1.set_bandwidth(session1_bw);
    
    // Create second session (high bandwidth)
    let config2 = RtpSessionConfig {
        local_addr: "127.0.0.1:40002".parse()?,
        remote_addr: Some(session1.local_addr()?),
        ssrc: Some(0x22222222),
        payload_type: 96,
        clock_rate: 8000,
        enable_jitter_buffer: false,
        ..Default::default()
    };
    
    let mut session2 = RtpSession::new(config2).await?;
    session2.set_bandwidth(session2_bw);
    
    // Subscribe to session events to monitor RTCP packets
    let mut events1 = session1.subscribe();
    let mut events2 = session2.subscribe();
    
    // Set up a task to monitor RTCP events from both sessions
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = events1.recv() => {
                    if let Ok(event) = result {
                        match event {
                            RtpSessionEvent::RtcpSenderReport { ssrc, packet_count, octet_count, .. } => {
                                info!("Session 1 sent RTCP SR: ssrc={:08x}, packets={}, bytes={}",
                                      ssrc, packet_count, octet_count);
                            }
                            RtpSessionEvent::RtcpReceiverReport { ssrc, .. } => {
                                info!("Session 1 sent RTCP RR: ssrc={:08x}", ssrc);
                            }
                            _ => {}
                        }
                    }
                }
                result = events2.recv() => {
                    if let Ok(event) = result {
                        match event {
                            RtpSessionEvent::RtcpSenderReport { ssrc, packet_count, octet_count, .. } => {
                                info!("Session 2 sent RTCP SR: ssrc={:08x}, packets={}, bytes={}",
                                      ssrc, packet_count, octet_count);
                            }
                            RtpSessionEvent::RtcpReceiverReport { ssrc, .. } => {
                                info!("Session 2 sent RTCP RR: ssrc={:08x}", ssrc);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    });
    
    // Create sender handles for RTP traffic generation
    let session1_sender = session1.create_sender_handle();
    let session2_sender = session2.create_sender_handle();
    
    // Start sending RTP packets from Session 1 (low bandwidth)
    let s1_handle = tokio::spawn(async move {
        info!("Starting RTP traffic generation for Session 1 (low bandwidth)");
        
        // Send a packet every 20ms for G.711 audio simulation (64 kbps)
        let mut interval = tokio::time::interval(Duration::from_millis(20));
        let payload_size = 160; // G.711 has 8 bytes/ms -> 160 bytes for 20ms
        
        for i in 0..750 { // 750 * 20ms = 15 seconds
            interval.tick().await;
            
            // Generate dummy payload
            let payload = Bytes::from(vec![0u8; payload_size]);
            
            // Send packet
            let timestamp = i * 160; // 8kHz = 8 samples/ms, 20ms = 160 samples
            if let Err(e) = session1_sender.send_packet(timestamp, payload, false).await {
                error!("Failed to send packet from session 1: {}", e);
            }
        }
    });
    
    // Start sending RTP packets from Session 2 (high bandwidth)
    let s2_handle = tokio::spawn(async move {
        info!("Starting RTP traffic generation for Session 2 (high bandwidth)");
        
        // Send a packet every 20ms for video simulation (1 Mbps)
        let mut interval = tokio::time::interval(Duration::from_millis(20));
        let payload_size = 2500; // ~1 Mbps at 50 packets/second
        
        for i in 0..750 { // 750 * 20ms = 15 seconds
            interval.tick().await;
            
            // Generate dummy payload
            let payload = Bytes::from(vec![0u8; payload_size]);
            
            // Send packet
            let timestamp = i * 90; // 90kHz = 90 samples/ms, 20ms = 1800 samples
            if let Err(e) = session2_sender.send_packet(timestamp, payload, i % 30 == 0).await {
                error!("Failed to send packet from session 2: {}", e);
            }
        }
    });
    
    // Calculate expected RTCP intervals
    info!("Expected theoretical RTCP intervals:");
    
    // Calculate expected intervals (simplified approximation)
    // Interval = (RTCP packet size * members) / RTCP bandwidth
    let rtcp_packet_size = 100 * 8; // 100 bytes * 8 bits
    let members = 2;
    
    let expected_interval1 = (rtcp_packet_size * members) as f64 / rtcp_bw1 as f64;
    let expected_interval2 = (rtcp_packet_size * members) as f64 / rtcp_bw2 as f64;
    
    info!("Session 1 (low bandwidth): ~{:.2} seconds", expected_interval1);
    info!("Session 2 (high bandwidth): ~{:.2} seconds", expected_interval2);
    info!("Note: Actual intervals include randomization (0.5 to 1.5 of calculated interval)");
    
    // Wait for 15 seconds to observe multiple RTCP packets
    info!("Running for 15 seconds to observe RTCP rate limiting...");
    sleep(Duration::from_secs(15)).await;
    
    // Wait for the sending tasks to complete
    let _ = tokio::join!(s1_handle, s2_handle);
    
    info!("RTCP rate limiting demonstration completed");
    
    Ok(())
} 