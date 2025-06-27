//! SSRC Demultiplexing API Example
//!
//! This example demonstrates using the SSRC demultiplexing API
//! to handle multiple streams with different SSRCs.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{info, debug, warn};
use rand::Rng;

use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::server::config::{ServerConfig, ServerConfigBuilder};
use rvoip_rtp_core::api::client::config::{ClientConfig, ClientConfigBuilder};
use rvoip_rtp_core::api::server::transport::MediaTransportServer;
use rvoip_rtp_core::api::client::transport::MediaTransportClient;
use rvoip_rtp_core::api::server::transport::DefaultMediaTransportServer;
use rvoip_rtp_core::api::client::transport::DefaultMediaTransportClient;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;
use rvoip_rtp_core::api::common::config::SecurityMode;

// Constants for our streams
const AUDIO1_SSRC: u32 = 0x1234A001;
const AUDIO2_SSRC: u32 = 0x1234A002;
const VIDEO1_SSRC: u32 = 0x5678B001;
const VIDEO2_SSRC: u32 = 0x5678B002;
const AUDIO_CLOCK_RATE: u32 = 48000; // 48kHz audio
const VIDEO_CLOCK_RATE: u32 = 90000; // 90kHz video

// Global timeout to ensure our example completes
const EXAMPLE_TIMEOUT_SECS: u64 = 30;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting SSRC Demultiplexing API example");
    
    // Create a timeout for the entire example
    let example_timeout = tokio::time::timeout(
        Duration::from_secs(EXAMPLE_TIMEOUT_SECS),
        async {
            // Configure server with SSRC demultiplexing enabled
            let server_config = ServerConfigBuilder::new()
                .local_address("127.0.0.1:0".parse().unwrap())
                .rtcp_mux(true)
                .ssrc_demultiplexing_enabled(true) // Enable SSRC demultiplexing
                .security_config(rvoip_rtp_core::api::server::security::ServerSecurityConfig { 
                    security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, 
                    ..Default::default() 
                })
                .build()
                .expect("Failed to build server config");
            
            // Create server
            let server = rvoip_rtp_core::api::server::transport::DefaultMediaTransportServer::new(server_config).await?;
            
            // Check if SSRC demultiplexing is enabled on the server
            let server_demux_enabled = server.is_ssrc_demultiplexing_enabled().await?;
            info!("Server SSRC demultiplexing enabled: {}", server_demux_enabled);
            
            // Start server
            info!("Starting server...");
            server.start().await?;
            
            // Get the server's bound address
            let server_addr = server.get_local_address().await?;
            info!("Server bound to {}", server_addr);
            
            // Register an event handler to monitor received frames
            let received_frames = Arc::new(Mutex::new(Vec::new()));
            let received_frames_clone = received_frames.clone();
            
            // Clone the server for the spawned task
            let server_for_task = server.clone();
            
            // Start a task to receive frames from the server
            tokio::spawn(async move {
                // Get a persistent frame receiver instead of calling receive_frame() repeatedly
                let mut frame_receiver = server_for_task.get_frame_receiver();
                
                loop {
                    // Use the persistent receiver instead of receive_frame()
                    match tokio::time::timeout(
                        Duration::from_millis(500), 
                        frame_receiver.recv()
                    ).await {
                        Ok(Ok((client_id, frame))) => {
                            info!("Received frame from client {} with SSRC={:08x}, PT={}, seq={}, ts={}", 
                                 client_id, frame.ssrc, frame.payload_type, frame.sequence, frame.timestamp);
                            
                            // Store the frame for analysis
                            let mut frames = received_frames_clone.lock().await;
                            frames.push((client_id, frame));
                        }
                        Ok(Err(e)) => {
                            warn!("Broadcast channel error: {}", e);
                            // Break on channel errors (usually means channel is closed)
                            break;
                        }
                        Err(_) => {
                            // Timeout - this is normal, just continue waiting
                            // No need to log timeouts as errors since they're expected
                            continue;
                        }
                    }
                }
            });
            
            // Configure client with SSRC demultiplexing enabled
            let client_config = ClientConfigBuilder::new()
                .remote_address(server_addr)
                .rtcp_mux(true)
                .ssrc_demultiplexing_enabled(true) // Enable SSRC demultiplexing
                .security_config(rvoip_rtp_core::api::client::security::ClientSecurityConfig { 
                    security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, 
                    ..Default::default() 
                })
                .build();
            
            // Create client
            let client = rvoip_rtp_core::api::client::transport::DefaultMediaTransportClient::new(client_config).await?;
            
            // Connect client to server
            info!("Connecting client to server at {}", server_addr);
            client.connect().await?;
            
            // Check if client is connected
            let is_connected = client.is_connected().await?;
            info!("Client connected: {}", is_connected);
            
            // Get client local address for debugging
            let client_addr = client.get_local_address().await?;
            info!("Client bound to {}", client_addr);
            
            // Check if SSRC demultiplexing is enabled
            let demux_enabled = client.is_ssrc_demultiplexing_enabled().await?;
            info!("Client SSRC demultiplexing enabled: {}", demux_enabled);
            
            // Pre-register our SSRCs for demultiplexing
            info!("Pre-registering SSRCs for demultiplexing");
            client.register_ssrc(AUDIO1_SSRC).await?;
            client.register_ssrc(AUDIO2_SSRC).await?;
            client.register_ssrc(VIDEO1_SSRC).await?;
            client.register_ssrc(VIDEO2_SSRC).await?;
            
            // List registered SSRCs
            let ssrcs = client.get_all_ssrcs().await?;
            info!("Registered SSRCs: {:?}", ssrcs);
            
            // Access the session for direct testing
            let session = client.get_session().await?;
            let session_ssrc = {
                let s = session.lock().await;
                s.get_ssrc()
            };
            info!("Client session SSRC: {:08x}", session_ssrc);
            
            // Test basic connectivity first with the default SSRC
            info!("Testing basic connectivity with default SSRC ({:08x})", session_ssrc);
            let default_frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: b"Test frame with default SSRC".to_vec(),
                timestamp: 12345,
                sequence: 0,
                marker: true,
                payload_type: 96,
                ssrc: session_ssrc,
                csrcs: Vec::new(),
            };
            
            // Send frame with default SSRC
            client.send_frame(default_frame).await?;
            info!("Sent frame with default SSRC");
            
            // Wait a bit to allow server to process
            info!("Waiting for server to process frame...");
            time::sleep(Duration::from_secs(2)).await;
            
            // Send frames with different SSRCs in sequence to see which ones work
            info!("Now testing with custom SSRCs");
            
            // Send a frame with Audio1 SSRC
            info!("Sending frame with AUDIO1_SSRC (0x{:08x})", AUDIO1_SSRC);
            let audio1_frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: b"Audio1 test frame".to_vec(),
                timestamp: 10000,
                sequence: 0,
                marker: true,
                payload_type: 96,
                ssrc: AUDIO1_SSRC,
                csrcs: Vec::new(),
            };
            
            client.send_frame(audio1_frame).await?;
            
            // Wait a bit to allow server to process
            time::sleep(Duration::from_secs(1)).await;
            
            // Send a frame with Video1 SSRC
            info!("Sending frame with VIDEO1_SSRC (0x{:08x})", VIDEO1_SSRC);
            let video1_frame = MediaFrame {
                frame_type: MediaFrameType::Video,
                data: b"Video1 test frame".to_vec(),
                timestamp: 20000,
                sequence: 0,
                marker: true,
                payload_type: 97,
                ssrc: VIDEO1_SSRC,
                csrcs: Vec::new(),
            };
            
            client.send_frame(video1_frame).await?;
            
            // Wait a bit for all frames to be processed
            info!("Waiting for all frames to be processed...");
            time::sleep(Duration::from_secs(5)).await;
            
            // Check SSRCs again after sending
            let ssrcs = client.get_all_ssrcs().await?;
            info!("SSRCs after sending: {:?}", ssrcs);
            
            // Analyze received frames
            let frames = received_frames.lock().await;
            info!("Received {} frames total", frames.len());
            
            // Count frames by SSRC
            let mut ssrc_counts = std::collections::HashMap::new();
            for (client_id, frame) in frames.iter() {
                *ssrc_counts.entry(frame.ssrc).or_insert(0) += 1;
                info!("  Frame: client={}, SSRC={:08x}, data={:?}", 
                     client_id, frame.ssrc, String::from_utf8_lossy(&frame.data));
            }
            
            // Display counts by SSRC
            for (ssrc, count) in ssrc_counts.iter() {
                info!("SSRC={:08x}: {} frames", ssrc, count);
            }
            
            // Disconnect client and stop server
            client.disconnect().await?;
            server.stop().await?;
            
            // Wait a bit for cleanup
            time::sleep(Duration::from_millis(100)).await;
            
            info!("SSRC Demultiplexing API example completed successfully");
            Ok(()) as Result<(), Box<dyn std::error::Error>>
        }
    );
    
    // Handle the timeout result
    match example_timeout.await {
        Ok(result) => {
            info!("Example completed within time limit");
            result
        },
        Err(_) => {
            // Timeout occurred
            info!("Example timed out after {} seconds - forcing termination", EXAMPLE_TIMEOUT_SECS);
            Ok(())
        }
    }
} 