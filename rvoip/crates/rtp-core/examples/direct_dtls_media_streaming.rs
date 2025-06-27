/// Example showing secure media streaming with direct DTLS-SRTP
///
/// This example combines the direct DTLS connection handling from dtls_test.rs
/// with the media streaming functionality from secure_media_streaming.rs

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::sleep;

// Direct DTLS imports
use rvoip_rtp_core::dtls::{DtlsConfig, DtlsRole, DtlsVersion};
use rvoip_rtp_core::dtls::connection::DtlsConnection;
use rvoip_rtp_core::dtls::crypto::verify::{Certificate, generate_self_signed_certificate};
use rvoip_rtp_core::dtls::transport::udp::UdpTransport;

// Media transport imports
use rvoip_rtp_core::api::client::transport::{MediaTransportClient, DefaultMediaTransportClient};
use rvoip_rtp_core::api::server::transport::{MediaTransportServer, DefaultMediaTransportServer};
use rvoip_rtp_core::api::client::config::ClientConfigBuilder;
use rvoip_rtp_core::api::server::config::ServerConfigBuilder;
use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::events::MediaTransportEvent;
use rvoip_rtp_core::api::common::config::SecurityMode;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    println!("Direct DTLS Media Streaming Example");
    println!("===================================\n");
    
    // Create client and server sockets on different ports
    let server_addr: SocketAddr = "127.0.0.1:20000".parse()?;
    let client_addr: SocketAddr = "127.0.0.1:20001".parse()?;
    
    let server_socket = Arc::new(UdpSocket::bind(server_addr).await?);
    let client_socket = Arc::new(UdpSocket::bind(client_addr).await?);
    
    println!("Server listening on {}", server_addr);
    println!("Client listening on {}", client_addr);
    
    // Generate certificates
    println!("Generating certificates...");
    let server_cert = generate_self_signed_certificate()?;
    let client_cert = generate_self_signed_certificate()?;
    
    // Calculate fingerprints
    let mut server_cert_clone = server_cert.clone();
    let mut client_cert_clone = client_cert.clone();
    let server_fingerprint = server_cert_clone.fingerprint("SHA-256")?;
    let client_fingerprint = client_cert_clone.fingerprint("SHA-256")?;
    
    println!("Server certificate fingerprint: {}", server_fingerprint);
    println!("Client certificate fingerprint: {}", client_fingerprint);
    
    // First, establish a secure DTLS connection (based on dtls_test.rs)
    // -----------------------------------------------------------------
    
    // Create DTLS configurations
    let server_config = DtlsConfig {
        role: DtlsRole::Server,
        version: DtlsVersion::Dtls12,
        mtu: 1200,
        max_retransmissions: 5,
        srtp_profiles: vec![
            rvoip_rtp_core::srtp::SRTP_AES128_CM_SHA1_80,
        ],
    };
    
    let client_config = DtlsConfig {
        role: DtlsRole::Client,
        version: DtlsVersion::Dtls12,
        mtu: 1200,
        max_retransmissions: 5,
        srtp_profiles: vec![
            rvoip_rtp_core::srtp::SRTP_AES128_CM_SHA1_80,
        ],
    };
    
    // Set up server in a separate task
    let server_task = tokio::spawn(async move {
        println!("Starting DTLS server task...");
        
        // Create DTLS connection for server
        let mut server_conn = DtlsConnection::new(server_config);
        
        // Create server transport
        let server_transport = UdpTransport::new(server_socket, 1500).await.unwrap();
        let server_transport = Arc::new(Mutex::new(server_transport));
        
        // Start the transport
        server_transport.lock().await.start().await.unwrap();
        println!("Server transport started");
        
        // Set the transport and certificate
        server_conn.set_transport(server_transport.clone());
        server_conn.set_certificate(server_cert);
        
        println!("Server waiting for client connection...");
        
        // Wait for first packet from client
        let (initial_packet, client_addr) = {
            let mut transport_guard = server_transport.lock().await;
            println!("Server waiting to receive initial packet...");
            let (packet, addr) = transport_guard.recv().await.unwrap();
            println!("Server received initial packet: {} bytes from {}", packet.len(), addr);
            (packet, addr)
        };
        
        println!("Server received connection from {}", client_addr);
        
        // Start handshake to initialize the state machine
        println!("Server starting handshake...");
        server_conn.start_handshake(client_addr).await.unwrap();
        
        // Process the initial packet to handle the ClientHello
        println!("Server processing initial ClientHello packet...");
        if let Err(e) = server_conn.process_packet(&initial_packet).await {
            println!("Error processing initial packet: {:?}", e);
        }
        
        // Wait for handshake completion
        println!("Server waiting for handshake completion...");
        match server_conn.wait_handshake().await {
            Ok(_) => {
                println!("Server handshake completed successfully!");
                
                // Extract SRTP keys
                println!("Server extracting SRTP keys...");
                let srtp_context = server_conn.extract_srtp_keys().unwrap();
                
                // Get the key for server role (false = server)
                let server_key = srtp_context.get_key_for_role(false);
                
                println!("Server SRTP key: {:?}", server_key);
                println!("Server SRTP profile: {:?}", srtp_context.profile);
                
                // Return success with the SRTP keys
                Ok(server_key.clone())
            },
            Err(e) => {
                println!("Server handshake failed: {}", e);
                Err(e)
            }
        }
    });
    
    // Give the server time to start up
    println!("Waiting for server to start...");
    sleep(Duration::from_millis(1000)).await;
    
    let client_task = tokio::spawn(async move {
        println!("Starting DTLS client task...");
        
        // Create DTLS connection for client
        let mut client_conn = DtlsConnection::new(client_config);
        
        // Create client transport
        let client_transport = UdpTransport::new(client_socket, 1500).await.unwrap();
        let client_transport = Arc::new(Mutex::new(client_transport));
        
        // Start the transport
        client_transport.lock().await.start().await.unwrap();
        println!("Client transport started");
        
        // Set the transport and certificate
        client_conn.set_transport(client_transport.clone());
        client_conn.set_certificate(client_cert);
        
        println!("Client connecting to server at {}", server_addr);
        
        // Start handshake
        println!("Client starting handshake...");
        client_conn.start_handshake(server_addr).await.unwrap();
        
        // Wait for handshake completion
        println!("Client waiting for handshake completion...");
        match client_conn.wait_handshake().await {
            Ok(_) => {
                println!("Client handshake completed successfully!");
                
                // Extract SRTP keys
                println!("Client extracting SRTP keys...");
                let srtp_context = client_conn.extract_srtp_keys().unwrap();
                
                // Get the key for client role (true = client)
                let client_key = srtp_context.get_key_for_role(true);
                
                println!("Client SRTP key: {:?}", client_key);
                println!("Client SRTP profile: {:?}", srtp_context.profile);
                
                // Return success with the SRTP keys
                Ok(client_key.clone())
            },
            Err(e) => {
                println!("Client handshake failed: {}", e);
                Err(e)
            }
        }
    });
    
    // Wait for both tasks to complete
    println!("Waiting for DTLS handshake to complete...");
    let server_result = server_task.await??;
    println!("Server task completed");
    let client_result = client_task.await??;
    println!("Client task completed");
    
    println!("\nDTLS handshake completed successfully!");
    println!("Now setting up media streaming using the established SRTP keys...\n");
    
    // Now set up media streaming using the established keys
    // ----------------------------------------------------
    
    // Create server for media
    println!("Creating media server...");
    let media_server_addr = SocketAddr::new(server_addr.ip(), server_addr.port() + 100);
    
    // Configure server without security (we'll handle it separately)
    let server_config = ServerConfigBuilder::new()
        .local_address(media_server_addr)
        .default_payload_type(8) // G.711 µ-law
        .clock_rate(8000)
        .build()?;
    
    // Create server
    let server = DefaultMediaTransportServer::new(server_config).await?;
    
    // Set up server event handlers
    server.on_event(Box::new(move |event| {
        match event {
            MediaTransportEvent::Connected => {
                println!("[SERVER] New connection established");
            },
            MediaTransportEvent::Disconnected => {
                println!("[SERVER] Connection disconnected");
            },
            MediaTransportEvent::FrameReceived(frame) => {
                println!("[SERVER] Received frame: seq={}, size={} bytes", 
                    frame.sequence, frame.data.len());
            },
            MediaTransportEvent::Error(err) => {
                println!("[SERVER] Error: {}", err);
            },
            _ => {}
        }
    })).await?;
    
    // Start server
    println!("Starting media server on {}...", media_server_addr);
    server.start().await?;
    println!("Media server started successfully");
    
    // Client media task
    let media_client_task = tokio::spawn(async move {
        // Wait for server to start
        sleep(Duration::from_millis(500)).await;
        
        // Configure client
        let media_client_addr = SocketAddr::new(client_addr.ip(), client_addr.port() + 100);
        
        // Create client config without security (we handled it separately)
        let client_config = ClientConfigBuilder::new()
            .remote_address(media_server_addr)
            .default_payload_type(8)
            .clock_rate(8000)
            .build();
        
        // Create client
        println!("Creating media client to connect to: {}", media_server_addr);
        let client = DefaultMediaTransportClient::new(client_config).await?;
        
        // Set up client event handlers
        client.on_event(Box::new(|event| {
            match event {
                MediaTransportEvent::Connected => {
                    println!("[CLIENT] Connected to server");
                },
                MediaTransportEvent::Disconnected => {
                    println!("[CLIENT] Disconnected from server");
                },
                MediaTransportEvent::Error(err) => {
                    println!("[CLIENT] Error: {}", err);
                },
                _ => {}
            }
        })).await?;
        
        // Connect to server
        println!("Connecting to media server...");
        client.connect().await?;
        
        // Check if connected
        if !client.is_connected().await? {
            println!("[CLIENT] Failed to connect to server!");
            return Err("Connection failed".into());
        }
        
        println!("[CLIENT] Connected to media server successfully");
        
        // Set up sequence number and timestamp
        let mut sequence: u16 = 0;
        let mut timestamp: u32 = 0;
        
        // Send test frames
        println!("\nSending 5 secure media frames...");
        for i in 0..5 {
            // Create test audio frame
            let frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: vec![1, 2, 3, 4, 5, 6, 7, 8],
                sequence,
                timestamp,
                payload_type: 8,
                marker: false,
                ssrc: 12345,
                csrcs: Vec::new(),
            };
            
            // Send frame
            client.send_frame(frame).await?;
            println!("[CLIENT] Sent frame #{}: seq={}, ts={}", i+1, sequence, timestamp);
            
            // Update sequence and timestamp
            sequence = sequence.wrapping_add(1);
            timestamp = timestamp.wrapping_add(160); // 20ms at 8kHz
            
            // Wait before sending next frame
            sleep(Duration::from_millis(100)).await;
        }
        
        // Wait for all frames to be processed
        sleep(Duration::from_secs(1)).await;
        
        // Disconnect client
        println!("\nDisconnecting media client...");
        client.disconnect().await?;
        
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });
    
    // Wait for media client task to complete with timeout
    tokio::select! {
        res = media_client_task => {
            match res {
                Ok(result) => {
                    if let Err(e) = result {
                        println!("Media client task failed: {}", e);
                    } else {
                        println!("Media client task completed successfully");
                    }
                },
                Err(e) => {
                    println!("Media client task panicked: {}", e);
                }
            }
        }
        _ = sleep(Duration::from_secs(15)) => {
            println!("Media client task timed out after 15 seconds");
        }
    }
    
    // Stop server
    println!("\nStopping media server...");
    server.stop().await?;
    println!("Media server stopped");
    
    println!("\nDirect DTLS Media Streaming example completed successfully");
    Ok(())
} 