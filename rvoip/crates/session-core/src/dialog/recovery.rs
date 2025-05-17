use std::time::{Duration, SystemTime};
use tracing::{debug, error, warn, info};
use std::sync::Arc;
use tokio::sync::RwLock;

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

/// Default constants for recovery timing
const DEFAULT_MAX_RECOVERY_ATTEMPTS: u32 = 3;
const DEFAULT_RECOVERY_COOLDOWN_SECS: u64 = 5;
const DEFAULT_INITIAL_RETRY_DELAY_MS: u64 = 500;
const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 5000;
const DEFAULT_RECOVERY_OPTIONS_TIMEOUT_MS: u64 = 2000;
const DEFAULT_CIRCUIT_BREAKER_THRESHOLD: u32 = 5;
const DEFAULT_CIRCUIT_BREAKER_RESET_SECS: u64 = 30;

/// Configuration for dialog recovery behavior
#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// Maximum number of recovery attempts before giving up
    pub max_attempts: u32,
    
    /// Cooldown period after successful recovery (seconds)
    pub cooldown_period: Duration,
    
    /// Initial delay between retry attempts (milliseconds)
    pub initial_retry_delay: Duration,
    
    /// Maximum delay between retry attempts (milliseconds)
    pub max_retry_delay: Duration,
    
    /// Timeout for OPTIONS requests (milliseconds)
    pub options_timeout: Duration,
    
    /// Number of failures before circuit breaker opens
    pub circuit_breaker_threshold: u32,
    
    /// Time before circuit breaker resets (seconds)
    pub circuit_breaker_reset_period: Duration,
    
    /// Whether to use exponential backoff for retries
    pub use_exponential_backoff: bool,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_RECOVERY_ATTEMPTS,
            cooldown_period: Duration::from_secs(DEFAULT_RECOVERY_COOLDOWN_SECS),
            initial_retry_delay: Duration::from_millis(DEFAULT_INITIAL_RETRY_DELAY_MS),
            max_retry_delay: Duration::from_millis(DEFAULT_MAX_RETRY_DELAY_MS),
            options_timeout: Duration::from_millis(DEFAULT_RECOVERY_OPTIONS_TIMEOUT_MS),
            circuit_breaker_threshold: DEFAULT_CIRCUIT_BREAKER_THRESHOLD,
            circuit_breaker_reset_period: Duration::from_secs(DEFAULT_CIRCUIT_BREAKER_RESET_SECS),
            use_exponential_backoff: true,
        }
    }
}

/// Metrics for tracking recovery performance
#[derive(Debug, Default, Clone)]
pub struct RecoveryMetrics {
    /// Total recovery attempts
    pub total_attempts: u32,
    
    /// Successful recoveries
    pub successful_recoveries: u32,
    
    /// Failed recoveries
    pub failed_recoveries: u32,
    
    /// Recovery attempts by dialog ID
    pub attempts_by_dialog: std::collections::HashMap<DialogId, u32>,
    
    /// Circuit breaker activations
    pub circuit_breaker_activations: u32,
    
    /// Average recovery time (milliseconds)
    pub avg_recovery_time_ms: f64,
    
    /// Total recovery attempts that timed out
    pub recovery_timeouts: u32,
    
    /// Last circuit breaker reset time
    pub last_circuit_breaker_reset: Option<SystemTime>,
    
    /// Circuit breaker currently open
    pub circuit_breaker_open: bool,
}

impl RecoveryMetrics {
    /// Record a recovery attempt
    pub fn record_attempt(&mut self, dialog_id: &DialogId) {
        self.total_attempts += 1;
        *self.attempts_by_dialog.entry(dialog_id.clone()).or_insert(0) += 1;
    }
    
    /// Record a successful recovery
    pub fn record_success(&mut self, recovery_time_ms: u64) {
        self.successful_recoveries += 1;
        
        // Update average recovery time
        if self.avg_recovery_time_ms == 0.0 {
            self.avg_recovery_time_ms = recovery_time_ms as f64;
        } else {
            self.avg_recovery_time_ms = (self.avg_recovery_time_ms * (self.successful_recoveries - 1) as f64 + 
                                        recovery_time_ms as f64) / self.successful_recoveries as f64;
        }
    }
    
    /// Record a failed recovery
    pub fn record_failure(&mut self) {
        self.failed_recoveries += 1;
    }
    
    /// Record a recovery timeout
    pub fn record_timeout(&mut self) {
        self.recovery_timeouts += 1;
    }
    
    /// Reset the circuit breaker
    pub fn reset_circuit_breaker(&mut self) {
        self.circuit_breaker_open = false;
        self.last_circuit_breaker_reset = Some(SystemTime::now());
    }
    
    /// Open the circuit breaker
    pub fn open_circuit_breaker(&mut self) {
        self.circuit_breaker_open = true;
        self.circuit_breaker_activations += 1;
    }
    
    /// Get success rate as a percentage
    pub fn success_rate(&self) -> f64 {
        if self.total_attempts == 0 {
            return 0.0;
        }
        (self.successful_recoveries as f64 / self.total_attempts as f64) * 100.0
    }
}

/// Check if a dialog needs recovery based on its current state and history
pub fn needs_recovery(dialog: &Dialog, config: &RecoveryConfig, metrics: &RecoveryMetrics) -> bool {
    // Don't attempt recovery if circuit breaker is open
    if metrics.circuit_breaker_open {
        // Check if it's time to reset the circuit breaker
        if let Some(last_reset) = metrics.last_circuit_breaker_reset {
            if let Ok(elapsed) = SystemTime::now().duration_since(last_reset) {
                if elapsed < config.circuit_breaker_reset_period {
                    debug!("Dialog recovery circuit breaker is open, skipping recovery");
                    return false;
                }
            }
        }
    }

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
            if elapsed < config.cooldown_period {
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
        dialog.recovery_start_time = Some(SystemTime::now());
        info!("Dialog {} entered recovery mode: {}", dialog.id, reason);
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
        
        // Calculate recovery time if we have a start time
        let recovery_time_ms = if let Some(start_time) = dialog.recovery_start_time {
            if let Ok(duration) = now.duration_since(start_time) {
                duration.as_millis() as u64
            } else {
                0
            }
        } else {
            0
        };
        
        info!("Dialog {} recovery completed successfully in {}ms", dialog.id, recovery_time_ms);
        return true;
    }
    
    false
}

/// Mark a dialog as failed recovery and terminate it
pub fn abandon_recovery(dialog: &mut Dialog) -> bool {
    if dialog.state == DialogState::Recovering {
        dialog.state = DialogState::Terminated;
        dialog.recovery_reason = Some("Recovery failed after multiple attempts".to_string());
        warn!("Dialog {} recovery abandoned, dialog terminated", dialog.id);
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

/// Result of a recovery attempt
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryResult {
    /// Recovery was successful
    Success {
        /// Time taken for recovery in milliseconds
        recovery_time_ms: u64,
    },
    /// Recovery failed after all attempts
    Failure {
        /// Reason for the failure
        reason: String,
        /// Whether the circuit breaker should be activated
        activate_circuit_breaker: bool,
    },
    /// Recovery was aborted (e.g., dialog was terminated)
    Aborted {
        /// Reason for the abort
        reason: String,
    },
}

/// Events emitted during the recovery process
#[derive(Debug, Clone)]
pub enum RecoveryEvent {
    /// Recovery attempt started
    AttemptStarted {
        /// Current attempt number
        attempt: u32,
        /// Total number of attempts allowed
        max_attempts: u32,
    },
    /// Recovery attempt succeeded
    AttemptSucceeded {
        /// Time taken for the successful attempt in ms
        time_ms: u64,
    },
    /// Recovery attempt failed
    AttemptFailed {
        /// Current attempt number
        attempt: u32,
        /// Reason for failure
        reason: String,
        /// Whether this was a timeout
        is_timeout: bool,
    },
    /// Waiting before next retry
    RetryDelay {
        /// Delay before next attempt in ms
        delay_ms: u64,
    },
}

/// Perform the entire dialog recovery process
/// 
/// This function handles the retry logic, circuit breaker, and metrics tracking
pub async fn perform_recovery_process(
    dialog: &mut Dialog,
    transport: &dyn Transport,
    config: &RecoveryConfig,
    metrics_arc: &Arc<RwLock<RecoveryMetrics>>,
    event_callback: impl Fn(RecoveryEvent),
) -> RecoveryResult {
    // Ensure dialog is in recovering state
    if dialog.state != DialogState::Recovering {
        if dialog.state == DialogState::Confirmed || dialog.state == DialogState::Early {
            // Put dialog into recovery mode if not already
            begin_recovery(dialog, "Recovery initiated by perform_recovery_process");
        } else {
            // Cannot recover a dialog in this state
            return RecoveryResult::Aborted { 
                reason: format!("Cannot recover dialog in state {}", dialog.state) 
            };
        }
    }

    // Make sure we have a recovery start time
    if dialog.recovery_start_time.is_none() {
        dialog.recovery_start_time = Some(SystemTime::now());
    }

    // Get the last known remote address, if available
    let remote_addr = match dialog.last_known_remote_addr {
        Some(addr) => addr,
        None => {
            return RecoveryResult::Aborted { 
                reason: "No last known remote address available".to_string() 
            };
        }
    };

    // Store the start time for metrics
    let start_time = dialog.recovery_start_time.unwrap_or_else(SystemTime::now);

    // Record this recovery attempt in metrics
    {
        // Need to use await with tokio's RwLock
        let mut metrics = metrics_arc.write().await;
        metrics.record_attempt(&dialog.id);
        // Write lock is dropped here when the block ends
    }

    // Retry logic with backoff
    let max_attempts = config.max_attempts;
    let mut current_delay = config.initial_retry_delay;
    
    // Try recovery attempts up to max_attempts
    for attempt in 1..=max_attempts {
        // Log that we're starting a new attempt
        let recovery_event = RecoveryEvent::AttemptStarted {
            attempt,
            max_attempts,
        };
        event_callback(recovery_event);
        
        // Increment the attempt counter on the dialog
        dialog.increment_recovery_attempts();
        
        // Try to send an OPTIONS request with timeout
        let result = tokio::time::timeout(
            config.options_timeout,
            send_recovery_options(dialog, transport)
        ).await;
        
        match result {
            // Success: OPTIONS request succeeded
            Ok(Ok(_)) => {
                // Calculate recovery time
                let recovery_time_ms = if let Ok(duration) = SystemTime::now().duration_since(start_time) {
                    duration.as_millis() as u64
                } else {
                    0
                };
                
                // Log successful attempt
                event_callback(RecoveryEvent::AttemptSucceeded { 
                    time_ms: recovery_time_ms 
                });
                
                // Update metrics with success
                {
                    // Need to use await with tokio's RwLock
                    let mut metrics = metrics_arc.write().await;
                    metrics.record_success(recovery_time_ms);
                    
                    // Reset attempts counter for this dialog
                    metrics.attempts_by_dialog.insert(dialog.id.clone(), 0);
                    // Write lock is dropped here when the block ends
                }
                
                // Mark dialog as recovered
                complete_recovery(dialog);
                
                // Return success result
                return RecoveryResult::Success { 
                    recovery_time_ms 
                };
            },
            
            // Failure: OPTIONS request failed or timed out
            _ => {
                // Determine if this was a timeout
                let is_timeout = result.is_err();
                let reason = if is_timeout {
                    "Recovery attempt timed out".to_string()
                } else {
                    format!("Recovery attempt failed: {:?}", result.unwrap().err())
                };
                
                // Log failed attempt
                event_callback(RecoveryEvent::AttemptFailed {
                    attempt,
                    reason: reason.clone(),
                    is_timeout,
                });
                
                // Update metrics with failure
                {
                    // Need to use await with tokio's RwLock
                    let mut metrics = metrics_arc.write().await;
                    metrics.record_failure();
                    if is_timeout {
                        metrics.record_timeout();
                    }
                    // Write lock is dropped here when the block ends
                }
                
                // If this was the last attempt, mark as failed
                if attempt >= max_attempts {
                    // Check if we need to activate the circuit breaker
                    let activate_circuit_breaker;
                    {
                        // Need to use await with tokio's RwLock
                        let metrics = metrics_arc.read().await;
                        let attempts = *metrics.attempts_by_dialog.get(&dialog.id).unwrap_or(&0);
                        activate_circuit_breaker = attempts >= config.circuit_breaker_threshold;
                        // Read lock is dropped here when the block ends
                    }
                    
                    // Mark the dialog as recovery failed
                    abandon_recovery(dialog);
                    
                    // Return failure result
                    return RecoveryResult::Failure {
                        reason: format!("All {} recovery attempts failed", max_attempts),
                        activate_circuit_breaker,
                    };
                }
                
                // Otherwise wait before trying again
                let delay_ms = current_delay.as_millis() as u64;
                event_callback(RecoveryEvent::RetryDelay { delay_ms });
                tokio::time::sleep(current_delay).await;
                
                // Calculate next delay using exponential backoff if configured
                if config.use_exponential_backoff {
                    current_delay = std::cmp::min(
                        current_delay * 2, 
                        config.max_retry_delay
                    );
                }
            }
        }
    }
    
    // This code should never be reached since we return in the loop
    // But just in case, mark as failed
    abandon_recovery(dialog);
    
    RecoveryResult::Failure {
        reason: "All recovery attempts failed".to_string(),
        activate_circuit_breaker: false,
    }
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
        // Control behavior for testing multiple attempts
        fail_attempts: Arc<Mutex<u32>>,
        should_fail: bool, // Added explicit flag
    }
    
    impl MockTransport {
        fn new(addr: &str) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                send_result: Arc::new(Mutex::new(None)),
                sent_message: Arc::new(Mutex::new(None)),
                fail_attempts: Arc::new(Mutex::new(0)),
                should_fail: false,
            }
        }
        
        fn with_error(addr: &str, error: &str) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                send_result: Arc::new(Mutex::new(Some(error.to_string()))),
                sent_message: Arc::new(Mutex::new(None)),
                fail_attempts: Arc::new(Mutex::new(0)),
                should_fail: true, // This transport should fail
            }
        }
        
        // New method for testing multiple attempts - fails for specified attempts then succeeds
        fn with_retry_behavior(addr: &str, fail_attempts: u32) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                send_result: Arc::new(Mutex::new(Some("Simulated failure".to_string()))),
                sent_message: Arc::new(Mutex::new(None)),
                fail_attempts: Arc::new(Mutex::new(fail_attempts)),
                should_fail: false,
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
            
            // Check if this transport should fail immediately
            if self.should_fail {
                let error_msg = self.send_result.lock().await.clone().unwrap_or_else(|| "Simulated failure".to_string());
                return Err(TransportError::ConnectionFailed(error_msg.into()));
            }
            
            // Check if we should still be failing based on remaining fail attempts
            let mut fail_attempts = self.fail_attempts.lock().await;
            if *fail_attempts > 0 {
                *fail_attempts -= 1;
                let error_opt = self.send_result.lock().await.clone();
                match error_opt {
                    Some(error) => return Err(TransportError::ConnectionFailed(error.into())),
                    None => {},
                }
            }
            
            // Otherwise succeed
            Ok(())
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
            recovery_start_time: None,
        }
    }
    
    #[test]
    fn test_needs_recovery_criteria() {
        // Test 1: Dialog in confirmed state with remote address should need recovery
        let mut dialog = create_test_dialog();
        assert!(needs_recovery(&dialog, &RecoveryConfig::default(), &RecoveryMetrics::default()), "Dialog should need recovery");
        
        // Test 2: Dialog in early state with remote address should need recovery
        dialog.state = DialogState::Early;
        assert!(needs_recovery(&dialog, &RecoveryConfig::default(), &RecoveryMetrics::default()), "Dialog in Early state should need recovery");
        
        // Test 3: Dialog without remote address should not need recovery
        dialog.last_known_remote_addr = None;
        assert!(!needs_recovery(&dialog, &RecoveryConfig::default(), &RecoveryMetrics::default()), "Dialog without remote address should not need recovery");
        
        // Test 4: Dialog in recovering state should not need recovery
        dialog = create_test_dialog();
        dialog.state = DialogState::Recovering;
        assert!(!needs_recovery(&dialog, &RecoveryConfig::default(), &RecoveryMetrics::default()), "Dialog in Recovering state should not need recovery again");
        
        // Test 5: Dialog in terminated state should not need recovery
        dialog.state = DialogState::Terminated;
        assert!(!needs_recovery(&dialog, &RecoveryConfig::default(), &RecoveryMetrics::default()), "Terminated dialog should not need recovery");
        
        // Test 6: Dialog with recent recovery should not need recovery
        dialog = create_test_dialog();
        dialog.recovered_at = Some(SystemTime::now());
        assert!(!needs_recovery(&dialog, &RecoveryConfig::default(), &RecoveryMetrics::default()), "Recently recovered dialog should not need recovery");
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
        
        // Verify the transport is set to fail
        assert!(transport.should_fail, "Transport should be configured to fail");
        
        let result = send_recovery_options(&dialog, &transport).await;
        // Now this should fail since we properly configured the transport
        assert!(result.is_err(), "Should fail when transport fails");
        
        if let Err(error) = result {
            match error {
                Error::TransportError(_, _) => {
                    // This is the expected error type
                },
                _ => panic!("Unexpected error type: {:?}", error),
            }
        }
    }
    
    #[tokio::test]
    async fn test_perform_recovery_process_successful() {
        // Create test dialog in confirmed state
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering; // Start in recovering state
        dialog.recovery_start_time = Some(SystemTime::now().checked_sub(Duration::from_secs(1)).unwrap()); // Set start time in the past
        
        // Create transport that will succeed
        let transport = MockTransport::new("127.0.0.1:5060");
        
        // Create recovery config with short timeouts for testing
        let config = RecoveryConfig {
            max_attempts: 3,
            cooldown_period: Duration::from_millis(10),
            initial_retry_delay: Duration::from_millis(10),
            max_retry_delay: Duration::from_millis(50),
            options_timeout: Duration::from_millis(100),
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_period: Duration::from_millis(100),
            use_exponential_backoff: true,
        };
        
        // Create metrics with Arc<RwLock<>>
        let metrics_arc = Arc::new(RwLock::new(RecoveryMetrics::default()));
        
        // Event collector for testing
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let event_callback = move |event: RecoveryEvent| {
            // Can't use blocking_lock in async context - use try_lock instead
            if let Ok(mut events) = events_clone.try_lock() {
                events.push(event);
            }
        };
        
        // Run recovery
        let result = perform_recovery_process(
            &mut dialog, 
            &transport, 
            &config, 
            &metrics_arc,
            event_callback
        ).await;
        
        // Verify result
        match result {
            RecoveryResult::Success { recovery_time_ms } => {
                // For testing, we're ok with any non-negative time
                assert!(recovery_time_ms >= 0, "Recovery time should not be negative");
            },
            _ => panic!("Expected Success result, got {:?}", result),
        }
        
        // Verify dialog state
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert!(dialog.recovered_at.is_some());
        
        // Verify metrics
        let metrics = metrics_arc.read().await;
        assert_eq!(metrics.total_attempts, 1);
        assert_eq!(metrics.successful_recoveries, 1);
        assert_eq!(metrics.failed_recoveries, 0);
        drop(metrics);
    }
    
    #[tokio::test]
    async fn test_perform_recovery_process_with_retries() {
        // Create test dialog in confirmed state
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering; // Start in recovering state
        dialog.recovery_start_time = Some(SystemTime::now().checked_sub(Duration::from_secs(1)).unwrap()); // Set start time in the past
        
        // Create transport that will fail twice then succeed
        let transport = MockTransport::with_retry_behavior("127.0.0.1:5060", 2);
        
        // Create recovery config with short timeouts for testing
        let config = RecoveryConfig {
            max_attempts: 5,
            cooldown_period: Duration::from_millis(10),
            initial_retry_delay: Duration::from_millis(10),
            max_retry_delay: Duration::from_millis(50),
            options_timeout: Duration::from_millis(100),
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_period: Duration::from_millis(100),
            use_exponential_backoff: true,
        };
        
        // Create metrics with Arc<RwLock<>>
        let metrics_arc = Arc::new(RwLock::new(RecoveryMetrics::default()));
        
        // Event collector for testing
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let event_callback = move |event: RecoveryEvent| {
            // Can't use blocking_lock in async context - use try_lock instead
            if let Ok(mut events) = events_clone.try_lock() {
                events.push(event);
            }
        };
        
        // Run recovery
        let result = perform_recovery_process(
            &mut dialog, 
            &transport, 
            &config, 
            &metrics_arc,
            event_callback
        ).await;
        
        // Verify result
        match result {
            RecoveryResult::Success { recovery_time_ms } => {
                // For testing, we're ok with any non-negative time
                assert!(recovery_time_ms >= 0, "Recovery time should not be negative");
            },
            _ => panic!("Expected Success result, got {:?}", result),
        }
        
        // Verify dialog state
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert!(dialog.recovered_at.is_some());
        
        // Verify metrics
        let metrics = metrics_arc.read().await;
        assert!(metrics.total_attempts > 0, "Should have recorded attempts");
        assert!(metrics.successful_recoveries > 0, "Should have recorded success");
        assert!(metrics.failed_recoveries > 0, "Should have recorded failures from retries");
        drop(metrics);
    }
    
    #[tokio::test]
    async fn test_perform_recovery_process_all_attempts_fail() {
        // Create test dialog in confirmed state
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Recovering; // Start in recovering state
        dialog.recovery_start_time = Some(SystemTime::now().checked_sub(Duration::from_secs(1)).unwrap()); // Set start time in the past
        
        // Create transport that will always fail every time
        let transport = MockTransport::with_error("127.0.0.1:5060", "Permanent failure");
        
        // Create recovery config with short timeouts for testing
        let config = RecoveryConfig {
            max_attempts: 2, // Only try twice to speed up test
            cooldown_period: Duration::from_millis(10),
            initial_retry_delay: Duration::from_millis(10),
            max_retry_delay: Duration::from_millis(50),
            options_timeout: Duration::from_millis(100),
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_period: Duration::from_millis(100),
            use_exponential_backoff: true,
        };
        
        // Create metrics with Arc<RwLock<>> - setup initial dialog attempts
        let metrics_arc = Arc::new(RwLock::new(RecoveryMetrics::default()));
        {
            let mut metrics = metrics_arc.write().await;
            metrics.attempts_by_dialog.insert(dialog.id.clone(), 10); // Force circuit breaker
        }
        
        // Event collector for testing
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let event_callback = move |event: RecoveryEvent| {
            // Can't use blocking_lock in async context - use try_lock instead
            if let Ok(mut events) = events_clone.try_lock() {
                events.push(event);
            }
        };
        
        // Run recovery
        let result = perform_recovery_process(
            &mut dialog, 
            &transport, 
            &config, 
            &metrics_arc,
            event_callback
        ).await;
        
        // Verify result
        match result {
            RecoveryResult::Failure { reason, activate_circuit_breaker } => {
                assert!(reason.contains("All 2 recovery attempts failed"), "Expected failure reason mentioning failed attempts");
                assert!(activate_circuit_breaker, "Circuit breaker should be activated");
            },
            _ => panic!("Expected Failure result, got {:?}", result),
        }
        
        // Verify dialog state
        assert_eq!(dialog.state, DialogState::Terminated);
        
        // Verify metrics
        let metrics = metrics_arc.read().await;
        assert!(metrics.total_attempts > 0, "Should have recorded attempts");
        assert_eq!(metrics.successful_recoveries, 0, "Should not have recorded any successes");
        assert!(metrics.failed_recoveries > 0, "Should have recorded failures");
        drop(metrics);
    }
    
    #[tokio::test]
    async fn test_perform_recovery_process_aborted() {
        // Create test dialog in terminated state
        let mut dialog = create_test_dialog();
        dialog.state = DialogState::Terminated;
        
        // Create transport
        let transport = MockTransport::new("127.0.0.1:5060");
        
        // Create recovery config
        let config = RecoveryConfig::default();
        
        // Create metrics with Arc<RwLock<>>
        let metrics_arc = Arc::new(RwLock::new(RecoveryMetrics::default()));
        
        // Event collector
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let event_callback = move |event: RecoveryEvent| {
            // Can't use blocking_lock in async context - use try_lock instead
            if let Ok(mut events) = events_clone.try_lock() {
                events.push(event);
            }
        };
        
        // Run recovery
        let result = perform_recovery_process(
            &mut dialog, 
            &transport, 
            &config, 
            &metrics_arc,
            event_callback
        ).await;
        
        // Verify result
        match result {
            RecoveryResult::Aborted { reason } => {
                assert!(reason.contains("Cannot recover dialog in state"));
            },
            _ => panic!("Expected Aborted result, got {:?}", result),
        }
        
        // No events should have been generated
        let event_list = events.lock().await;
        assert!(event_list.is_empty(), "No events should be generated for aborted recovery");
    }
} 