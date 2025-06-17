//! Server-oriented Types
//! 
//! Additional types for server applications like call centers.

use crate::api::types::{SessionId};
use std::collections::HashMap;

/// Enhanced incoming call event with detailed information
#[derive(Debug, Clone)]
pub struct IncomingCallEvent {
    pub session_id: SessionId,
    pub caller_info: CallerInfo,
    pub call_id: String,
    pub headers: HashMap<String, String>,
    pub sdp: Option<String>,
}

/// Detailed caller information
#[derive(Debug, Clone)]
pub struct CallerInfo {
    pub from: String,
    pub to: String,
    pub display_name: Option<String>,
    pub user_agent: Option<String>,
    pub contact: Option<String>,
}

impl CallerInfo {
    /// Create basic caller info with just from and to
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            display_name: None,
            user_agent: None,
            contact: None,
        }
    }
    
    /// Builder method to set display name
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
    
    /// Builder method to set user agent
    pub fn with_user_agent(mut self, agent: impl Into<String>) -> Self {
        self.user_agent = Some(agent.into());
        self
    }
    
    /// Builder method to set contact
    pub fn with_contact(mut self, contact: impl Into<String>) -> Self {
        self.contact = Some(contact.into());
        self
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
} 