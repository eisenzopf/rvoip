//! Dialog implementation for RFC 3261 SIP dialogs
//!
//! This module contains the main Dialog struct and its implementation,
//! handling dialog creation, state management, and request/response processing.

use std::net::SocketAddr;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use tracing::debug;

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, 
    Uri, HeaderName, TypedHeader
};

use crate::transaction::utils::DialogRequestTemplate;

use super::dialog_state::DialogState;
use super::dialog_id::DialogId;
use super::dialog_utils::extract_uri_from_contact;
use super::subscription_state::SubscriptionState;
use crate::errors::{DialogError, DialogResult};
use tokio::task::JoinHandle;

/// A SIP dialog as defined in RFC 3261
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dialog {
    /// Unique identifier for this dialog
    pub id: DialogId,
    
    /// Current state of the dialog
    pub state: DialogState,
    
    /// Call-ID for this dialog
    pub call_id: String,
    
    /// Local URI
    pub local_uri: Uri,
    
    /// Remote URI
    pub remote_uri: Uri,
    
    /// Local tag
    pub local_tag: Option<String>,
    
    /// Remote tag
    pub remote_tag: Option<String>,
    
    /// Local sequence number
    pub local_cseq: u32,
    
    /// Remote sequence number
    pub remote_cseq: u32,
    
    /// Remote target URI (where to send requests)
    pub remote_target: Uri,
    
    /// Route set for this dialog
    pub route_set: Vec<Uri>,
    
    /// Whether this dialog was created by local UA (true) or remote UA (false)
    pub is_initiator: bool,
    
    /// Last known good remote socket address
    pub last_known_remote_addr: Option<std::net::SocketAddr>,
    
    /// Time of the last successful transaction
    pub last_successful_transaction_time: Option<std::time::SystemTime>,
    
    /// Number of recovery attempts made
    pub recovery_attempts: u32,
    
    /// Reason for recovery (if in recovering state)
    pub recovery_reason: Option<String>,
    
    /// Time when the dialog was last successfully recovered
    pub recovered_at: Option<std::time::SystemTime>,
    
    /// Time when recovery was started
    pub recovery_start_time: Option<std::time::SystemTime>,
    
    // Subscription-specific fields (RFC 6665)
    
    /// Subscription state for event subscriptions
    pub subscription_state: Option<SubscriptionState>,
    
    /// Event package being subscribed to (e.g., "presence", "dialog", "message-summary")
    pub event_package: Option<String>,
    
    /// Event ID for this subscription (if any)
    pub event_id: Option<String>,
    
    /// Number of failed refresh attempts
    pub refresh_failures: u32,
    
    /// Maximum refresh failures before termination
    pub max_refresh_failures: u32,
}

impl Dialog {
    /// Create a new dialog
    pub fn new(
        call_id: String,
        local_uri: Uri,
        remote_uri: Uri,
        local_tag: Option<String>,
        remote_tag: Option<String>,
        is_initiator: bool,
    ) -> Self {
        Self {
            id: DialogId::new(),
            state: DialogState::Initial,
            call_id,
            local_uri,
            remote_uri: remote_uri.clone(),
            local_tag,
            remote_tag,
            local_cseq: 0,
            remote_cseq: 0,
            remote_target: remote_uri, // Initially same as remote URI
            route_set: Vec::new(),
            is_initiator,
            last_known_remote_addr: None,
            last_successful_transaction_time: None,
            recovery_attempts: 0,
            recovery_reason: None,
            recovered_at: None,
            recovery_start_time: None,
            subscription_state: None,
            event_package: None,
            event_id: None,
            refresh_failures: 0,
            max_refresh_failures: 3,
        }
    }
    
    /// Create a new early dialog
    pub fn new_early(
        call_id: String,
        local_uri: Uri,
        remote_uri: Uri,
        local_tag: Option<String>,
        remote_tag: Option<String>,
        is_initiator: bool,
    ) -> Self {
        let mut dialog = Self::new(call_id, local_uri, remote_uri, local_tag, remote_tag, is_initiator);
        dialog.state = DialogState::Early;
        dialog
    }
    
    /// Generate a local tag for this dialog
    pub fn generate_local_tag(&self) -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        format!("{:08x}", rng.gen::<u32>())
    }
    
    /// Confirm the dialog with a local tag
    pub fn confirm_with_tag(&mut self, local_tag: String) {
        self.local_tag = Some(local_tag);
        self.state = DialogState::Confirmed;
    }
    
    /// Update remote sequence number from an incoming request
    pub fn update_remote_sequence(&mut self, request: &Request) -> DialogResult<()> {
        if let Some(TypedHeader::CSeq(cseq)) = request.header(&HeaderName::CSeq) {
            let new_seq = cseq.sequence();
            
            // Validate sequence number (should be higher than last known)
            if new_seq <= self.remote_cseq && self.remote_cseq != 0 {
                return Err(DialogError::protocol_error(&format!(
                    "Invalid CSeq: got {}, expected > {}", new_seq, self.remote_cseq
                )));
            }
            
            self.remote_cseq = new_seq;
            Ok(())
        } else {
            Err(DialogError::protocol_error("Request missing CSeq header"))
        }
    }
    
    /// Get the remote target address (for sending requests)
    pub async fn get_remote_target_address(&self) -> Option<SocketAddr> {
        // Use the last known address if available
        if let Some(addr) = self.last_known_remote_addr {
            return Some(addr);
        }
        
        // Otherwise, try to resolve the remote target URI
        crate::dialog::dialog_utils::resolve_uri_to_socketaddr(&self.remote_target).await
    }
    
    /// Create a dialog from a 2xx response to an INVITE
    pub fn from_2xx_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        if !matches!(response.status, StatusCode::Ok | StatusCode::Accepted) {
            debug!("Dialog creation failed: Response status is not 200 OK or 202 Accepted ({})", response.status);
            return None;
        }
        
        if request.method != Method::Invite {
            debug!("Dialog creation failed: Request method is not INVITE ({})", request.method);
            return None;
        }
        
        // Extract Call-ID
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => {
                debug!("Dialog creation failed: Missing or invalid Call-ID header");
                return None;
            }
        };
        
        // Extract CSeq
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => {
                debug!("Dialog creation failed: Missing or invalid CSeq header in request");
                return None;
            }
        };
        
        // Extract To and From headers
        let to_header = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => to,
            _ => {
                debug!("Dialog creation failed: Missing or invalid To header");
                return None;
            }
        };
        
        let from_header = match response.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => from,
            _ => {
                debug!("Dialog creation failed: Missing or invalid From header");
                return None;
            }
        };
        
        // Extract tags
        let to_tag = to_header.tag();
        let from_tag = from_header.tag();
        
        // Set local and remote tags and URIs based on initiator status
        let (local_tag, remote_tag, local_uri, remote_uri) = if is_initiator {
            // Local UA initiated, so local is From, remote is To
            (from_tag.map(|s| s.to_string()), 
             to_tag.map(|s| s.to_string()),
             from_header.uri().clone(), 
             to_header.uri().clone())
        } else {
            // Remote UA initiated, so local is To, remote is From
            (to_tag.map(|s| s.to_string()), 
             from_tag.map(|s| s.to_string()),
             to_header.uri().clone(), 
             from_header.uri().clone())
        };
        
        // Extract contact URI
        let remote_target = match response.header(&HeaderName::Contact) {
            Some(TypedHeader::Contact(contacts)) => {
                if let Some(contact) = contacts.0.first() {
                    extract_uri_from_contact(contact).ok()?
                } else {
                    debug!("Dialog creation failed: Empty Contact header");
                    return None;
                }
            },
            _ => {
                debug!("Dialog creation failed: Missing Contact header");
                return None;
            }
        };
        
        // Extract Route set from Record-Route headers
        let route_set = extract_route_set(response, is_initiator);
        
        Some(Self {
            id: DialogId::new(),
            state: DialogState::Confirmed,
            call_id,
            local_uri,
            remote_uri,
            local_tag,
            remote_tag,
            local_cseq: if is_initiator { cseq_number } else { 0 },
            remote_cseq: if is_initiator { 0 } else { cseq_number },
            remote_target,
            route_set,
            is_initiator,
            last_known_remote_addr: None,
            last_successful_transaction_time: None,
            recovery_attempts: 0,
            recovery_reason: None,
            recovered_at: None,
            recovery_start_time: None,
            subscription_state: None,
            event_package: None,
            event_id: None,
            refresh_failures: 0,
            max_refresh_failures: 3,
        })
    }
    
    /// Create a dialog from an early (1xx) response to an INVITE
    pub fn from_provisional_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        // Only certain provisional responses can create dialogs
        if !matches!(response.status,
            StatusCode::Ringing | 
            StatusCode::SessionProgress | 
            StatusCode::CallIsBeingForwarded | 
            StatusCode::Queued) {
            return None;
        }
        
        if request.method != Method::Invite {
            return None;
        }
        
        // To tag is required for early dialog
        let to_header = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => to,
            _ => return None
        };
        
        if to_header.tag().is_none() {
            return None;  // No tag in To header, can't create early dialog
        }
        
        // Similar extraction logic to from_2xx_response but for early dialog
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => return None
        };
        
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => return None
        };
        
        let from_header = match response.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => from,
            _ => return None
        };
        
        let (local_tag, remote_tag, local_uri, remote_uri) = if is_initiator {
            (from_header.tag().map(|s| s.to_string()), 
             to_header.tag().map(|s| s.to_string()),
             from_header.uri().clone(), 
             to_header.uri().clone())
        } else {
            (to_header.tag().map(|s| s.to_string()), 
             from_header.tag().map(|s| s.to_string()),
             to_header.uri().clone(), 
             from_header.uri().clone())
        };
        
        let remote_target = match response.header(&HeaderName::Contact) {
            Some(TypedHeader::Contact(contacts)) => {
                if let Some(contact) = contacts.0.first() {
                    extract_uri_from_contact(contact).ok()?
                } else {
                    return None;
                }
            },
            _ => return None
        };
        
        let route_set = extract_route_set(response, is_initiator);
        
        Some(Self {
            id: DialogId::new(),
            state: DialogState::Early,
            call_id,
            local_uri,
            remote_uri,
            local_tag,
            remote_tag,
            local_cseq: if is_initiator { cseq_number } else { 0 },
            remote_cseq: if is_initiator { 0 } else { cseq_number },
            remote_target,
            route_set,
            is_initiator,
            last_known_remote_addr: None,
            last_successful_transaction_time: None,
            recovery_attempts: 0,
            recovery_reason: None,
            recovered_at: None,
            recovery_start_time: None,
            subscription_state: None,
            event_package: None,
            event_id: None,
            refresh_failures: 0,
            max_refresh_failures: 3,
        })
    }
    
    // ===== Subscription-specific methods (RFC 6665) =====
    
    /// Initialize dialog for subscription with event package
    pub fn init_subscription(&mut self, event_package: String, event_id: Option<String>, expires: u32) {
        use std::time::{Duration, Instant};
        
        self.event_package = Some(event_package);
        self.event_id = event_id;
        
        if expires > 0 {
            self.subscription_state = Some(SubscriptionState::Active {
                remaining_duration: Duration::from_secs(expires as u64),
                original_duration: Duration::from_secs(expires as u64),
            });
        } else {
            // Expires: 0 means immediate termination
            self.subscription_state = Some(SubscriptionState::Terminated {
                reason: Some(crate::dialog::SubscriptionTerminationReason::ClientRequested),
            });
        }
    }
    
    /// Update subscription state from received NOTIFY
    pub fn update_subscription_from_notify(&mut self, subscription_state_header: &str) {
        self.subscription_state = Some(SubscriptionState::from_header_value(subscription_state_header));
    }
    
    /// Check if subscription needs refresh
    pub fn subscription_needs_refresh(&self) -> bool {
        use std::time::Duration;
        
        if let Some(ref state) = self.subscription_state {
            // Refresh 30 seconds before expiry
            state.needs_refresh(Duration::from_secs(30))
        } else {
            false
        }
    }
    
    /// Mark subscription as refreshing
    pub fn start_subscription_refresh(&mut self, new_expires: u32) {
        use std::time::{Duration, Instant};
        
        if let Some(SubscriptionState::Active { remaining_duration, .. }) = self.subscription_state {
            self.subscription_state = Some(SubscriptionState::Refreshing {
                current_remaining: remaining_duration,
                requested_duration: Duration::from_secs(new_expires as u64),
            });
        }
    }
    
    /// Complete subscription refresh
    pub fn complete_subscription_refresh(&mut self, new_expires: u32) {
        self.subscription_state = Some(SubscriptionState::Active {
            remaining_duration: Duration::from_secs(new_expires as u64),
            original_duration: Duration::from_secs(new_expires as u64),
        });
        self.refresh_failures = 0; // Reset failure counter on success
    }
    
    /// Record subscription refresh failure
    pub fn record_refresh_failure(&mut self) {
        self.refresh_failures += 1;
        
        if self.refresh_failures >= self.max_refresh_failures {
            self.subscription_state = Some(SubscriptionState::Terminated {
                reason: Some(crate::dialog::SubscriptionTerminationReason::RefreshFailed),
            });
        }
    }
    
    /// Terminate subscription
    pub fn terminate_subscription(&mut self, reason: Option<crate::dialog::SubscriptionTerminationReason>) {
        self.subscription_state = Some(SubscriptionState::Terminated { reason });
        
        // Refresh timer will be handled by SubscriptionManager
    }
    
    /// Check if this is a subscription dialog
    pub fn is_subscription(&self) -> bool {
        self.event_package.is_some()
    }
    
    /// Get subscription expiry time
    pub fn subscription_expiry(&self) -> Option<std::time::Duration> {
        self.subscription_state.as_ref()?.time_until_expiry()
    }
    
    /// Create a new request within this dialog
    /// 
    /// **ARCHITECTURAL NOTE**: This method creates a dialog-aware request template
    /// that should be processed by transaction-core helpers for proper RFC 3261 compliance.
    /// The DialogManager's transaction integration layer handles the complete request creation.
    pub fn create_request_template(&mut self, method: Method) -> DialogRequestTemplate {
        // Increment local sequence number for new request (except ACK)
        if method != Method::Ack {
            self.local_cseq += 1;
        }
        
        DialogRequestTemplate {
            method: method.clone(),
            target_uri: self.remote_target.clone(),
            call_id: self.call_id.clone(),
            local_uri: self.local_uri.clone(),
            remote_uri: self.remote_uri.clone(),
            local_tag: self.local_tag.clone(),
            remote_tag: self.remote_tag.clone(),
            cseq_number: self.local_cseq,
            route_set: self.route_set.clone(),
        }
    }
    
    /// Get the dialog ID tuple (Call-ID, local tag, remote tag)
    pub fn dialog_id_tuple(&self) -> Option<(String, String, String)> {
        if let (Some(local_tag), Some(remote_tag)) = (&self.local_tag, &self.remote_tag) {
            Some((self.call_id.clone(), local_tag.clone(), remote_tag.clone()))
        } else {
            None
        }
    }
    
    /// Update dialog state from a 2xx response
    pub fn update_from_2xx(&mut self, response: &Response) -> bool {
        if self.state == DialogState::Early {
            self.state = DialogState::Confirmed;
            
            // Update remote tag if not set
            if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
                if let Some(tag) = to.tag() {
                    self.remote_tag = Some(tag.to_string());
                }
            }
            
            // Update remote target from Contact
            if let Some(TypedHeader::Contact(contacts)) = response.header(&HeaderName::Contact) {
                if let Some(contact) = contacts.0.first() {
                    if let Ok(uri) = extract_uri_from_contact(contact) {
                        self.remote_target = uri;
                    }
                }
            }
            
            true
        } else {
            false
        }
    }
    
    /// Terminate the dialog
    pub fn terminate(&mut self) {
        self.state = DialogState::Terminated;
    }
    
    /// Check if dialog is terminated
    pub fn is_terminated(&self) -> bool {
        self.state == DialogState::Terminated
    }
    
    /// Update remote address tracking
    pub fn update_remote_address(&mut self, remote_addr: std::net::SocketAddr) {
        self.last_known_remote_addr = Some(remote_addr);
        self.last_successful_transaction_time = Some(std::time::SystemTime::now());
    }
    
    /// Set the remote tag for this dialog
    /// 
    /// Updates the remote tag, typically when receiving a response with a to-tag.
    /// This is used during dialog state transitions and response processing.
    pub fn set_remote_tag(&mut self, tag: String) {
        debug!("Setting remote tag for dialog {}: {}", self.id, tag);
        self.remote_tag = Some(tag);
    }
    
    /// Enter recovery mode
    pub fn enter_recovery_mode(&mut self, reason: &str) {
        if self.state != DialogState::Terminated {
            self.state = DialogState::Recovering;
            self.recovery_reason = Some(reason.to_string());
            self.recovery_start_time = Some(std::time::SystemTime::now());
        }
    }
    
    /// Check if dialog is in recovery mode
    pub fn is_recovering(&self) -> bool {
        self.state == DialogState::Recovering
    }
    
    /// Complete recovery
    pub fn complete_recovery(&mut self) -> bool {
        if self.state == DialogState::Recovering {
            self.state = DialogState::Confirmed;
            self.recovery_reason = None;
            self.recovered_at = Some(std::time::SystemTime::now());
            self.recovery_start_time = None;
            true
        } else {
            false
        }
    }
    
    /// Increment the local CSeq number
    /// 
    /// Used for sequence number management during dialog operations.
    pub fn increment_local_cseq(&mut self) {
        self.local_cseq += 1;
    }
}

/// Extract route set from Record-Route headers
fn extract_route_set(response: &Response, is_initiator: bool) -> Vec<Uri> {
    let routes: Vec<Uri> = response.headers.iter()
        .filter_map(|h| {
            if h.name() == HeaderName::RecordRoute {
                match h {
                    TypedHeader::RecordRoute(routes) => {
                        Some(routes.0.iter()
                            .map(|route| route.uri().clone())
                            .collect::<Vec<Uri>>())
                    },
                    _ => None
                }
            } else {
                None
            }
        })
        .flatten()
        .collect();
    
    if is_initiator {
        // Reverse for initiator
        routes.into_iter().rev().collect()
    } else {
        routes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dialog_creation() {
        let dialog = Dialog::new(
            "test-call-id".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("tag1".to_string()),
            Some("tag2".to_string()),
            true,
        );
        
        assert_eq!(dialog.call_id, "test-call-id");
        assert_eq!(dialog.state, DialogState::Initial);
        assert!(dialog.is_initiator);
    }
    
    #[test]
    fn test_dialog_id_tuple() {
        let dialog = Dialog::new(
            "test-call-id".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("tag1".to_string()),
            Some("tag2".to_string()),
            true,
        );
        
        let tuple = dialog.dialog_id_tuple().unwrap();
        assert_eq!(tuple.0, "test-call-id");
        assert_eq!(tuple.1, "tag1");
        assert_eq!(tuple.2, "tag2");
    }
    
    #[test]
    fn test_dialog_termination() {
        let mut dialog = Dialog::new(
            "test-call-id".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("tag1".to_string()),
            Some("tag2".to_string()),
            true,
        );
        
        assert!(!dialog.is_terminated());
        dialog.terminate();
        assert!(dialog.is_terminated());
        assert_eq!(dialog.state, DialogState::Terminated);
    }
} 