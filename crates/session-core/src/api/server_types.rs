//! Server-oriented Types
//! 
//! This module provides enhanced types for server-side SIP applications such as
//! call centers, PBX systems, and SIP proxies that need more detailed call information.
//! 
//! # Overview
//! 
//! While the basic `IncomingCall` type provides essential information, server
//! applications often need more detailed metadata about calls. This module provides:
//! 
//! - **IncomingCallEvent**: Enhanced call info with headers and call-id
//! - **CallerInfo**: Detailed caller metadata including display name and UA
//! 
//! # Usage Example
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! 
//! // Extract detailed info from SIP headers
//! fn process_incoming_event(event: IncomingCallEvent) {
//!     let caller = &event.caller_info;
//!     
//!     println!("Incoming call from: {}", caller.from);
//!     if let Some(name) = &caller.display_name {
//!         println!("Display Name: {}", name);
//!     }
//!     
//!     // Check user agent for compatibility
//!     if let Some(agent) = &caller.user_agent {
//!         if agent.contains("incompatible") {
//!             // Handle incompatible clients
//!         }
//!     }
//!     
//!     // Extract custom headers
//!     if let Some(priority) = event.headers.get("X-Priority") {
//!         println!("Call priority: {}", priority);
//!     }
//! }
//! ```

use crate::api::types::{SessionId};
use std::collections::HashMap;

/// Enhanced incoming call event with detailed information
/// 
/// This struct provides comprehensive information about an incoming call,
/// including all SIP headers and parsed caller details. It's designed for
/// server applications that need to make routing or policy decisions based
/// on detailed call metadata.
/// 
/// # Example
/// 
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// fn route_by_headers(event: &IncomingCallEvent) -> String {
///     // Route based on custom headers
///     if let Some(dept) = event.headers.get("X-Department") {
///         return format!("sip:queue@{}.internal", dept);
///     }
///     
///     // Route based on user agent
///     if let Some(ua) = &event.caller_info.user_agent {
///         if ua.contains("Mobile") {
///             return "sip:mobile@queue.internal".to_string();
///         }
///     }
///     
///     // Default route
///     "sip:default@queue.internal".to_string()
/// }
/// ```
#[derive(Debug, Clone)]
pub struct IncomingCallEvent {
    /// Unique session identifier
    pub session_id: SessionId,
    /// Detailed caller information
    pub caller_info: CallerInfo,
    /// SIP Call-ID header value
    pub call_id: String,
    /// All SIP headers from the INVITE
    pub headers: HashMap<String, String>,
    /// SDP offer from the caller (if present)
    pub sdp: Option<String>,
}

/// Detailed caller information extracted from SIP headers
/// 
/// This struct provides parsed information about the caller, making it
/// easier to implement routing rules, access control, or logging without
/// manually parsing SIP headers.
/// 
/// # Builder Pattern
/// 
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// let caller = CallerInfo::new("sip:alice@example.com", "sip:support@ourcompany.com")
///     .with_display_name("Alice Smith")
///     .with_user_agent("SoftPhone/2.0")
///     .with_contact("sip:alice@192.168.1.100:5060");
/// ```
#[derive(Debug, Clone)]
pub struct CallerInfo {
    /// SIP From URI (caller's address)
    pub from: String,
    /// SIP To URI (called party address)
    pub to: String,
    /// Display name from From header (e.g., "Alice Smith")
    pub display_name: Option<String>,
    /// User-Agent header value (client software identification)
    pub user_agent: Option<String>,
    /// Contact header value (direct contact address)
    pub contact: Option<String>,
}

impl CallerInfo {
    /// Create basic caller info with just from and to addresses
    /// 
    /// # Arguments
    /// * `from` - Caller's SIP URI
    /// * `to` - Called party's SIP URI
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            display_name: None,
            user_agent: None,
            contact: None,
        }
    }
    
    /// Set the display name (builder pattern)
    /// 
    /// The display name is typically shown in call logs and UI.
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
    
    /// Set the user agent (builder pattern)
    /// 
    /// The user agent identifies the calling software/device.
    pub fn with_user_agent(mut self, agent: impl Into<String>) -> Self {
        self.user_agent = Some(agent.into());
        self
    }
    
    /// Set the contact address (builder pattern)
    /// 
    /// The contact address is the direct SIP address where the caller can be reached.
    pub fn with_contact(mut self, contact: impl Into<String>) -> Self {
        self.contact = Some(contact.into());
        self
    }
    
    /// Extract the username portion from the 'from' URI
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::CallerInfo;
    /// 
    /// let caller = CallerInfo::new("sip:alice@example.com", "sip:bob@example.com");
    /// assert_eq!(caller.from_username(), Some("alice"));
    /// ```
    pub fn from_username(&self) -> Option<&str> {
        self.from.strip_prefix("sip:")
            .and_then(|s| s.split('@').next())
    }
    
    /// Extract the domain portion from the 'from' URI
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::CallerInfo;
    /// 
    /// let caller = CallerInfo::new("sip:alice@example.com", "sip:bob@example.com");
    /// assert_eq!(caller.from_domain(), Some("example.com"));
    /// ```
    pub fn from_domain(&self) -> Option<&str> {
        self.from.split('@').nth(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_caller_info_builder() {
        let info = CallerInfo::new("sip:alice@example.com", "sip:bob@example.com")
            .with_display_name("Alice")
            .with_user_agent("RVOIP/1.0");
            
        assert_eq!(info.from, "sip:alice@example.com");
        assert_eq!(info.to, "sip:bob@example.com");
        assert_eq!(info.display_name, Some("Alice".to_string()));
        assert_eq!(info.user_agent, Some("RVOIP/1.0".to_string()));
        assert_eq!(info.contact, None);
    }
    
    #[test]
    fn test_uri_parsing() {
        let info = CallerInfo::new("sip:alice@example.com", "sip:bob@company.org");
        
        assert_eq!(info.from_username(), Some("alice"));
        assert_eq!(info.from_domain(), Some("example.com"));
    }
} 