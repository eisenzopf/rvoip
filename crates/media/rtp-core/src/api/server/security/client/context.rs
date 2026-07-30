//! Client security context implementation
//!
//! This module handles client security contexts managed by the server.

use async_trait::async_trait;
use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::api::common::config::SecurityInfo;
use crate::api::common::error::SecurityError;
use crate::api::server::security::dtls::handshake;
use crate::api::server::security::{ClientSecurityContext, ServerSecurityConfig, SocketHandle};
use crate::dtls::DtlsConnection;
use crate::srtp::SrtpContext;

/// Client security context managed by the server
pub struct DefaultClientSecurityContext {
    /// Client address
    pub address: SocketAddr,
    /// DTLS connection for this client
    pub connection: Arc<Mutex<Option<DtlsConnection>>>,
    /// SRTP context for secure media with this client
    pub srtp_context: Arc<Mutex<Option<SrtpContext>>>,
    /// Handshake completed flag
    pub handshake_completed: Arc<Mutex<bool>>,
    /// Socket for DTLS
    pub socket: Arc<Mutex<Option<SocketHandle>>>,
    /// Server config (shared)
    pub config: ServerSecurityConfig,
    /// Transport used for DTLS
    pub transport: Arc<Mutex<Option<Arc<Mutex<crate::dtls::transport::udp::UdpTransport>>>>>,
    /// Flag indicating that handshake is waiting for first packet
    pub waiting_for_first_packet: Arc<Mutex<bool>>,
    /// Initial packet from client (if received)
    pub initial_packet: Arc<Mutex<Option<Vec<u8>>>>,
}

impl DefaultClientSecurityContext {
    /// Create a new DefaultClientSecurityContext
    pub fn new(
        address: SocketAddr,
        connection: Option<DtlsConnection>,
        socket: Option<SocketHandle>,
        config: ServerSecurityConfig,
        transport: Option<Arc<Mutex<crate::dtls::transport::udp::UdpTransport>>>,
    ) -> Result<Self, SecurityError> {
        if config.security_mode != crate::api::common::config::SecurityMode::DtlsSrtp {
            return Err(SecurityError::UnsupportedFeature(format!(
                "server-managed DTLS client context cannot implement {:?}",
                config.security_mode
            )));
        }
        config.validate()?;
        Ok(Self {
            address,
            connection: Arc::new(Mutex::new(connection)),
            srtp_context: Arc::new(Mutex::new(None)),
            handshake_completed: Arc::new(Mutex::new(false)),
            socket: Arc::new(Mutex::new(socket)),
            config,
            transport: Arc::new(Mutex::new(transport)),
            initial_packet: Arc::new(Mutex::new(None)),
            waiting_for_first_packet: Arc::new(Mutex::new(false)),
        })
    }

    /// Process a DTLS packet received from the client
    pub async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        let mut conn_guard = self.connection.lock().await;

        if let Some(conn) = conn_guard.as_mut() {
            // Delegate to the handshake module to process the packet
            handshake::process_dtls_packet(
                conn,
                data,
                self.address,
                &self.handshake_completed,
                &self.srtp_context,
            )
            .await
        } else {
            Err(SecurityError::NotInitialized(
                "DTLS connection not initialized for client".to_string(),
            ))
        }
    }

    /// Spawn a task to wait for handshake completion
    pub async fn spawn_handshake_task(&self) -> Result<(), SecurityError> {
        Err(SecurityError::UnsupportedFeature(
            "DTLS handshake tasks are unavailable in 0.3.5".to_string(),
        ))
    }

    /// Start a handshake with the remote
    pub async fn start_handshake_with_remote(
        &self,
        remote_addr: SocketAddr,
    ) -> Result<(), SecurityError> {
        // Access the DTLS connection
        let mut conn_guard = self.connection.lock().await;

        if let Some(conn) = conn_guard.as_mut() {
            // Delegate to the handshake module
            handshake::start_handshake(conn, remote_addr).await
        } else {
            Err(SecurityError::NotInitialized(
                "DTLS connection not initialized".to_string(),
            ))
        }
    }
}

#[async_trait]
impl ClientSecurityContext for DefaultClientSecurityContext {
    async fn set_socket(&self, _socket: SocketHandle) -> Result<(), SecurityError> {
        Err(SecurityError::UnsupportedFeature(
            "DTLS transport setup is unavailable in 0.3.5".to_string(),
        ))
    }

    async fn get_remote_fingerprint(&self) -> Result<Option<String>, SecurityError> {
        let conn = self.connection.lock().await;
        if let Some(conn) = conn.as_ref() {
            // Check if handshake is complete and remote certificate is available
            if let Some(remote_cert) = conn.remote_certificate() {
                // Create a mutable copy of the certificate to compute fingerprint
                let mut remote_cert_copy = remote_cert.clone();
                match remote_cert_copy.fingerprint("SHA-256") {
                    Ok(fingerprint) => Ok(Some(fingerprint)),
                    Err(e) => Err(SecurityError::Internal(format!(
                        "Failed to get remote fingerprint: {}",
                        e
                    ))),
                }
            } else {
                // If no remote certificate yet, return None (not an error)
                Ok(None)
            }
        } else {
            Err(SecurityError::NotInitialized(
                "DTLS connection not initialized".to_string(),
            ))
        }
    }

    /// Wait for the DTLS handshake to complete
    async fn wait_for_handshake(&self) -> Result<(), SecurityError> {
        let mut conn_guard = self.connection.lock().await;

        if let Some(conn) = conn_guard.as_mut() {
            conn.wait_handshake()
                .await
                .map_err(|e| SecurityError::Handshake(format!("DTLS handshake failed: {}", e)))?;

            // Set handshake completed flag
            let mut completed = self.handshake_completed.lock().await;
            *completed = true;

            Ok(())
        } else {
            Err(SecurityError::HandshakeError(
                "No DTLS connection available".to_string(),
            ))
        }
    }

    async fn is_handshake_complete(&self) -> Result<bool, SecurityError> {
        let completed = *self.handshake_completed.lock().await;
        let has_connection = self.connection.lock().await.is_some();
        Ok(completed && has_connection)
    }

    async fn close(&self) -> Result<(), SecurityError> {
        // Close DTLS connection
        let mut conn = self.connection.lock().await;
        if let Some(conn) = conn.as_mut() {
            // Await the future first, then handle the Result
            match conn.close().await {
                Ok(_) => {}
                Err(e) => {
                    return Err(SecurityError::Internal(format!(
                        "Failed to close DTLS connection: {}",
                        e
                    )))
                }
            }
        }
        *conn = None;

        // Reset handshake state
        let mut completed = self.handshake_completed.lock().await;
        *completed = false;

        // Clear SRTP context
        let mut srtp = self.srtp_context.lock().await;
        *srtp = None;

        Ok(())
    }

    fn is_secure(&self) -> bool {
        self.config.security_mode == crate::api::common::config::SecurityMode::DtlsSrtp
            && self
                .connection
                .try_lock()
                .is_ok_and(|connection| connection.is_some())
            && self
                .handshake_completed
                .try_lock()
                .is_ok_and(|completed| *completed)
            && self
                .srtp_context
                .try_lock()
                .is_ok_and(|context| context.is_some())
    }

    fn get_security_info(&self) -> SecurityInfo {
        // A caller can build this historically field-public struct directly.
        // Advertise nothing until both the DTLS handshake and its derived SRTP
        // context are actually present.
        if !self.is_secure() {
            return SecurityInfo::default();
        }
        let crypto_suites =
            crate::api::common::config::implemented_srtp_profile_names(&self.config.srtp_profiles)
                .unwrap_or_default();
        SecurityInfo {
            mode: self.config.security_mode,
            fingerprint: None,
            fingerprint_algorithm: Some(self.config.fingerprint_algorithm.clone()),
            srtp_profile: crypto_suites.first().cloned(),
            crypto_suites,
            key_params: None,
        }
    }

    async fn get_fingerprint(&self) -> Result<String, SecurityError> {
        let conn_guard = self.connection.lock().await;

        if let Some(conn) = conn_guard.as_ref() {
            // Get the certificate from the connection
            if let Some(cert) = conn.local_certificate() {
                // Create a mutable copy of the certificate to compute fingerprint
                let mut cert_copy = cert.clone();
                match cert_copy.fingerprint("SHA-256") {
                    Ok(fingerprint) => Ok(fingerprint),
                    Err(e) => Err(SecurityError::Internal(format!(
                        "Failed to get fingerprint: {}",
                        e
                    ))),
                }
            } else {
                Err(SecurityError::Configuration(
                    "No certificate available".to_string(),
                ))
            }
        } else {
            Err(SecurityError::NotInitialized(
                "DTLS connection not initialized".to_string(),
            ))
        }
    }

    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError> {
        // Return the default algorithm used
        Ok("sha-256".to_string())
    }

    /// Process a DTLS packet received from the client
    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        self.process_dtls_packet(data).await
    }

    /// Start a handshake with the remote
    async fn start_handshake_with_remote(
        &self,
        remote_addr: SocketAddr,
    ) -> Result<(), SecurityError> {
        self.start_handshake_with_remote(remote_addr).await
    }

    /// Allow downcasting for internal implementation details
    fn as_any(&self) -> &dyn Any {
        self
    }
}
