use std::net::SocketAddr;
use std::sync::Arc;
use std::io;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::security::srtp::{SrtpKeys, SrtpConfig};

// Import from rtp-core
use rvoip_rtp_core::dtls as dtls_core;

/// DTLS events
#[derive(Debug, Clone)]
pub enum DtlsEvent {
    /// Connection established
    Connected,
    
    /// Connection closed
    Closed,
    
    /// Data received
    Data(Vec<u8>),
    
    /// Error occurred
    Error(String),
}

/// DTLS roles
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtlsRole {
    /// Act as client (active)
    Client,
    
    /// Act as server (passive)
    Server,
}

impl From<DtlsRole> for dtls_core::DtlsRole {
    fn from(role: DtlsRole) -> Self {
        match role {
            DtlsRole::Client => dtls_core::DtlsRole::Client,
            DtlsRole::Server => dtls_core::DtlsRole::Server,
        }
    }
}

impl From<dtls_core::DtlsRole> for DtlsRole {
    fn from(role: dtls_core::DtlsRole) -> Self {
        match role {
            dtls_core::DtlsRole::Client => DtlsRole::Client,
            dtls_core::DtlsRole::Server => DtlsRole::Server,
        }
    }
}

/// SRTP protection profile
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpProtectionProfile {
    /// SRTP_AES128_CM_HMAC_SHA1_80
    Aes128CmHmacSha1_80,
    
    /// SRTP_AEAD_AES_128_GCM
    AeadAes128Gcm,
}

/// Network transport connection trait
pub trait TransportConn: Send + Sync {
    /// Send data
    fn send(&self, data: &[u8]) -> Result<usize>;
    
    /// Close connection
    fn close(&self) -> Result<()>;
}

/// DTLS configuration
#[derive(Debug, Clone)]
pub struct DtlsConfig {
    /// Certificate and key in PEM format
    pub cert_pem: String,
    
    /// Private key in PEM format
    pub key_pem: String,
    
    /// Role (client or server)
    pub role: DtlsRole,
    
    /// SRTP protection profile
    pub srtp_profile: SrtpProtectionProfile,
    
    /// Connection timeout
    pub timeout: Duration,
}

impl Default for DtlsConfig {
    fn default() -> Self {
        Self {
            cert_pem: String::new(),
            key_pem: String::new(),
            role: DtlsRole::Client,
            srtp_profile: SrtpProtectionProfile::Aes128CmHmacSha1_80,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Converts media-core DtlsConfig to rtp-core DtlsConfig
fn convert_config(config: &DtlsConfig) -> dtls_core::DtlsConfig {
    // Convert to rtp-core SRTP profile
    let srtp_profiles = match config.srtp_profile {
        SrtpProtectionProfile::Aes128CmHmacSha1_80 => vec![rvoip_rtp_core::srtp::SRTP_AES128_CM_SHA1_80],
        SrtpProtectionProfile::AeadAes128Gcm => vec![rvoip_rtp_core::srtp::SRTP_AES128_CM_SHA1_80], // Fallback to supported profile
    };
    
    dtls_core::DtlsConfig {
        role: config.role.into(),
        version: dtls_core::DtlsVersion::Dtls12,
        mtu: 1200,
        max_retransmissions: 5,
        srtp_profiles,
    }
}

/// DTLS connection
pub struct DtlsConnection {
    /// Configuration
    config: DtlsConfig,
    
    /// State
    state: Arc<RwLock<DtlsConnectionState>>,
    
    /// Event sender
    event_tx: Arc<Mutex<Option<mpsc::Sender<DtlsEvent>>>>,
    
    /// Whether the connection is open
    is_open: Arc<RwLock<bool>>,
    
    /// Inner DTLS connection from rtp-core (when connected)
    inner: Option<Arc<dtls_core::DtlsConnection>>,
}

/// Internal connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DtlsConnectionState {
    /// Not connected
    New,
    
    /// Connecting
    Connecting,
    
    /// Connected
    Connected,
    
    /// Closed
    Closed,
}

impl DtlsConnection {
    /// Create a new DTLS connection
    pub async fn new(config: DtlsConfig) -> Result<(Self, mpsc::Receiver<DtlsEvent>)> {
        debug!("Creating DTLS connection using rtp-core");
        
        // Create event channel
        let (tx, rx) = mpsc::channel(100);
        
        // Create connection
        let conn = Self {
            config,
            state: Arc::new(RwLock::new(DtlsConnectionState::New)),
            event_tx: Arc::new(Mutex::new(Some(tx))),
            is_open: Arc::new(RwLock::new(false)),
            inner: None,
        };
        
        Ok((conn, rx))
    }
    
    /// Connect to remote peer
    pub async fn connect(&self, addr: SocketAddr) -> Result<()> {
        debug!("DTLS connect to {}", addr);
        
        // Set connecting state
        *self.state.write().await = DtlsConnectionState::Connecting;
        
        // TODO: Create an actual connection using rtp-core when it's fully implemented
        // For now we'll simulate the connection since rtp-core's DTLS implementation 
        // appears to be unimplemented according to the module file
        
        // Set connected state
        *self.state.write().await = DtlsConnectionState::Connected;
        *self.is_open.write().await = true;
        
        // Send connected event
        if let Some(tx) = &*self.event_tx.lock().await {
            let _ = tx.send(DtlsEvent::Connected).await;
        }
        
        Ok(())
    }
    
    /// Accept connection from remote peer
    pub async fn accept(&self, addr: SocketAddr) -> Result<()> {
        debug!("DTLS accept from {}", addr);
        
        // Set connecting state
        *self.state.write().await = DtlsConnectionState::Connecting;
        
        // TODO: Accept an actual connection using rtp-core when it's fully implemented
        // For now we'll simulate the connection
        
        // Set connected state
        *self.state.write().await = DtlsConnectionState::Connected;
        *self.is_open.write().await = true;
        
        // Send connected event
        if let Some(tx) = &*self.event_tx.lock().await {
            let _ = tx.send(DtlsEvent::Connected).await;
        }
        
        Ok(())
    }
    
    /// Get SRTP keys derived from DTLS handshake
    pub async fn get_srtp_keys(&self) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)> {
        debug!("Get SRTP keys");
        
        // In a real implementation, these would be derived from the DTLS handshake via rtp-core
        // For now, just return dummy keys
        let local_key = vec![0; 16];
        let local_salt = vec![0; 14];
        let remote_key = vec![0; 16];
        let remote_salt = vec![0; 14];
        
        Ok((local_key, local_salt, remote_key, remote_salt))
    }
    
    /// Send data
    pub async fn send(&self, data: &[u8]) -> Result<usize> {
        debug!("DTLS send {} bytes", data.len());
        
        // Check if connection is open
        if !*self.is_open.read().await {
            return Err(Error::InvalidState("DTLS connection not open".to_string()));
        }
        
        // TODO: Implement actual sending when rtp-core is ready
        // Just pretend we sent it for now
        Ok(data.len())
    }
    
    /// Receive data (with timeout)
    pub async fn receive(&self, timeout: Duration) -> Result<Vec<u8>> {
        debug!("DTLS receive with timeout {:?}", timeout);
        
        // Check if connection is open
        if !*self.is_open.read().await {
            return Err(Error::InvalidState("DTLS connection not open".to_string()));
        }
        
        // TODO: Implement actual receiving when rtp-core is ready
        // Just return timeout error for now
        Err(Error::Timeout("DTLS receive timeout".to_string()))
    }
    
    /// Close connection
    pub async fn close(&self) -> Result<()> {
        debug!("DTLS close");
        
        // Set closed state
        *self.state.write().await = DtlsConnectionState::Closed;
        *self.is_open.write().await = false;
        
        // Send closed event
        if let Some(tx) = &*self.event_tx.lock().await {
            let _ = tx.send(DtlsEvent::Closed).await;
        }
        
        Ok(())
    }
    
    /// Check if connection is open
    pub async fn is_open(&self) -> bool {
        *self.is_open.read().await
    }
} 