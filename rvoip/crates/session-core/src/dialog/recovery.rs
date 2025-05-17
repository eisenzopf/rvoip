use std::time::{Duration, SystemTime};
use tracing::{debug, error, warn};
use std::sync::Arc;

use rvoip_sip_core::{Method, Request, StatusCode, TypedHeader, Uri, HeaderName, Message};
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::from::From as FromHeader;
use rvoip_sip_core::types::to::To as ToHeader;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_transport::Transport;

use crate::errors::{Error, ErrorContext};
use super::dialog_id::DialogId;
use super::dialog_impl::Dialog;
use super::dialog_state::DialogState;

/// Constants for recovery timing
pub const MAX_RECOVERY_ATTEMPTS: u32 = 3;
pub const RECOVERY_COOLDOWN_SECS: u64 = 5;
pub const INITIAL_RETRY_DELAY_MS: u64 = 500;
pub const MAX_RETRY_DELAY_MS: u64 = 5000;
pub const RECOVERY_OPTIONS_TIMEOUT_MS: u64 = 2000;

/// Check if a dialog needs recovery based on its current state and history
pub fn needs_recovery(dialog: &Dialog) -> bool {
    // Only active dialogs can be recovered
    if dialog.state != DialogState::Confirmed && dialog.state != DialogState::Early {
        debug!("Dialog doesn't need recovery: not in Confirmed or Early state");
        return false;
    }
    
    // Check if it's already in recovery mode
    if dialog.is_recovering() {
        debug!("Dialog doesn't need recovery: already in recovery mode");
        return false;
    }
    
    // Check if we have a last known remote address
    if dialog.last_known_remote_addr.is_none() {
        debug!("Dialog doesn't need recovery: no last known remote address");
        return false;
    }
    
    // Don't attempt recovery if this dialog was recently recovered
    if let Some(recovered_time) = dialog.recovered_at {
        if let Ok(elapsed) = SystemTime::now().duration_since(recovered_time) {
            if elapsed < Duration::from_secs(RECOVERY_COOLDOWN_SECS) {
                debug!("Dialog was recently recovered ({:?} ago), not attempting recovery yet", elapsed);
                return false;
            }
        }
    }
    
    true
}

/// Mark a dialog as entering recovery mode
pub fn begin_recovery(dialog: &mut Dialog, reason: &str) -> bool {
    if dialog.state == DialogState::Confirmed || dialog.state == DialogState::Early {
        dialog.state = DialogState::Recovering;
        dialog.recovery_reason = Some(reason.to_string());
        dialog.recovery_attempts = 0;
        debug!("Dialog entered recovery mode: {}", reason);
        return true;
    }
    
    false
}

/// Mark a dialog as successfully recovered
pub fn complete_recovery(dialog: &mut Dialog) -> bool {
    if dialog.state == DialogState::Recovering {
        let now = SystemTime::now();
        dialog.state = DialogState::Confirmed;
        dialog.recovery_reason = None;
        dialog.last_successful_transaction_time = Some(now);
        dialog.recovered_at = Some(now);
        debug!("Dialog recovery completed successfully");
        return true;
    }
    
    false
}

/// Mark a dialog as failed recovery and terminate it
pub fn abandon_recovery(dialog: &mut Dialog) -> bool {
    if dialog.state == DialogState::Recovering {
        dialog.state = DialogState::Terminated;
        dialog.recovery_reason = Some("Recovery failed after multiple attempts".to_string());
        debug!("Dialog recovery abandoned, dialog terminated");
        return true;
    }
    
    false
}

/// Create an OPTIONS request for probing connectivity during recovery
pub async fn create_recovery_options_request(
    dialog: &Dialog,
    transport: &dyn Transport
) -> Result<(Request, std::net::SocketAddr), Error> {
    if !dialog.is_recovering() {
        return Err(Error::InvalidDialogState {
            current: dialog.state.to_string(),
            expected: "Recovering".to_string(),
            context: ErrorContext::default()
        });
    }
    
    // Get the remote address - this should be available if we're in recovery
    let remote_addr = match dialog.last_known_remote_addr {
        Some(addr) => addr,
        None => return Err(Error::MissingDialogData {
            context: ErrorContext::default().with_message(
                "Dialog does not have a last known remote address"
            )
        })
    };
    
    // Create an OPTIONS request
    let mut request = Request::new(Method::Options, dialog.remote_target.clone());
    
    // Add dialog headers
    request.headers.push(TypedHeader::CallId(
        rvoip_sip_core::types::call_id::CallId(dialog.call_id.clone())
    ));
    
    // Add From header with our tag
    if let Some(local_tag) = &dialog.local_tag {
        let mut from_addr = Address::new(dialog.local_uri.clone());
        from_addr.set_tag(local_tag);
        let from = FromHeader(from_addr);
        request.headers.push(TypedHeader::From(from));
    } else {
        let from_addr = Address::new(dialog.local_uri.clone());
        request.headers.push(TypedHeader::From(FromHeader(from_addr)));
    }
    
    // Add To header with remote tag
    if let Some(remote_tag) = &dialog.remote_tag {
        let mut to_addr = Address::new(dialog.remote_uri.clone());
        to_addr.set_tag(remote_tag);
        let to = ToHeader(to_addr);
        request.headers.push(TypedHeader::To(to));
    } else {
        let to_addr = Address::new(dialog.remote_uri.clone());
        request.headers.push(TypedHeader::To(ToHeader(to_addr)));
    }
    
    // Add CSeq header - no need to increment dialog CSeq for OPTIONS
    request.headers.push(TypedHeader::CSeq(
        rvoip_sip_core::types::cseq::CSeq::new(1, Method::Options)
    ));
    
    // Add Max-Forwards
    request.headers.push(TypedHeader::MaxForwards(
        rvoip_sip_core::types::max_forwards::MaxForwards::new(70)
    ));
    
    // Add Via header
    let local_addr = transport.local_addr()
        .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
    
    let branch = format!("z9hG4bK{}", rand::random::<u32>());
    let via = rvoip_sip_core::types::via::Via::new(
        "SIP", "2.0", "UDP", 
        &local_addr.ip().to_string(), 
        Some(local_addr.port()),
        vec![Param::branch(&branch)]
    ).map_err(|e| Error::SipError(e, ErrorContext::default()))?;
    
    request.headers.push(TypedHeader::Via(via));
    
    // Add Content-Length: 0
    request.headers.push(TypedHeader::ContentLength(ContentLength::new(0)));
    
    Ok((request, remote_addr))
}

/// Send an OPTIONS request and wait for the response
pub async fn send_recovery_options(
    dialog: &Dialog,
    transport: &dyn Transport
) -> Result<(), Error> {
    // Create the OPTIONS request
    let (request, remote_addr) = create_recovery_options_request(dialog, transport).await?;
    
    // Send the request
    transport.send_message(
        Message::Request(request), 
        remote_addr
    ).await.map_err(|e| Error::transport_error(e, "Failed to send OPTIONS request during recovery"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::net::SocketAddr;
    use async_trait::async_trait;
    use rvoip_sip_transport::error::Error as TransportError;
    use tokio::sync::Mutex;
    use std::sync::Arc;
    
    // Mock transport for testing recovery
    #[derive(Debug, Clone)]
    struct MockTransport {
        local_addr: SocketAddr,
        send_result: Arc<Mutex<Option<String>>>, // Store error message or None for success
        sent_message: Arc<Mutex<Option<Message>>>,
    }
    
    impl MockTransport {
        fn new(addr: &str) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                send_result: Arc::new(Mutex::new(None)),
                sent_message: Arc::new(Mutex::new(None)),
            }
        }
        
        fn with_error(addr: &str, error: &str) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                send_result: Arc::new(Mutex::new(Some(error.to_string()))),
                sent_message: Arc::new(Mutex::new(None)),
            }
        }
        
        async fn get_last_message(&self) -> Option<Message> {
            self.sent_message.lock().await.clone()
        }
    }
    
    #[async_trait]
    impl Transport for MockTransport {
        fn local_addr(&self) -> Result<SocketAddr, TransportError> {
            Ok(self.local_addr)
        }
        
        async fn send_message(&self, message: Message, _destination: SocketAddr) -> Result<(), TransportError> {
            // Store the message for inspection
            *self.sent_message.lock().await = Some(message);
            
            // Return success or error based on configuration
            let error_opt = self.send_result.lock().await.clone();
            match error_opt {
                Some(error) => Err(TransportError::ConnectionFailed(error.into())),
                None => Ok(())
            }
        }
        
        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
        
        fn is_closed(&self) -> bool {
            false
        }
    }
    
    // Helper to create a test dialog
    fn create_test_dialog() -> Dialog {
        Dialog {
            id: DialogId::new(),
            state: DialogState::Confirmed,
            call_id: "test-recovery-call-id".to_string(),
            local_uri: Uri::sip("alice@example.com"),
            remote_uri: Uri::sip("bob@example.com"),
            local_tag: Some("alice-tag".to_string()),
            remote_tag: Some("bob-tag".to_string()),
            local_seq: 1,
            remote_seq: 0,
            remote_target: Uri::sip("bob@192.168.1.100"),
            route_set: Vec::new(),
            is_initiator: true,
            sdp_context: crate::sdp::SdpContext::new(),
            last_known_remote_addr: Some(SocketAddr::from_str("192.168.1.100:5060").unwrap()),
            last_successful_transaction_time: Some(SystemTime::now()),
            recovery_attempts: 0,
            recovery_reason: None,
            recovered_at: None,
        }
    }
    
    #[test]
    fn test_needs_recovery_criteria() {
        // Test 1: Dialog in confirmed state with remote address should need recovery
        let mut dialog = create_test_dialog();
        assert!(needs_recovery(&dialog), "Dialog should need recovery");
        
        // Test 2: Dialog in early state with remote address should need recovery
        dialog.state = DialogState::Early;
        assert!(needs_recovery(&dialog), "Dialog in Early state should need recovery");
        
        // Test 3: Dialog without remote address should not need recovery
        dialog.last_known_remote_addr = None;
        assert!(!needs_recovery(&dialog), "Dialog without remote address should not need recovery");
        
        // Test 4: Dialog in recovering state should not need recovery
        dialog = create_test_dialog();
        dialog.state = DialogState::Recovering;
        assert!(!needs_recovery(&dialog), "Dialog in Recovering state should not need recovery again");
        
        // Test 5: Dialog in terminated state should not need recovery
        dialog.state = DialogState::Terminated;
        assert!(!needs_recovery(&dialog), "Terminated dialog should not need recovery");
        
        // Test 6: Dialog with recent recovery should not need recovery
        dialog = create_test_dialog();
        dialog.recovered_at = Some(SystemTime::now());
        assert!(!needs_recovery(&dialog), "Recently recovered dialog should not need recovery");
    }
    
    #[test]
    fn test_begin_recovery() {
        // Test 1: Begin recovery on a confirmed dialog
        let mut dialog = create_test_dialog();
        assert!(begin_recovery(&mut dialog, "Test recovery"), "Should be able to start recovery");
        assert_eq!(dialog.state, DialogState::Recovering, "Dialog state should be Recovering");
        assert_eq!(dialog.recovery_reason, Some("Test recovery".to_string()), "Recovery reason should be set");
        assert_eq!(dialog.recovery_attempts, 0, "Recovery attempts should be reset");
        
        // Test 2: Begin recovery on a terminated dialog should fail
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Terminated;
        assert!(!begin_recovery(&mut dialog, "Test recovery"), "Should not recover terminated dialog");
        assert_eq!(dialog.state, DialogState::Terminated, "Dialog state should remain Terminated");
    }
    
    #[test]
    fn test_complete_recovery() {
        // Test 1: Complete recovery on a recovering dialog
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering;
        dialog.recovery_reason = Some("Test recovery".to_string());
        
        assert!(complete_recovery(&mut dialog), "Recovery should complete successfully");
        assert_eq!(dialog.state, DialogState::Confirmed, "Dialog state should be Confirmed");
        assert!(dialog.recovery_reason.is_none(), "Recovery reason should be cleared");
        assert!(dialog.recovered_at.is_some(), "Recovered timestamp should be set");
        
        // Test 2: Complete recovery on a non-recovering dialog should fail
        let mut dialog = create_test_dialog();
        assert!(!complete_recovery(&mut dialog), "Cannot complete recovery when not in recovery mode");
        assert_eq!(dialog.state, DialogState::Confirmed, "Dialog state should remain unchanged");
    }
    
    #[test]
    fn test_abandon_recovery() {
        // Test 1: Abandon recovery on a recovering dialog
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering;
        
        assert!(abandon_recovery(&mut dialog), "Should be able to abandon recovery");
        assert_eq!(dialog.state, DialogState::Terminated, "Dialog state should be Terminated");
        assert!(dialog.recovery_reason.is_some(), "Recovery reason should indicate failure");
        
        // Test 2: Abandon recovery on a non-recovering dialog should fail
        let mut dialog = create_test_dialog();
        assert!(!abandon_recovery(&mut dialog), "Cannot abandon recovery when not in recovery mode");
        assert_eq!(dialog.state, DialogState::Confirmed, "Dialog state should remain unchanged");
    }
    
    #[tokio::test]
    async fn test_create_recovery_options_request() {
        // Test 1: Create OPTIONS request for a recovering dialog
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering;
        let transport = MockTransport::new("127.0.0.1:5060");
        
        let result = create_recovery_options_request(&dialog, &transport).await;
        assert!(result.is_ok(), "Should be able to create OPTIONS request");
        
        let (request, addr) = result.unwrap();
        assert_eq!(request.method, Method::Options, "Request method should be OPTIONS");
        assert_eq!(addr.to_string(), "192.168.1.100:5060", "Remote address should match");
        
        // Test 2: Attempt to create OPTIONS for non-recovering dialog should fail
        let dialog = create_test_dialog(); // Confirmed state
        let result = create_recovery_options_request(&dialog, &transport).await;
        assert!(result.is_err(), "Creating OPTIONS request should fail for non-recovering dialog");
    }
    
    #[tokio::test]
    async fn test_send_recovery_options() {
        // Test 1: Send OPTIONS with successful transport
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering;
        let transport = MockTransport::new("127.0.0.1:5060");
        
        let result = send_recovery_options(&dialog, &transport).await;
        assert!(result.is_ok(), "Should successfully send OPTIONS request");
        
        // Verify the sent message
        let message = transport.get_last_message().await;
        assert!(message.is_some(), "Message should have been sent");
        
        if let Some(Message::Request(request)) = message {
            assert_eq!(request.method, Method::Options, "Sent message should be OPTIONS");
        } else {
            panic!("Sent message was not a request");
        }
        
        // Test 2: Send OPTIONS with failing transport
        let transport = MockTransport::with_error("127.0.0.1:5060", "Simulated failure");
        let result = send_recovery_options(&dialog, &transport).await;
        assert!(result.is_err(), "Should fail when transport fails");
    }
} 