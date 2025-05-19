use std::net::SocketAddr;
use std::sync::Arc;
use std::io;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};

// Import from rtp-core
use rvoip_rtp_core::srtp as srtp_core;
use rvoip_rtp_core::packet::RtpPacket;

/// SRTP session wrapper
pub struct SrtpSession {
    /// Configuration
    config: SrtpConfig,
    
    /// Socket for sending/receiving
    socket: Arc<UdpSocket>,
    
    /// Local address
    local_addr: SocketAddr,
    
    /// Remote address
    remote_addr: SocketAddr,
    
    /// RTP receiver channel
    rtp_rx: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    
    /// Running flag
    running: Arc<RwLock<bool>>,
    
    /// Receiver task
    receiver_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Inner SRTP context from rtp-core
    inner: Option<srtp_core::SrtpContext>,
}

/// RTP packet statistics
#[derive(Debug, Default, Clone)]
pub struct RtpStats {
    /// Number of packets sent
    pub packets_sent: u64,
    
    /// Number of bytes sent
    pub bytes_sent: u64,
    
    /// Number of packets received
    pub packets_recv: u64,
    
    /// Number of bytes received
    pub bytes_recv: u64,
    
    /// Number of packets lost
    pub packets_lost: u64,
    
    /// Number of packets discarded
    pub packets_discarded: u64,
}

/// SRTP keys
#[derive(Debug, Clone)]
pub struct SrtpKeys {
    /// Master key
    pub master_key: Vec<u8>,
    
    /// Master salt
    pub master_salt: Vec<u8>,
}

impl SrtpKeys {
    /// Create new SRTP keys
    pub fn new(master_key: Vec<u8>, master_salt: Vec<u8>) -> Self {
        Self {
            master_key,
            master_salt,
        }
    }
    
    /// Generate random SRTP keys
    pub fn generate() -> Self {
        // Generate 16 bytes of master key
        let mut master_key = vec![0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut master_key);
        
        // Generate 14 bytes of master salt
        let mut master_salt = vec![0u8; 14];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut master_salt);
        
        Self {
            master_key,
            master_salt,
        }
    }
}

/// Convert to rtp-core's SrtpCryptoKey
impl From<&SrtpKeys> for srtp_core::SrtpCryptoKey {
    fn from(keys: &SrtpKeys) -> Self {
        srtp_core::SrtpCryptoKey::new(keys.master_key.clone(), keys.master_salt.clone())
    }
}

/// SRTP configuration
#[derive(Debug, Clone)]
pub struct SrtpConfig {
    /// Local SRTP keys
    pub local_keys: SrtpKeys,
    
    /// Remote SRTP keys
    pub remote_keys: SrtpKeys,
    
    /// SRTP protection profile
    pub profile: srtp_core::SrtpCryptoSuite,
}

impl Default for SrtpConfig {
    fn default() -> Self {
        Self {
            local_keys: SrtpKeys::generate(),
            remote_keys: SrtpKeys::generate(),
            profile: srtp_core::SRTP_AES128_CM_SHA1_80,
        }
    }
}

impl SrtpSession {
    /// Create a new SRTP session
    pub async fn new(config: SrtpConfig) -> Result<Self> {
        debug!("Creating SRTP session with protection profile: {:?}", config.profile);
        
        // Create a socket
        let socket = UdpSocket::bind("0.0.0.0:0").await
            .map_err(|e| Error::Network(format!("Failed to bind UDP socket: {}", e)))?;
        
        Ok(Self {
            config,
            socket: Arc::new(socket),
            local_addr: "0.0.0.0:0".parse().unwrap(),
            remote_addr: "0.0.0.0:0".parse().unwrap(),
            rtp_rx: Arc::new(Mutex::new(None)),
            running: Arc::new(RwLock::new(false)),
            receiver_task: None,
            inner: None,
        })
    }
    
    /// Initialize SRTP context
    fn initialize_context(&mut self) -> Result<()> {
        // Create crypto key from config
        let local_key = srtp_core::SrtpCryptoKey::new(
            self.config.local_keys.master_key.clone(),
            self.config.local_keys.master_salt.clone()
        );
        
        // Create SRTP context
        let context = srtp_core::SrtpContext::new(
            self.config.profile.clone(),
            local_key
        ).map_err(|e| Error::Security(format!("Failed to create SRTP context: {:?}", e)))?;
        
        self.inner = Some(context);
        Ok(())
    }
    
    /// Process incoming RTP packet
    pub async fn process_rtp(&self, packet_data: &[u8]) -> Result<Vec<u8>> {
        debug!("Processing RTP packet");
        
        // If we have an inner context, use it
        if let Some(mut context) = self.inner.clone() {
            match context.unprotect(packet_data) {
                Ok(packet) => {
                    // Serialize the unprotected packet
                    let packet_bytes = packet.serialize()
                        .map_err(|e| Error::InvalidData(format!("Failed to serialize RTP packet: {:?}", e)))?;
                    Ok(packet_bytes.to_vec())
                },
                Err(e) => {
                    // If SRTP is not enabled or the context is not set up correctly,
                    // just pass the packet through
                    debug!("SRTP unprotect failed (passing through): {:?}", e);
                    Ok(packet_data.to_vec())
                }
            }
        } else {
            // If no context, pass through
            Ok(packet_data.to_vec())
        }
    }
    
    /// Send RTP packet
    pub async fn send_rtp(&self, data: &[u8]) -> Result<Vec<u8>> {
        debug!("Sending RTP packet");
        
        // Try to parse as RTP packet
        let packet = match RtpPacket::parse(data) {
            Ok(p) => p,
            Err(e) => {
                debug!("Failed to parse RTP packet for SRTP protection: {:?}", e);
                // Just send as-is if not a valid RTP packet
                self.socket.send_to(data, self.remote_addr).await
                    .map_err(|e| Error::Network(format!("Failed to send RTP packet: {}", e)))?;
                return Ok(data.to_vec());
            }
        };
        
        // If we have an inner context, protect the packet
        if let Some(mut context) = self.inner.clone() {
            match context.protect(&packet) {
                Ok(protected) => {
                    // Serialize the protected packet
                    let protected_bytes = protected.serialize()
                        .map_err(|e| Error::InvalidData(format!("Failed to serialize protected RTP packet: {:?}", e)))?;
                    
                    // Send the protected packet
                    self.socket.send_to(&protected_bytes, self.remote_addr).await
                        .map_err(|e| Error::Network(format!("Failed to send protected RTP packet: {}", e)))?;
                    
                    Ok(protected_bytes.to_vec())
                },
                Err(e) => {
                    // If protection fails, send unprotected
                    debug!("SRTP protect failed (sending unprotected): {:?}", e);
                    self.socket.send_to(data, self.remote_addr).await
                        .map_err(|e| Error::Network(format!("Failed to send RTP packet: {}", e)))?;
                    Ok(data.to_vec())
                }
            }
        } else {
            // If no context, send unprotected
            self.socket.send_to(data, self.remote_addr).await
                .map_err(|e| Error::Network(format!("Failed to send RTP packet: {}", e)))?;
            Ok(data.to_vec())
        }
    }
    
    /// Set remote address
    pub fn set_remote_addr(&mut self, addr: SocketAddr) {
        debug!("Setting remote address to {}", addr);
        self.remote_addr = addr;
    }
} 