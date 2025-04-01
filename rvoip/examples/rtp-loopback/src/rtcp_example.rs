use anyhow::{Context, Result};
use bytes::Bytes;
use rvoip_rtp_core::rtcp::{RtcpPacket, RtcpSenderReport, RtcpReportBlock, NtpTimestamp};
use rvoip_rtp_core::{RtpSession, RtpSsrc};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, interval};
use tracing::{info, error, debug};

/// Example of RTCP sender reports
///
/// This function demonstrates how to create and process RTCP packets
pub async fn run_rtcp_example() -> Result<()> {
    info!("Starting RTCP example");

    // Create sender and receiver addresses
    let sender_addr = "127.0.0.1:15000".parse::<SocketAddr>().unwrap();
    let receiver_addr = "127.0.0.1:15001".parse::<SocketAddr>().unwrap();
    let rtcp_sender_addr = "127.0.0.1:15002".parse::<SocketAddr>().unwrap();
    let rtcp_receiver_addr = "127.0.0.1:15003".parse::<SocketAddr>().unwrap();

    // SSRC for sender and receiver
    let sender_ssrc: RtpSsrc = 0x12345678;
    let receiver_ssrc: RtpSsrc = 0x87654321;
    
    // Shared packet counter for statistics
    let packet_count = Arc::new(AtomicU32::new(0));
    let byte_count = Arc::new(AtomicU32::new(0));
    
    // Create sender RTP session
    let sender_config = rvoip_rtp_core::session::RtpSessionConfig {
        local_addr: sender_addr,
        remote_addr: Some(receiver_addr),
        ssrc: Some(sender_ssrc),
        payload_type: 0,  // PCMU
        clock_rate: 8000, // 8kHz
        enable_jitter_buffer: false,
        ..Default::default()
    };
    
    let mut sender = RtpSession::new(sender_config)
        .await
        .context("Failed to create sender RTP session")?;
    
    info!("Sender created and bound to {}", sender_addr);
    
    // Create receiver RTP session
    let receiver_config = rvoip_rtp_core::session::RtpSessionConfig {
        local_addr: receiver_addr,
        remote_addr: Some(sender_addr),
        ssrc: Some(receiver_ssrc),
        payload_type: 0,  // PCMU
        clock_rate: 8000, // 8kHz
        ..Default::default()
    };
    
    let mut receiver = RtpSession::new(receiver_config)
        .await
        .context("Failed to create receiver RTP session")?;
    
    info!("Receiver created and bound to {}", receiver_addr);
    
    // Create channel for packet information
    let (packet_tx, mut packet_rx) = mpsc::channel::<(u16, u32)>(100);
    
    // Start packet sender task
    let packet_count_clone = Arc::clone(&packet_count);
    let byte_count_clone = Arc::clone(&byte_count);
    let packet_tx_clone = packet_tx.clone();
    let _send_handle = tokio::spawn(async move {
        let mut seq: u16 = 0;
        let mut timestamp: u32 = 0;
        let mut interval_timer = interval(Duration::from_millis(20)); // 20ms per packet
        
        // Send 100 RTP packets
        for i in 0..100 {
            interval_timer.tick().await;
            
            // Create payload
            let payload_data = format!("RTCP Test {}", i);
            let payload = Bytes::from(payload_data);
            let payload_len = payload.len() as u32;
            
            // Send packet with timestamp
            if let Err(e) = sender.send_packet(timestamp, payload, false).await {
                error!("Failed to send packet: {}", e);
                break;
            }
            
            // Update counters
            packet_count_clone.fetch_add(1, Ordering::SeqCst);
            byte_count_clone.fetch_add(payload_len, Ordering::SeqCst);
            
            // Send packet info to RTCP sender
            let _ = packet_tx_clone.send((seq, timestamp)).await;
            
            // Increment sequence and timestamp
            seq = seq.wrapping_add(1);
            timestamp = timestamp.wrapping_add(160); // 20ms at 8kHz
        }
    });
    
    // Start packet receiver task
    let packet_rx_clone = packet_tx.clone();
    let _receive_handle = tokio::spawn(async move {
        let mut last_seq: Option<u16> = None;
        let mut lost_packets = 0;
        
        // Receive packets
        loop {
            match receiver.receive_packet().await {
                Ok(packet) => {
                    let seq = packet.header.sequence_number;
                    let ts = packet.header.timestamp;
                    
                    // Check for lost packets
                    if let Some(expected_seq) = last_seq {
                        let expected = expected_seq.wrapping_add(1);
                        if expected != seq {
                            let lost = if seq > expected {
                                seq - expected
                            } else {
                                (0xFFFF - expected) + seq + 1
                            };
                            lost_packets += lost;
                            debug!("Detected {} lost packets", lost);
                        }
                    }
                    
                    last_seq = Some(seq);
                    let _ = packet_rx_clone.send((seq, ts)).await;
                }
                Err(e) => {
                    error!("Error receiving packet: {}", e);
                    break;
                }
            }
        }
    });
    
    // Start RTCP sender task - sends RTCP sender reports periodically
    let packet_count_for_rtcp = Arc::clone(&packet_count);
    let byte_count_for_rtcp = Arc::clone(&byte_count);
    tokio::spawn(async move {
        // Wait for packets to start flowing
        sleep(Duration::from_millis(100)).await;
        
        // Create a simple UDP socket for RTCP
        let socket = tokio::net::UdpSocket::bind(rtcp_sender_addr)
            .await
            .expect("Failed to bind RTCP sender socket");
            
        socket.connect(rtcp_receiver_addr)
            .await
            .expect("Failed to connect RTCP socket");
        
        // Send RTCP packets every 5 seconds
        let mut rtcp_interval = interval(Duration::from_secs(5));
        
        // Process sequence and timestamp updates
        let mut last_seq = 0;
        let mut last_timestamp = 0;
        
        // Take packet info from channel to track sequence
        tokio::spawn(async move {
            while let Some((seq, ts)) = packet_rx.recv().await {
                last_seq = seq;
                last_timestamp = ts;
            }
        });
        
        // Send RTCP reports
        for i in 0..3 {
            rtcp_interval.tick().await;
            
            // Create a sender report
            let mut sr = RtcpSenderReport::new(sender_ssrc);
            sr.ntp_timestamp = NtpTimestamp::now();
            sr.rtp_timestamp = last_timestamp;
            sr.sender_packet_count = packet_count_for_rtcp.load(Ordering::SeqCst) as u32;
            sr.sender_octet_count = byte_count_for_rtcp.load(Ordering::SeqCst);
            
            // Add a report block (normally would contain receiver stats)
            let mut report_block = RtcpReportBlock::new(receiver_ssrc);
            report_block.highest_seq = last_seq as u32;
            sr.report_blocks.push(report_block);
            
            // Create RTCP packet
            let rtcp_packet = RtcpPacket::SenderReport(sr);
            
            // This would normally serialize the packet, but we're missing that impl
            // For now, just log that we would send it
            info!("RTCP SR {}: would send report: {:?}", i, rtcp_packet);
            
            // In a real implementation, we would serialize and send:
            // let rtcp_data = rtcp_packet.serialize().unwrap();
            // socket.send(&rtcp_data).await.unwrap();
        }
    });
    
    // Let the example run for a while
    info!("RTCP example running, waiting for completion...");
    sleep(Duration::from_secs(20)).await;
    
    info!("RTCP example completed");
    Ok(())
} 