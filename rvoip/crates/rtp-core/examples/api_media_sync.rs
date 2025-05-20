//! Media Synchronization API Example
//!
//! This example demonstrates using the Media Synchronization API
//! to synchronize audio and video streams with the client/server architecture.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{info, debug, warn};

use rvoip_rtp_core::api::{
    client::{
        transport::{MediaTransportClient, MediaSyncInfo},
        config::{ClientConfig, ClientConfigBuilder},
    },
    server::{
        transport::MediaTransportServer,
        config::{ServerConfig, ServerConfigBuilder},
    },
    common::{
        frame::MediaFrame,
        frame::MediaFrameType,
        events::MediaTransportEvent,
    },
};

// Constants for our streams
const AUDIO_SSRC: u32 = 0x1234ABCD;
const VIDEO_SSRC: u32 = 0x5678DCBA;
const AUDIO_CLOCK_RATE: u32 = 48000; // 48kHz audio
const VIDEO_CLOCK_RATE: u32 = 90000; // 90kHz video

// Global timeout to ensure our example completes
const EXAMPLE_TIMEOUT_SECS: u64 = 15;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("Starting Media Synchronization API example");
    
    // Create a timeout for the entire example
    let example_timeout = tokio::time::timeout(
        Duration::from_secs(EXAMPLE_TIMEOUT_SECS),
        async {
            // Configure server
            let server_config = ServerConfigBuilder::new()
                .local_address("127.0.0.1:0".parse().unwrap())
                .rtcp_mux(true)
                .media_sync_enabled(true)
                .security_config(rvoip_rtp_core::api::server::security::ServerSecurityConfig { 
                    security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, 
                    ..Default::default() 
                })
                .build()
                .expect("Failed to build server config");
            
            // Create server
            let server = rvoip_rtp_core::api::server::transport::server_transport_impl::DefaultMediaTransportServer::new(server_config).await?;
            
            // Start server
            server.start().await?;
            
            // Get the server's bound address
            let server_addr = server.get_local_address().await?;
            info!("Server bound to {}", server_addr);
            
            // Configure client
            let client_config = ClientConfigBuilder::new()
                .remote_address(server_addr)
                .rtcp_mux(true)
                .media_sync_enabled(true)
                .security_config(rvoip_rtp_core::api::client::security::ClientSecurityConfig { 
                    security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, 
                    ..Default::default() 
                })
                .build();
            
            // Create client
            let client = rvoip_rtp_core::api::client::transport::client_transport_impl::DefaultMediaTransportClient::new(client_config).await?;
            
            // Connect client to server
            info!("Connecting client to server");
            client.connect().await?;
            
            // Check if media sync is enabled
            let media_sync_enabled = client.is_media_sync_enabled().await?;
            info!("Media synchronization enabled: {}", media_sync_enabled);
            
            // Register audio and video streams for synchronization
            info!("Registering audio and video streams for synchronization");
            client.register_sync_stream(AUDIO_SSRC, AUDIO_CLOCK_RATE).await?;
            client.register_sync_stream(VIDEO_SSRC, VIDEO_CLOCK_RATE).await?;
            
            // Set audio as reference stream (typical for lip sync)
            info!("Setting audio as reference stream");
            client.set_sync_reference_stream(AUDIO_SSRC).await?;
            
            // Exchange some media packets to establish the session
            info!("Exchanging media packets");
            
            // Send audio frames
            for i in 0..5 {
                // Create a simple audio frame
                let frame = MediaFrame {
                    frame_type: MediaFrameType::Audio,
                    data: format!("Audio frame {}", i).into_bytes(),
                    timestamp: i * (AUDIO_CLOCK_RATE / 50), // 20ms intervals
                    sequence: 0, // Will be set by the transport
                    marker: i == 0, // First packet has marker bit
                    payload_type: 96, // Dynamic audio
                    ssrc: AUDIO_SSRC,
                    csrcs: Vec::new(), // Empty CSRC list
                };
                
                // Send frame from client to server
                client.send_frame(frame).await?;
                
                // Wait a bit to allow server to process
                time::sleep(Duration::from_millis(10)).await;
            }
            
            // Send video frames with an offset (simulating potential sync issues)
            for i in 0..5 {
                // Create a simple video frame with a 100ms offset
                let frame = MediaFrame {
                    frame_type: MediaFrameType::Video,
                    data: format!("Video frame {}", i).into_bytes(),
                    timestamp: i * (VIDEO_CLOCK_RATE / 30) + VIDEO_CLOCK_RATE / 10, // 33ms intervals with +100ms offset
                    sequence: 0, // Will be set by the transport
                    marker: i == 0, // First packet has marker bit
                    payload_type: 97, // Dynamic video
                    ssrc: VIDEO_SSRC,
                    csrcs: Vec::new(), // Empty CSRC list
                };
                
                // Send frame from client to server
                client.send_frame(frame).await?;
                
                // Wait a bit to allow server to process
                time::sleep(Duration::from_millis(10)).await;
            }
            
            // Wait a bit for RTP transmission to stabilize
            time::sleep(Duration::from_millis(50)).await;
            
            // Send RTCP Sender Reports to establish timing relationship
            info!("Sending RTCP Sender Reports to establish timing relationship");
            
            // Send sender reports
            client.send_rtcp_sender_report().await?;
            
            // Wait a bit for server to process
            time::sleep(Duration::from_millis(200)).await;
            
            // Send another round of sender reports after some time has passed
            // to establish drift patterns
            info!("Sending second round of RTCP Sender Reports after delay");
            
            // Wait a bit to simulate time passing
            time::sleep(Duration::from_secs(2)).await;
            
            // Send another sender report
            client.send_rtcp_sender_report().await?;
            
            // Wait a bit for server to process
            time::sleep(Duration::from_millis(200)).await;
            
            // Get sync information for audio stream
            info!("Retrieving synchronization information");
            if let Some(audio_info) = client.get_sync_info(AUDIO_SSRC).await? {
                info!("Audio stream sync info:");
                info!("  SSRC: {:08x}", audio_info.ssrc);
                info!("  Clock rate: {} Hz", audio_info.clock_rate);
                info!("  Last RTP timestamp: {:?}", audio_info.last_rtp);
                info!("  Last NTP timestamp: {:?}", audio_info.last_ntp);
                info!("  Clock drift: {:.2} PPM", audio_info.clock_drift_ppm);
            } else {
                warn!("No synchronization info available for audio stream");
            }
            
            if let Some(video_info) = client.get_sync_info(VIDEO_SSRC).await? {
                info!("Video stream sync info:");
                info!("  SSRC: {:08x}", video_info.ssrc);
                info!("  Clock rate: {} Hz", video_info.clock_rate);
                info!("  Last RTP timestamp: {:?}", video_info.last_rtp);
                info!("  Last NTP timestamp: {:?}", video_info.last_ntp);
                info!("  Clock drift: {:.2} PPM", video_info.clock_drift_ppm);
            } else {
                warn!("No synchronization info available for video stream");
            }
            
            // Demonstrate timestamp conversion
            info!("Demonstrating timestamp conversion:");
            let audio_ts = AUDIO_CLOCK_RATE * 2; // 2 seconds in
            if let Some(video_ts) = client.convert_timestamp(AUDIO_SSRC, VIDEO_SSRC, audio_ts).await? {
                info!("Audio timestamp {} maps to video timestamp {}", audio_ts, video_ts);
                info!("  Video time: {:.2}s", video_ts as f64 / VIDEO_CLOCK_RATE as f64);
            } else {
                warn!("Failed to convert audio timestamp to video timestamp");
            }
            
            // Check if streams are synchronized
            let sync_status = client.are_streams_synchronized(AUDIO_SSRC, VIDEO_SSRC, 50.0).await?;
            info!("Streams synchronized within 50ms tolerance: {}", sync_status);
            
            // Get all sync info
            let all_info = client.get_all_sync_info().await?;
            info!("Number of registered streams: {}", all_info.len());
            
            // Disconnect client
            client.disconnect().await?;
            
            // Stop server
            server.stop().await?;
            
            // Short delay before returning to ensure everything is cleaned up
            time::sleep(Duration::from_millis(100)).await;
            
            info!("Media Synchronization API example completed successfully");
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