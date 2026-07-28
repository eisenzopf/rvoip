//! Event definitions for registrar and presence

use crate::types::{ContactInfo, PresenceStatus};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt;

// Import Event trait from infra-common
use rvoip_infra_common::events::types::{Event, EventPriority};

/// Registration-related events
#[derive(Clone, Serialize, Deserialize)]
pub enum RegistrarEvent {
    /// User registered
    UserRegistered { user: String, contact: ContactInfo },

    /// User unregistered
    UserUnregistered { user: String },

    /// Registration expired
    RegistrationExpired { user: String },

    /// Registration refreshed
    RegistrationRefreshed { user: String, expires: u32 },

    /// Contact added
    ContactAdded { user: String, contact: ContactInfo },

    /// Contact removed
    ContactRemoved { user: String, uri: String },
}

impl fmt::Debug for RegistrarEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserRegistered { contact, .. } => formatter
                .debug_struct("UserRegistered")
                .field("contact_transport", &contact.transport)
                .field("contact_method_count", &contact.methods.len())
                .finish(),
            Self::UserUnregistered { .. } => formatter.write_str("UserUnregistered"),
            Self::RegistrationExpired { .. } => formatter.write_str("RegistrationExpired"),
            Self::RegistrationRefreshed { expires, .. } => formatter
                .debug_struct("RegistrationRefreshed")
                .field("expires", expires)
                .finish(),
            Self::ContactAdded { contact, .. } => formatter
                .debug_struct("ContactAdded")
                .field("contact_transport", &contact.transport)
                .field("contact_method_count", &contact.methods.len())
                .finish(),
            Self::ContactRemoved { .. } => formatter.write_str("ContactRemoved"),
        }
    }
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
#[derive(Clone, Serialize, Deserialize)]
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
    Unsubscribed { subscription_id: String },

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
    BuddyListUpdated { user: String, buddy_count: usize },
}

impl fmt::Debug for PresenceEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Updated {
                status,
                note,
                watchers_notified,
                ..
            } => formatter
                .debug_struct("Updated")
                .field("status", &status.diagnostic_kind())
                .field("note_present", &note.is_some())
                .field("watchers_notified", watchers_notified)
                .finish(),
            Self::Subscribed { .. } => formatter.write_str("Subscribed"),
            Self::Unsubscribed { .. } => formatter.write_str("Unsubscribed"),
            Self::SubscriptionExpired { .. } => formatter.write_str("SubscriptionExpired"),
            Self::NotificationSent { .. } => formatter.write_str("NotificationSent"),
            Self::BuddyListUpdated { buddy_count, .. } => formatter
                .debug_struct("BuddyListUpdated")
                .field("buddy_count", buddy_count)
                .finish(),
        }
    }
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
#[derive(Clone, Serialize, Deserialize)]
pub enum RegistrarServiceEvent {
    Registrar(RegistrarEvent),
    Presence(PresenceEvent),
}

impl fmt::Debug for RegistrarServiceEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registrar(event) => formatter.debug_tuple("Registrar").field(event).finish(),
            Self::Presence(event) => formatter.debug_tuple("Presence").field(event).finish(),
        }
    }
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
        let _reg_event = RegistrarEvent::UserRegistered {
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
                reg_id: None,
                flow_id: None,
                reachability: crate::types::ContactReachability::Unknown,
            },
        };

        // The event_type method is on the type itself
        assert_eq!(<RegistrarEvent as Event>::event_type(), "registrar");
        assert_eq!(<PresenceEvent as Event>::event_type(), "presence");
        assert_eq!(
            <RegistrarServiceEvent as Event>::event_type(),
            "registrar_service"
        );
    }

    #[test]
    fn test_event_priority() {
        assert_eq!(RegistrarEvent::priority(), EventPriority::Normal);
        assert_eq!(PresenceEvent::priority(), EventPriority::Normal);
    }

    #[test]
    fn public_event_debug_is_payload_free_while_serde_remains_exact() {
        const CANARY: &str = "registrar-event-direct-secret-canary";
        let contact = ContactInfo {
            uri: format!("sip:{CANARY}@example.test"),
            instance_id: CANARY.into(),
            transport: crate::types::Transport::TLS,
            user_agent: CANARY.into(),
            expires: chrono::Utc::now(),
            q_value: 1.0,
            received: Some(CANARY.into()),
            path: vec![CANARY.into()],
            methods: vec!["INVITE".into()],
            reg_id: Some(1),
            flow_id: Some(CANARY.into()),
            reachability: crate::types::ContactReachability::Reachable,
        };
        let registrar = [
            RegistrarEvent::UserRegistered {
                user: CANARY.into(),
                contact: contact.clone(),
            },
            RegistrarEvent::UserUnregistered {
                user: CANARY.into(),
            },
            RegistrarEvent::RegistrationExpired {
                user: CANARY.into(),
            },
            RegistrarEvent::RegistrationRefreshed {
                user: CANARY.into(),
                expires: 60,
            },
            RegistrarEvent::ContactAdded {
                user: CANARY.into(),
                contact,
            },
            RegistrarEvent::ContactRemoved {
                user: CANARY.into(),
                uri: CANARY.into(),
            },
        ];
        for event in registrar {
            assert!(!format!("{event:?}").contains(CANARY));
            assert!(serde_json::to_string(&event).unwrap().contains(CANARY));
        }

        let presence = [
            PresenceEvent::Updated {
                user: CANARY.into(),
                status: PresenceStatus::Custom(CANARY.into()),
                note: Some(CANARY.into()),
                watchers_notified: 2,
            },
            PresenceEvent::Subscribed {
                subscriber: CANARY.into(),
                target: CANARY.into(),
                subscription_id: CANARY.into(),
            },
            PresenceEvent::Unsubscribed {
                subscription_id: CANARY.into(),
            },
            PresenceEvent::SubscriptionExpired {
                subscription_id: CANARY.into(),
                subscriber: CANARY.into(),
                target: CANARY.into(),
            },
            PresenceEvent::NotificationSent {
                subscription_id: CANARY.into(),
                target: CANARY.into(),
                subscriber: CANARY.into(),
            },
            PresenceEvent::BuddyListUpdated {
                user: CANARY.into(),
                buddy_count: 3,
            },
        ];
        for event in presence {
            assert!(!format!("{event:?}").contains(CANARY));
            assert!(serde_json::to_string(&event).unwrap().contains(CANARY));
        }
    }
}
