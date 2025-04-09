use std::net::SocketAddr;
use std::sync::Arc;
use std::io;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use webrtc_srtp::{session::Session, session::SessionKeys, session::Context, protection_profile::ProtectionProfile};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};

/// SRTP key material
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

/// SRTP configuration
#[derive(Debug, Clone)]
pub struct SrtpConfig {
    /// Local keys
    pub local_keys: SrtpKeys,
    
    /// Remote keys
    pub remote_keys: SrtpKeys,
    
    /// Protection profile
    pub profile: ProtectionProfile,
}

impl Default for SrtpConfig {
    fn default() -> Self {
        Self {
            local_keys: SrtpKeys::generate(),
            remote_keys: SrtpKeys::generate(),
            profile: ProtectionProfile::Aes128CmHmacSha1_80,
        }
    }
}

/// SRTP session for secure RTP transmission
pub struct SrtpSession {
    /// SRTP configuration
    config: SrtpConfig,
    
    /// Outbound SRTP session
    outbound: Arc<Mutex<Session>>,
    
    /// Inbound SRTP session
    inbound: Arc<Mutex<Session>>,
    
    /// UDP socket for RTP communication
    socket: Arc<UdpSocket>,
    
    /// Local RTP address
    local_addr: SocketAddr,
    
    /// Remote RTP address
    remote_addr: SocketAddr,
    
    /// RTP receiver
    rtp_rx: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    
    /// Running flag
    running: Arc<RwLock<bool>>,
    
    /// Receiver task
    receiver_task: Option<tokio::task::JoinHandle<()>>,
}

impl SrtpSession {
    /// Create a new SRTP session
    pub async fn new(
        config: SrtpConfig,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        // Create UDP socket
        let socket = UdpSocket::bind(local_addr).await
            .map_err(|e| Error::Network(format!("Failed to bind UDP socket: {}", e)))?;
        
        // Create SRTP sessions
        let local_keys = SessionKeys {
            master_key: &config.local_keys.master_key,
            master_salt: &config.local_keys.master_salt,
        };
        
        let remote_keys = SessionKeys {
            master_key: &config.remote_keys.master_key,
            master_salt: &config.remote_keys.master_salt,
        };
        
        // Create contexts
        let mut outbound_context = Context::new();
        outbound_context.set_profile(config.profile);
        
        let mut inbound_context = Context::new();
        inbound_context.set_profile(config.profile);
        
        // Create sessions
        let outbound = Session::new(outbound_context, Some(local_keys))
            .map_err(|e| Error::Media(format!("Failed to create outbound SRTP session: {}", e)))?;
        
        let inbound = Session::new(inbound_context, Some(remote_keys))
            .map_err(|e| Error::Media(format!("Failed to create inbound SRTP session: {}", e)))?;
        
        // Create RTP channel
        let (tx, rx) = mpsc::channel(100);
        
        // Create session
        let session = Self {
            config,
            outbound: Arc::new(Mutex::new(outbound)),
            inbound: Arc::new(Mutex::new(inbound)),
            socket: Arc::new(socket),
            local_addr,
            remote_addr,
            rtp_rx: Arc::new(Mutex::new(Some(tx))),
            running: Arc::new(RwLock::new(false)),
            receiver_task: None,
        };
        
        Ok((session, rx))
    }
    
    /// Start the SRTP session
    pub async fn start(&mut self) -> Result<()> {
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Set running flag
        *self.running.write().await = true;
        
        // Start receiver task
        let socket = self.socket.clone();
        let inbound = self.inbound.clone();
        let rtp_rx = self.rtp_rx.clone();
        let running = self.running.clone();
        
        let receiver_task = tokio::spawn(async move {
            let mut buffer = vec![0u8; 2000]; // RTP packets are typically small
            
            while *running.read().await {
                // Receive data
                match socket.recv_from(&mut buffer).await {
                    Ok((size, addr)) => {
                        // Use the received data
                        let packet_data = &buffer[..size];
                        
                        // Decrypt SRTP packet
                        match inbound.lock().await.decrypt_rtp(&packet_data) {
                            Ok(decrypted) => {
                                // Forward packet to receiver
                                if let Some(tx) = &*rtp_rx.lock().await {
                                    let _ = tx.try_send(decrypted);
                                }
                            },
                            Err(e) => {
                                warn!("Failed to decrypt SRTP packet: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        if e.kind() != io::ErrorKind::WouldBlock && e.kind() != io::ErrorKind::TimedOut {
                            error!("Error receiving RTP data: {}", e);
                            break;
                        }
                    }
                }
            }
            
            info!("SRTP receiver task stopped");
        });
        
        self.receiver_task = Some(receiver_task);
        
        Ok(())
    }
    
    /// Send RTP data through SRTP
    pub async fn send_rtp(&self, data: &[u8]) -> Result<()> {
        // Encrypt the packet
        let encrypted = self.outbound.lock().await.encrypt_rtp(data)
            .map_err(|e| Error::Media(format!("Failed to encrypt RTP packet: {}", e)))?;
        
        // Send the encrypted packet
        self.socket.send_to(&encrypted, self.remote_addr).await
            .map_err(|e| Error::Network(format!("Failed to send SRTP packet: {}", e)))?;
        
        Ok(())
    }
    
    /// Stop the SRTP session
    pub async fn stop(&mut self) -> Result<()> {
        // Check if running
        if !*self.running.read().await {
            return Ok(());
        }
        
        // Set running flag to false
        *self.running.write().await = false;
        
        // Abort receiver task
        if let Some(task) = self.receiver_task.take() {
            task.abort();
        }
        
        // Clear RTP receiver
        *self.rtp_rx.lock().await = None;
        
        Ok(())
    }
    
    /// Get local address
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
    
    /// Get remote address
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
    
    /// Set remote address
    pub fn set_remote_addr(&mut self, addr: SocketAddr) {
        self.remote_addr = addr;
    }
} 