//! # SIP Subscription-State Header
//!
//! This module provides an implementation of the SIP Subscription-State header as defined in
//! [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665).
//!
//! The Subscription-State header field is used in NOTIFY requests to indicate the state of
//! a subscription. It tells the subscriber whether the subscription is active, pending, or
//! terminated, and provides additional information such as expiration time and reason for
//! termination.
//!
//! ## Structure
//!
//! The Subscription-State header contains:
//! - A state value (active, pending, or terminated)
//! - Optional parameters:
//!   - `expires`: Time remaining until subscription expires (in seconds)
//!   - `reason`: Reason for termination (for terminated state)
//!   - `retry-after`: Suggested wait time before re-subscribing (in seconds)
//!
//! ## Format
//!
//! ```text
//! Subscription-State: active;expires=3600
//! Subscription-State: pending;expires=600
//! Subscription-State: terminated;reason=timeout
//! Subscription-State: terminated;reason=giveup;retry-after=3600
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::subscription_state::{SubscriptionState, SubState, TerminationReason};
//! use std::str::FromStr;
//!
//! // Create an active subscription state
//! let active = SubscriptionState::active(3600);
//! assert_eq!(active.to_string(), "active;expires=3600");
//!
//! // Create a pending subscription state
//! let pending = SubscriptionState::pending(600);
//! assert_eq!(pending.to_string(), "pending;expires=600");
//!
//! // Create a terminated subscription state with reason
//! let terminated = SubscriptionState::terminated(TerminationReason::Timeout);
//! assert_eq!(terminated.to_string(), "terminated;reason=timeout");
//!
//! // Create a terminated state with retry-after
//! let terminated_retry = SubscriptionState::terminated_with_retry(
//!     TerminationReason::Giveup,
//!     3600
//! );
//! assert_eq!(terminated_retry.to_string(), "terminated;reason=giveup;retry-after=3600");
//! ```

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

use crate::types::headers::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::{Error, Result};

/// The state of a subscription
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubState {
    /// The subscription is active and notifications will be sent
    Active,
    /// The subscription is pending authorization
    Pending,
    /// The subscription has been terminated
    Terminated,
}

impl fmt::Display for SubState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubState::Active => write!(f, "active"),
            SubState::Pending => write!(f, "pending"),
            SubState::Terminated => write!(f, "terminated"),
        }
    }
}

impl FromStr for SubState {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "active" => Ok(SubState::Active),
            "pending" => Ok(SubState::Pending),
            "terminated" => Ok(SubState::Terminated),
            _ => Err(Error::ParseError(format!("Invalid subscription state: {}", s))),
        }
    }
}

/// Reasons for subscription termination
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TerminationReason {
    /// The subscription expired
    Timeout,
    /// The notifier could not obtain authorization
    Rejected,
    /// The resource state no longer exists
    NoResource,
    /// The subscription has been terminated by the notifier
    Deactivated,
    /// The notifier experienced an error
    Probation,
    /// The notifier is giving up on this subscription
    Giveup,
    /// No reason provided
    Noresource,
    /// Custom termination reason
    Other(String),
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminationReason::Timeout => write!(f, "timeout"),
            TerminationReason::Rejected => write!(f, "rejected"),
            TerminationReason::NoResource => write!(f, "noresource"),
            TerminationReason::Deactivated => write!(f, "deactivated"),
            TerminationReason::Probation => write!(f, "probation"),
            TerminationReason::Giveup => write!(f, "giveup"),
            TerminationReason::Noresource => write!(f, "noresource"),
            TerminationReason::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for TerminationReason {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_lowercase().as_str() {
            "timeout" => TerminationReason::Timeout,
            "rejected" => TerminationReason::Rejected,
            "noresource" => TerminationReason::NoResource,
            "deactivated" => TerminationReason::Deactivated,
            "probation" => TerminationReason::Probation,
            "giveup" => TerminationReason::Giveup,
            _ => TerminationReason::Other(s.to_string()),
        })
    }
}

/// Represents the Subscription-State header
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionState {
    /// The subscription state
    pub state: SubState,
    
    /// Time until subscription expires (in seconds)
    pub expires: Option<u32>,
    
    /// Reason for termination (only for terminated state)
    pub reason: Option<TerminationReason>,
    
    /// Suggested wait time before re-subscribing (in seconds)
    pub retry_after: Option<u32>,
}

impl SubscriptionState {
    /// Create a new SubscriptionState with the given state
    pub fn new(state: SubState) -> Self {
        Self {
            state,
            expires: None,
            reason: None,
            retry_after: None,
        }
    }
    
    /// Create an active subscription state with expiration
    pub fn active(expires: u32) -> Self {
        Self {
            state: SubState::Active,
            expires: Some(expires),
            reason: None,
            retry_after: None,
        }
    }
    
    /// Create a pending subscription state with expiration
    pub fn pending(expires: u32) -> Self {
        Self {
            state: SubState::Pending,
            expires: Some(expires),
            reason: None,
            retry_after: None,
        }
    }
    
    /// Create a terminated subscription state with reason
    pub fn terminated(reason: TerminationReason) -> Self {
        Self {
            state: SubState::Terminated,
            expires: None,
            reason: Some(reason),
            retry_after: None,
        }
    }
    
    /// Create a terminated subscription state with reason and retry-after
    pub fn terminated_with_retry(reason: TerminationReason, retry_after: u32) -> Self {
        Self {
            state: SubState::Terminated,
            expires: None,
            reason: Some(reason),
            retry_after: Some(retry_after),
        }
    }
    
    /// Set the expires parameter
    pub fn with_expires(mut self, expires: u32) -> Self {
        self.expires = Some(expires);
        self
    }
    
    /// Set the reason parameter
    pub fn with_reason(mut self, reason: TerminationReason) -> Self {
        self.reason = Some(reason);
        self
    }
    
    /// Set the retry-after parameter
    pub fn with_retry_after(mut self, retry_after: u32) -> Self {
        self.retry_after = Some(retry_after);
        self
    }
}

impl fmt::Display for SubscriptionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.state)?;
        
        if let Some(expires) = self.expires {
            write!(f, ";expires={}", expires)?;
        }
        
        if let Some(reason) = &self.reason {
            write!(f, ";reason={}", reason)?;
        }
        
        if let Some(retry_after) = self.retry_after {
            write!(f, ";retry-after={}", retry_after)?;
        }
        
        Ok(())
    }
}

impl FromStr for SubscriptionState {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(';').collect();
        if parts.is_empty() {
            return Err(Error::ParseError("Empty Subscription-State header".to_string()));
        }
        
        let state = SubState::from_str(parts[0])?;
        let mut subscription_state = SubscriptionState::new(state);
        
        // Parse parameters
        for part in &parts[1..] {
            let param_parts: Vec<&str> = part.splitn(2, '=').collect();
            match param_parts[0].trim() {
                "expires" if param_parts.len() == 2 => {
                    subscription_state.expires = param_parts[1].parse().ok();
                }
                "reason" if param_parts.len() == 2 => {
                    subscription_state.reason = Some(TerminationReason::from_str(param_parts[1])?);
                }
                "retry-after" if param_parts.len() == 2 => {
                    subscription_state.retry_after = param_parts[1].parse().ok();
                }
                _ => {} // Ignore unknown parameters
            }
        }
        
        Ok(subscription_state)
    }
}

impl TypedHeaderTrait for SubscriptionState {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::SubscriptionState
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Invalid header name for Subscription-State: expected {}, got {}",
                Self::header_name(),
                header.name
            )));
        }
        
        match &header.value {
            HeaderValue::Raw(bytes) => {
                let value = String::from_utf8_lossy(bytes);
                Self::from_str(&value)
            }
            _ => Err(Error::InvalidHeader(
                "Subscription-State header value must be raw text".to_string()
            )),
        }
    }
    
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(self.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_subscription_state_active() {
        let state = SubscriptionState::active(3600);
        assert_eq!(state.to_string(), "active;expires=3600");
        
        let parsed = SubscriptionState::from_str("active;expires=3600").unwrap();
        assert_eq!(parsed, state);
    }
    
    #[test]
    fn test_subscription_state_pending() {
        let state = SubscriptionState::pending(600);
        assert_eq!(state.to_string(), "pending;expires=600");
        
        let parsed = SubscriptionState::from_str("pending;expires=600").unwrap();
        assert_eq!(parsed, state);
    }
    
    #[test]
    fn test_subscription_state_terminated() {
        let state = SubscriptionState::terminated(TerminationReason::Timeout);
        assert_eq!(state.to_string(), "terminated;reason=timeout");
        
        let parsed = SubscriptionState::from_str("terminated;reason=timeout").unwrap();
        assert_eq!(parsed, state);
    }
    
    #[test]
    fn test_subscription_state_terminated_with_retry() {
        let state = SubscriptionState::terminated_with_retry(
            TerminationReason::Giveup,
            3600
        );
        assert_eq!(state.to_string(), "terminated;reason=giveup;retry-after=3600");
        
        let parsed = SubscriptionState::from_str("terminated;reason=giveup;retry-after=3600").unwrap();
        assert_eq!(parsed, state);
    }
    
    #[test]
    fn test_substate_from_str() {
        assert_eq!(SubState::from_str("active").unwrap(), SubState::Active);
        assert_eq!(SubState::from_str("PENDING").unwrap(), SubState::Pending);
        assert_eq!(SubState::from_str("Terminated").unwrap(), SubState::Terminated);
        assert!(SubState::from_str("invalid").is_err());
    }
    
    #[test]
    fn test_termination_reason_from_str() {
        assert_eq!(TerminationReason::from_str("timeout").unwrap(), TerminationReason::Timeout);
        assert_eq!(TerminationReason::from_str("rejected").unwrap(), TerminationReason::Rejected);
        assert_eq!(TerminationReason::from_str("custom").unwrap(), TerminationReason::Other("custom".to_string()));
    }
}