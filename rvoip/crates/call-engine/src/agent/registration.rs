//! SIP Registration handling for agents
//!
//! This module provides functionality for agents to register via SIP REGISTER
//! and maintains their registration state.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::error::Result;

/// Default registration expiry time (1 hour)
const DEFAULT_EXPIRY: Duration = Duration::from_secs(3600);

/// Minimum allowed registration time (60 seconds)
const MIN_EXPIRY: Duration = Duration::from_secs(60);

/// Registration information for an agent
#[derive(Debug, Clone)]
pub struct Registration {
    /// Agent ID
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
pub struct SipRegistrar {
    /// Active registrations indexed by AOR (Address of Record)
    registrations: HashMap<String, Registration>,
    
    /// Reverse lookup: contact URI -> AOR
    contact_to_aor: HashMap<String, String>,
}

impl SipRegistrar {
    /// Create a new SIP registrar
    pub fn new() -> Self {
        Self {
            registrations: HashMap::new(),
            contact_to_aor: HashMap::new(),
        }
    }
    
    /// Process a REGISTER request with simplified string-based interface
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
    pub fn remove_registration(&mut self, aor: &str) {
        if let Some(reg) = self.registrations.remove(aor) {
            self.contact_to_aor.remove(&reg.contact_uri);
            info!("🗑️ Removed registration for {}", aor);
        }
    }
    
    /// Get registration for an AOR
    pub fn get_registration(&self, aor: &str) -> Option<&Registration> {
        self.registrations.get(aor)
            .filter(|reg| reg.expires_at > Instant::now())
    }
    
    /// Find AOR by contact URI
    pub fn find_aor_by_contact(&self, contact_uri: &str) -> Option<&str> {
        self.contact_to_aor.get(contact_uri).map(|s| s.as_str())
    }
    
    /// Clean up expired registrations
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
#[derive(Debug)]
pub struct RegistrationResponse {
    pub status: RegistrationStatus,
    pub expires: u32,
}

/// Registration status
#[derive(Debug)]
pub enum RegistrationStatus {
    Created,
    Refreshed,
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