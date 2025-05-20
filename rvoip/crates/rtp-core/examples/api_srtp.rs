//! RTP API Example with SRTP security
//!
//! This example demonstrates the RTP API usage pattern with SRTP encryption.

use rvoip_rtp_core::{
    api::{
        client::transport::MediaTransportClient,
        client::config::ClientConfigBuilder,
        server::transport::MediaTransportServer,
        server::config::ServerConfigBuilder,
        common::frame::{MediaFrame, MediaFrameType},
        common::config::SrtpProfile,
        MediaTransportError,
    },
    srtp::{SrtpCryptoKey, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80},
};

use std::time::Duration;
use tokio::time;
use tracing::{info, debug, warn, error};
use std::process;
use std::net::SocketAddr;
use std::str::FromStr;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Set example timeout to force termination
const MAX_RUNTIME_SECONDS: u64 = 10;
const FORCE_KILL_AFTER_SECONDS: u64 = 5;
const CONNECT_TIMEOUT_SECONDS: u64 = 2;

// Simple custom error type for the example
#[derive(Debug)]
struct ExampleError(String);

impl fmt::Display for ExampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ExampleError {}

impl From<MediaTransportError> for ExampleError {
    fn from(err: MediaTransportError) -> Self {
        ExampleError(err.to_string())
    }
}

impl From<std::io::Error> for ExampleError {
    fn from(err: std::io::Error) -> Self {
        ExampleError(err.to_string())
    }
}

#[tokio::main]
async fn main() -> Result<(), ExampleError> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    // Shared flag for graceful shutdown
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown_requested.clone();
    
    // Set a top-level timeout to ensure the program terminates no matter what
    std::thread::spawn(move || {
        // Create a clone for the thread
        let thread_shutdown = shutdown_clone.clone();
        
        // Wait for the maximum runtime
        std::thread::sleep(Duration::from_secs(MAX_RUNTIME_SECONDS));
        
        // Signal graceful shutdown
        info!("Maximum runtime reached - requesting graceful shutdown");
        thread_shutdown.store(true, Ordering::SeqCst);
        
        // Wait for FORCE_KILL_AFTER_SECONDS for graceful shutdown to complete
        std::thread::sleep(Duration::from_secs(FORCE_KILL_AFTER_SECONDS));
        
        // Force exit if still running
        eprintln!("Graceful shutdown timed out - terminating process");
        process::exit(1);
    });
    
    info!("RTP API Example with SRTP");
    info!("========================");
    
    // Create SRTP key for secure communication
    // Example key (16 bytes for AES-128) and salt (14 bytes)
    let key_data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 
                     0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
    
    let salt_data = vec![0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 
                     0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E];
    
    // Key information for SDP in base64 format
    let mut combined = Vec::with_capacity(key_data.len() + salt_data.len());
    combined.extend_from_slice(&key_data);
    combined.extend_from_slice(&salt_data);
    let base64_key = base64::encode(&combined);
    
    // Display key information for demonstration purposes
    info!("Using SRTP crypto suite: AES_CM_128_HMAC_SHA1_80");
    info!("SRTP key+salt (base64): {}", base64_key);
    info!("SDP crypto line: 1 AES_CM_128_HMAC_SHA1_80 inline:{}", base64_key);
        
    // Server setup
    info!("Setting up server with SRTP...");
    let local_addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
    let server_config = ServerConfigBuilder::new()
        .local_address(local_addr)
        .with_srtp_key(key_data.clone())
        .build()?;
    
    let server = rvoip_rtp_core::api::create_server(server_config).await?;
    
    // Start server
    info!("Starting server...");
    server.start().await?;
    
    // Get server address
    let server_addr = server.get_local_address().await?;
    info!("Server listening on {}", server_addr);
    
    // Client setup with the same SRTP config for simplicity
    // (in real-world, keys would be exchanged through signaling)
    info!("Setting up client with SRTP...");
    let client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
        .with_srtp_key(key_data)
        .build();
    
    let client = rvoip_rtp_core::api::create_client(client_config).await?;
    
    // Connect client to server with timeout
    info!("Connecting client to server...");
    match time::timeout(Duration::from_secs(CONNECT_TIMEOUT_SECONDS), client.connect()).await {
        Ok(result) => {
            match result {
                Ok(_) => info!("Client connected successfully"),
                Err(e) => {
                    error!("Client connection failed: {}", e);
                    return Err(ExampleError(format!("Client connection failed: {}", e)));
                }
            }
        },
        Err(_) => {
            error!("Client connection timed out after {} seconds", CONNECT_TIMEOUT_SECONDS);
            // Continue with the example even though connection may not be fully established
            info!("Continuing with example despite connection issues");
        }
    }
    
    // Launch a server receive task
    let server_clone = server.clone();
    let server_shutdown = shutdown_requested.clone();
    let _server_task = tokio::spawn(async move {
        loop {
            // Check for shutdown signal
            if server_shutdown.load(Ordering::SeqCst) {
                info!("Server receive task shutting down");
                break;
            }
            
            match time::timeout(Duration::from_millis(500), server_clone.receive_frame()).await {
                Ok(Ok((client_id, frame))) => {
                    info!("Server received from {}: {} bytes of type {:?}", 
                          client_id, frame.data.len(), frame.frame_type);
                    
                    // Display first few bytes of payload data (for verification)
                    let preview: String = frame.data.iter().take(8)
                        .map(|b| format!("{:02x}", b))
                        .collect::<Vec<String>>()
                        .join(" ");
                    info!("Frame data preview: {}", preview);
                },
                Ok(Err(e)) => {
                    error!("Server receive error: {}", e);
                    break;
                },
                Err(_) => {
                    debug!("Server receive timed out, continuing");
                }
            }
        }
    });
    
    // Send test frames from client to server
    info!("Sending encrypted test frames...");
    for i in 0..5 {
        // Check for shutdown signal
        if shutdown_requested.load(Ordering::SeqCst) {
            info!("Client sending task shutting down");
            break;
        }
        
        let test_data = format!("Secure test frame {}", i);
        let frame = MediaFrame {
            frame_type: MediaFrameType::Audio,
            data: test_data.clone().into_bytes(),
            timestamp: i * 160, // 20ms at 8kHz
            sequence: i as u16,
            marker: i == 0,
            payload_type: 0, // PCMU
            ssrc: 0x1234ABCD,
            csrcs: Vec::new(),
        };
        
        // Log the original data for comparison
        let original_preview: String = frame.data.iter().take(8)
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<String>>()
            .join(" ");
        info!("Sending frame {}: '{}' (preview: {})", i, test_data, original_preview);
        
        match time::timeout(Duration::from_millis(500), client.send_frame(frame)).await {
            Ok(Ok(_)) => info!("Client sent encrypted frame {}", i),
            Ok(Err(e)) => warn!("Failed to send frame {}: {}", i, e),
            Err(_) => warn!("Sending frame {} timed out", i),
        }
        
        // Wait a bit between frames
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    
    // Wait for frames to be processed
    info!("Waiting for encrypted frames to be processed...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Clean up
    info!("Cleaning up...");
    
    // Try to disconnect client
    match time::timeout(Duration::from_millis(500), client.disconnect()).await {
        Ok(Ok(_)) => info!("Client disconnected successfully"),
        Ok(Err(e)) => warn!("Client disconnect error: {}", e),
        Err(_) => warn!("Client disconnect timed out"),
    }
    
    // Try to stop server
    match time::timeout(Duration::from_millis(500), server.stop()).await {
        Ok(Ok(_)) => info!("Server stopped successfully"),
        Ok(Err(e)) => warn!("Server stop error: {}", e),
        Err(_) => warn!("Server stop timed out"),
    }
    
    // Added small delay to ensure cleanup completes
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    info!("SRTP Example completed successfully");
    
    Ok(())
} 