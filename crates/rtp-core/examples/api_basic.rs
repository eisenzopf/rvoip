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
        common::config::SecurityMode,
        client::security::ClientSecurityConfig,
        server::security::ServerSecurityConfig,
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
const MAX_RUNTIME_SECONDS: u64 = 15;
const FORCE_KILL_AFTER_SECONDS: u64 = 5;
const CONNECT_TIMEOUT_SECONDS: u64 = 5;
const CONNECTION_RETRY_COUNT: u32 = 3;

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
        tracing::error!("Emergency timeout triggered - terminating process");
        process::exit(1);
    });
    
    info!("RTP API Example");
    info!("==============");
    
    // Server setup
    info!("Setting up server...");
    let local_addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
    
    // Create server config with security explicitly disabled
    let mut server_config = ServerConfigBuilder::new()
        .local_address(local_addr)
        .build()?;
    
    // Disable security for server
    let mut server_security_config = ServerSecurityConfig::default();
    server_security_config.security_mode = SecurityMode::None;
    server_config.security_config = server_security_config;
    
    let server = rvoip_rtp_core::api::create_server(server_config).await?;
    
    // Start server
    info!("Starting server...");
    server.start().await?;
    
    // Get server address
    let server_addr = server.get_local_address().await?;
    info!("Server listening on {}", server_addr);
    
    // Wait a short time for server to fully initialize
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Client setup
    info!("Setting up client...");
    
    // Create client config with security explicitly disabled
    let mut client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
        .build();
    
    // Disable security for client
    let mut client_security_config = ClientSecurityConfig::default();
    client_security_config.security_mode = SecurityMode::None;
    client_config.security_config = client_security_config;
    
    let client = rvoip_rtp_core::api::create_client(client_config).await?;
    
    // Connect client to server with retries and improved timeout
    info!("Connecting client to server...");
    let mut connected = false;
    let mut retry_count = 0;
    
    while !connected && retry_count < CONNECTION_RETRY_COUNT {
        match time::timeout(Duration::from_secs(CONNECT_TIMEOUT_SECONDS), client.connect()).await {
            Ok(result) => {
                match result {
                    Ok(_) => {
                        info!("Client connected successfully");
                        connected = true;
                    },
                    Err(e) => {
                        warn!("Client connection attempt {} failed: {}", retry_count + 1, e);
                        retry_count += 1;
                        if retry_count < CONNECTION_RETRY_COUNT {
                            info!("Retrying connection in 500ms...");
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
            },
            Err(_) => {
                warn!("Client connection attempt {} timed out after {} seconds", 
                     retry_count + 1, CONNECT_TIMEOUT_SECONDS);
                retry_count += 1;
                if retry_count < CONNECTION_RETRY_COUNT {
                    info!("Retrying connection in 500ms...");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    
    if !connected {
        error!("Failed to connect after {} attempts", CONNECTION_RETRY_COUNT);
        return Err(ExampleError("Connection failed after multiple attempts".to_string()));
    }
    
    // Verify connection is established
    info!("Verifying connection status...");
    match time::timeout(Duration::from_secs(1), client.is_connected()).await {
        Ok(Ok(is_connected)) => {
            if is_connected {
                info!("Connection verified");
            } else {
                error!("Connection verification failed: client reports as disconnected");
                return Err(ExampleError("Connection verification failed".to_string()));
            }
        },
        Ok(Err(e)) => {
            error!("Connection verification error: {}", e);
            return Err(ExampleError(format!("Connection verification error: {}", e)));
        },
        Err(_) => {
            error!("Connection verification timed out");
            return Err(ExampleError("Connection verification timed out".to_string()));
        }
    }
    
    // Launch a server receive task before sending frames
    let server_clone = server.clone();
    let server_ready = std::sync::Arc::new(tokio::sync::Notify::new());
    let server_ready_clone = server_ready.clone();
    
    let _server_task = tokio::spawn(async move {
        info!("Server receiver task started");
        server_ready_clone.notify_one(); // Signal that the server is ready to receive
        
        let mut consecutive_errors = 0;
        let max_consecutive_errors = 5;
        
        loop {
            match time::timeout(Duration::from_secs(1), server_clone.receive_frame()).await {
                Ok(Ok((client_id, frame))) => {
                    info!("Server received from {}: {} bytes of type {:?}", 
                          client_id, frame.data.len(), frame.frame_type);
                    
                    if let Ok(text) = std::str::from_utf8(&frame.data) {
                        info!("Frame data: {}", text);
                    }
                    
                    consecutive_errors = 0; // Reset error counter on success
                },
                Ok(Err(e)) => {
                    if e.to_string().contains("Timeout") {
                        debug!("Server receive timeout, continuing");
                    } else {
                        error!("Server receive error: {}", e);
                        consecutive_errors += 1;
                    }
                },
                Err(_) => {
                    debug!("Server receive timed out, continuing");
                }
            }
            
            // Break the loop if too many consecutive errors
            if consecutive_errors >= max_consecutive_errors {
                error!("Too many consecutive errors, stopping server receiver");
                break;
            }
        }
    });
    
    // Wait for server receive task to be ready
    info!("Waiting for server receiver to be ready...");
    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    
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
    
    // Send with timeout and retry
    let mut ping_sent = false;
    for attempt in 0..3 {
        match time::timeout(Duration::from_secs(1), client.send_frame(test_frame.clone())).await {
            Ok(result) => {
                match result {
                    Ok(_) => {
                        info!("PING frame sent successfully");
                        ping_sent = true;
                        break;
                    },
                    Err(e) => warn!("Failed to send PING frame (attempt {}): {}", attempt + 1, e),
                }
            },
            Err(_) => warn!("Sending PING frame timed out (attempt {})", attempt + 1),
        }
        
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    
    if !ping_sent {
        error!("Failed to send PING frame after 3 attempts");
    }
    
    // Send test frames from client to server
    info!("Sending test frames...");
    let mut frames_sent = 0;
    
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
        
        // Send with timeout and retry logic
        let mut sent = false;
        for attempt in 0..2 {
            match time::timeout(Duration::from_millis(500), client.send_frame(frame.clone())).await {
                Ok(Ok(_)) => {
                    info!("Client sent frame {} successfully", i);
                    frames_sent += 1;
                    sent = true;
                    break;
                },
                Ok(Err(e)) => warn!("Failed to send frame {} (attempt {}): {}", i, attempt + 1, e),
                Err(_) => warn!("Sending frame {} timed out (attempt {})", i, attempt + 1),
            }
            
            if !sent && attempt < 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        
        // Wait a bit between frames
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    info!("Successfully sent {}/5 frames", frames_sent);
    
    // Wait for frames to be processed
    info!("Waiting for frames to be processed...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Clean up
    info!("Cleaning up...");
    
    // Try to disconnect client with retry
    let mut disconnected = false;
    for attempt in 0..3 {
        match time::timeout(Duration::from_millis(500), client.disconnect()).await {
            Ok(Ok(_)) => {
                info!("Client disconnected successfully");
                disconnected = true;
                break;
            },
            Ok(Err(e)) => warn!("Client disconnect error (attempt {}): {}", attempt + 1, e),
            Err(_) => warn!("Client disconnect timed out (attempt {})", attempt + 1),
        }
        
        if !disconnected && attempt < 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    
    // Try to stop server with retry
    let mut server_stopped = false;
    for attempt in 0..3 {
        match time::timeout(Duration::from_millis(500), server.stop()).await {
            Ok(Ok(_)) => {
                info!("Server stopped successfully");
                server_stopped = true;
                break;
            },
            Ok(Err(e)) => warn!("Server stop error (attempt {}): {}", attempt + 1, e),
            Err(_) => warn!("Server stop timed out (attempt {})", attempt + 1),
        }
        
        if !server_stopped && attempt < 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    
    // Added delay to ensure cleanup completes
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    info!("Example completed successfully");
    
    Ok(())
} 