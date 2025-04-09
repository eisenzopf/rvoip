use std::net::SocketAddr;
use std::sync::Arc;
use std::io;

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};
use bytes::Bytes;
use webrtc_dtls::{
    config::{Config, ExtendedMasterSecretType},
    conn::DTLSConn, cipher_suite::CipherSuiteId,
    crypto::Certificate,
};
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::Conn;

use crate::error::{Error, Result};
use crate::media::srtp::{SrtpKeys, SrtpConfig};
use crate::ice::IceAgent;

/// DTLS configuration
#[derive(Debug, Clone)]
pub struct DtlsConfig {
    /// The DTLS role (client or server)
    pub is_client: bool,
    
    /// Certificate for DTLS
    pub certificate: Option<Certificate>,
    
    /// SRTP protection profile
    pub srtp_profile: ProtectionProfile,
}

impl Default for DtlsConfig {
    fn default() -> Self {
        Self {
            is_client: true, // Default to client role
            certificate: None,
            srtp_profile: ProtectionProfile::Aes128CmHmacSha1_80,
        }
    }
}

/// DTLS event
#[derive(Debug)]
pub enum DtlsEvent {
    /// DTLS connection established
    Connected,
    
    /// SRTP keys derived from DTLS handshake
    SrtpKeysReady(SrtpConfig),
    
    /// DTLS connection closed
    Closed,
    
    /// DTLS error
    Error(String),
}

/// DTLS connection for secure key exchange
pub struct DtlsConnection {
    /// DTLS configuration
    config: DtlsConfig,
    
    /// DTLS connection
    conn: Option<Arc<DTLSConn>>,
    
    /// UDP socket for DTLS
    socket: Arc<RwLock<Option<Arc<UdpSocket>>>>,
    
    /// Local address
    local_addr: SocketAddr,
    
    /// Remote address
    remote_addr: SocketAddr,
    
    /// Event sender
    event_tx: Arc<Mutex<Option<mpsc::Sender<DtlsEvent>>>>,
    
    /// Running flag
    running: Arc<RwLock<bool>>,
}

impl DtlsConnection {
    /// Create a new DTLS connection
    pub async fn new(
        config: DtlsConfig,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> Result<(Self, mpsc::Receiver<DtlsEvent>)> {
        // Create event channel
        let (event_tx, event_rx) = mpsc::channel(100);
        
        // Create connection
        let conn = Self {
            config,
            conn: None,
            socket: Arc::new(RwLock::new(None)),
            local_addr,
            remote_addr,
            event_tx: Arc::new(Mutex::new(Some(event_tx))),
            running: Arc::new(RwLock::new(false)),
        };
        
        Ok((conn, event_rx))
    }
    
    /// Set the ICE agent to use for DTLS
    pub async fn set_ice_agent(&self, ice_agent: Arc<IceAgent>) -> Result<()> {
        // TODO: Implement ICE-DTLS integration
        Ok(())
    }
    
    /// Set the UDP socket to use for DTLS
    pub async fn set_socket(&self, socket: Arc<UdpSocket>) -> Result<()> {
        let mut sock_guard = self.socket.write().await;
        *sock_guard = Some(socket);
        Ok(())
    }
    
    /// Start the DTLS handshake
    pub async fn start(&mut self) -> Result<()> {
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Set running flag
        *self.running.write().await = true;
        
        // Get socket
        let socket = {
            let socket_guard = self.socket.read().await;
            match &*socket_guard {
                Some(socket) => socket.clone(),
                None => return Err(Error::Media("No socket set for DTLS".into())),
            }
        };
        
        // Create DTLS config
        let mut dtls_config = Config::default();
        
        // Set SRTP profile
        dtls_config.srtp_protection_profiles = vec![self.config.srtp_profile];
        
        // Set extended master secret
        dtls_config.extended_master_secret = ExtendedMasterSecretType::Require;
        
        // Set cipher suites (use secure defaults)
        dtls_config.cipher_suites = vec![
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        ];
        
        // Set certificate
        if let Some(cert) = &self.config.certificate {
            dtls_config.certificates = vec![cert.clone()];
        } else {
            // Generate a self-signed certificate
            let cert = Certificate::generate_self_signed(vec!["RVOIP".to_string()])
                .map_err(|e| Error::Media(format!("Failed to generate certificate: {}", e)))?;
            
            dtls_config.certificates = vec![cert];
        }
        
        // Create connection
        let conn = if self.config.is_client {
            // Connect as client
            DTLSConn::connect(Arc::new(socket.clone()), self.remote_addr, dtls_config).await
                .map_err(|e| Error::Media(format!("Failed to connect DTLS: {}", e)))?
        } else {
            // Listen as server
            DTLSConn::accept(Arc::new(socket.clone()), self.remote_addr, dtls_config).await
                .map_err(|e| Error::Media(format!("Failed to accept DTLS: {}", e)))?
        };
        
        // Store connection
        self.conn = Some(Arc::new(conn.clone()));
        
        // Start a task to monitor DTLS state
        let event_tx = self.event_tx.clone();
        let running = self.running.clone();
        
        tokio::spawn(async move {
            // Report connected
            if let Some(tx) = &*event_tx.lock().await {
                let _ = tx.send(DtlsEvent::Connected).await;
            }
            
            // Get SRTP keys
            match conn.get_srtp_protection_profile() {
                Ok(profile) => {
                    // Extract SRTP keys
                    if let Ok((local_key, remote_key)) = conn.export_keying_material("EXTRACTOR-dtls_srtp", &[], 30) {
                        // Split into key and salt
                        // SRTP master key and master salt
                        let remote_master_key = remote_key[0..16].to_vec();
                        let remote_master_salt = remote_key[16..30].to_vec();
                        
                        let local_master_key = local_key[0..16].to_vec();
                        let local_master_salt = local_key[16..30].to_vec();
                        
                        // Create SRTP config
                        let srtp_config = SrtpConfig {
                            local_keys: SrtpKeys::new(local_master_key, local_master_salt),
                            remote_keys: SrtpKeys::new(remote_master_key, remote_master_salt),
                            profile,
                        };
                        
                        // Send SRTP keys
                        if let Some(tx) = &*event_tx.lock().await {
                            let _ = tx.send(DtlsEvent::SrtpKeysReady(srtp_config)).await;
                        }
                    } else {
                        if let Some(tx) = &*event_tx.lock().await {
                            let _ = tx.send(DtlsEvent::Error("Failed to export keying material".into())).await;
                        }
                    }
                },
                Err(e) => {
                    if let Some(tx) = &*event_tx.lock().await {
                        let _ = tx.send(DtlsEvent::Error(format!("No SRTP protection profile: {}", e))).await;
                    }
                },
            }
            
            // Keep connection alive until stopped
            while *running.read().await {
                // Sleep for a bit
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
            
            // Report closed
            if let Some(tx) = &*event_tx.lock().await {
                let _ = tx.send(DtlsEvent::Closed).await;
            }
        });
        
        Ok(())
    }
    
    /// Send data over DTLS
    pub async fn send(&self, data: &[u8]) -> Result<usize> {
        if let Some(conn) = &self.conn {
            conn.write(data).await
                .map_err(|e| Error::Media(format!("Failed to send DTLS data: {}", e)))
        } else {
            Err(Error::Media("DTLS connection not established".into()))
        }
    }
    
    /// Receive data over DTLS
    pub async fn recv(&self, buffer: &mut [u8]) -> Result<usize> {
        if let Some(conn) = &self.conn {
            conn.read(buffer).await
                .map_err(|e| Error::Media(format!("Failed to receive DTLS data: {}", e)))
        } else {
            Err(Error::Media("DTLS connection not established".into()))
        }
    }
    
    /// Close the DTLS connection
    pub async fn close(&self) -> Result<()> {
        // Set running flag to false
        *self.running.write().await = false;
        
        // Close connection
        if let Some(conn) = &self.conn {
            conn.close().await
                .map_err(|e| Error::Media(format!("Failed to close DTLS connection: {}", e)))?;
        }
        
        // Clear event sender
        *self.event_tx.lock().await = None;
        
        Ok(())
    }
} 