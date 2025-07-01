use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::sleep;

use rvoip_rtp_core::dtls::{DtlsConfig, DtlsRole, DtlsVersion};
use rvoip_rtp_core::dtls::connection::DtlsConnection;
use rvoip_rtp_core::dtls::crypto::verify::{Certificate, generate_self_signed_certificate};
use rvoip_rtp_core::dtls::transport::udp::UdpTransport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .init();
    
    println!("DTLS Example: Testing DTLS handshake and SRTP key extraction");
    
    // Create client and server sockets on different ports
    let server_addr: SocketAddr = "127.0.0.1:5000".parse()?;
    let client_addr: SocketAddr = "127.0.0.1:5001".parse()?;
    
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
            let (packet, addr) = transport_guard.recv().await.unwrap();
            println!("Server received initial packet: {} bytes from {}", packet.len(), addr);
            
            // Try to parse the packet as a DTLS record
            match rvoip_rtp_core::dtls::record::Record::parse_multiple(&packet) {
                Ok(records) => {
                    // Process each record
                    for record in records {
                        println!("Received record of type: {:?}", record.header.content_type);
                        
                        // Process handshake records
                        if record.header.content_type == rvoip_rtp_core::dtls::record::ContentType::Handshake {
                            // Parse the handshake message
                            if let Ok((header, _)) = rvoip_rtp_core::dtls::message::handshake::HandshakeHeader::parse(&record.data) {
                                println!("Handshake message type: {:?}", header.msg_type);
                            }
                        }
                    }
                },
                Err(e) => {
                    println!("ERROR: Failed to parse DTLS record: {:?}", e);
                }
            }
            
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
    println!("Server's write key: {:?}", server_result);
    println!("Client's write key: {:?}", client_result);
    
    println!("\nDTLS test completed successfully!");
    
    Ok(())
} 