//! Registration operations for ClientManager

use std::sync::Arc;
use uuid::Uuid;
use std::time::Duration;

use crate::{
    ClientResult, ClientError,
    call::CallId,
    registration::{RegistrationConfig, RegistrationInfo},
    events::ClientEvent,
    client::recovery::{RetryConfig, retry_with_backoff, ErrorContext},
};
use rvoip_session_core::api::SipClient;

use super::ClientManager;

impl ClientManager {
    /// Register with a SIP server
    /// 
    /// This method registers the client with a SIP server using the REGISTER method.
    /// Registration allows the client to receive incoming calls and establishes its
    /// presence on the SIP network. The method handles authentication challenges
    /// automatically and includes retry logic for network issues.
    /// 
    /// # Arguments
    /// 
    /// * `config` - Registration configuration including server URI, user credentials,
    ///              and expiration settings
    /// 
    /// # Returns
    /// 
    /// Returns a `Uuid` that uniquely identifies this registration for future operations.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::AuthenticationFailed` - Invalid credentials or auth challenge failed
    /// * `ClientError::RegistrationFailed` - Server rejected registration (403, etc.)
    /// * `ClientError::NetworkError` - Network timeout or connectivity issues
    /// 
    /// # Examples
    /// 
    /// ## Basic Registration
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn basic_registration() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5073".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:sip.example.com:5060".to_string(),
    ///         from_uri: "sip:alice@example.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5073".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     println!("✅ Registered with ID: {}", reg_id);
    ///     
    ///     client.unregister(reg_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Registration with Authentication
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn authenticated_registration() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5074".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:pbx.company.com".to_string(),
    ///         from_uri: "sip:user@company.com".to_string(),
    ///         contact_uri: "sip:user@127.0.0.1:5074".to_string(),
    ///         expires: 1800, // 30 minutes
    ///         username: Some("user".to_string()),
    ///         password: Some("password123".to_string()),
    ///         realm: Some("company.com".to_string()),
    ///     };
    ///     
    ///     match client.register(reg_config).await {
    ///         Ok(reg_id) => {
    ///             println!("✅ Authenticated registration successful: {}", reg_id);
    ///             client.unregister(reg_id).await?;
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Registration failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Multiple Registrations
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn multiple_registrations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5075".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Register with multiple servers
    ///     let reg1_config = RegistrationConfig {
    ///         server_uri: "sip:server1.com".to_string(),
    ///         from_uri: "sip:alice@server1.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5075".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg2_config = RegistrationConfig {
    ///         server_uri: "sip:server2.com".to_string(),
    ///         from_uri: "sip:alice@server2.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5075".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg1_id = client.register(reg1_config).await?;
    ///     let reg2_id = client.register(reg2_config).await?;
    ///     
    ///     println!("✅ Registered with {} servers", 2);
    ///     
    ///     // Check all registrations
    ///     let all_regs = client.get_all_registrations().await;
    ///     assert_eq!(all_regs.len(), 2);
    ///     
    ///     // Clean up
    ///     client.unregister(reg1_id).await?;
    ///     client.unregister(reg2_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        // Use SipClient trait to register with retry logic for network errors.
        // If credentials are provided, use register_with_credentials for 401/407 handling.
        let has_credentials = config.username.is_some() && config.password.is_some();
        let config_clone = config.clone();
        let registration_handle = retry_with_backoff(
            "sip_registration",
            RetryConfig::slow(),  // Use slower retry for registration
            || {
                let cfg = config_clone.clone();
                let coord = self.coordinator.clone();
                async move {
                    let result = if has_credentials {
                        let username = cfg.username.as_deref().unwrap_or_default();
                        let password = cfg.password.as_deref().unwrap_or_default();
                        SipClient::register_with_credentials(
                            &coord,
                            &cfg.server_uri,
                            &cfg.from_uri,
                            &cfg.contact_uri,
                            cfg.expires,
                            username,
                            password,
                        ).await
                    } else {
                        SipClient::register(
                            &coord,
                            &cfg.server_uri,
                            &cfg.from_uri,
                            &cfg.contact_uri,
                            cfg.expires,
                        ).await
                    };
                    result.map_err(|e| {
                        // Categorize the error properly based on response
                        let error_msg = e.to_string();
                        if error_msg.contains("401") || error_msg.contains("407") {
                            ClientError::AuthenticationFailed {
                                reason: format!("Authentication required: {}", e)
                            }
                        } else if error_msg.contains("timeout") {
                            ClientError::NetworkError {
                                reason: format!("Registration timeout: {}", e)
                            }
                        } else if error_msg.contains("403") {
                            ClientError::RegistrationFailed {
                                reason: format!("Registration forbidden: {}", e)
                            }
                        } else {
                            ClientError::RegistrationFailed {
                                reason: format!("Registration failed: {}", e)
                            }
                        }
                    })
                }
            }
        )
        .await
        .with_context(|| format!("Failed to register {} with {}", config.from_uri, config.server_uri))?;
        
        // Create registration info
        let reg_id = Uuid::new_v4();
        let registration_info = RegistrationInfo {
            id: reg_id,
            server_uri: config.server_uri.clone(),
            from_uri: config.from_uri.clone(),
            contact_uri: config.contact_uri.clone(),
            expires: config.expires,
            status: crate::registration::RegistrationStatus::Active,
            registration_time: chrono::Utc::now(),
            refresh_time: None,
            handle: Some(registration_handle),
        };
        
        // Store registration
        self.registrations.write().await.insert(reg_id, registration_info);
        
        // Update stats
        let mut stats = self.stats.lock().await;
        stats.total_registrations += 1;
        stats.active_registrations += 1;
        
        // Broadcast registration event
        if let Err(e) = self.event_tx.send(ClientEvent::RegistrationStatusChanged {
            info: crate::events::RegistrationStatusInfo {
                registration_id: reg_id,
                server_uri: config.server_uri.clone(),
                user_uri: config.from_uri.clone(),
                status: crate::registration::RegistrationStatus::Active,
                reason: Some("Registration successful".to_string()),
                timestamp: chrono::Utc::now(),
            },
            priority: crate::events::EventPriority::Normal,
        }) {
            tracing::debug!("Registration event receiver dropped: {}", e);
        }
        
        tracing::info!("Registered {} with server {}", config.from_uri, config.server_uri);
        Ok(reg_id)
    }
    
    /// Unregister from a SIP server
    /// 
    /// This method removes a registration from a SIP server by sending a REGISTER
    /// request with expires=0. This gracefully removes the client's presence from
    /// the server and stops receiving incoming calls for that registration.
    /// 
    /// # Arguments
    /// 
    /// * `reg_id` - The UUID of the registration to remove
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the unregistration was successful.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the registration ID is not found
    /// * `ClientError::InternalError` - If the unregistration request fails
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn unregister_example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5079".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:server.example.com".to_string(),
    ///         from_uri: "sip:alice@example.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5079".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     println!("✅ Registered with ID: {}", reg_id);
    ///     
    ///     // Unregister
    ///     client.unregister(reg_id).await?;
    ///     println!("✅ Successfully unregistered");
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn unregister(&self, reg_id: Uuid) -> ClientResult<()> {
        let mut registrations = self.registrations.write().await;
        
        if let Some(registration_info) = registrations.get_mut(&reg_id) {
            // To unregister, send REGISTER with expires=0
            if let Some(handle) = &registration_info.handle {
                SipClient::register(
                    &self.coordinator,
                    &handle.registrar_uri,
                    &registration_info.from_uri,
                    &handle.contact_uri,
                    0, // expires=0 means unregister
                )
                .await
                .map_err(|e| ClientError::InternalError { 
                    message: format!("Failed to unregister: {}", e) 
                })?;
            }
            
            // Update status
            registration_info.status = crate::registration::RegistrationStatus::Cancelled;
            registration_info.handle = None;
            
            // Update stats
            let mut stats = self.stats.lock().await;
            if stats.active_registrations > 0 {
                stats.active_registrations -= 1;
            }
            
            tracing::info!("Unregistered {}", registration_info.from_uri);
            Ok(())
        } else {
            Err(ClientError::InvalidConfiguration { 
                field: "registration_id".to_string(),
                reason: "Registration not found".to_string() 
            })
        }
    }
    
    /// Get registration information
    /// 
    /// Retrieves detailed information about a specific registration including
    /// status, timestamps, and server details.
    /// 
    /// # Arguments
    /// 
    /// * `reg_id` - The UUID of the registration to retrieve
    /// 
    /// # Returns
    /// 
    /// Returns the `RegistrationInfo` struct containing all registration details.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the registration ID is not found
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn get_registration_info() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5080".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:server.example.com".to_string(),
    ///         from_uri: "sip:user@example.com".to_string(),
    ///         contact_uri: "sip:user@127.0.0.1:5080".to_string(),
    ///         expires: 1800,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     
    ///     // Get registration details
    ///     let reg_info = client.get_registration(reg_id).await?;
    ///     println!("Registration status: {:?}", reg_info.status);
    ///     println!("Server: {}", reg_info.server_uri);
    ///     println!("User: {}", reg_info.from_uri);
    ///     println!("Expires: {} seconds", reg_info.expires);
    ///     
    ///     client.unregister(reg_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_registration(&self, reg_id: Uuid) -> ClientResult<crate::registration::RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.get(&reg_id)
            .cloned()
            .ok_or(ClientError::InvalidConfiguration { 
                field: "registration_id".to_string(),
                reason: "Registration not found".to_string() 
            })
    }
    
    /// Get all active registrations
    /// 
    /// Returns a list of all currently active registrations. This includes only
    /// registrations with status `Active`, filtering out expired or cancelled ones.
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<RegistrationInfo>` containing all active registrations.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn list_registrations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5081".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Create multiple registrations
    ///     let reg1_config = RegistrationConfig {
    ///         server_uri: "sip:server1.com".to_string(),
    ///         from_uri: "sip:alice@server1.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5081".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg2_config = RegistrationConfig {
    ///         server_uri: "sip:server2.com".to_string(),
    ///         from_uri: "sip:alice@server2.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5081".to_string(),
    ///         expires: 1800,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let _reg1_id = client.register(reg1_config).await?;
    ///     let _reg2_id = client.register(reg2_config).await?;
    ///     
    ///     // List all active registrations
    ///     let active_regs = client.get_all_registrations().await;
    ///     println!("Active registrations: {}", active_regs.len());
    ///     
    ///     for reg in active_regs {
    ///         println!("- {} at {}", reg.from_uri, reg.server_uri);
    ///     }
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_all_registrations(&self) -> Vec<crate::registration::RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.values()
            .filter(|r| r.status == crate::registration::RegistrationStatus::Active)
            .cloned()
            .collect()
    }
    
    /// Refresh a registration
    /// 
    /// Manually refreshes a registration by sending a new REGISTER request with
    /// the same parameters. This is useful for extending registration lifetime
    /// before expiration or after network connectivity issues.
    /// 
    /// # Arguments
    /// 
    /// * `reg_id` - The UUID of the registration to refresh
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the registration was successfully refreshed.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the registration ID is not found
    /// * `ClientError::InternalError` - If the refresh request fails
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn refresh_registration_example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5082".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:server.example.com".to_string(),
    ///         from_uri: "sip:user@example.com".to_string(),
    ///         contact_uri: "sip:user@127.0.0.1:5082".to_string(),
    ///         expires: 300, // Short expiration for demo
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     println!("✅ Initial registration completed");
    ///     
    ///     // Refresh the registration
    ///     client.refresh_registration(reg_id).await?;
    ///     println!("✅ Registration refreshed successfully");
    ///     
    ///     // Check registration info
    ///     if let Ok(reg_info) = client.get_registration(reg_id).await {
    ///         if let Some(refresh_time) = reg_info.refresh_time {
    ///             println!("Last refreshed: {}", refresh_time);
    ///         }
    ///     }
    ///     
    ///     client.unregister(reg_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn refresh_registration(&self, reg_id: Uuid) -> ClientResult<()> {
        // Get registration data
        let (registrar_uri, from_uri, contact_uri, expires) = {
            let registrations = self.registrations.read().await;
            
            if let Some(registration_info) = registrations.get(&reg_id) {
                if let Some(handle) = &registration_info.handle {
                    (
                        handle.registrar_uri.clone(),
                        registration_info.from_uri.clone(),
                        handle.contact_uri.clone(),
                        registration_info.expires,
                    )
                } else {
                    return Err(ClientError::InvalidConfiguration { 
                        field: "registration".to_string(),
                        reason: "Registration has no handle".to_string() 
                    });
                }
            } else {
                return Err(ClientError::InvalidConfiguration { 
                    field: "registration_id".to_string(),
                    reason: "Registration not found".to_string() 
                });
            }
        };
        
        // Re-register with the same parameters
        let new_handle = SipClient::register(
            &self.coordinator,
            &registrar_uri,
            &from_uri,
            &contact_uri,
            expires,
        )
        .await
        .map_err(|e| ClientError::InternalError { 
            message: format!("Failed to refresh registration: {}", e) 
        })?;
        
        // Update registration with new handle
        let mut registrations = self.registrations.write().await;
        if let Some(reg) = registrations.get_mut(&reg_id) {
            reg.handle = Some(new_handle);
            reg.refresh_time = Some(chrono::Utc::now());
        }
        
        tracing::info!("Refreshed registration for {}", from_uri);
        Ok(())
    }
    
    /// Clear expired registrations
    /// 
    /// Removes all registrations with `Expired` status from the internal storage.
    /// This is a maintenance operation that cleans up stale registration entries
    /// and updates statistics accordingly.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn cleanup_registrations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5083".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // In a real application, you might have some expired registrations
    ///     // This method would clean them up
    ///     client.clear_expired_registrations().await;
    ///     println!("✅ Expired registrations cleaned up");
    ///     
    ///     // Check remaining active registrations
    ///     let active_count = client.get_all_registrations().await.len();
    ///     println!("Active registrations remaining: {}", active_count);
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn clear_expired_registrations(&self) {
        let mut registrations = self.registrations.write().await;
        let mut to_remove = Vec::new();
        
        for (id, reg) in registrations.iter() {
            if reg.status == crate::registration::RegistrationStatus::Expired {
                to_remove.push(*id);
            }
        }
        
        for id in to_remove {
            registrations.remove(&id);
            
            // Update stats
            let mut stats = self.stats.lock().await;
            if stats.active_registrations > 0 {
                stats.active_registrations -= 1;
            }
        }
    }
    
    // ===== CONVENIENCE METHODS FOR EXAMPLES =====
    
    /// Convenience method: Register with simple parameters (for examples)
    /// 
    /// This is a simplified registration method that takes basic parameters and
    /// constructs a complete `RegistrationConfig` automatically. It's designed
    /// for quick testing and simple use cases.
    /// 
    /// # Arguments
    /// 
    /// * `agent_uri` - The SIP URI for this agent (e.g., "sip:alice@example.com")
    /// * `server_addr` - The SIP server address and port
    /// * `duration` - How long the registration should last
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if registration was successful.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// use std::time::Duration;
    /// 
    /// async fn simple_register() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5084".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let server_addr = "192.168.1.100:5060".parse()?;
    ///     let duration = Duration::from_secs(3600); // 1 hour
    ///     
    ///     // Simple registration
    ///     client.register_simple(
    ///         "sip:testuser@example.com",
    ///         &server_addr,
    ///         duration
    ///     ).await?;
    ///     
    ///     println!("✅ Simple registration completed");
    ///     
    ///     // Cleanup using the simple unregister method
    ///     client.unregister_simple(
    ///         "sip:testuser@example.com",
    ///         &server_addr
    ///     ).await?;
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn register_simple(
        &self, 
        agent_uri: &str, 
        server_addr: &std::net::SocketAddr,
        duration: std::time::Duration
    ) -> ClientResult<()> {
        let config = RegistrationConfig {
            server_uri: format!("sip:{}", server_addr),
            from_uri: agent_uri.to_string(),
            contact_uri: format!("sip:{}:{}", self.local_sip_addr.ip(), self.local_sip_addr.port()),
            expires: duration.as_secs() as u32,
            username: None,
            password: None,
            realm: None,
        };
        
        self.register(config).await?;
        Ok(())
    }
    
    /// Convenience method: Unregister with simple parameters (for examples)
    /// 
    /// This method finds and unregisters a registration that matches the given
    /// agent URI and server address. It's the counterpart to `register_simple()`
    /// and provides an easy way to clean up simple registrations.
    /// 
    /// # Arguments
    /// 
    /// * `agent_uri` - The SIP URI that was registered
    /// * `server_addr` - The SIP server address that was used
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if unregistration was successful.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If no matching registration is found
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// use std::time::Duration;
    /// 
    /// async fn simple_unregister() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5085".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let agent_uri = "sip:testuser@example.com";
    ///     let server_addr = "192.168.1.100:5060".parse()?;
    ///     
    ///     // Register first
    ///     client.register_simple(
    ///         agent_uri,
    ///         &server_addr,
    ///         Duration::from_secs(3600)
    ///     ).await?;
    ///     
    ///     println!("✅ Registration completed");
    ///     
    ///     // Now unregister using the same parameters
    ///     client.unregister_simple(agent_uri, &server_addr).await?;
    ///     println!("✅ Unregistration completed");
    ///     
    ///     // Verify no active registrations remain
    ///     let active_regs = client.get_all_registrations().await;
    ///     assert_eq!(active_regs.len(), 0);
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn unregister_simple(
        &self, 
        agent_uri: &str, 
        server_addr: &std::net::SocketAddr
    ) -> ClientResult<()> {
        // Find the registration matching these parameters
        let registrations = self.registrations.read().await;
        let reg_id = registrations.iter()
            .find(|(_, reg)| {
                reg.from_uri == agent_uri && 
                reg.server_uri == format!("sip:{}", server_addr)
            })
            .map(|(id, _)| *id);
        drop(registrations);
        
        if let Some(id) = reg_id {
            self.unregister(id).await
        } else {
            Err(ClientError::InvalidConfiguration { 
                field: "registration".to_string(),
                reason: "No matching registration found".to_string() 
            })
        }
    }
}
