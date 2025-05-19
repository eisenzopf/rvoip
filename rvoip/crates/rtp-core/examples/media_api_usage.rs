/// Example showing how media-core can integrate with rtp-core using the new API.
///
/// This example demonstrates:
/// 1. Creating a MediaTransportSession
/// 2. Configuring security with DTLS-SRTP
/// 3. Setting up jitter buffer
/// 4. Sending and receiving media frames
/// 5. Monitoring statistics

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::api::transport::{
    MediaTransportSession, MediaTransportConfig, MediaTransportConfigBuilder,
    MediaFrame, MediaFrameType, MediaTransportFactory
};
use rvoip_rtp_core::api::security::{SecurityConfigBuilder, SecurityMode, SecureMediaContext};
use rvoip_rtp_core::api::buffer::{MediaBufferConfigBuilder, NetworkPreset};
use rvoip_rtp_core::api::stats::{QualityLevel, StatsFactory};

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    println!("RTP-Core Media API Example");
    println!("=========================");
    
    // Create local addresses
    let local_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000);
    let remote_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    
    // Configure media transport
    let transport_config = MediaTransportConfigBuilder::new()
        .local_address(local_addr)
        .remote_address(remote_addr)
        .rtcp_mux(true)
        .media_types(vec![MediaFrameType::Audio])
        .mtu(1200)
        .build()?;
    
    // Configure security
    let security_config = SecurityConfigBuilder::webrtc()
        .mode(SecurityMode::DtlsSrtp)
        .require_secure(true)
        .dtls_client(true)
        .build()?;
    
    // Configure buffer
    let buffer_config = MediaBufferConfigBuilder::new()
        .preset(NetworkPreset::Balanced)
        .audio()
        .build()?;
    
    // Create media transport session
    println!("Creating media transport session...");
    let session = MediaTransportFactory::create_session(
        transport_config,
        Some(security_config),
        Some(buffer_config.clone())
    ).await?;
    
    // Register event handler
    session.on_event(Box::new(|event| {
        println!("Media transport event: {:?}", event);
    }))?;
    
    // For this example, we need another session to act as the remote peer
    // In a real application, this would be a separate process or device
    println!("Creating peer media transport session...");
    
    // Create reverse configuration (local/remote swapped)
    let peer_transport_config = MediaTransportConfigBuilder::new()
        .local_address(remote_addr)
        .remote_address(local_addr)
        .rtcp_mux(true)
        .media_types(vec![MediaFrameType::Audio])
        .mtu(1200)
        .build()?;
    
    // Configure peer security as server
    let peer_security_config = SecurityConfigBuilder::webrtc()
        .mode(SecurityMode::DtlsSrtp)
        .require_secure(true)
        .dtls_client(false) // Server role
        .build()?;
    
    // Create peer session
    let peer_session = MediaTransportFactory::create_session(
        peer_transport_config,
        Some(peer_security_config),
        Some(buffer_config.clone())
    ).await?;
    
    // Get security info from both sessions to exchange fingerprints
    let session_security_info = session
        .get_security_info()
        .await?;
    
    let peer_security_info = peer_session
        .get_security_info()
        .await?;
    
    // Exchange fingerprints (simulating SDP exchange)
    println!("Exchanging DTLS fingerprints between peers");
    println!("Local fingerprint: {}", session_security_info.fingerprint.as_ref().unwrap());
    println!("Peer fingerprint: {}", peer_security_info.fingerprint.as_ref().unwrap());
    
    // Set remote fingerprints
    session.set_remote_fingerprint(
        peer_security_info.fingerprint.as_ref().unwrap(),
        peer_security_info.fingerprint_algorithm.as_ref().unwrap()
    ).await?;
    
    peer_session.set_remote_fingerprint(
        session_security_info.fingerprint.as_ref().unwrap(),
        session_security_info.fingerprint_algorithm.as_ref().unwrap()
    ).await?;
    
    // Make sure remote addresses are set
    session.set_remote_address(remote_addr).await?;
    peer_session.set_remote_address(local_addr).await?;
    
    // Create stats collector
    let stats_collector = StatsFactory::create_collector();
    
    // Register quality change callback
    stats_collector.on_quality_change(Box::new(|quality: QualityLevel| {
        println!("Quality changed to: {:?}", quality);
    })).await;
    
    // Register bandwidth update callback
    stats_collector.on_bandwidth_update(Box::new(|bps: u32| {
        println!("Bandwidth estimate updated: {} bps", bps);
    })).await;
    
    // Start both transports
    println!("Starting transport session and peer...");
    peer_session.start().await?;
    session.start().await?;
    
    // Give some time for DTLS handshake to complete
    println!("Waiting for DTLS handshake to complete...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Setup is complete
    println!("Media transport session is ready");
    
    // Create dummy audio frames for demo
    let audio_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9], // Sample audio data
        timestamp: 1000,
        sequence: 1,
        marker: false,
        payload_type: 0, // G.711 µ-law
        ssrc: 12345,
    };
    
    // Send some frames
    println!("Sending 10 audio frames...");
    for i in 0..10 {
        // Update sequence and timestamp
        let mut frame = audio_frame.clone();
        frame.sequence = i + 1;
        frame.timestamp = 1000 + (i as u32 * 160); // 20ms frames at 8kHz
        
        // Send the frame
        session.send_frame(frame).await?;
        
        // Small delay to simulate real-time
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    
    // Try to receive frames
    println!("Checking for received frames on peer...");
    for _ in 0..5 {
        match peer_session.receive_frame(Duration::from_millis(100)).await? {
            Some(frame) => {
                println!("Peer received frame: type={:?}, pt={}, seq={}, ts={}",
                         frame.frame_type, frame.payload_type, frame.sequence, frame.timestamp);
            },
            None => {
                println!("No frame received within timeout");
            }
        }
    }
    
    // Check stats
    println!("Media transport statistics:");
    match session.get_stats().await {
        Ok(stats) => {
            println!("  Session duration: {:?}", stats.session_duration);
            println!("  Active streams: {}", stats.streams.len());
            println!("  Upstream bandwidth: {} bps", stats.upstream_bandwidth_bps);
            println!("  Downstream bandwidth: {} bps", stats.downstream_bandwidth_bps);
            println!("  Quality level: {:?}", stats.quality);
            
            // Print details for each stream
            for (ssrc, stream) in stats.streams {
                println!("  Stream SSRC {}:", ssrc);
                println!("    Direction: {:?}", stream.direction);
                println!("    Media type: {:?}", stream.media_type);
                println!("    Packets: {}", stream.packet_count);
                println!("    Lost: {} ({:.2}%)", 
                       stream.packets_lost, 
                       stream.fraction_lost * 100.0);
                println!("    Jitter: {:.2} ms", stream.jitter_ms);
                if let Some(rtt) = stream.rtt_ms {
                    println!("    RTT: {:.2} ms", rtt);
                }
                if let Some(mos) = stream.mos {
                    println!("    MOS: {:.1}", mos);
                }
                println!("    Bitrate: {} bps", stream.bitrate_bps);
            }
        },
        Err(e) => {
            println!("Failed to get stats: {}", e);
        }
    }
    
    // Clean up
    println!("Stopping transport...");
    session.stop().await?;
    peer_session.stop().await?;
    
    println!("Example completed successfully");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_example().await
} 