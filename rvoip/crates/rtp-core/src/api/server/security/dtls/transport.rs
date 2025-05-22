//! DTLS transport functionality
//!
//! This module handles DTLS transport setup and management.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::api::common::error::SecurityError;
use crate::api::server::security::{SocketHandle};
use crate::dtls::transport::udp::UdpTransport;

/// Create a UDP transport for DTLS
pub async fn create_udp_transport(
    socket: &SocketHandle,
    mtu: usize,
) -> Result<UdpTransport, SecurityError> {
    debug!("Creating UDP transport for DTLS");
    
    match UdpTransport::new(socket.socket.clone(), mtu).await {
        Ok(mut transport) => {
            // Start the transport
            if let Err(e) = transport.start().await {
                return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)));
            }
            debug!("DTLS transport started successfully");
            Ok(transport)
        },
        Err(e) => Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e))),
    }
}

/// Start a packet handler for DTLS
pub async fn start_packet_handler(
    socket: &SocketHandle,
    handler: impl Fn(Vec<u8>, SocketAddr) -> Result<(), SecurityError> + Send + Sync + 'static,
) -> Result<(), SecurityError> {
    debug!("Starting automatic DTLS packet handler task");
    
    let socket_clone = socket.socket.clone();
    
    // Spawn the packet handler task
    tokio::spawn(async move {
        // Create a transport for packet reception
        let transport = match UdpTransport::new(socket_clone.clone(), 1500).await {
            Ok(mut t) => {
                // Start the transport - this is essential
                if let Err(e) = t.start().await {
                    error!("Failed to start server transport for packet handler: {}", e);
                    return;
                }
                debug!("Server packet handler transport started successfully");
                Arc::new(Mutex::new(t))
            },
            Err(e) => {
                error!("Failed to create server transport for packet handler: {}", e);
                return;
            }
        };
        
        // Main packet handling loop
        loop {
            // Use the transport to receive packets
            let receive_result = transport.lock().await.recv().await;
            
            match receive_result {
                Some((data, addr)) => {
                    debug!("Server received {} bytes from {}", data.len(), addr);
                    
                    // Process the packet through the handler - convert Bytes to Vec<u8>
                    match handler(data.to_vec(), addr) {
                        Ok(_) => debug!("Server successfully processed client DTLS packet"),
                        Err(e) => debug!("Error processing client DTLS packet: {:?}", e),
                    }
                },
                None => {
                    // Transport returned None - likely an error or shutdown
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    });
    
    Ok(())
}

/// Capture an initial packet from a client
pub async fn capture_initial_packet(
    socket: &SocketHandle,
    timeout_secs: u64,
) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError> {
    debug!("Attempting to capture initial client packet with timeout of {} seconds", timeout_secs);
    
    // Create a buffer to receive packet
    let mut buffer = vec![0u8; 2048];
    
    // Set a timeout to avoid blocking indefinitely
    match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        socket.socket.recv_from(&mut buffer)
    ).await {
        Ok(Ok((size, addr))) => {
            debug!("Captured initial packet: {} bytes from {}", size, addr);
            Ok(Some((buffer[..size].to_vec(), addr)))
        },
        Ok(Err(e)) => {
            Err(SecurityError::HandshakeError(format!("Error receiving initial packet: {}", e)))
        },
        Err(_) => {
            // Timeout occurred
            debug!("Timeout occurred while waiting for initial packet");
            Ok(None)
        }
    }
}

/// Start a UDP transport
pub async fn start_udp_transport(
    transport: &mut UdpTransport,
) -> Result<(), SecurityError> {
    debug!("Starting UDP transport");
    
    match transport.start().await {
        Ok(_) => {
            debug!("UDP transport started successfully");
            Ok(())
        },
        Err(e) => Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e))),
    }
} 