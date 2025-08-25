//! Event package trait and implementations for SIP events (RFC 6665)
//!
//! This module defines the EventPackage trait that all SIP event packages
//! must implement, along with built-in implementations for common packages.

use std::time::Duration;
use rvoip_sip_core::types::content_type::ContentType;
use std::str::FromStr;

/// Trait for SIP event packages
///
/// Event packages define the semantics and data formats for specific
/// types of SIP event subscriptions (presence, dialog, message-summary, etc.)
pub trait EventPackage: Send + Sync {
    /// Get the name of this event package
    fn name(&self) -> &str;
    
    /// Get the accepted content types for this package
    fn accept_types(&self) -> Vec<ContentType>;
    
    /// Validate a message body for this event package
    fn validate_body(&self, body: &[u8]) -> Result<(), String>;
    
    /// Get the default subscription duration
    fn default_expires(&self) -> Duration;
    
    /// Get the minimum allowed subscription duration
    fn min_expires(&self) -> Duration {
        Duration::from_secs(60) // 1 minute default minimum
    }
    
    /// Get the maximum allowed subscription duration  
    fn max_expires(&self) -> Duration {
        Duration::from_secs(86400) // 24 hours default maximum
    }
    
    /// Check if this package supports event lists (RFC 4662)
    fn supports_event_lists(&self) -> bool {
        false
    }
    
    /// Check if this package requires authentication
    fn requires_auth(&self) -> bool {
        true
    }
}

/// Presence event package (RFC 3856)
pub struct PresencePackage;

impl EventPackage for PresencePackage {
    fn name(&self) -> &str {
        "presence"
    }
    
    fn accept_types(&self) -> Vec<ContentType> {
        vec![
            ContentType::from_str("application/pidf+xml").unwrap(),
            ContentType::from_str("application/xpidf+xml").unwrap(),
            ContentType::from_str("application/simple-message-summary").unwrap(),
        ]
    }
    
    fn validate_body(&self, body: &[u8]) -> Result<(), String> {
        // Dialog-core only validates basic requirements
        // Session-core handles PIDF parsing and validation
        if body.is_empty() {
            Ok(()) // Empty body is allowed for some NOTIFY messages
        } else {
            Ok(()) // Accept any non-empty body, session-core will validate
        }
    }
    
    fn default_expires(&self) -> Duration {
        Duration::from_secs(3600) // 1 hour default
    }
}

/// Dialog event package (RFC 4235)
pub struct DialogPackage;

impl EventPackage for DialogPackage {
    fn name(&self) -> &str {
        "dialog"
    }
    
    fn accept_types(&self) -> Vec<ContentType> {
        vec![
            ContentType::from_str("application/dialog-info+xml").unwrap(),
        ]
    }
    
    fn validate_body(&self, _body: &[u8]) -> Result<(), String> {
        // Session-core handles dialog-info XML validation
        Ok(())
    }
    
    fn default_expires(&self) -> Duration {
        Duration::from_secs(3600) // 1 hour default
    }
}

/// Message summary event package (RFC 3842)
pub struct MessageSummaryPackage;

impl EventPackage for MessageSummaryPackage {
    fn name(&self) -> &str {
        "message-summary"
    }
    
    fn accept_types(&self) -> Vec<ContentType> {
        vec![
            ContentType::from_str("application/simple-message-summary").unwrap(),
        ]
    }
    
    fn validate_body(&self, _body: &[u8]) -> Result<(), String> {
        // Session-core handles message-summary validation
        Ok(())
    }
    
    fn default_expires(&self) -> Duration {
        Duration::from_secs(3600) // 1 hour default
    }
}

/// Refer event package (RFC 3515)
pub struct ReferPackage;

impl EventPackage for ReferPackage {
    fn name(&self) -> &str {
        "refer"
    }
    
    fn accept_types(&self) -> Vec<ContentType> {
        vec![
            ContentType::from_str("message/sipfrag").unwrap(),
        ]
    }
    
    fn validate_body(&self, _body: &[u8]) -> Result<(), String> {
        // Session-core handles SIP fragment validation for REFER
        Ok(())
    }
    
    fn default_expires(&self) -> Duration {
        Duration::from_secs(60) // 1 minute for refer subscriptions
    }
    
    fn supports_event_lists(&self) -> bool {
        false // Refer doesn't support event lists
    }
}