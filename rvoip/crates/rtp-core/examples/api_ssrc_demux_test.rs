//! SSRC Demultiplexing API Test
//!
//! This test verifies that the SSRC demultiplexing feature works correctly
//! when enabled in the client and server APIs.

use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::time;
use tracing::{info, debug, warn, Level, error};

use rvoip_rtp_core::api::{
    client::{
        transport::{MediaTransportClient, DefaultMediaTransportClient},
        config::{ClientConfig, ClientConfigBuilder},
    },
    server::{
        transport::{MediaTransportServer, DefaultMediaTransportServer},
        config::{ServerConfig, ServerConfigBuilder},
    },
    common::{
        frame::MediaFrame,
        frame::MediaFrameType,
    },
};

/// Test SSRC demultiplexing with static SSRCs to avoid conversion issues
const AUDIO_SSRC: u32 = 0x12345678;
const VIDEO_SSRC: u32 = 0x87654321;

/// Timeout for the entire test to ensure it doesn't hang
const TEST_TIMEOUT_SECS: u64 = 60;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with more verbose output
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .init();
    
    info!("Starting SSRC Demultiplexing API test");
    
    // Use a timeout for the entire test to prevent hanging
    tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), run_test()).await??;
    
    info!("SSRC Demultiplexing API test completed");
    Ok(())
}

async fn run_test() -> Result<(), Box<dyn std::error::Error>> {
    // ====== Step 1: Setup basic clients and connection without demux ======
    info!("=== Step 1: Setting up server and regular client ===");
    // Create basic server
    let server_config = ServerConfigBuilder::new()
        .local_address("127.0.0.1:0".parse().unwrap())
        .rtcp_mux(true)
        .build()?;
    
    // Create and start server (without SSRC demultiplexing)
    let server = DefaultMediaTransportServer::new(server_config).await?;
    server.start().await?;
    
    // Get the server's bound address
    let server_addr = server.get_local_address().await?;
    info!("Server bound to {}", server_addr);
    
    // ====== Step 2: Create and verify a client with demultiplexing ======
    info!("=== Step 2: Setting up client with SSRC demultiplexing ===");
    // Configure client with SSRC demultiplexing enabled
    let client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
        .rtcp_mux(true)
        .ssrc_demultiplexing_enabled(true)
        .build();
    
    // Create client
    let client = DefaultMediaTransportClient::new(client_config).await?;
    
    // Connect client to server
    info!("Connecting client to server at {}", server_addr);
    client.connect().await?;
    info!("Client connected: {}", client.is_connected().await?);
    
    // Get the client's session info
    let session = client.get_session().await?;
    let session_guard = session.lock().await;
    let default_ssrc = session_guard.get_ssrc();
    info!("Client session default SSRC: {:08x}", default_ssrc);
    drop(session_guard);
    
    // ====== Step 3: Enable SSRC demux on the server ======
    info!("=== Step 3: Enabling SSRC demultiplexing on server ===");
    // Check if SSRC demultiplexing is enabled on the server
    let server_demux_enabled = server.is_ssrc_demultiplexing_enabled().await?;
    info!("Server SSRC demultiplexing initially enabled: {}", server_demux_enabled);
    
    // Explicitly enable it if not already enabled
    if !server_demux_enabled {
        info!("Enabling SSRC demultiplexing on server");
        let result = server.enable_ssrc_demultiplexing().await?;
        info!("Server SSRC demultiplexing enabled result: {}", result);
    }
    
    // Verify it's now enabled
    let server_demux_enabled = server.is_ssrc_demultiplexing_enabled().await?;
    info!("Server SSRC demultiplexing now enabled: {}", server_demux_enabled);
    
    // ====== Step 4: Register custom SSRCs with the client ======
    info!("=== Step 4: Registering custom SSRCs with client ===");
    // Check if client demux is enabled
    let client_demux_enabled = client.is_ssrc_demultiplexing_enabled().await?;
    info!("Client SSRC demultiplexing enabled: {}", client_demux_enabled);
    
    // Pre-register the audio SSRC
    info!("Pre-registering AUDIO_SSRC {:08x} for demultiplexing", AUDIO_SSRC);
    match client.register_ssrc(AUDIO_SSRC).await {
        Ok(created) => info!("Audio SSRC registration result: {}", created),
        Err(e) => warn!("Failed to register Audio SSRC: {}", e),
    }
    
    // Pre-register the video SSRC
    info!("Pre-registering VIDEO_SSRC {:08x} for demultiplexing", VIDEO_SSRC);
    match client.register_ssrc(VIDEO_SSRC).await {
        Ok(created) => info!("Video SSRC registration result: {}", created),
        Err(e) => warn!("Failed to register Video SSRC: {}", e),
    }
    
    // Check if sequence numbers were initialized
    match client.get_sequence_number(AUDIO_SSRC).await {
        Some(seq) => info!("Found sequence number {} for AUDIO_SSRC", seq),
        None => warn!("No sequence number found for AUDIO_SSRC"),
    }
    
    match client.get_sequence_number(VIDEO_SSRC).await {
        Some(seq) => info!("Found sequence number {} for VIDEO_SSRC", seq),
        None => warn!("No sequence number found for VIDEO_SSRC"),
    }
    
    // List all registered SSRCs
    let ssrcs = client.get_all_ssrcs().await?;
    info!("All registered SSRCs: {:?}", ssrcs);
    
    // ====== Step 5: Set up frame receiver ======
    info!("=== Step 5: Setting up frame receiver ===");
    // Flag to signal successful receipt of AUDIO test packet
    let audio_packet_received = Arc::new(AtomicBool::new(false));
    let audio_packet_received_clone = audio_packet_received.clone();
    
    // Flag to signal successful receipt of VIDEO test packet
    let video_packet_received = Arc::new(AtomicBool::new(false));
    let video_packet_received_clone = video_packet_received.clone();
    
    // Channel to track received frames
    let received_frames = Arc::new(Mutex::new(Vec::new()));
    let received_frames_clone = received_frames.clone();
    
    // Start a task to receive frames from the server
    let server_clone = server.clone();
    tokio::spawn(async move {
        info!("Frame receiver task started");
        loop {
            match tokio::time::timeout(Duration::from_millis(100), server_clone.receive_frame()).await {
                Ok(Ok((client_id, frame))) => {
                    info!("Received frame from client {}: SSRC={:08x}, PT={}, seq={}, len={}",
                          client_id, frame.ssrc, frame.payload_type, frame.sequence, frame.data.len());
                    
                    // Check for our test packets
                    if frame.ssrc == AUDIO_SSRC {
                        if let Ok(text) = std::str::from_utf8(&frame.data) {
                            info!("FOUND AUDIO TEST PACKET: {}", text);
                        }
                        audio_packet_received_clone.store(true, Ordering::SeqCst);
                    } else if frame.ssrc == VIDEO_SSRC {
                        if let Ok(text) = std::str::from_utf8(&frame.data) {
                            info!("FOUND VIDEO TEST PACKET: {}", text);
                        }
                        video_packet_received_clone.store(true, Ordering::SeqCst);
                    }
                    
                    // Store received frame for analysis
                    let mut frames = received_frames_clone.lock().await;
                    frames.push((client_id, frame));
                }
                Ok(Err(e)) => {
                    if !e.to_string().contains("Timeout") {
                        error!("Error receiving frame: {}", e);
                        break;
                    }
                }
                Err(_) => {
                    // Just timeout, continue
                }
            }
        }
    });
    
    // Wait for receiver to be ready
    time::sleep(Duration::from_millis(500)).await;
    
    // ====== Step 6: Send test frames with custom SSRCs ======
    info!("=== Step 6: Sending test frames with custom SSRCs ===");
    // Send an AUDIO test frame
    info!("Sending AUDIO test frame with SSRC={:08x}", AUDIO_SSRC);
    let audio_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: format!("AUDIO test frame with SSRC={:08x}", AUDIO_SSRC).into_bytes(),
        timestamp: 1000,
        sequence: client.get_sequence_number(AUDIO_SSRC).await.unwrap_or(100),
        marker: true,
        payload_type: 8, // Audio
        ssrc: AUDIO_SSRC,
    };
    
    match client.send_frame(audio_frame).await {
        Ok(()) => info!("Successfully sent AUDIO test frame"),
        Err(e) => error!("Error sending AUDIO test frame: {}", e),
    }
    
    // Send a VIDEO test frame
    info!("Sending VIDEO test frame with SSRC={:08x}", VIDEO_SSRC);
    let video_frame = MediaFrame {
        frame_type: MediaFrameType::Video,
        data: format!("VIDEO test frame with SSRC={:08x}", VIDEO_SSRC).into_bytes(),
        timestamp: 2000,
        sequence: client.get_sequence_number(VIDEO_SSRC).await.unwrap_or(200),
        marker: true,
        payload_type: 96, // Video
        ssrc: VIDEO_SSRC,
    };
    
    match client.send_frame(video_frame).await {
        Ok(()) => info!("Successfully sent VIDEO test frame"),
        Err(e) => error!("Error sending VIDEO test frame: {}", e),
    }
    
    // Wait for frames to be processed
    info!("Waiting for test frames to be processed...");
    
    // Wait up to 5 seconds for both test packets to be received
    let mut attempts = 0;
    while (!audio_packet_received.load(Ordering::SeqCst) || 
           !video_packet_received.load(Ordering::SeqCst)) && 
          attempts < 50 {
        time::sleep(Duration::from_millis(100)).await;
        attempts += 1;
        if attempts % 10 == 0 {
            info!("Still waiting for packets... ({} attempts)", attempts);
            info!("  - AUDIO packet received: {}", audio_packet_received.load(Ordering::SeqCst));
            info!("  - VIDEO packet received: {}", video_packet_received.load(Ordering::SeqCst));
        }
    }
    
    // ====== Step 7: Evaluate results ======
    info!("=== Step 7: Evaluating results ===");
    // Display summary of received frames
    let frames = received_frames.lock().await;
    info!("Received {} frames in total", frames.len());
    for (i, (client_id, frame)) in frames.iter().enumerate() {
        if let Ok(text) = std::str::from_utf8(&frame.data) {
            info!("Frame #{}: client={}, SSRC={:08x}, payload={}",
                  i+1, client_id, frame.ssrc, text);
        } else {
            info!("Frame #{}: client={}, SSRC={:08x}, binary data, len={}",
                  i+1, client_id, frame.ssrc, frame.data.len());
        }
    }
    
    let audio_received = audio_packet_received.load(Ordering::SeqCst);
    let video_received = video_packet_received.load(Ordering::SeqCst);
    
    // Print final results
    info!("Final results:");
    info!("  - AUDIO packet received: {}", audio_received);
    info!("  - VIDEO packet received: {}", video_received);
    
    // Close connections
    client.disconnect().await?;
    server.stop().await?;
    
    if audio_received && video_received {
        info!("SUCCESS: Both AUDIO and VIDEO packets were received with their own SSRCs!");
        Ok(())
    } else {
        let msg = format!("FAILURE: Not all test packets were received. AUDIO={}, VIDEO={}", 
                         audio_received, video_received);
        error!("{}", msg);
        Err(msg.into())
    }
} 