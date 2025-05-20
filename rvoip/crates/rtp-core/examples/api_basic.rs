//! RTP API Example (without security)
//!
//! This example demonstrates the basic RTP API usage pattern.

use rvoip_rtp_core::{
    api::{
        client::transport::MediaTransportClient,
        client::config::ClientConfigBuilder,
        server::transport::MediaTransportServer,
        server::config::ServerConfigBuilder,
        common::frame::{MediaFrame, MediaFrameType},
        MediaTransportError,
    },
};

use std::time::Duration;
use tokio::time;
use tracing::{info, debug, warn, error};
use std::process;
use std::net::SocketAddr;
use std::str::FromStr;
use std::fmt;

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
    
    // Set a top-level timeout to ensure the program terminates no matter what
    std::thread::spawn(|| {
        // Total maximum runtime including any cleanup
        std::thread::sleep(Duration::from_secs(MAX_RUNTIME_SECONDS + FORCE_KILL_AFTER_SECONDS));
        eprintln!("Emergency timeout triggered - terminating process");
        process::exit(1);
    });
    
    info!("RTP API Example");
    info!("==============");
    
    // Server setup
    info!("Setting up server...");
    let local_addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
    let server_config = ServerConfigBuilder::new()
        .local_address(local_addr)
        .build()?;
    
    let server = rvoip_rtp_core::api::create_server(server_config).await?;
    
    // Start server
    info!("Starting server...");
    server.start().await?;
    
    // Get server address
    let server_addr = server.get_local_address().await?;
    info!("Server listening on {}", server_addr);
    
    // Client setup
    info!("Setting up client...");
    let client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
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
    
    // Try simple Ping/Pong to validate connection
    info!("Testing connection with a simple frame...");
    let test_frame = MediaFrame {
        frame_type: MediaFrameType::Audio, // Using Audio type since Control is not available
        data: "PING".as_bytes().to_vec(),
        timestamp: 0,
        sequence: 0,
        marker: true,
        payload_type: 0,
        ssrc: 0x1234ABCD,
        csrcs: Vec::new(),
    };
    
    // Send with timeout
    match time::timeout(Duration::from_secs(1), client.send_frame(test_frame)).await {
        Ok(result) => {
            match result {
                Ok(_) => info!("PING frame sent successfully"),
                Err(e) => warn!("Failed to send PING frame: {}", e),
            }
        },
        Err(_) => warn!("Sending PING frame timed out"),
    }
    
    // Launch a server receive task
    let server_clone = server.clone();
    let _server_task = tokio::spawn(async move {
        loop {
            match time::timeout(Duration::from_secs(1), server_clone.receive_frame()).await {
                Ok(Ok((client_id, frame))) => {
                    info!("Server received from {}: {} bytes of type {:?}", 
                          client_id, frame.data.len(), frame.frame_type);
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
    info!("Sending test frames...");
    for i in 0..5 {
        let frame = MediaFrame {
            frame_type: MediaFrameType::Audio,
            data: format!("Test frame {}", i).into_bytes(),
            timestamp: i * 160, // 20ms at 8kHz
            sequence: i as u16,
            marker: i == 0,
            payload_type: 0, // PCMU
            ssrc: 0x1234ABCD,
            csrcs: Vec::new(),
        };
        
        match time::timeout(Duration::from_millis(500), client.send_frame(frame)).await {
            Ok(Ok(_)) => info!("Client sent frame {}", i),
            Ok(Err(e)) => warn!("Failed to send frame {}: {}", i, e),
            Err(_) => warn!("Sending frame {} timed out", i),
        }
        
        // Wait a bit between frames
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for frames to be processed
    info!("Waiting for frames to be processed...");
    tokio::time::sleep(Duration::from_secs(1)).await;
    
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
    
    info!("Example completed successfully");
    
    Ok(())
} 