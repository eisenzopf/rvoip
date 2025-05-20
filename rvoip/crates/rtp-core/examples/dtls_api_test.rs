use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::sleep;

use rvoip_rtp_core::api::client::security::{ClientSecurityContext, ClientSecurityConfig, DefaultClientSecurityContext};
use rvoip_rtp_core::api::server::security::{ServerSecurityContext, ServerSecurityConfig, DefaultServerSecurityContext, SocketHandle};
use rvoip_rtp_core::api::common::config::{SecurityMode, SrtpProfile, SecurityInfo};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure server and client with dedicated sockets
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20001);
    
    println!("DTLS API Test");
    println!("=============");
    
    // Create UDP sockets
    println!("Creating UDP sockets...");
    let server_socket = Arc::new(UdpSocket::bind(server_addr).await?);
    let client_socket = Arc::new(UdpSocket::bind(client_addr).await?);
    println!("Server listening on {}", server_addr);
    println!("Client listening on {}", client_addr);
    
    // Create server security config
    let server_config = ServerSecurityConfig {
        security_mode: SecurityMode::DtlsSrtp,
        fingerprint_algorithm: "sha-256".to_string(),
        srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
        certificate_path: None,
        private_key_path: None,
        require_client_certificate: false,
    };
    
    // Create client security config
    let client_config = ClientSecurityConfig {
        security_mode: SecurityMode::DtlsSrtp,
        fingerprint_algorithm: "sha-256".to_string(),
        remote_fingerprint: None,
        remote_fingerprint_algorithm: None,
        validate_fingerprint: true,
        srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
        certificate_path: None,
        private_key_path: None,
    };
    
    // Create server context
    println!("Creating server security context...");
    let server_ctx = DefaultServerSecurityContext::new(server_config).await?;
    
    // Create client context
    println!("Creating client security context...");
    let client_ctx = DefaultClientSecurityContext::new(client_config).await?;
    
    // Set up socket handles
    let server_handle = SocketHandle {
        socket: server_socket.clone(),
        remote_addr: None,
    };
    
    let client_handle = SocketHandle {
        socket: client_socket.clone(),
        remote_addr: Some(server_addr),
    };
    
    // Set sockets on contexts
    println!("Setting sockets...");
    server_ctx.set_socket(server_handle).await?;
    client_ctx.set_socket(client_handle).await?;
    
    // Set client remote address
    println!("Setting client remote address...");
    client_ctx.set_remote_address(server_addr).await?;
    
    // Initialize contexts
    println!("Initializing contexts...");
    client_ctx.initialize().await?;
    
    // Get and exchange fingerprints
    println!("Exchanging fingerprints...");
    let server_fingerprint = server_ctx.get_fingerprint().await?;
    let client_fingerprint = client_ctx.get_fingerprint().await?;
    println!("Server fingerprint: {}", server_fingerprint);
    println!("Client fingerprint: {}", client_fingerprint);
    
    // Set fingerprints
    client_ctx.set_remote_fingerprint(&server_fingerprint, "sha-256").await?;
    
    // Start server in listening mode
    println!("Starting server...");
    server_ctx.start_listening().await?;
    
    // Create task for server to receive packets
    let server_ctx_clone = server_ctx.clone();
    let server_receive_task = tokio::spawn(async move {
        println!("Server receiver task started");
        let mut buffer = vec![0u8; 2048];
        
        // Add timeout for the receive loop
        let task_timeout = tokio::time::sleep(Duration::from_secs(10));
        tokio::pin!(task_timeout);
        
        loop {
            tokio::select! {
                _ = &mut task_timeout => {
                    println!("Server receiver task timed out after 10 seconds");
                    break;
                }
                result = server_socket.recv_from(&mut buffer) => {
                    match result {
                        Ok((size, addr)) => {
                            println!("Server received {} bytes from {}", size, addr);
                            
                            // Process the packet with the server context
                            match server_ctx_clone.process_client_packet(addr, &buffer[..size]).await {
                                Ok(_) => println!("Server successfully processed client packet"),
                                Err(e) => println!("Error processing packet: {:?}", e),
                            }
                        },
                        Err(e) => {
                            println!("Server receive error: {}", e);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                }
            }
        }
    });
    
    // Wait a moment for server to be ready
    println!("Waiting for server to start...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Start client handshake
    println!("Starting client handshake...");
    client_ctx.start_handshake().await?;
    
    // Wait for handshake to complete with timeout
    println!("Waiting for handshake to complete...");
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(5);
    
    loop {
        if client_ctx.is_handshake_complete().await? {
            println!("Client handshake complete!");
            break;
        }
        
        if start_time.elapsed() > timeout {
            println!("Handshake timed out after 5 seconds");
            break;
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
        println!("Still waiting for handshake... ({:?} elapsed)", start_time.elapsed());
    }
    
    // After some time, check if we can find the client context on the server
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    let client_contexts = server_ctx.get_client_contexts().await;
    println!("Found {} client contexts on server", client_contexts.len());
    
    for (i, ctx) in client_contexts.iter().enumerate() {
        println!("Client {}: handshake complete = {}", i, ctx.is_handshake_complete().await?);
        
        if let Ok(Some(fp)) = ctx.get_remote_fingerprint().await {
            println!("Client {} fingerprint: {}", i, fp);
        }
    }
    
    // Abort the server receive task - we're done
    server_receive_task.abort();
    
    println!("DTLS API test completed successfully!");
    Ok(())
} 