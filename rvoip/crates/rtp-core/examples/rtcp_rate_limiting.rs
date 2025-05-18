//! RTCP Rate Limiting Example
//! 
//! This example demonstrates how RTCP packet transmission rate is limited
//! to 5% of the session bandwidth as per RFC 3550. It creates two RTP
//! sessions with different bandwidths and shows how the RTCP intervals
//! are calculated differently.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{info, debug, warn, error};
use bytes;

use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("=== RTCP Rate Limiting Example ===");
    info!("This example demonstrates how RTCP packet bandwidth is limited to 5% of session bandwidth");
    
    // Create two RTP sessions with different bandwidths
    let addr1 = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
    let addr2 = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
    
    // Session 1: Low bandwidth (64 kbps)
    let session1_config = RtpSessionConfig {
        local_addr: addr1,
        remote_addr: None, // Will be set after session2 is created
        ssrc: None, // Auto-generate SSRC
        payload_type: 0,  // PCM μ-law
        clock_rate: 8000, // 8 kHz
        jitter_buffer_size: Some(50),
        max_packet_age_ms: Some(200),
        enable_jitter_buffer: true,
    };
    
    // Session 2: High bandwidth (1 Mbps)
    let session2_config = RtpSessionConfig {
        local_addr: addr2,
        remote_addr: None, // Will be set after session1 is created
        ssrc: None, // Auto-generate SSRC
        payload_type: 0,  // PCM μ-law
        clock_rate: 8000, // 8 kHz
        jitter_buffer_size: Some(50),
        max_packet_age_ms: Some(200),
        enable_jitter_buffer: true,
    };
    
    // Create the sessions
    let mut session1 = RtpSession::new(session1_config).await?;
    let mut session2 = RtpSession::new(session2_config).await?;
    
    // Get the local addresses
    let session1_addr = session1.local_addr()?;
    let session2_addr = session2.local_addr()?;
    
    info!("Session 1 (low bandwidth) bound to {}", session1_addr);
    info!("Session 2 (high bandwidth) bound to {}", session2_addr);
    
    // Set remote addresses - they'll communicate with each other
    session1.set_remote_addr(session2_addr).await;
    session2.set_remote_addr(session1_addr).await;
    
    // Set bandwidths
    let low_bandwidth = 64000; // 64 kbps
    let high_bandwidth = 1000000; // 1 Mbps
    
    session1.set_bandwidth(low_bandwidth);
    session2.set_bandwidth(high_bandwidth);
    
    info!("Session 1 bandwidth: {} kbps", low_bandwidth / 1000);
    info!("Session 2 bandwidth: {} kbps", high_bandwidth / 1000);
    
    // RTCP bandwidth is 5% of session bandwidth
    let rtcp_bw1 = (low_bandwidth as f64 * 0.05) as u32;
    let rtcp_bw2 = (high_bandwidth as f64 * 0.05) as u32;
    
    info!("Session 1 RTCP bandwidth: {} bps (5%)", rtcp_bw1);
    info!("Session 2 RTCP bandwidth: {} bps (5%)", rtcp_bw2);
    
    // Generate some RTP traffic to trigger RTCP reports
    let session1_sender = session1.create_sender_handle();
    let session2_sender = session2.create_sender_handle();
    
    // Session 1 sending task
    let s1_handle = tokio::spawn(async move {
        info!("Starting RTP traffic generation for Session 1");
        
        // Send a packet every 20ms to generate traffic
        let mut interval = tokio::time::interval(Duration::from_millis(20));
        
        for _ in 0..750 { // 750 * 20ms = 15 seconds
            interval.tick().await;
            
            // Generate dummy payload
            let payload = bytes::Bytes::from(vec![0; 160]); // G.711 frame size
            
            // Send the packet
            if let Err(e) = session1_sender.send_packet(0, payload, false).await {
                error!("Failed to send RTP packet from Session 1: {}", e);
            }
        }
    });
    
    // Session 2 sending task
    let s2_handle = tokio::spawn(async move {
        info!("Starting RTP traffic generation for Session 2");
        
        // Send a packet every 20ms to generate traffic
        let mut interval = tokio::time::interval(Duration::from_millis(20));
        
        for _ in 0..750 { // 750 * 20ms = 15 seconds
            interval.tick().await;
            
            // Generate dummy payload
            let payload = bytes::Bytes::from(vec![0; 160]); // G.711 frame size
            
            // Send the packet
            if let Err(e) = session2_sender.send_packet(0, payload, false).await {
                error!("Failed to send RTP packet from Session 2: {}", e);
            }
        }
    });
    
    // Subscribe to events from both sessions to monitor RTCP packets
    let session1_events = Arc::new(Mutex::new(session1.subscribe()));
    let session2_events = Arc::new(Mutex::new(session2.subscribe()));
    
    // Session 1 event handler
    let session1_events_clone = session1_events.clone();
    let session1_handle = tokio::spawn(async move {
        let mut rtcp_packet_times = Vec::new();
        let mut last_time = std::time::Instant::now();
        
        info!("Monitoring RTCP traffic from Session 1 (low bandwidth)...");
        
        loop {
            let event = {
                let mut events = session1_events_clone.lock().await;
                events.recv().await
            };
            
            match event {
                Ok(RtpSessionEvent::RtcpSenderReport { ssrc, .. }) => {
                    let now = std::time::Instant::now();
                    let interval = now.duration_since(last_time);
                    rtcp_packet_times.push(interval);
                    
                    info!("Session 1 sent RTCP SR from SSRC={:08x}, interval: {:?}", ssrc, interval);
                    
                    last_time = now;
                },
                Ok(RtpSessionEvent::Error(e)) => {
                    error!("Session 1 error: {}", e);
                },
                Err(e) => {
                    warn!("Session 1 event channel error: {}", e);
                    break;
                },
                // Ignore other events
                _ => {}
            }
        }
    });
    
    // Session 2 event handler
    let session2_events_clone = session2_events.clone();
    let session2_handle = tokio::spawn(async move {
        let mut rtcp_packet_times = Vec::new();
        let mut last_time = std::time::Instant::now();
        
        info!("Monitoring RTCP traffic from Session 2 (high bandwidth)...");
        
        loop {
            let event = {
                let mut events = session2_events_clone.lock().await;
                events.recv().await
            };
            
            match event {
                Ok(RtpSessionEvent::RtcpSenderReport { ssrc, .. }) => {
                    let now = std::time::Instant::now();
                    let interval = now.duration_since(last_time);
                    rtcp_packet_times.push(interval);
                    
                    info!("Session 2 sent RTCP SR from SSRC={:08x}, interval: {:?}", ssrc, interval);
                    
                    last_time = now;
                },
                Ok(RtpSessionEvent::Error(e)) => {
                    error!("Session 2 error: {}", e);
                },
                Err(e) => {
                    warn!("Session 2 event channel error: {}", e);
                    break;
                },
                // Ignore other events
                _ => {}
            }
        }
    });
    
    // Wait for 15 seconds to observe multiple RTCP packets
    info!("Running for 15 seconds to observe RTCP rate limiting...");
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
    
    sleep(Duration::from_secs(15)).await;
    
    // Stop the sessions
    info!("Shutting down sessions...");
    session1.close().await?;
    session2.close().await?;
    
    // Abort the event handlers
    session1_handle.abort();
    session2_handle.abort();
    s1_handle.abort();
    s2_handle.abort();
    
    info!("Example completed. Compare the RTCP intervals observed in the logs.");
    info!("Session 1 (low bandwidth) should have much larger intervals than Session 2 (high bandwidth).");
    
    Ok(())
} 