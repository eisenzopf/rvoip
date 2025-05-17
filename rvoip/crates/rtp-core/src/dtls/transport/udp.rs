//! UDP transport for DTLS
//!
//! This module implements a UDP transport for DTLS.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use bytes::Bytes;

use crate::dtls::Result;

/// UDP transport for DTLS
pub struct UdpTransport {
    /// UDP socket
    socket: Arc<UdpSocket>,
    
    /// Maximum packet size
    mtu: usize,
    
    /// Packet receiver channel
    rx: mpsc::Receiver<(Bytes, SocketAddr)>,
    
    /// Packet sender channel
    tx: mpsc::Sender<(Bytes, SocketAddr)>,
    
    /// Running state
    running: bool,
}

impl UdpTransport {
    /// Create a new UDP transport
    pub async fn new(socket: Arc<UdpSocket>, mtu: usize) -> Result<Self> {
        // Create channels for packet passing
        let (tx, rx) = mpsc::channel(100);
        
        let transport = Self {
            socket,
            mtu,
            rx,
            tx,
            running: false,
        };
        
        Ok(transport)
    }
    
    /// Start the transport
    pub async fn start(&mut self) -> Result<()> {
        if self.running {
            return Ok(());
        }
        
        self.running = true;
        
        // Clone values for the receiver task
        let socket = self.socket.clone();
        let tx = self.tx.clone();
        let mtu = self.mtu;
        
        // Spawn receiver task
        tokio::spawn(async move {
            let mut buf = vec![0u8; mtu];
            
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((n, addr)) => {
                        let packet = Bytes::copy_from_slice(&buf[..n]);
                        if tx.send((packet, addr)).await.is_err() {
                            // Sender dropped, exit loop
                            break;
                        }
                    }
                    Err(err) => {
                        // Socket error, log and continue
                        tracing::error!("DTLS UDP transport error: {}", err);
                    }
                }
            }
        });
        
        Ok(())
    }
    
    /// Stop the transport
    pub async fn stop(&mut self) {
        self.running = false;
    }
    
    /// Send a packet
    pub async fn send(&self, data: &[u8], addr: SocketAddr) -> Result<usize> {
        match self.socket.send_to(data, addr).await {
            Ok(n) => Ok(n),
            Err(err) => Err(crate::error::Error::IoError(err.to_string())),
        }
    }
    
    /// Receive a packet
    pub async fn recv(&mut self) -> Option<(Bytes, SocketAddr)> {
        self.rx.recv().await
    }
    
    /// Get the local address
    pub fn local_addr(&self) -> Result<SocketAddr> {
        match self.socket.local_addr() {
            Ok(addr) => Ok(addr),
            Err(err) => Err(crate::error::Error::IoError(err.to_string())),
        }
    }
    
    /// Get the maximum packet size
    pub fn mtu(&self) -> usize {
        self.mtu
    }
} 