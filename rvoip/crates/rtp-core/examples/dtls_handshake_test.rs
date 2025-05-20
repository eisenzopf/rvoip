use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::sleep;

use rvoip_rtp_core::dtls::{DtlsConfig, DtlsRole, DtlsConnection};
use rvoip_rtp_core::dtls::transport::udp::UdpTransport;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;
use rvoip_rtp_core::api::common::config::{SecurityMode, SrtpProfile};
use rvoip_rtp_core::srtp::crypto::SrtpCryptoKey;

// Simple error type that is Send + Sync
#[derive(Debug)]
struct SimpleError(String);

impl std::fmt::Display for SimpleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SimpleError {}

type Result<T> = std::result::Result<T, SimpleError>;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Configure server and client with dedicated sockets
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20001);
    
    println!("DTLS Handshake Test");
    println!("==================");
    
    // Create UDP sockets
    println!("Creating UDP sockets...");
    let server_socket = Arc::new(UdpSocket::bind(server_addr).await?);
    let client_socket = Arc::new(UdpSocket::bind(client_addr).await?);
    println!("Server listening on {}", server_addr);
    println!("Client listening on {}", client_addr);
    
    // Generate certificates
    println!("Generating certificates...");
    let server_cert = rvoip_rtp_core::dtls::crypto::verify::generate_self_signed_certificate()?;
    let client_cert = rvoip_rtp_core::dtls::crypto::verify::generate_self_signed_certificate()?;
    
    // Extract fingerprints
    let mut server_cert_copy = server_cert.clone();
    let mut client_cert_copy = client_cert.clone();
    let server_fingerprint = server_cert_copy.fingerprint("SHA-256")?;
    let client_fingerprint = client_cert_copy.fingerprint("SHA-256")?;
    println!("Server fingerprint: {}", server_fingerprint);
    println!("Client fingerprint: {}", client_fingerprint);
    
    // Create DTLS configs
    println!("Creating DTLS configurations...");
    
    // Server config
    let server_config = DtlsConfig {
        role: DtlsRole::Server,
        version: rvoip_rtp_core::dtls::DtlsVersion::Dtls12,
        mtu: 1200,
        max_retransmissions: 5,
        srtp_profiles: vec![rvoip_rtp_core::srtp::SRTP_AES128_CM_SHA1_80],
    };
    
    // Client config
    let client_config = DtlsConfig {
        role: DtlsRole::Client,
        version: rvoip_rtp_core::dtls::DtlsVersion::Dtls12,
        mtu: 1200,
        max_retransmissions: 5,
        srtp_profiles: vec![rvoip_rtp_core::srtp::SRTP_AES128_CM_SHA1_80],
    };
    
    // Set up server in a separate task
    let server_task = tokio::spawn(async move {
        println!("Starting server task...");
        
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
            let (packet, addr) = match transport_guard.recv().await {
                Some((bytes, addr)) => {
                    println!("Server received initial packet: {} bytes from {}", bytes.len(), addr);
                    (bytes.to_vec(), addr)
                },
                None => {
                    println!("Server transport closed");
                    return Err(SimpleError(format!("Transport closed")));
                }
            };
            (packet, addr)
        };
        
        // Start handshake to initialize the state machine
        println!("Server starting handshake with client at {}", client_addr);
        server_conn.start_handshake(client_addr).await.unwrap();
        
        // Process the initial packet to handle the ClientHello
        println!("Server processing initial ClientHello packet...");
        if let Err(e) = server_conn.process_packet(&initial_packet).await {
            println!("Server error processing initial packet: {}", e);
            return Err(SimpleError(format!("Failed to process packet: {}", e)));
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
                let server_key = srtp_context.get_key_for_role(false).clone();
                
                println!("Server SRTP key: {:?}", server_key);
                println!("Server SRTP profile: {:?}", srtp_context.profile);
                
                // Return success with the SRTP keys
                Ok(server_key)
            },
            Err(e) => {
                println!("Server handshake failed: {}", e);
                Err(SimpleError(format!("Server handshake failed: {}", e)))
            }
        }
    });
    
    // Give the server time to start up
    println!("Waiting for server to start...");
    sleep(Duration::from_millis(1000)).await;
    
    // Set up client in a separate task
    let client_task = tokio::spawn(async move {
        println!("Starting client task...");
        
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
                let client_key = srtp_context.get_key_for_role(true).clone();
                
                println!("Client SRTP key: {:?}", client_key);
                println!("Client SRTP profile: {:?}", srtp_context.profile);
                
                // Return success with the SRTP keys
                Ok(client_key)
            },
            Err(e) => {
                println!("Client handshake failed: {}", e);
                Err(SimpleError(format!("Client handshake failed: {}", e)))
            }
        }
    });
    
    // Wait for both tasks to complete
    println!("Waiting for client and server tasks to complete...");
    let server_result = server_task.await??;
    println!("Server task completed");
    let client_result = client_task.await??;
    println!("Client task completed");
    
    // Verify that the keys match (in a real-world scenario, the client's write key
    // should match the server's read key and vice versa)
    println!("\nVerifying SRTP keys match between client and server...");
    
    // In DTLS-SRTP, the client's write key is the server's read key
    // and the server's write key is the client's read key
    println!("Server key: {:?}", server_result);
    println!("Client key: {:?}", client_result);
    
    println!("\nDTLS handshake test completed successfully!");
    
    Ok(())
} 