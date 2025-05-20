use std::time::Duration;
use tokio::time::sleep;
use tokio::time::timeout;

use rvoip_rtp_core::api::client::{MediaTransportClient, ClientConfigBuilder, ClientFactory};
use rvoip_rtp_core::api::server::{MediaTransportServer, ServerConfigBuilder, ServerFactory};
use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::events::{MediaTransportEvent, MediaEventCallback};
use rvoip_rtp_core::api::common::config::SecurityMode;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing (uncomment if tracing_subscriber is available)
    // tracing_subscriber::fmt::init();
    
    println!("Starting RTP client-server example with DTLS-SRTP security...");
    
    // Create server security config with DTLS-SRTP enabled
    let server_security_config = ServerSecurityConfig {
        security_mode: SecurityMode::DtlsSrtp,
        fingerprint_algorithm: "sha-256".to_string(),
        srtp_profiles: vec![
            rvoip_rtp_core::api::common::config::SrtpProfile::AesCm128HmacSha1_80,
            rvoip_rtp_core::api::common::config::SrtpProfile::AesGcm128,
        ],
        certificate_path: Some("server-cert.pem".to_string()),
        private_key_path: Some("server-key.pem".to_string()),
        require_client_certificate: false,
    };
    
    // Create a server config with security enabled
    let server_config_builder = ServerConfigBuilder::new()
        .local_address("127.0.0.1:9000".parse().unwrap())
        .default_payload_type(8) // G.711 A-law
        .clock_rate(8000)
        .security_config(server_security_config);
    
    let server_config = server_config_builder.build()?;
    
    // Create client security config with DTLS-SRTP enabled
    let client_security_config = ClientSecurityConfig {
        security_mode: SecurityMode::DtlsSrtp,
        fingerprint_algorithm: "sha-256".to_string(),
        srtp_profiles: vec![
            rvoip_rtp_core::api::common::config::SrtpProfile::AesCm128HmacSha1_80,
            rvoip_rtp_core::api::common::config::SrtpProfile::AesGcm128,
        ],
        remote_fingerprint: None,
        remote_fingerprint_algorithm: None,
        validate_fingerprint: false,
        certificate_path: Some("client-cert.pem".to_string()),
        private_key_path: Some("client-key.pem".to_string()),
    };
    
    // Create a client config connecting to the server
    let client_config_builder = ClientConfigBuilder::new()
        .remote_address("127.0.0.1:9000".parse().unwrap())
        .default_payload_type(8) // G.711 A-law
        .clock_rate(8000)
        .security_config(client_security_config);
    
    let client_config = client_config_builder.build();
    
    println!("Creating server and client with secure connection...");
    
    // Create server with timeout
    println!("Creating server...");
    let server = match timeout(DEFAULT_TIMEOUT, ServerFactory::create_server(server_config)).await {
        Ok(result) => {
            println!("Server created successfully");
            result?
        },
        Err(_) => {
            println!("Timeout while creating server");
            return Ok(());
        }
    };
    
    // Create client with timeout
    println!("Creating client...");
    let client = match timeout(DEFAULT_TIMEOUT, ClientFactory::create_client(client_config)).await {
        Ok(result) => {
            println!("Client created successfully");
            result?
        },
        Err(_) => {
            println!("Timeout while creating client");
            return Ok(());
        }
    };
    
    // Start server with timeout
    println!("Starting server...");
    match timeout(DEFAULT_TIMEOUT, server.start()).await {
        Ok(result) => {
            match result {
                Ok(_) => println!("Server started successfully"),
                Err(e) => {
                    println!("Error starting server: {}", e);
                    return Ok(());
                }
            }
        },
        Err(_) => {
            println!("Timeout while starting server");
            return Ok(());
        }
    }
    
    // Get the actual server address after binding
    let actual_server_addr = match server.get_local_address().await {
        Ok(addr) => {
            println!("Server bound to: {}", addr);
            addr
        },
        Err(e) => {
            println!("Failed to get server address: {}", e);
            return Ok(());
        }
    };
    
    // Connect client with timeout
    println!("Connecting client...");
    match timeout(DEFAULT_TIMEOUT, client.connect()).await {
        Ok(result) => {
            match result {
                Ok(_) => println!("Client connected successfully"),
                Err(e) => {
                    println!("Error connecting client: {}", e);
                    return Ok(());
                }
            }
        },
        Err(_) => {
            println!("Timeout while connecting client");
            return Ok(());
        }
    }
    
    println!("Waiting for secure connection establishment...");
    sleep(Duration::from_secs(2)).await;  // Increased wait time for DTLS handshake
    
    // Send test frame with timeout
    println!("Sending test frame from client...");
    let test_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![1, 2, 3, 4, 5],
        timestamp: 1000,
        sequence: 1,
        marker: true,
        payload_type: 8,
        ssrc: 12345,
    };
    
    match timeout(DEFAULT_TIMEOUT, client.send_frame(test_frame.clone())).await {
        Ok(result) => {
            match result {
                Ok(_) => println!("Frame sent successfully through secure connection"),
                Err(e) => println!("Error sending frame: {}", e),
            }
        },
        Err(_) => println!("Timeout while sending frame"),
    }
    
    // Short delay to allow processing
    sleep(Duration::from_millis(500)).await;
    
    // Disconnect client with timeout
    println!("Disconnecting client...");
    match timeout(DEFAULT_TIMEOUT, client.disconnect()).await {
        Ok(result) => {
            match result {
                Ok(_) => println!("Client disconnected successfully"),
                Err(e) => println!("Error disconnecting client: {}", e),
            }
        },
        Err(_) => println!("Timeout while disconnecting client"),
    }
    
    // Stop server with timeout
    println!("Stopping server...");
    match timeout(DEFAULT_TIMEOUT, server.stop()).await {
        Ok(result) => {
            match result {
                Ok(_) => println!("Server stopped successfully"),
                Err(e) => println!("Error stopping server: {}", e),
            }
        },
        Err(_) => println!("Timeout while stopping server"),
    }
    
    println!("Secure client-server example completed successfully");
    Ok(())
} 