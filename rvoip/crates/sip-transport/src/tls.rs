use std::net::SocketAddr;
use std::sync::Arc;
use std::io;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{self, ServerConfig, Certificate, PrivateKey};
use tokio_rustls::server::TlsStream;
use tracing::{debug, error, info, warn};
use bytes::Bytes;

use crate::{Transport, TransportEvent};
use crate::error::{Error, Result};

/// TLS transport implementation for SIP
pub struct TlsTransport {
    /// Local address
    local_addr: SocketAddr,
    
    /// TLS acceptor
    acceptor: TlsAcceptor,
    
    /// TLS connections
    connections: Arc<tokio::sync::Mutex<Vec<(SocketAddr, mpsc::Sender<Bytes>)>>>,
    
    /// Transport event sender
    event_tx: Option<mpsc::Sender<TransportEvent>>,
    
    /// Closed flag
    closed: Arc<AtomicBool>,
}

impl fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsTransport")
            .field("local_addr", &self.local_addr)
            .field("connections", &self.connections)
            .field("closed", &self.closed)
            .finish()
    }
}

impl TlsTransport {
    /// Create a new TLS transport
    pub async fn bind(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Load TLS certificate and key
        let cert = load_certs(cert_path)?;
        let key = load_private_key(key_path)?;
        
        // Create TLS config
        let config = ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert, key)
            .map_err(|e| Error::TlsError(format!("TLS config error: {}", e)))?;
        
        // Create TLS acceptor
        let acceptor = TlsAcceptor::from(Arc::new(config));
        
        // Create transport event channel if not provided
        let (tx, rx) = if let Some(tx) = event_tx {
            (tx, mpsc::channel::<TransportEvent>(100).1)
        } else {
            mpsc::channel::<TransportEvent>(100)
        };
        
        // Create TLS transport
        let transport = Self {
            local_addr,
            acceptor,
            connections: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            event_tx: Some(tx),
            closed: Arc::new(AtomicBool::new(false)),
        };
        
        // Start listening
        tokio::spawn(Self::listen(
            local_addr, 
            transport.acceptor.clone(),
            transport.connections.clone(),
            transport.event_tx.clone().unwrap(),
        ));
        
        Ok((transport, rx))
    }
    
    /// Listen for incoming connections
    async fn listen(
        addr: SocketAddr,
        acceptor: TlsAcceptor,
        connections: Arc<tokio::sync::Mutex<Vec<(SocketAddr, mpsc::Sender<Bytes>)>>>,
        event_tx: mpsc::Sender<TransportEvent>,
    ) {
        // Create TCP listener
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind TLS listener to {}: {}", addr, e);
                let _ = event_tx.send(TransportEvent::Error {
                    error: format!("Failed to bind TLS listener: {}", e),
                }).await;
                return;
            }
        };
        
        info!("TLS transport listening on {}", addr);
        
        // Accept connections
        loop {
            match listener.accept().await {
                Ok((stream, remote_addr)) => {
                    debug!("New TCP connection from {}", remote_addr);
                    
                    // Clone resources for the connection handler
                    let acceptor = acceptor.clone();
                    let connections = connections.clone();
                    let event_tx = event_tx.clone();
                    let local_addr = addr;
                    
                    // Handle the connection in a new task
                    tokio::spawn(async move {
                        // Perform TLS handshake
                        match acceptor.accept(stream).await {
                            Ok(tls_stream) => {
                                debug!("TLS handshake with {} successful", remote_addr);
                                Self::handle_connection(tls_stream, remote_addr, local_addr, connections, event_tx).await;
                            },
                            Err(e) => {
                                error!("TLS handshake with {} failed: {}", remote_addr, e);
                            }
                        }
                    });
                },
                Err(e) => {
                    error!("Failed to accept TCP connection: {}", e);
                }
            }
        }
    }
    
    /// Handle a TLS connection
    async fn handle_connection(
        tls_stream: TlsStream<TcpStream>,
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
        connections: Arc<tokio::sync::Mutex<Vec<(SocketAddr, mpsc::Sender<Bytes>)>>>,
        event_tx: mpsc::Sender<TransportEvent>,
    ) {
        // Split the stream into read and write parts
        let (mut reader, mut writer) = tokio::io::split(tls_stream);
        
        // Create a channel for sending data to the write half
        let (tx, mut rx) = mpsc::channel::<Bytes>(100);
        
        // Store the connection
        {
            let mut connections_guard = connections.lock().await;
            connections_guard.push((remote_addr, tx.clone()));
        }
        
        // Spawn a task for the write half
        let write_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                if let Err(e) = writer.write_all(&data).await {
                    error!("Failed to write to TLS stream: {}", e);
                    break;
                }
            }
        });
        
        // Handle the read half
        let mut buffer = vec![0u8; 8192];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => {
                    // Connection closed
                    break;
                },
                Ok(n) => {
                    // Got data, process it
                    let data = buffer[..n].to_vec();
                    
                    // Attempt to parse the buffered data as a SIP message
                    let data_clone = data.clone(); // Clone data for parsing attempt
                    let result = tokio::task::block_in_place(|| {
                        // Call the standalone parse function
                        rvoip_sip_core::parse_message(&data_clone)
                    });

                    match result {
                        Ok(msg) => {
                            // Forward the data as a transport event
                            let _ = event_tx.send(TransportEvent::MessageReceived {
                                message: msg,
                                source: remote_addr,
                                destination: local_addr,
                            }).await;
                        },
                        Err(e) => {
                            error!("Failed to parse SIP message: {}", e);
                        }
                    }
                },
                Err(e) => {
                    error!("Failed to read from TLS stream: {}", e);
                    break;
                }
            }
        }
        
        // Connection closed, clean up
        write_task.abort();
        
        // Remove the connection
        {
            let mut connections_guard = connections.lock().await;
            connections_guard.retain(|(addr, _)| *addr != remote_addr);
        }
        
        debug!("TLS connection closed: {}", remote_addr);
    }
    
    /// Send data to a specific remote address
    async fn send_to_addr(&self, data: Bytes, addr: SocketAddr) -> io::Result<()> {
        let connections_guard = self.connections.lock().await;
        
        // Find connection for this address
        if let Some((_, tx)) = connections_guard.iter().find(|(a, _)| *a == addr) {
            // Send data
            if let Err(_) = tx.send(data).await {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Failed to send to TLS connection"));
            }
            Ok(())
        } else {
            // No existing connection, try to establish one
            Err(io::Error::new(io::ErrorKind::NotConnected, "No TLS connection to target"))
        }
    }
    
    /// Connect to a remote address
    pub async fn connect(&self, remote_addr: SocketAddr) -> Result<()> {
        // Check if we already have a connection
        {
            let connections_guard = self.connections.lock().await;
            if connections_guard.iter().any(|(addr, _)| *addr == remote_addr) {
                return Ok(());
            }
        }
        
        // Connect to remote (not implemented yet)
        Err(Error::NotImplemented("TLS client connection not implemented yet".to_string()))
    }
}

#[async_trait]
impl Transport for TlsTransport {
    /// Send a message
    async fn send_message(&self, message: rvoip_sip_core::Message, destination: SocketAddr) -> crate::error::Result<()> {
        // Convert message to bytes
        let bytes = message.to_string().into_bytes();
        
        // Send to destination
        self.send_to_addr(bytes.into(), destination).await
            .map_err(|e| crate::error::Error::IoError(e.to_string()))
    }
    
    /// Get the local address
    fn local_addr(&self) -> crate::error::Result<SocketAddr> {
        Ok(self.local_addr)
    }
    
    /// Close the transport
    async fn close(&self) -> crate::error::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
    
    /// Check if the transport is closed
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// Helper function to load TLS certificates
fn load_certs(path: &Path) -> Result<Vec<Certificate>> {
    // Load certificate file
    let mut cert_file = File::open(path)
        .map_err(|e| Error::IoError(format!("Failed to open certificate file: {}", e)))?;
    
    // Read certificate data
    let mut cert_data = Vec::new();
    cert_file.read_to_end(&mut cert_data)
        .map_err(|e| Error::IoError(format!("Failed to read certificate file: {}", e)))?;
    
    // Parse PEM certificates
    let certs = rustls_pemfile::certs(&mut cert_data.as_slice())
        .map_err(|_| Error::TlsError("Failed to parse certificate".into()))?
        .iter()
        .map(|v| Certificate(v.clone()))
        .collect();
    
    Ok(certs)
}

/// Helper function to load private key
fn load_private_key(path: &Path) -> Result<PrivateKey> {
    // Load key file
    let mut key_file = File::open(path)
        .map_err(|e| Error::IoError(format!("Failed to open key file: {}", e)))?;
    
    // Read key data
    let mut key_data = Vec::new();
    key_file.read_to_end(&mut key_data)
        .map_err(|e| Error::IoError(format!("Failed to read key file: {}", e)))?;
    
    // Parse PEM key
    let keys = rustls_pemfile::pkcs8_private_keys(&mut key_data.as_slice())
        .map_err(|_| Error::TlsError("Failed to parse private key".into()))?;
    
    if keys.is_empty() {
        return Err(Error::TlsError("No private keys found".into()));
    }
    
    Ok(PrivateKey(keys[0].clone()))
} 