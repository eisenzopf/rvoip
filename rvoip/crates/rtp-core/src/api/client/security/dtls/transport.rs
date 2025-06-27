//! DTLS transport setup and configuration
//!
//! This module provides functions for setting up and configuring DTLS transports.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use std::time::Duration;

use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::dtls::transport::udp::UdpTransport;
use crate::dtls::DtlsConnection;
use crate::api::client::security::ClientSecurityContext;

/// Set up a DTLS transport with the given socket
pub async fn setup_transport(
    socket: &SocketHandle,
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    // If we have a connection initialized, we need to update its transport
    let mut conn_guard = connection.lock().await;
    if let Some(conn) = conn_guard.as_mut() {
        // Create a transport from the socket
        let transport = Arc::new(Mutex::new(
            match create_udp_transport(socket.socket.clone(), 1500).await {
                Ok(t) => t,
                Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e)))
            }
        ));
        
        // Start the transport
        match transport.lock().await.start().await {
            Ok(_) => debug!("DTLS transport started successfully"),
            Err(e) => return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)))
        }
        
        // Set transport on connection
        conn.set_transport(transport);
        
        debug!("DTLS transport set up successfully");
        Ok(())
    } else {
        debug!("No DTLS connection to set up transport for");
        Ok(())
    }
}

/// Start a packet handler for the DTLS transport
pub async fn start_packet_handler(
    socket: &SocketHandle,
    remote_addr: SocketAddr,
    context: Arc<dyn ClientSecurityContext>,
) -> Result<(), SecurityError> {
    debug!("Client: Starting DTLS packet handler for server {}", remote_addr);
    
    // Clone socket for the task
    let socket_clone = socket.socket.clone();
    
    // Spawn the packet handler task
    tokio::spawn(async move {
        debug!("Client packet handler task started for server {}", remote_addr);
        
        // Buffer for receiving packets
        let mut buffer = vec![0u8; 2048];
        
        // Main packet handling loop
        loop {
            match socket_clone.recv_from(&mut buffer).await {
                Ok((size, addr)) => {
                    if addr == remote_addr {
                        debug!("Client received {} bytes from server {}", size, addr);
                        
                        // Process the packet through the client context
                        if let Err(e) = context.process_packet(&buffer[..size]).await {
                            error!("Error processing server DTLS packet: {:?}", e);
                        }
                    } else {
                        debug!("Ignoring packet from unknown sender {}", addr);
                    }
                },
                Err(e) => {
                    debug!("Client receive error: {}", e);
                    // Sleep a bit to avoid tight loop in case of continuous errors
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    });
    
    Ok(())
}

/// Create a UDP transport for DTLS
pub async fn create_udp_transport(
    socket: Arc<UdpSocket>,
    mtu: usize,
) -> Result<UdpTransport, SecurityError> {
    match UdpTransport::new(socket, mtu).await {
        Ok(transport) => Ok(transport),
        Err(e) => Err(SecurityError::Configuration(format!("Failed to create UDP transport: {}", e)))
    }
} 