//! Event definitions for registrar and presence

use serde::{Deserialize, Serialize};
use std::any::Any;
use crate::types::{ContactInfo, PresenceStatus};

// Import Event trait from infra-common
use infra_common::events::types::{Event, EventPriority};

/// Registration-related events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistrarEvent {
    /// User registered
    UserRegistered {
        user: String,
        contact: ContactInfo,
    },
    
    /// User unregistered
    UserUnregistered {
        user: String,
    },
    
    /// Registration expired
    RegistrationExpired {
        user: String,
    },
    
    /// Registration refreshed
    RegistrationRefreshed {
        user: String,
        expires: u32,
    },
    
    /// Contact added
    ContactAdded {
        user: String,
        contact: ContactInfo,
    },
    
    /// Contact removed
    ContactRemoved {
        user: String,
        uri: String,
    },
}

impl Event for RegistrarEvent {
    fn event_type() -> &'static str {
        "registrar"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Presence-related events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PresenceEvent {
    /// Presence updated
    Updated {
        user: String,
        status: PresenceStatus,
        note: Option<String>,
        watchers_notified: usize,
    },
    
    /// Subscription created
    Subscribed {
        subscriber: String,
        target: String,
        subscription_id: String,
    },
    
    /// Subscription terminated
    Unsubscribed {
        subscription_id: String,
    },
    
    /// Subscription expired
    SubscriptionExpired {
        subscription_id: String,
        subscriber: String,
        target: String,
    },
    
    /// Notification sent
    NotificationSent {
        subscription_id: String,
        target: String,
        subscriber: String,
    },
    
    /// Buddy list updated
    BuddyListUpdated {
        user: String,
        buddy_count: usize,
    },
}

impl Event for PresenceEvent {
    fn event_type() -> &'static str {
        "presence"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Combined event type for convenience
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistrarServiceEvent {
    Registrar(RegistrarEvent),
    Presence(PresenceEvent),
}

impl Event for RegistrarServiceEvent {
    fn event_type() -> &'static str {
        "registrar_service"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Event adapter for session-core integration
/// 
/// This adapter subscribes to registrar events and triggers
/// appropriate SIP signaling through session-core
pub struct RegistrarEventAdapter {
    /// Reference to session-core's event handler
    session_handler: Option<Box<dyn Fn(RegistrarServiceEvent) + Send + Sync>>,
}

impl RegistrarEventAdapter {
    pub fn new() -> Self {
        Self {
            session_handler: None,
        }
    }
    
    /// Set the session-core event handler
    pub fn set_session_handler<F>(&mut self, handler: F)
    where
        F: Fn(RegistrarServiceEvent) + Send + Sync + 'static,
    {
        self.session_handler = Some(Box::new(handler));
    }
    
    /// Handle a registrar event
    pub fn handle_registrar_event(&self, event: RegistrarEvent) {
        if let Some(handler) = &self.session_handler {
            handler(RegistrarServiceEvent::Registrar(event));
        }
    }
    
    /// Handle a presence event
    pub fn handle_presence_event(&self, event: PresenceEvent) {
        if let Some(handler) = &self.session_handler {
            handler(RegistrarServiceEvent::Presence(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_event_types() {
        // Event trait requires an instance, not a type
        let reg_event = RegistrarEvent::UserRegistered {
            user: "test".to_string(),
            contact: ContactInfo {
                uri: "sip:test@example.com".to_string(),
                instance_id: "device1".to_string(),
                transport: crate::types::Transport::UDP,
                user_agent: "test".to_string(),
                expires: chrono::Utc::now(),
                q_value: 1.0,
                received: None,
                path: Vec::new(),
                methods: Vec::new(),
            },
        };
        
        // The event_type method is on the type itself
        assert_eq!(<RegistrarEvent as Event>::event_type(), "registrar");
        assert_eq!(<PresenceEvent as Event>::event_type(), "presence");
        assert_eq!(<RegistrarServiceEvent as Event>::event_type(), "registrar_service");
    }
    
    #[test]
    fn test_event_priority() {
        assert_eq!(RegistrarEvent::priority(), EventPriority::Normal);
        assert_eq!(PresenceEvent::priority(), EventPriority::Normal);
    }
}