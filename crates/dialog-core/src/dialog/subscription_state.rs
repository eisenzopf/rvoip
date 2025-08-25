//! Subscription state management for SIP event subscriptions (RFC 6665)
//!
//! This module provides types for managing the state of SIP event subscriptions,
//! including the subscription lifecycle states and expiry tracking.

use std::fmt;
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// Represents the state of a SIP event subscription
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionState {
    /// Subscription is pending (waiting for initial NOTIFY)
    Pending,
    
    /// Subscription is active with expiry time
    Active { 
        /// Remaining duration until expiry
        remaining_duration: Duration,
        /// Original expiry duration
        original_duration: Duration,
    },
    
    /// Subscription is being refreshed
    Refreshing {
        /// Current remaining duration
        current_remaining: Duration,
        /// Requested new duration
        requested_duration: Duration,
    },
    
    /// Subscription has been terminated
    Terminated { 
        /// Optional reason for termination
        reason: Option<SubscriptionTerminationReason>,
    },
}

/// Reasons for subscription termination
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionTerminationReason {
    /// Client requested termination (SUBSCRIBE with Expires: 0)
    ClientRequested,
    
    /// Server terminated (deactivated, no resource, etc.)
    ServerTerminated(String),
    
    /// Subscription expired without refresh
    Expired,
    
    /// Too many failed refresh attempts
    RefreshFailed,
    
    /// Resource no longer exists
    NoResource,
    
    /// Subscription rejected by policy
    Rejected,
    
    /// Network or transport error
    NetworkError,
    
    /// Other/unknown reason
    Other(String),
}

impl fmt::Display for SubscriptionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscriptionState::Pending => write!(f, "pending"),
            SubscriptionState::Active { .. } => write!(f, "active"),
            SubscriptionState::Refreshing { .. } => write!(f, "refreshing"),
            SubscriptionState::Terminated { reason } => {
                match reason {
                    Some(r) => write!(f, "terminated ({})", r),
                    None => write!(f, "terminated"),
                }
            }
        }
    }
}

impl fmt::Display for SubscriptionTerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscriptionTerminationReason::ClientRequested => write!(f, "client requested"),
            SubscriptionTerminationReason::ServerTerminated(msg) => write!(f, "server terminated: {}", msg),
            SubscriptionTerminationReason::Expired => write!(f, "expired"),
            SubscriptionTerminationReason::RefreshFailed => write!(f, "refresh failed"),
            SubscriptionTerminationReason::NoResource => write!(f, "no resource"),
            SubscriptionTerminationReason::Rejected => write!(f, "rejected"),
            SubscriptionTerminationReason::NetworkError => write!(f, "network error"),
            SubscriptionTerminationReason::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl SubscriptionState {
    /// Check if the subscription is active
    pub fn is_active(&self) -> bool {
        matches!(self, SubscriptionState::Active { .. })
    }
    
    /// Check if the subscription is pending
    pub fn is_pending(&self) -> bool {
        matches!(self, SubscriptionState::Pending)
    }
    
    /// Check if the subscription is terminated
    pub fn is_terminated(&self) -> bool {
        matches!(self, SubscriptionState::Terminated { .. })
    }
    
    /// Check if the subscription needs refresh
    pub fn needs_refresh(&self, advance_time: Duration) -> bool {
        match self {
            SubscriptionState::Active { remaining_duration, .. } => {
                *remaining_duration <= advance_time
            },
            _ => false,
        }
    }
    
    /// Get the remaining time until expiry
    pub fn time_until_expiry(&self) -> Option<Duration> {
        match self {
            SubscriptionState::Active { remaining_duration, .. } => Some(*remaining_duration),
            SubscriptionState::Refreshing { current_remaining, .. } => Some(*current_remaining),
            _ => None,
        }
    }
    
    /// Parse from Subscription-State header value
    pub fn from_header_value(value: &str) -> Self {
        let parts: Vec<&str> = value.split(';').collect();
        let state = parts[0].trim().to_lowercase();
        
        match state.as_str() {
            "pending" => SubscriptionState::Pending,
            "active" => {
                // Look for expires parameter
                let expires = parts.iter()
                    .find_map(|part| {
                        let kv: Vec<&str> = part.split('=').collect();
                        if kv.len() == 2 && kv[0].trim() == "expires" {
                            kv[1].trim().parse::<u64>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(3600); // Default to 1 hour
                
                SubscriptionState::Active {
                    remaining_duration: Duration::from_secs(expires),
                    original_duration: Duration::from_secs(expires),
                }
            },
            "terminated" => {
                // Look for reason parameter
                let reason = parts.iter()
                    .find_map(|part| {
                        let kv: Vec<&str> = part.split('=').collect();
                        if kv.len() == 2 && kv[0].trim() == "reason" {
                            Some(kv[1].trim().to_string())
                        } else {
                            None
                        }
                    });
                
                let termination_reason = reason.map(|r| match r.as_str() {
                    "deactivated" => SubscriptionTerminationReason::ClientRequested,
                    "noresource" => SubscriptionTerminationReason::NoResource,
                    "rejected" => SubscriptionTerminationReason::Rejected,
                    "timeout" => SubscriptionTerminationReason::Expired,
                    other => SubscriptionTerminationReason::ServerTerminated(other.to_string()),
                });
                
                SubscriptionState::Terminated { reason: termination_reason }
            },
            _ => SubscriptionState::Pending, // Default to pending for unknown states
        }
    }
    
    /// Convert to Subscription-State header value
    pub fn to_header_value(&self) -> String {
        match self {
            SubscriptionState::Pending => "pending".to_string(),
            SubscriptionState::Active { original_duration, .. } => {
                format!("active;expires={}", original_duration.as_secs())
            },
            SubscriptionState::Refreshing { requested_duration, .. } => {
                format!("active;expires={}", requested_duration.as_secs())
            },
            SubscriptionState::Terminated { reason } => {
                match reason {
                    Some(SubscriptionTerminationReason::ClientRequested) => "terminated;reason=deactivated".to_string(),
                    Some(SubscriptionTerminationReason::NoResource) => "terminated;reason=noresource".to_string(),
                    Some(SubscriptionTerminationReason::Rejected) => "terminated;reason=rejected".to_string(),
                    Some(SubscriptionTerminationReason::Expired) => "terminated;reason=timeout".to_string(),
                    Some(SubscriptionTerminationReason::ServerTerminated(msg)) => format!("terminated;reason={}", msg),
                    _ => "terminated".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_subscription_state_display() {
        assert_eq!(SubscriptionState::Pending.to_string(), "pending");
        
        let active = SubscriptionState::Active {
            remaining_duration: Duration::from_secs(3600),
            original_duration: Duration::from_secs(3600),
        };
        assert_eq!(active.to_string(), "active");
        
        let terminated = SubscriptionState::Terminated {
            reason: Some(SubscriptionTerminationReason::Expired),
        };
        assert_eq!(terminated.to_string(), "terminated (expired)");
    }
    
    #[test]
    fn test_from_header_value() {
        let pending = SubscriptionState::from_header_value("pending");
        assert!(pending.is_pending());
        
        let active = SubscriptionState::from_header_value("active;expires=1800");
        assert!(active.is_active());
        
        let terminated = SubscriptionState::from_header_value("terminated;reason=noresource");
        assert!(terminated.is_terminated());
    }
    
    #[test]
    fn test_to_header_value() {
        assert_eq!(SubscriptionState::Pending.to_header_value(), "pending");
        
        let active = SubscriptionState::Active {
            remaining_duration: Duration::from_secs(3600),
            original_duration: Duration::from_secs(3600),
        };
        assert_eq!(active.to_header_value(), "active;expires=3600");
        
        let terminated = SubscriptionState::Terminated {
            reason: Some(SubscriptionTerminationReason::NoResource),
        };
        assert_eq!(terminated.to_header_value(), "terminated;reason=noresource");
    }
}