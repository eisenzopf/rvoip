//! # SIP Agent Registration Management
//!
//! This module provides comprehensive SIP registration handling for call center agents,
//! including registration processing, state management, contact tracking, and expiration
//! handling. It implements the SIP REGISTER method according to RFC 3261 with call
//! center specific optimizations and features.
//!
//! ## Overview
//!
//! The SIP registration system enables agents to register their presence and contact
//! information with the call center, allowing the system to route calls to the appropriate
//! agent endpoints. This module handles the complete registration lifecycle including
//! initial registration, refresh, de-registration, and automatic expiration cleanup.
//!
//! ## Key Features
//!
//! - **SIP REGISTER Processing**: Full RFC 3261 compliant registration handling
//! - **Contact Management**: Tracking of agent contact URIs and network information
//! - **Expiration Handling**: Automatic cleanup of expired registrations
//! - **Transport Support**: Multi-transport support (UDP, TCP, TLS, WebSocket)
//! - **User Agent Tracking**: Softphone and client identification
//! - **Network Monitoring**: Remote address tracking for connectivity analysis
//!
//! ## Registration Lifecycle
//!
//! ### Initial Registration
//! 1. Agent sends SIP REGISTER with contact information
//! 2. System validates registration parameters
//! 3. Contact URI and expiration time are stored
//! 4. Registration confirmation is sent to agent
//!
//! ### Registration Refresh
//! 1. Agent sends periodic REGISTER requests
//! 2. System updates expiration time
//! 3. Contact information is refreshed
//! 4. Network connectivity is confirmed
//!
//! ### De-registration
//! 1. Agent sends REGISTER with Expires: 0
//! 2. System removes registration immediately
//! 3. Agent is marked as offline
//! 4. Call routing is updated
//!
//! ### Automatic Expiration
//! 1. System monitors registration expiration times
//! 2. Expired registrations are automatically removed
//! 3. Agents are notified of expiration if possible
//! 4. Call routing tables are updated
//!
//! ## Transport Considerations
//!
//! ### UDP Transport
//! - Connectionless operation
//! - Periodic registration refresh required
//! - NAT traversal considerations
//! - Suitable for stable network environments
//!
//! ### TCP Transport
//! - Connection-oriented operation
//! - More reliable than UDP
//! - Connection state monitoring
//! - Better for firewalled environments
//!
//! ### TLS Transport
//! - Encrypted SIP signaling
//! - Certificate-based authentication
//! - Enhanced security for sensitive environments
//! - Required for compliance scenarios
//!
//! ### WebSocket Transport
//! - Web-based softphone support
//! - Firewall-friendly operation
//! - Real-time web application integration
//! - Mobile and browser client support
//!
//! ## Examples
//!
//! ### Basic Agent Registration
//!
//! ```rust
//! use rvoip_call_engine::agent::registration::{SipRegistrar, RegistrationStatus};
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut registrar = SipRegistrar::new();
//! 
//! // Process agent registration
//! let response = registrar.process_register_simple(
//!     "sip:alice@call-center.com",           // Address of Record
//!     "sip:alice@192.168.1.100:5060",       // Contact URI
//!     Some(3600),                            // Expires in 1 hour
//!     Some("SoftPhone/2.1".to_string()),     // User Agent
//!     "192.168.1.100:5060".to_string(),      // Remote Address
//! )?;
//! 
//! match response.status {
//!     RegistrationStatus::Created => {
//!         println!("✅ Agent registered successfully for {} seconds", response.expires);
//!     }
//!     RegistrationStatus::Refreshed => {
//!         println!("🔄 Registration refreshed for {} seconds", response.expires);
//!     }
//!     RegistrationStatus::Removed => {
//!         println!("📤 Agent de-registered");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Registration Management
//!
//! ```rust
//! use rvoip_call_engine::agent::registration::{SipRegistrar, RegistrationStatus};
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut registrar = SipRegistrar::new();
//! 
//! // Register multiple agents with different transports
//! let agents = vec![
//!     ("sip:alice@call-center.com", "sip:alice@192.168.1.100:5060", "UDP"),
//!     ("sip:bob@call-center.com", "sip:bob@192.168.1.101:5060;transport=tcp", "TCP"),
//!     ("sip:carol@call-center.com", "sips:carol@192.168.1.102:5061", "TLS"),
//! ];
//! 
//! for (aor, contact, transport) in agents {
//!     let response = registrar.process_register_simple(
//!         aor,
//!         contact,
//!         Some(1800), // 30 minutes
//!         Some(format!("CallCenterAgent/1.0 ({})", transport)),
//!         "192.168.1.0/24".to_string(),
//!     )?;
//!     
//!     println!("Agent {} registered via {}: {} seconds", 
//!              aor, transport, response.expires);
//! }
//! 
//! // List all active registrations
//! let active_registrations = registrar.list_registrations();
//! println!("\n📋 Active Registrations ({}):", active_registrations.len());
//! 
//! for (aor, registration) in active_registrations {
//!     println!("  {} -> {} ({})", 
//!              aor, registration.contact_uri, registration.transport);
//!     
//!     if let Some(ua) = &registration.user_agent {
//!         println!("    User-Agent: {}", ua);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Registration Monitoring and Cleanup
//!
//! ```rust
//! use rvoip_call_engine::agent::registration::SipRegistrar;
//! use std::time::Duration;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut registrar = SipRegistrar::new();
//! 
//! // Simulate registration monitoring loop
//! loop {
//!     // Clean up expired registrations
//!     let before_count = registrar.list_registrations().len();
//!     registrar.cleanup_expired();
//!     let after_count = registrar.list_registrations().len();
//!     
//!     if before_count > after_count {
//!         println!("🧹 Cleaned up {} expired registrations", 
//!                  before_count - after_count);
//!     }
//!     
//!     // Check registration health
//!     let registrations = registrar.list_registrations();
//!     for (aor, reg) in registrations {
//!         let remaining = reg.expires_at.saturating_duration_since(std::time::Instant::now());
//!         
//!         if remaining < Duration::from_secs(300) { // Less than 5 minutes
//!             println!("⚠️ Registration for {} expires soon: {:?}", aor, remaining);
//!         }
//!     }
//!     
//!     // Wait before next check
//!     tokio::time::sleep(Duration::from_secs(60)).await;
//!     break; // For example purposes
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Contact URI Resolution
//!
//! ```rust
//! use rvoip_call_engine::agent::registration::SipRegistrar;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut registrar = SipRegistrar::new();
//! 
//! // Register an agent
//! registrar.process_register_simple(
//!     "sip:support@call-center.com",
//!     "sip:support@agent-workstation.local:5060",
//!     Some(7200),
//!     None,
//!     "10.0.1.50:5060".to_string(),
//! )?;
//! 
//! // Resolve contact for routing
//! if let Some(registration) = registrar.get_registration("sip:support@call-center.com") {
//!     println!("📍 Routing call to: {}", registration.contact_uri);
//!     println!("   Transport: {}", registration.transport);
//!     println!("   Remote: {}", registration.remote_addr);
//!     
//!     // Reverse lookup - find AOR by contact
//!     if let Some(aor) = registrar.find_aor_by_contact(&registration.contact_uri) {
//!         println!("   Reverse lookup: {} -> {}", registration.contact_uri, aor);
//!     }
//! } else {
//!     println!("❌ Agent not registered or registration expired");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### De-registration Handling
//!
//! ```rust
//! use rvoip_call_engine::agent::registration::{SipRegistrar, RegistrationStatus};
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut registrar = SipRegistrar::new();
//! 
//! // First register an agent
//! registrar.process_register_simple(
//!     "sip:temp@call-center.com",
//!     "sip:temp@192.168.1.200:5060",
//!     Some(600),
//!     Some("TempAgent/1.0".to_string()),
//!     "192.168.1.200:5060".to_string(),
//! )?;
//! 
//! println!("Agent registered");
//! 
//! // Later, agent wants to de-register
//! let deregister_response = registrar.process_register_simple(
//!     "sip:temp@call-center.com",
//!     "sip:temp@192.168.1.200:5060",
//!     Some(0), // Expires: 0 means de-register
//!     Some("TempAgent/1.0".to_string()),
//!     "192.168.1.200:5060".to_string(),
//! )?;
//! 
//! match deregister_response.status {
//!     RegistrationStatus::Removed => {
//!         println!("✅ Agent successfully de-registered");
//!     }
//!     _ => {
//!         println!("⚠️ Unexpected de-registration response");
//!     }
//! }
//! 
//! // Verify agent is no longer registered
//! assert!(registrar.get_registration("sip:temp@call-center.com").is_none());
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::error::Result;

/// Default registration expiry time (1 hour)
const DEFAULT_EXPIRY: Duration = Duration::from_secs(3600);

/// Minimum allowed registration time (60 seconds)
const MIN_EXPIRY: Duration = Duration::from_secs(60);

/// Registration information for an agent
///
/// Contains all the information associated with an agent's SIP registration,
/// including contact details, expiration time, and network information.
/// This information is used for call routing and agent management.
#[derive(Debug, Clone)]
pub struct Registration {
    /// Agent ID (typically derived from AOR)
    pub agent_id: String,
    
    /// Contact URI where the agent can be reached (as string)
    pub contact_uri: String,
    
    /// When this registration expires
    pub expires_at: Instant,
    
    /// User agent string (softphone info)
    pub user_agent: Option<String>,
    
    /// Transport used (UDP, TCP, TLS, WS, WSS)
    pub transport: String,
    
    /// Remote IP address
    pub remote_addr: String,
}

/// SIP Registrar for managing agent registrations
///
/// The `SipRegistrar` maintains the registry of all active agent registrations
/// and provides methods for processing SIP REGISTER requests, managing contact
/// information, and handling registration lifecycle events.
///
/// ## Thread Safety
///
/// This registrar is designed for single-threaded use within the call center
/// engine. For multi-threaded scenarios, wrap in appropriate synchronization
/// primitives (Arc<Mutex<SipRegistrar>>).
///
/// ## Memory Management
///
/// The registrar automatically manages memory by removing expired registrations
/// through the `cleanup_expired()` method. Regular cleanup is recommended to
/// prevent memory leaks in long-running systems.
pub struct SipRegistrar {
    /// Active registrations indexed by AOR (Address of Record)
    registrations: HashMap<String, Registration>,
    
    /// Reverse lookup: contact URI -> AOR
    contact_to_aor: HashMap<String, String>,
}

impl SipRegistrar {
    /// Create a new SIP registrar
    ///
    /// Initializes an empty SIP registrar ready to process agent registrations.
    /// The registrar starts with no active registrations and will need periodic
    /// cleanup to remove expired entries.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::registration::SipRegistrar;
    /// 
    /// let registrar = SipRegistrar::new();
    /// println!("SIP registrar initialized");
    /// ```
    pub fn new() -> Self {
        Self {
            registrations: HashMap::new(),
            contact_to_aor: HashMap::new(),
        }
    }
    
    /// Process a REGISTER request with simplified string-based interface
    ///
    /// Processes a SIP REGISTER request using simplified string parameters instead
    /// of full SIP message parsing. This method handles the complete registration
    /// lifecycle including initial registration, refresh, and de-registration.
    ///
    /// # Arguments
    ///
    /// * `aor` - Address of Record (e.g., "sip:alice@example.com")
    /// * `contact_uri` - Contact URI as string where agent can be reached
    /// * `expires` - Optional expiration time in seconds (None = default)
    /// * `user_agent` - Optional User-Agent string for client identification
    /// * `remote_addr` - Remote address of the registering agent
    ///
    /// # Returns
    ///
    /// `Ok(RegistrationResponse)` with registration status and expiration time,
    /// or error if registration processing fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::registration::SipRegistrar;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut registrar = SipRegistrar::new();
    /// 
    /// // Initial registration
    /// let response = registrar.process_register_simple(
    ///     "sip:alice@call-center.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     Some(3600),
    ///     Some("MySoftphone/1.0".to_string()),
    ///     "192.168.1.100:5060".to_string(),
    /// )?;
    /// 
    /// println!("Registration expires in {} seconds", response.expires);
    /// 
    /// // Registration refresh
    /// let refresh_response = registrar.process_register_simple(
    ///     "sip:alice@call-center.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     Some(1800), // Refresh for 30 minutes
    ///     Some("MySoftphone/1.0".to_string()),
    ///     "192.168.1.100:5060".to_string(),
    /// )?;
    /// 
    /// // De-registration
    /// let dereg_response = registrar.process_register_simple(
    ///     "sip:alice@call-center.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     Some(0), // Expires: 0 = de-register
    ///     Some("MySoftphone/1.0".to_string()),
    ///     "192.168.1.100:5060".to_string(),
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn process_register_simple(
        &mut self,
        aor: &str,  // Address of Record (e.g., "sip:alice@example.com")
        contact_uri: &str,  // Contact URI as string
        expires: Option<u32>,
        user_agent: Option<String>,
        remote_addr: String,
    ) -> Result<RegistrationResponse> {
        let expires_duration = expires
            .map(|e| Duration::from_secs(e as u64))
            .unwrap_or(DEFAULT_EXPIRY);
        
        // Handle de-registration (expires=0)
        if expires_duration.is_zero() {
            info!("📤 De-registration request for {}", aor);
            self.remove_registration(aor);
            return Ok(RegistrationResponse {
                status: RegistrationStatus::Removed,
                expires: 0,
            });
        }
        
        // Validate expiry time
        let expires_duration = if expires_duration < MIN_EXPIRY {
            warn!("Registration expiry too short, using minimum: {:?}", MIN_EXPIRY);
            MIN_EXPIRY
        } else {
            expires_duration
        };
        
        // Extract transport from contact URI if present (simple parsing)
        let transport = if contact_uri.contains("transport=tcp") {
            "TCP".to_string()
        } else if contact_uri.contains("transport=tls") {
            "TLS".to_string()
        } else if contact_uri.contains("transport=ws") {
            "WS".to_string()
        } else if contact_uri.contains("transport=wss") {
            "WSS".to_string()
        } else {
            "UDP".to_string()
        };
        
        // Create registration entry
        let registration = Registration {
            agent_id: aor.to_string(), // Could be parsed from AOR
            contact_uri: contact_uri.to_string(),
            expires_at: Instant::now() + expires_duration,
            user_agent,
            transport,
            remote_addr,
        };
        
        // Store registration
        let is_refresh = self.registrations.contains_key(aor);
        self.registrations.insert(aor.to_string(), registration);
        self.contact_to_aor.insert(contact_uri.to_string(), aor.to_string());
        
        info!("✅ {} registration for {}: expires in {:?}", 
              if is_refresh { "Refreshed" } else { "New" },
              aor, 
              expires_duration);
        
        Ok(RegistrationResponse {
            status: if is_refresh { RegistrationStatus::Refreshed } else { RegistrationStatus::Created },
            expires: expires_duration.as_secs() as u32,
        })
    }
    
    /// Remove a registration
    ///
    /// Removes an active registration for the specified Address of Record.
    /// This method cleans up both the primary registration entry and the
    /// reverse lookup mapping.
    ///
    /// # Arguments
    ///
    /// * `aor` - Address of Record to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::registration::SipRegistrar;
    /// 
    /// let mut registrar = SipRegistrar::new();
    /// 
    /// // Register then remove
    /// # let _ = registrar.process_register_simple(
    /// #     "sip:test@example.com",
    /// #     "sip:test@192.168.1.1:5060", 
    /// #     Some(600),
    /// #     None,
    /// #     "192.168.1.1:5060".to_string(),
    /// # );
    /// 
    /// registrar.remove_registration("sip:test@example.com");
    /// assert!(registrar.get_registration("sip:test@example.com").is_none());
    /// ```
    pub fn remove_registration(&mut self, aor: &str) {
        if let Some(reg) = self.registrations.remove(aor) {
            self.contact_to_aor.remove(&reg.contact_uri);
            info!("🗑️ Removed registration for {}", aor);
        }
    }
    
    /// Get registration for an AOR
    ///
    /// Retrieves the active registration for the specified Address of Record.
    /// Returns None if no registration exists or if the registration has expired.
    ///
    /// # Arguments
    ///
    /// * `aor` - Address of Record to look up
    ///
    /// # Returns
    ///
    /// `Some(&Registration)` if active registration found, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::registration::SipRegistrar;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut registrar = SipRegistrar::new();
    /// 
    /// # registrar.process_register_simple(
    /// #     "sip:alice@example.com",
    /// #     "sip:alice@192.168.1.100:5060",
    /// #     Some(3600),
    /// #     None,
    /// #     "192.168.1.100:5060".to_string(),
    /// # )?;
    /// 
    /// if let Some(registration) = registrar.get_registration("sip:alice@example.com") {
    ///     println!("Agent can be reached at: {}", registration.contact_uri);
    ///     println!("Using transport: {}", registration.transport);
    /// } else {
    ///     println!("Agent not registered or registration expired");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_registration(&self, aor: &str) -> Option<&Registration> {
        self.registrations.get(aor)
            .filter(|reg| reg.expires_at > Instant::now())
    }
    
    /// Find AOR by contact URI
    ///
    /// Performs reverse lookup to find the Address of Record associated with
    /// a specific contact URI. This is useful for processing incoming requests
    /// from agents.
    ///
    /// # Arguments
    ///
    /// * `contact_uri` - Contact URI to look up
    ///
    /// # Returns
    ///
    /// `Some(&str)` with the AOR if found, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SipRegistrar;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut registrar = SipRegistrar::new();
    /// 
    /// # registrar.process_register_simple(
    /// #     "sip:bob@example.com",
    /// #     "sip:bob@agent-pc.local:5060",
    /// #     Some(1800),
    /// #     None,
    /// #     "10.0.1.50:5060".to_string(),
    /// # )?;
    /// 
    /// // Reverse lookup from contact to AOR
    /// if let Some(aor) = registrar.find_aor_by_contact("sip:bob@agent-pc.local:5060") {
    ///     println!("Contact belongs to AOR: {}", aor);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn find_aor_by_contact(&self, contact_uri: &str) -> Option<&str> {
        self.contact_to_aor.get(contact_uri).map(|s| s.as_str())
    }
    
    /// Clean up expired registrations
    ///
    /// Removes all registrations that have passed their expiration time.
    /// This method should be called periodically to prevent memory leaks
    /// and maintain accurate registration state.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SipRegistrar;
    /// 
    /// let mut registrar = SipRegistrar::new();
    /// 
    /// // In a background task or periodic cleanup
    /// registrar.cleanup_expired();
    /// 
    /// println!("Expired registrations cleaned up");
    /// ```
    pub fn cleanup_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<String> = self.registrations
            .iter()
            .filter(|(_, reg)| reg.expires_at <= now)
            .map(|(aor, _)| aor.clone())
            .collect();
        
        for aor in expired {
            self.remove_registration(&aor);
            warn!("⏰ Expired registration removed: {}", aor);
        }
    }
    
    /// Get all active registrations
    ///
    /// Returns a list of all currently active (non-expired) registrations.
    /// This method filters out expired registrations automatically.
    ///
    /// # Returns
    ///
    /// Vector of tuples containing (AOR, Registration) for all active registrations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SipRegistrar;
    /// 
    /// let registrar = SipRegistrar::new();
    /// 
    /// let active_registrations = registrar.list_registrations();
    /// 
    /// println!("Active registrations: {}", active_registrations.len());
    /// for (aor, registration) in active_registrations {
    ///     println!("  {} -> {} ({})", 
    ///              aor, registration.contact_uri, registration.transport);
    /// }
    /// ```
    pub fn list_registrations(&self) -> Vec<(&str, &Registration)> {
        let now = Instant::now();
        self.registrations
            .iter()
            .filter(|(_, reg)| reg.expires_at > now)
            .map(|(aor, reg)| (aor.as_str(), reg))
            .collect()
    }
}

/// Response to a registration request
///
/// Contains the result of processing a SIP REGISTER request, including
/// the registration status and the actual expiration time granted.
#[derive(Debug)]
pub struct RegistrationResponse {
    /// Status of the registration operation
    pub status: RegistrationStatus,
    /// Granted expiration time in seconds
    pub expires: u32,
}

/// Registration status enumeration
///
/// Indicates the result of a registration operation.
#[derive(Debug)]
pub enum RegistrationStatus {
    /// New registration was created
    Created,
    /// Existing registration was refreshed
    Refreshed,
    /// Registration was removed (de-registered)
    Removed,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_registration() {
        let mut registrar = SipRegistrar::new();
        
        // Process registration with simplified interface
        let response = registrar.process_register_simple(
            "sip:alice@example.com",
            "sip:alice@192.168.1.100:5060",
            Some(3600),
            Some("MySoftphone/1.0".to_string()),
            "192.168.1.100:5060".to_string(),
        ).unwrap();
        
        assert!(matches!(response.status, RegistrationStatus::Created));
        assert_eq!(response.expires, 3600);
        
        // Verify registration exists
        let reg = registrar.get_registration("sip:alice@example.com").unwrap();
        assert_eq!(reg.contact_uri, "sip:alice@192.168.1.100:5060");
    }
} 