//! Security Manager
//!
//! This module retains the high-level security-manager API. DTLS-SRTP is
//! unavailable in 0.3.5, and its direct-PSK setup helpers are also unavailable
//! because these manager contexts do not install crypto into a media transport.

use crate::api::client::security::srtp::SrtpClientSecurityContext;
use crate::api::client::security::{ClientSecurityContext, DefaultClientSecurityContext};
use crate::api::common::config::{SecurityConfig, SecurityMode};
use crate::api::common::error::SecurityError;
use crate::api::server::security::srtp::SrtpServerSecurityContext;
use crate::api::server::security::{
    DefaultServerSecurityContext, ServerSecurityContext, SocketHandle,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// High-level security manager for RTP transport
///
/// The configuration and method names are retained for compatibility. Use the
/// default media client/server for direct PSK SRTP, or the unified security
/// context's protect/unprotect API. This manager fails closed instead of
/// reporting a configuration-only context as an active secure transport.
pub struct SecurityManager {
    config: SecurityConfig,
    client_context: Option<Arc<dyn ClientSecurityContext>>,
    server_context: Option<Arc<dyn ServerSecurityContext>>,
}

impl SecurityManager {
    /// Create a new SecurityManager with the given configuration
    pub fn new(config: SecurityConfig) -> Self {
        Self {
            config,
            client_context: None,
            server_context: None,
        }
    }

    /// Create a retained WebRTC/DTLS-SRTP manager configuration.
    ///
    /// DTLS-SRTP is unavailable in 0.3.5; setup and handshake methods return a
    /// typed unsupported-feature error.
    pub fn webrtc_compatible() -> Self {
        Self::new(SecurityConfig::webrtc_compatible())
    }

    /// Create a SecurityManager with no security
    pub fn unsecured() -> Self {
        Self::new(SecurityConfig::unsecured())
    }

    /// Create a SecurityManager with SRTP using a pre-shared key
    pub fn with_srtp_key(key: Vec<u8>) -> Self {
        Self::new(SecurityConfig::srtp_with_key(key))
    }

    fn validate_setup_request(&self) -> Result<(), SecurityError> {
        self.config.validate()?;
        if self.config.mode == SecurityMode::Srtp {
            return Err(SecurityError::UnsupportedFeature(
                "SecurityManager direct-PSK setup does not install SRTP crypto into the media transport; use the default media transport or unified protect/unprotect API"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Set up a server for secure communications
    pub async fn setup_server(
        &mut self,
        socket_addr: SocketAddr,
    ) -> Result<Arc<dyn ServerSecurityContext>, SecurityError> {
        self.validate_setup_request()?;
        // Create a security context for the server
        let server_security_config = match self.config.mode {
            SecurityMode::None => {
                return Err(SecurityError::Configuration(
                    "Cannot set up security for 'None' mode".to_string(),
                ));
            }
            SecurityMode::Srtp => {
                // Basic SRTP configuration
                crate::api::server::security::ServerSecurityConfig {
                    security_mode: SecurityMode::Srtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: None,
                    private_key_path: None,
                    require_client_certificate: false,
                    srtp_key: self.config.srtp_key.clone(),
                }
            }
            SecurityMode::DtlsSrtp => {
                // DTLS-SRTP configuration
                crate::api::server::security::ServerSecurityConfig {
                    security_mode: SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: self.config.certificate_path.clone(),
                    private_key_path: self.config.private_key_path.clone(),
                    require_client_certificate: self.config.require_client_certificate,
                    srtp_key: None, // Not used for DTLS-SRTP
                }
            }
            SecurityMode::SdesSrtp | SecurityMode::MikeySrtp | SecurityMode::ZrtpSrtp => {
                // SIP-derived SRTP methods use the unified security context instead
                return Err(SecurityError::Configuration(
                    "SIP-derived SRTP methods should use SecurityContextManager".to_string(),
                ));
            }
        };

        // Create the server security context
        let server_ctx: Arc<dyn ServerSecurityContext> = match self.config.mode {
            SecurityMode::Srtp => SrtpServerSecurityContext::new(server_security_config).await?,
            SecurityMode::DtlsSrtp => {
                DefaultServerSecurityContext::new(server_security_config).await?
            }
            mode => {
                return Err(SecurityError::UnsupportedFeature(format!(
                    "server context for {mode:?} is not implemented"
                )))
            }
        };

        // Bind a socket for DTLS
        let socket =
            Arc::new(UdpSocket::bind(socket_addr).await.map_err(|e| {
                SecurityError::Configuration(format!("Failed to bind socket: {}", e))
            })?);

        // Create a socket handle for the server
        let socket_handle = SocketHandle {
            socket: socket.clone(),
            remote_addr: None,
        };

        // Set the socket on the server context
        server_ctx.set_socket(socket_handle).await?;

        // Start listening for connections
        server_ctx.start_listening().await?;

        // Start the packet handler for automatic DTLS processing
        if self.config.mode == SecurityMode::DtlsSrtp {
            server_ctx.start_packet_handler().await?;
        }

        // Store and return the context
        self.server_context = Some(server_ctx.clone());
        Ok(server_ctx)
    }

    /// Set up a server for secure communications with an existing socket
    pub async fn setup_server_with_socket(
        &mut self,
        socket: Arc<UdpSocket>,
    ) -> Result<Arc<dyn ServerSecurityContext>, SecurityError> {
        self.validate_setup_request()?;
        // Create a security context for the server
        let server_security_config = match self.config.mode {
            SecurityMode::None => {
                return Err(SecurityError::Configuration(
                    "Cannot set up security for 'None' mode".to_string(),
                ));
            }
            SecurityMode::Srtp => {
                // Basic SRTP configuration
                crate::api::server::security::ServerSecurityConfig {
                    security_mode: SecurityMode::Srtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: None,
                    private_key_path: None,
                    require_client_certificate: false,
                    srtp_key: self.config.srtp_key.clone(),
                }
            }
            SecurityMode::DtlsSrtp => {
                // DTLS-SRTP configuration
                crate::api::server::security::ServerSecurityConfig {
                    security_mode: SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: self.config.certificate_path.clone(),
                    private_key_path: self.config.private_key_path.clone(),
                    require_client_certificate: self.config.require_client_certificate,
                    srtp_key: None, // Not used for DTLS-SRTP
                }
            }
            SecurityMode::SdesSrtp | SecurityMode::MikeySrtp | SecurityMode::ZrtpSrtp => {
                // SIP-derived SRTP methods use the unified security context instead
                return Err(SecurityError::Configuration(
                    "SIP-derived SRTP methods should use SecurityContextManager".to_string(),
                ));
            }
        };

        // Create the server security context
        let server_ctx: Arc<dyn ServerSecurityContext> = match self.config.mode {
            SecurityMode::Srtp => SrtpServerSecurityContext::new(server_security_config).await?,
            SecurityMode::DtlsSrtp => {
                DefaultServerSecurityContext::new(server_security_config).await?
            }
            mode => {
                return Err(SecurityError::UnsupportedFeature(format!(
                    "server context for {mode:?} is not implemented"
                )))
            }
        };

        // Create a socket handle for the server
        let socket_handle = SocketHandle {
            socket: socket.clone(),
            remote_addr: None,
        };

        // Set the socket on the server context
        server_ctx.set_socket(socket_handle).await?;

        // Start listening for connections
        server_ctx.start_listening().await?;

        // Start the packet handler for automatic DTLS processing
        if self.config.mode == SecurityMode::DtlsSrtp {
            server_ctx.start_packet_handler().await?;
        }

        // Store and return the context
        self.server_context = Some(server_ctx.clone());
        Ok(server_ctx)
    }

    /// Set up a client for secure communications
    pub async fn setup_client(
        &mut self,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ClientSecurityContext>, SecurityError> {
        self.validate_setup_request()?;
        // Create a security context for the client
        let client_security_config = match self.config.mode {
            SecurityMode::None => {
                return Err(SecurityError::Configuration(
                    "Cannot set up security for 'None' mode".to_string(),
                ));
            }
            SecurityMode::Srtp => {
                // Basic SRTP configuration
                crate::api::client::security::ClientSecurityConfig {
                    security_mode: SecurityMode::Srtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                    validate_fingerprint: false,
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: None,
                    private_key_path: None,
                    srtp_key: self.config.srtp_key.clone(),
                }
            }
            SecurityMode::DtlsSrtp => {
                // DTLS-SRTP configuration
                crate::api::client::security::ClientSecurityConfig {
                    security_mode: SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    remote_fingerprint: self.config.remote_fingerprint.clone(),
                    remote_fingerprint_algorithm: self.config.remote_fingerprint_algorithm.clone(),
                    validate_fingerprint: true,
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: self.config.certificate_path.clone(),
                    private_key_path: self.config.private_key_path.clone(),
                    srtp_key: None, // Not used for DTLS-SRTP
                }
            }
            SecurityMode::SdesSrtp | SecurityMode::MikeySrtp | SecurityMode::ZrtpSrtp => {
                // SIP-derived SRTP methods use the unified security context instead
                return Err(SecurityError::Configuration(
                    "SIP-derived SRTP methods should use SecurityContextManager".to_string(),
                ));
            }
        };

        // Create the client security context
        let client_ctx: Arc<dyn ClientSecurityContext> = match self.config.mode {
            SecurityMode::Srtp => SrtpClientSecurityContext::new(client_security_config).await?,
            SecurityMode::DtlsSrtp => {
                DefaultClientSecurityContext::new(client_security_config).await?
            }
            mode => {
                return Err(SecurityError::UnsupportedFeature(format!(
                    "client context for {mode:?} is not implemented"
                )))
            }
        };

        // Bind a socket for DTLS
        let local_addr = SocketAddr::new(remote_addr.ip(), 0); // Use ephemeral port
        let socket =
            Arc::new(UdpSocket::bind(local_addr).await.map_err(|e| {
                SecurityError::Configuration(format!("Failed to bind socket: {}", e))
            })?);

        // Create a socket handle for the client
        let socket_handle = SocketHandle {
            socket: socket.clone(),
            remote_addr: Some(remote_addr),
        };

        // Set the socket on the client context
        client_ctx.set_socket(socket_handle).await?;

        // Set remote address
        client_ctx.set_remote_address(remote_addr).await?;

        // Initialize the client context
        client_ctx.initialize().await?;

        // Store and return the context
        self.client_context = Some(client_ctx.clone());
        Ok(client_ctx)
    }

    /// Set up a client for secure communications with an existing socket
    pub async fn setup_client_with_socket(
        &mut self,
        socket: Arc<UdpSocket>,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ClientSecurityContext>, SecurityError> {
        self.validate_setup_request()?;
        // Create a security context for the client
        let client_security_config = match self.config.mode {
            SecurityMode::None => {
                return Err(SecurityError::Configuration(
                    "Cannot set up security for 'None' mode".to_string(),
                ));
            }
            SecurityMode::Srtp => {
                // Basic SRTP configuration
                crate::api::client::security::ClientSecurityConfig {
                    security_mode: SecurityMode::Srtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                    validate_fingerprint: false,
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: None,
                    private_key_path: None,
                    srtp_key: self.config.srtp_key.clone(),
                }
            }
            SecurityMode::DtlsSrtp => {
                // DTLS-SRTP configuration
                crate::api::client::security::ClientSecurityConfig {
                    security_mode: SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: self.config.fingerprint_algorithm.clone(),
                    remote_fingerprint: self.config.remote_fingerprint.clone(),
                    remote_fingerprint_algorithm: self.config.remote_fingerprint_algorithm.clone(),
                    validate_fingerprint: true,
                    srtp_profiles: self.config.srtp_profiles.clone(),
                    certificate_path: self.config.certificate_path.clone(),
                    private_key_path: self.config.private_key_path.clone(),
                    srtp_key: None, // Not used for DTLS-SRTP
                }
            }
            SecurityMode::SdesSrtp | SecurityMode::MikeySrtp | SecurityMode::ZrtpSrtp => {
                // SIP-derived SRTP methods use the unified security context instead
                return Err(SecurityError::Configuration(
                    "SIP-derived SRTP methods should use SecurityContextManager".to_string(),
                ));
            }
        };

        // Create the client security context
        let client_ctx: Arc<dyn ClientSecurityContext> = match self.config.mode {
            SecurityMode::Srtp => SrtpClientSecurityContext::new(client_security_config).await?,
            SecurityMode::DtlsSrtp => {
                DefaultClientSecurityContext::new(client_security_config).await?
            }
            mode => {
                return Err(SecurityError::UnsupportedFeature(format!(
                    "client context for {mode:?} is not implemented"
                )))
            }
        };

        // Create a socket handle for the client
        let socket_handle = SocketHandle {
            socket: socket.clone(),
            remote_addr: Some(remote_addr),
        };

        // Set the socket on the client context
        client_ctx.set_socket(socket_handle).await?;

        // Set remote address
        client_ctx.set_remote_address(remote_addr).await?;

        // Initialize the client context
        client_ctx.initialize().await?;

        // Store and return the context
        self.client_context = Some(client_ctx.clone());
        Ok(client_ctx)
    }

    /// Retained DTLS handshake entry point.
    ///
    /// DTLS-SRTP is unavailable in 0.3.5, so this returns a typed
    /// unsupported-feature error before examining or mutating manager state.
    pub async fn perform_handshake(
        &self,
        remote_fingerprint: Option<String>,
    ) -> Result<(), SecurityError> {
        self.config.validate()?;
        if self.config.mode != SecurityMode::DtlsSrtp {
            return Err(SecurityError::Configuration(
                "Handshake only applicable for DTLS-SRTP mode".to_string(),
            ));
        }

        // Get client and server contexts
        let client_ctx = match &self.client_context {
            Some(ctx) => ctx,
            None => {
                return Err(SecurityError::NotInitialized(
                    "Client context not initialized".to_string(),
                ))
            }
        };

        let server_ctx = match &self.server_context {
            Some(ctx) => ctx,
            None => {
                // It's okay if we don't have a server context - we're operating as a client only
                return self
                    .perform_client_handshake(client_ctx, remote_fingerprint)
                    .await;
            }
        };

        // We have both client and server - this is the local testing case
        println!("SecurityManager: Performing local handshake (client and server in same process)");

        // If we have the remote fingerprint, set it on the client
        if let Some(fingerprint) =
            remote_fingerprint.or_else(|| self.config.remote_fingerprint.clone())
        {
            let algorithm = self
                .config
                .remote_fingerprint_algorithm
                .clone()
                .unwrap_or_else(|| self.config.fingerprint_algorithm.clone());

            client_ctx
                .set_remote_fingerprint(&fingerprint, &algorithm)
                .await?;
        }

        // Following dtls_test.rs exactly:
        // 1. Start the packet handlers FIRST
        // 2. Make sure server is ready before client starts handshake
        // 3. Client initiates handshake with proper sequencing

        // First, ensure server is ready to process packets
        println!("SecurityManager: Starting server packet handler");
        server_ctx.start_packet_handler().await?;

        // Verify server is fully ready - new addition based on dtls_test.rs
        if !server_ctx.is_ready().await? {
            println!("SecurityManager: Waiting for server to be ready...");
            // Small delay to let server fully initialize
            let max_attempts = 30; // 3 seconds total
            for _ in 0..max_attempts {
                if server_ctx.is_ready().await? {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            if !server_ctx.is_ready().await? {
                return Err(SecurityError::Timeout(
                    "Server not ready after waiting".to_string(),
                ));
            }
        }
        println!("SecurityManager: Server is ready to receive handshake messages");

        // Next, start client packet handling
        println!("SecurityManager: Starting client packet handler");
        client_ctx.start_packet_handler().await?;

        // Verify client is ready
        if !client_ctx.is_ready().await? {
            println!("SecurityManager: Waiting for client to be ready...");
            let max_attempts = 30; // 3 seconds total
            for _ in 0..max_attempts {
                if client_ctx.is_ready().await? {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            if !client_ctx.is_ready().await? {
                return Err(SecurityError::Timeout(
                    "Client not ready after waiting".to_string(),
                ));
            }
        }
        println!("SecurityManager: Client is ready to start handshake");

        // NOW start the handshake - AFTER transport is ready
        println!("SecurityManager: Starting client handshake");
        client_ctx.start_handshake().await?;

        // Poll for handshake completion with timeout
        let start_time = std::time::Instant::now();
        let handshake_timeout = std::time::Duration::from_secs(15);

        println!("SecurityManager: Waiting for handshake to complete");

        while !client_ctx.is_handshake_complete().await? {
            if start_time.elapsed() > handshake_timeout {
                return Err(SecurityError::Timeout(
                    "Handshake timed out after 15 seconds".to_string(),
                ));
            }

            // Small delay between checks
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            // Debugging info every 5 seconds for first handshake
            let elapsed_secs = start_time.elapsed().as_secs();
            if elapsed_secs > 0 && elapsed_secs % 5 == 0 {
                println!(
                    "SecurityManager: Handshake in progress... ({:?} elapsed)",
                    start_time.elapsed()
                );
            }
        }

        println!("SecurityManager: DTLS handshake completed successfully");
        return Ok(());
    }

    /// Perform a client-only handshake (when we don't have a server context)
    async fn perform_client_handshake(
        &self,
        client_ctx: &Arc<dyn ClientSecurityContext>,
        remote_fingerprint: Option<String>,
    ) -> Result<(), SecurityError> {
        // If we have the remote fingerprint, set it on the client
        if let Some(fingerprint) =
            remote_fingerprint.or_else(|| self.config.remote_fingerprint.clone())
        {
            let algorithm = self
                .config
                .remote_fingerprint_algorithm
                .clone()
                .unwrap_or_else(|| self.config.fingerprint_algorithm.clone());

            client_ctx
                .set_remote_fingerprint(&fingerprint, &algorithm)
                .await?;
        }

        // Following dtls_test.rs sequence:
        // 1. Start packet handler FIRST to ensure transport is ready
        // 2. THEN start handshake after transport is ready

        // Start the packet handler first so it's ready to process responses
        println!("SecurityManager: Starting client packet handler");
        client_ctx.start_packet_handler().await?;

        // Verify client is ready before starting handshake
        if !client_ctx.is_ready().await? {
            println!("SecurityManager: Waiting for client to be ready...");
            let max_attempts = 30; // 3 seconds total
            for _ in 0..max_attempts {
                if client_ctx.is_ready().await? {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            if !client_ctx.is_ready().await? {
                return Err(SecurityError::Timeout(
                    "Client not ready after waiting".to_string(),
                ));
            }
        }
        println!("SecurityManager: Client is ready to start handshake");

        // Then start the handshake - AFTER we've verified readiness
        println!("SecurityManager: Starting client handshake");
        client_ctx.start_handshake().await?;

        // Poll for handshake completion with timeout
        let start_time = std::time::Instant::now();
        let handshake_timeout = std::time::Duration::from_secs(15);

        println!("SecurityManager: Waiting for handshake to complete");

        while !client_ctx.is_handshake_complete().await? {
            if start_time.elapsed() > handshake_timeout {
                return Err(SecurityError::Timeout(
                    "Handshake timed out after 15 seconds".to_string(),
                ));
            }

            // Small delay between checks
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            // Debugging info every 5 seconds for first handshake
            let elapsed_secs = start_time.elapsed().as_secs();
            if elapsed_secs > 0 && elapsed_secs % 5 == 0 {
                println!(
                    "SecurityManager: Handshake in progress... ({:?} elapsed)",
                    start_time.elapsed()
                );
            }
        }

        println!("SecurityManager: DTLS handshake completed successfully");
        return Ok(());
    }

    /// Get the client's fingerprint
    pub async fn get_client_fingerprint(&self) -> Result<String, SecurityError> {
        match &self.client_context {
            Some(ctx) => ctx.get_fingerprint().await,
            None => Err(SecurityError::NotInitialized(
                "Client context not initialized".to_string(),
            )),
        }
    }

    /// Get the server's fingerprint
    pub async fn get_server_fingerprint(&self) -> Result<String, SecurityError> {
        match &self.server_context {
            Some(ctx) => ctx.get_fingerprint().await,
            None => Err(SecurityError::NotInitialized(
                "Server context not initialized".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srtp_config() -> SecurityConfig {
        SecurityConfig::srtp_with_key(vec![0x42; 30])
    }

    #[tokio::test]
    async fn all_security_manager_psk_setup_paths_fail_before_state_mutation() {
        let bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));

        let mut server_manager = SecurityManager::new(srtp_config());
        assert!(matches!(
            server_manager.setup_server(bind_addr).await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(server_manager.server_context.is_none());

        let server_socket = Arc::new(UdpSocket::bind(bind_addr).await.unwrap());
        let mut existing_server_manager = SecurityManager::new(srtp_config());
        assert!(matches!(
            existing_server_manager
                .setup_server_with_socket(server_socket)
                .await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(existing_server_manager.server_context.is_none());

        let remote = SocketAddr::from(([127, 0, 0, 1], 9));
        let mut client_manager = SecurityManager::new(srtp_config());
        assert!(matches!(
            client_manager.setup_client(remote).await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(client_manager.client_context.is_none());

        let client_socket = Arc::new(UdpSocket::bind(bind_addr).await.unwrap());
        let mut existing_client_manager = SecurityManager::new(srtp_config());
        assert!(matches!(
            existing_client_manager
                .setup_client_with_socket(client_socket, remote)
                .await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(existing_client_manager.client_context.is_none());
    }

    #[tokio::test]
    async fn retained_dtls_handshake_entry_point_returns_typed_unsupported() {
        let manager = SecurityManager::webrtc_compatible();
        assert!(matches!(
            manager.perform_handshake(None).await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }
}
