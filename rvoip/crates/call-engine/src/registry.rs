use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::Mutex;
use uuid::Uuid;
use anyhow::Result;
use tracing::{debug, info, warn, error};

use rvoip_sip_core::{Uri, Request, Response, Method, StatusCode};

use crate::errors::Error;

/// Registration information for a SIP endpoint
#[derive(Debug, Clone)]
pub struct Registration {
    /// SIP URI of the registered user
    pub uri: Uri,
    
    /// Contact URI where the user can be reached
    pub contact: Uri,
    
    /// Network address of the endpoint
    pub address: SocketAddr,
    
    /// User agent information
    pub user_agent: Option<String>,
    
    /// Time when the registration was created
    pub created_at: Instant,
    
    /// Time when the registration expires
    pub expires_at: Instant,
    
    /// Last seen time
    pub last_seen: Instant,
    
    /// Registration identifier
    pub id: Uuid,
}

impl Registration {
    /// Create a new registration
    pub fn new(uri: Uri, contact: Uri, address: SocketAddr, user_agent: Option<String>, expires: Duration) -> Self {
        let now = Instant::now();
        Self {
            uri,
            contact,
            address,
            user_agent,
            created_at: now,
            expires_at: now + expires,
            last_seen: now,
            id: Uuid::new_v4(),
        }
    }
    
    /// Check if the registration is expired
    pub fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }
    
    /// Update the registration expiry
    pub fn update_expiry(&mut self, expires: Duration) {
        self.expires_at = Instant::now() + expires;
        self.last_seen = Instant::now();
    }
}

/// A registry of SIP users and endpoints
pub struct Registry {
    /// Map of AOR (Address of Record) to registrations
    registrations: DashMap<String, Vec<Registration>>,
    
    /// Default registration expiry
    default_expiry: Duration,
    
    /// Maximum registration expiry
    max_expiry: Duration,
    
    /// Minimum registration expiry
    min_expiry: Duration,
}

impl Registry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            registrations: DashMap::new(),
            default_expiry: Duration::from_secs(3600), // 1 hour
            max_expiry: Duration::from_secs(86400),    // 24 hours
            min_expiry: Duration::from_secs(60),       // 1 minute
        }
    }
    
    /// Handle a REGISTER request
    pub fn handle_register(&self, request: &Request, source: SocketAddr) -> Result<Response, Error> {
        // Extract the AOR (Address of Record)
        let to_header = request.header(&rvoip_sip_core::HeaderName::To)
            .ok_or_else(|| Error::other("Missing To header"))?;
        
        let to_uri = to_header.value.as_text()
            .and_then(|text| text.find('<').and_then(|start| {
                text[start+1..].find('>').map(|end| text[start+1..start+1+end].to_string())
            }))
            .ok_or_else(|| Error::other("Invalid To header format"))?;
        
        // Parse the AOR
        let aor = to_uri.parse::<Uri>()
            .map_err(|_| Error::other("Invalid AOR format"))?;
        
        // Extract Contact header
        let contact_header = match request.header(&rvoip_sip_core::HeaderName::Contact) {
            Some(h) => h,
            None => {
                // No Contact header, this is a query request
                return self.get_registrations_for_aor(&aor.to_string())
                    .map(|regs| {
                        let mut response = Response::new(StatusCode::Ok);
                        // Add contact headers for each registration
                        for reg in regs {
                            let contact_str = format!("<{}>;expires={}", 
                                reg.contact, 
                                reg.expires_at.duration_since(Instant::now()).as_secs());
                            response.headers.push(rvoip_sip_core::Header::text(
                                rvoip_sip_core::HeaderName::Contact, 
                                contact_str
                            ));
                        }
                        response
                    })
                    .map_err(|e| Error::other(e));
            }
        };
        
        // Extract expires parameter
        let expires_header = request.header(&rvoip_sip_core::HeaderName::Expires);
        let contact_expires = contact_header.value.as_text()
            .and_then(|text| {
                text.find("expires=").map(|idx| {
                    let expires_str = &text[idx + 8..];
                    expires_str.split(|c: char| !c.is_digit(10))
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                })
            })
            .flatten();
        
        let expires_value = expires_header
            .and_then(|h| h.value.as_text())
            .and_then(|t| t.parse::<u64>().ok())
            .or(contact_expires)
            .unwrap_or(self.default_expiry.as_secs());
        
        // Extract contact URI
        let contact_uri = contact_header.value.as_text()
            .and_then(|text| text.find('<').and_then(|start| {
                text[start+1..].find('>').map(|end| &text[start+1..start+1+end])
            }))
            .ok_or_else(|| Error::other("Invalid Contact header format"))?;
        
        let contact_uri = contact_uri.parse::<Uri>()
            .map_err(|_| Error::other("Invalid Contact URI format"))?;
        
        // Get user agent
        let user_agent = request.header(&rvoip_sip_core::HeaderName::UserAgent)
            .and_then(|h| h.value.as_text().map(|s| s.to_string()));
        
        // Handle registration expiry
        let expires = if expires_value == 0 {
            // This is a request to remove registration
            self.remove_registration(&aor.to_string(), &contact_uri.to_string())?;
            
            let mut response = Response::new(StatusCode::Ok);
            response.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::Expires, 
                "0"
            ));
            return Ok(response);
        } else {
            // Cap expiry to min/max values
            let expires = Duration::from_secs(
                expires_value.clamp(
                    self.min_expiry.as_secs(),
                    self.max_expiry.as_secs()
                )
            );
            
            // Update or create registration
            self.update_registration(
                aor.to_string(), 
                contact_uri, 
                source, 
                user_agent, 
                expires
            )?;
            
            expires
        };
        
        // Create response
        let mut response = Response::new(StatusCode::Ok);
        response.headers.push(rvoip_sip_core::Header::text(
            rvoip_sip_core::HeaderName::Expires, 
            expires.as_secs().to_string()
        ));
        
        // Add Date header
        // TODO: Add Date header with current time
        
        Ok(response)
    }
    
    /// Update or create a registration
    fn update_registration(
        &self, 
        aor: String,
        contact: Uri,
        address: SocketAddr,
        user_agent: Option<String>,
        expires: Duration
    ) -> Result<(), Error> {
        let mut regs = self.registrations.entry(aor.clone())
            .or_insert_with(Vec::new);
        
        let contact_str = contact.to_string();
        
        if let Some(reg_idx) = regs.iter().position(|r| r.contact.to_string() == contact_str) {
            // Update existing registration
            regs[reg_idx].update_expiry(expires);
            regs[reg_idx].address = address;
            regs[reg_idx].user_agent = user_agent;
            debug!("Updated registration for {}: {}", aor, contact_str);
        } else {
            // Create new registration
            regs.push(Registration::new(
                aor.parse::<Uri>().unwrap(),
                contact,
                address,
                user_agent,
                expires
            ));
            debug!("Created new registration for {}: {}", aor, contact_str);
        }
        
        Ok(())
    }
    
    /// Remove a registration
    fn remove_registration(&self, aor: &str, contact: &str) -> Result<(), Error> {
        if let Some(mut regs) = self.registrations.get_mut(aor) {
            let before_len = regs.len();
            regs.retain(|r| r.contact.to_string() != contact);
            
            if regs.len() < before_len {
                debug!("Removed registration for {}: {}", aor, contact);
                return Ok(());
            }
        }
        
        warn!("Registration not found for {}: {}", aor, contact);
        Err(Error::other("Registration not found"))
    }
    
    /// Get all registrations for an AOR
    pub fn get_registrations_for_aor(&self, aor: &str) -> Result<Vec<Registration>, String> {
        if let Some(regs) = self.registrations.get(aor) {
            // Filter out expired registrations
            let active_regs: Vec<Registration> = regs.iter()
                .filter(|r| !r.is_expired())
                .cloned()
                .collect();
            
            if active_regs.is_empty() {
                return Err(format!("No active registrations for {}", aor));
            }
            
            Ok(active_regs)
        } else {
            Err(format!("No registrations found for {}", aor))
        }
    }
    
    /// Lookup a SIP URI to find where to route it
    pub fn lookup(&self, uri: &Uri) -> Option<Registration> {
        // Check if user exists
        let user_part = match &uri.user {
            Some(user) => user,
            None => return None,
        };
        
        // Check if host exists - host is always present in Uri
        let host_part = &uri.host;
        
        // Try exact match first (user@host)
        let aor = format!("{}@{}", user_part, host_part);
        if let Ok(regs) = self.get_registrations_for_aor(&aor) {
            return Some(regs[0].clone());
        }
        
        // Try domain match (any user in this domain)
        let domain_regs = self.registrations.iter()
            .filter_map(|entry| {
                let key = entry.key();
                if key.ends_with(&format!("@{}", host_part)) {
                    if let Ok(regs) = self.get_registrations_for_aor(key) {
                        if !regs.is_empty() {
                            return Some(regs[0].clone());
                        }
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        
        domain_regs.first().cloned()
    }
    
    /// Clean expired registrations
    pub fn clean_expired(&self) {
        for mut entry in self.registrations.iter_mut() {
            let aor = entry.key().clone();
            let before_len = entry.value().len();
            
            entry.value_mut().retain(|r| !r.is_expired());
            
            let after_len = entry.value().len();
            if before_len != after_len {
                debug!("Cleaned {} expired registrations for {}", before_len - after_len, aor);
            }
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
} 