# Call Transfer Implementation Plan (Revised)

## Executive Summary
This revised plan leverages existing SIP parsing infrastructure (ReferTo header in sip-core) and creates a dedicated transfer.rs module to avoid overloading coordinator.rs. We will create a new `TransferRequest` event in the `SessionCoordinationEvent` enum to provide type safety and clear semantic separation from other SIP methods.

## RFC-Compliant Transfer Steps

### Blind Transfer (Unattended Transfer) - RFC 3515/5589
According to the SIP RFCs, a proper blind transfer follows these steps:

**Actors:** Alice (transferor), Bob (transferee), Charlie (transfer target)

1. **Active Call:** Alice and Bob are in an active call
2. **Hold:** Alice puts Bob on hold (sends re-INVITE with sendonly/inactive)
3. **REFER Request:** Alice sends REFER to Bob with `Refer-To: <sip:Charlie>`
4. **Accept REFER:** Bob responds with 202 Accepted to Alice
5. **Initial NOTIFY:** Bob sends NOTIFY to Alice with "SIP/2.0 100 Trying"
6. **New Call:** Bob sends INVITE to Charlie (new dialog)
7. **Progress NOTIFY:** Bob sends NOTIFY to Alice with "SIP/2.0 180 Ringing" when Charlie's phone rings
8. **Connect:** When Charlie answers, Bob connects with Charlie
9. **Success NOTIFY:** Bob sends NOTIFY to Alice with "SIP/2.0 200 OK"
10. **Terminate Original:** Bob sends BYE to Alice to end original call
11. **Complete:** Bob and Charlie are now connected, Alice's call is terminated

### Attended Transfer (Consultative Transfer) - RFC 3891/5589
For completeness, attended transfer follows these steps:

1. **Active Call:** Alice and Bob are in an active call
2. **Hold:** Alice puts Bob on hold
3. **Consult Call:** Alice calls Charlie separately (new dialog)
4. **Consultation:** Alice talks to Charlie ("Bob is on line 1")
5. **REFER with Replaces:** Alice sends REFER to Bob with `Refer-To: <sip:Charlie?Replaces=dialog-id>`
6. **Accept:** Bob responds with 202 Accepted
7. **INVITE with Replaces:** Bob sends INVITE to Charlie with Replaces header
8. **Replace Dialog:** Charlie's phone replaces Alice's call with Bob's call
9. **Success:** Bob and Charlie connected, both Alice's calls terminated

### Key RFC Requirements
- **RFC 3515 (REFER Method):** Defines REFER and implicit subscription
- **RFC 5589 (Call Transfer):** Standard transfer flows
- **RFC 3891 (Replaces):** For attended transfers
- **202 Accepted:** REFER must be immediately accepted, not wait for completion
- **NOTIFY Required:** Transferee MUST send NOTIFY messages for progress
- **Subscription:** REFER creates implicit subscription for transfer events

## Problem Statement
When a remote party attempts to transfer a call, they send a REFER request. Currently:
- Dialog-core receives the REFER and forwards it as a `ReInvite` event
- Session-core's handler doesn't recognize REFER method
- The request falls through to default case returning 501 Not Implemented
- Transfer fails

## Decision: Create Dedicated TransferRequest Event

We will create a new `TransferRequest` event in the `SessionCoordinationEvent` enum for the following reasons:

### Benefits of Dedicated Event:
- **Clear semantic separation**: REFER requests are fundamentally different from re-INVITE requests
- **Type safety**: Handlers immediately know they're dealing with a transfer request
- **Pre-parsed data**: ReferTo, ReferredBy, and Replaces headers are parsed once in dialog-core using sip-core types
- **Better documentation**: Clear intent and purpose in the code
- **Follows single responsibility principle**: Each event type has a specific purpose
- **Easier debugging**: Can trace transfer requests specifically

### Implementation Impact:
- Requires adding new event variant to `SessionCoordinationEvent` enum in dialog-core
- Update REFER handler in dialog-core to use new event
- Add handler case in session-core's EventHandler
- Clean separation allows for future enhancements (attended transfer, etc.)

## Proposed Solution

### 1. Dialog-Core Changes

#### File: `/crates/dialog-core/src/events/session_coordination.rs`

**Add new TransferRequest variant to existing enum:**
```rust
use rvoip_sip_core::types::refer_to::ReferTo;

// Add this import at the top of the file
use rvoip_sip_core::types::refer_to::ReferTo;

// In the existing SessionCoordinationEvent enum, add this new variant:
pub enum SessionCoordinationEvent {
    // ... existing events ...
    
    /// Call transfer request received (REFER)
    TransferRequest {
        /// Dialog ID for the call being transferred
        dialog_id: DialogId,
        
        /// Transaction ID for the REFER request
        transaction_id: TransactionKey,
        
        /// The parsed Refer-To header (target of transfer)
        refer_to: ReferTo,
        
        /// Optional Referred-By header (who initiated transfer)
        referred_by: Option<String>,
        
        /// Optional Replaces header (for attended transfer)
        replaces: Option<String>,
    },
}
```

#### File: `/crates/dialog-core/src/manager/protocol_handlers.rs`

**Update existing REFER handler to use new TransferRequest event:**
```rust
// Add these imports at the top of the file
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::types::header::TypedHeaderTrait;

// Replace the existing handle_refer_method implementation (lines 290-333)
async fn handle_refer_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
    debug!("Processing REFER request from {}", source);
    
    if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
        // Create server transaction
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for REFER: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        // Parse Refer-To header using sip-core's ReferTo type
        let refer_to = request.typed_header::<ReferTo>()
            .map_err(|e| DialogError::protocol_error(&format!("Missing or invalid Refer-To header: {}", e)))?;
        
        // Extract optional Referred-By header
        let referred_by = request.header("Referred-By")
            .and_then(|h| h.value_str().ok())
            .map(|s| s.to_string());
        
        // Extract optional Replaces header (for attended transfer)
        let replaces = request.header("Replaces")
            .and_then(|h| h.value_str().ok())
            .map(|s| s.to_string());
        
        // Create new dedicated transfer event instead of ReInvite
        let event = SessionCoordinationEvent::TransferRequest {
            dialog_id: dialog_id.clone(),
            transaction_id,
            refer_to,
            referred_by,
            replaces,
        };
        
        self.notify_session_layer(event).await?;
        debug!("REFER request forwarded to session layer as TransferRequest for dialog {}", dialog_id);
        Ok(())
    } else {
        // Send 481 response - no dialog found (existing logic from lines 315-333)
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for REFER: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        let response = rvoip_transaction_core::utils::response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
        
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send 481 response to REFER: {}", e),
            })?;
        
        debug!("REFER processed with 481 response (no dialog found)");
        Ok(())
    }
}
```

### 2. Session-Core Changes

#### New File: `/crates/session-core/src/coordinator/transfer.rs`

```rust
//! Call transfer handling for session coordinator
//!
//! This module handles REFER-based call transfers according to RFC 3515.
//! It manages the transfer flow including:
//! - Processing incoming REFER requests
//! - Sending 202 Accepted responses
//! - Initiating new calls to transfer targets
//! - Sending NOTIFY progress updates
//! - Coordinating call termination upon successful transfer

use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, error, warn};
use tokio::sync::RwLock;
use uuid::Uuid;

use rvoip_dialog_core::{DialogId, TransactionKey};
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::{Request, Response, StatusCode, Method};

use crate::api::types::{SessionId, CallState};
use crate::coordinator::SessionCoordinator;
use crate::errors::{SessionError, SessionResult};

/// Manages REFER subscriptions for transfer progress
#[derive(Debug, Clone)]
pub struct ReferSubscription {
    pub event_id: String,
    pub dialog_id: DialogId,
    pub original_session_id: SessionId,
    pub transfer_session_id: Option<SessionId>,
    pub created_at: std::time::Instant,
}

/// Transfer handler implementation for SessionCoordinator
pub struct TransferHandler {
    coordinator: Arc<SessionCoordinator>,
    /// Active REFER subscriptions indexed by event ID
    subscriptions: Arc<RwLock<HashMap<String, ReferSubscription>>>,
}

impl TransferHandler {
    /// Create a new transfer handler
    pub fn new(coordinator: Arc<SessionCoordinator>) -> Self {
        Self {
            coordinator,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Handle incoming REFER request for call transfer
    /// 
    /// This implements the transferee behavior according to RFC 3515:
    /// 1. Extract and validate the Refer-To header
    /// 2. Send 202 Accepted to acknowledge the transfer request
    /// 3. Initiate a new call to the transfer target
    /// 4. Send NOTIFY messages to report transfer progress
    /// 5. Terminate the original call upon successful transfer
    pub async fn handle_refer_request(
        &self,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        refer_to: ReferTo,
        referred_by: Option<String>,
        replaces: Option<String>,
    ) -> SessionResult<()> {
        info!("Handling REFER request for dialog {}", dialog_id);
        
        // Extract target URI from ReferTo
        let target_uri = refer_to.uri().to_string();
        info!("Transfer target: {}", target_uri);
        
        // Validate we have an active session for this dialog
        let session_id = self.get_session_id_for_dialog(&dialog_id).await?;
        
        // Check if this is an attended transfer (has Replaces parameter)
        if let Some(replaces_value) = replaces {
            info!("Attended transfer with Replaces: {}", replaces_value);
            // TODO: Implement attended transfer in phase 2
            return self.send_error_response(
                &transaction_id,
                501,
                "Attended transfer not yet implemented"
            ).await;
        }
        
        // Send 202 Accepted response immediately
        self.send_response(&transaction_id, 202, "Accepted").await?;
        info!("Sent 202 Accepted for REFER request");
        
        // Create subscription for transfer progress notifications
        let event_id = self.create_refer_subscription(
            &dialog_id,
            &session_id
        ).await?;
        
        // Send initial NOTIFY (transfer pending)
        self.send_transfer_notify(
            &dialog_id,
            &event_id,
            "SIP/2.0 100 Trying\r\n",
            false, // subscription not terminated
        ).await?;
        
        // Initiate new call to transfer target
        let transfer_result = self.initiate_transfer_call(
            &session_id,
            &target_uri,
            referred_by.as_deref()
        ).await;
        
        match transfer_result {
            Ok(new_session_id) => {
                info!("Transfer call initiated successfully to {}", target_uri);
                
                // Update subscription with new session ID
                self.update_subscription(&event_id, new_session_id.clone()).await;
                
                // Monitor new call and send progress NOTIFYs
                self.spawn_transfer_monitor(
                    dialog_id.clone(),
                    session_id.clone(),
                    new_session_id,
                    event_id,
                ).await;
            }
            Err(e) => {
                error!("Failed to initiate transfer call: {}", e);
                
                // Send failure NOTIFY and terminate subscription
                self.send_transfer_notify(
                    &dialog_id,
                    &event_id,
                    "SIP/2.0 503 Service Unavailable\r\n",
                    true, // terminate subscription
                ).await?;
                
                // Clean up subscription
                self.remove_subscription(&event_id).await;
            }
        }
        
        Ok(())
    }

    /// Get session ID for a dialog
    async fn get_session_id_for_dialog(&self, dialog_id: &DialogId) -> SessionResult<SessionId> {
        self.coordinator
            .dialog_to_session
            .get(dialog_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SessionError::NotFound(
                format!("No session found for dialog {}", dialog_id)
            ))
    }

    /// Send a SIP response
    async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: u16,
        reason_phrase: &str,
    ) -> SessionResult<()> {
        self.coordinator
            .dialog_api
            .send_response(transaction_id, status_code, reason_phrase.to_string())
            .await
            .map_err(|e| SessionError::Internal(
                format!("Failed to send response: {}", e)
            ))
    }

    /// Send an error response
    async fn send_error_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: u16,
        reason_phrase: &str,
    ) -> SessionResult<()> {
        self.send_response(transaction_id, status_code, reason_phrase).await
    }

    /// Create a REFER subscription
    async fn create_refer_subscription(
        &self,
        dialog_id: &DialogId,
        session_id: &SessionId,
    ) -> SessionResult<String> {
        let event_id = format!("refer-{}", Uuid::new_v4());
        
        let subscription = ReferSubscription {
            event_id: event_id.clone(),
            dialog_id: dialog_id.clone(),
            original_session_id: session_id.clone(),
            transfer_session_id: None,
            created_at: std::time::Instant::now(),
        };
        
        self.subscriptions.write().await.insert(event_id.clone(), subscription);
        
        Ok(event_id)
    }

    /// Update subscription with transfer session ID
    async fn update_subscription(&self, event_id: &str, transfer_session_id: SessionId) {
        if let Some(mut sub) = self.subscriptions.write().await.get_mut(event_id) {
            sub.transfer_session_id = Some(transfer_session_id);
        }
    }

    /// Remove a subscription
    async fn remove_subscription(&self, event_id: &str) {
        self.subscriptions.write().await.remove(event_id);
    }

    /// Send NOTIFY for transfer progress
    async fn send_transfer_notify(
        &self,
        dialog_id: &DialogId,
        event_id: &str,
        sipfrag_body: &str,
        terminate: bool,
    ) -> SessionResult<()> {
        let subscription_state = if terminate {
            "terminated;reason=noresource"
        } else {
            "active;expires=60"
        };
        
        // Build NOTIFY headers
        let mut notify_headers = HashMap::new();
        notify_headers.insert("Event".to_string(), format!("refer;id={}", event_id));
        notify_headers.insert("Subscription-State".to_string(), subscription_state.to_string());
        notify_headers.insert("Content-Type".to_string(), "message/sipfrag".to_string());
        
        // Send NOTIFY through dialog API
        self.coordinator
            .dialog_api
            .send_notify(dialog_id, "refer".to_string(), Some(sipfrag_body.to_string()))
            .await
            .map_err(|e| SessionError::Internal(
                format!("Failed to send NOTIFY: {}", e)
            ))?;
        
        info!("Sent transfer NOTIFY for dialog {}", dialog_id);
        Ok(())
    }

    /// Initiate new call to transfer target
    async fn initiate_transfer_call(
        &self,
        original_session_id: &SessionId,
        target_uri: &str,
        referred_by: Option<&str>,
    ) -> SessionResult<SessionId> {
        // Get original session details for caller ID
        let original_session = self.coordinator.registry
            .get_session(original_session_id)
            .await?
            .ok_or_else(|| SessionError::NotFound("Session not found".to_string()))?;
        
        // Create new outgoing call to transfer target
        // Keep original caller ID (from) and add Referred-By if present
        let mut metadata = HashMap::new();
        if let Some(referrer) = referred_by {
            metadata.insert("Referred-By".to_string(), referrer.to_string());
        }
        metadata.insert("Transfer-From".to_string(), original_session_id.to_string());
        
        let new_session_id = self.coordinator
            .create_session(
                original_session.from().clone(),  // Keep original caller ID
                target_uri.to_string(),
                None, // SDP will be generated
            )
            .await?;
        
        // Store metadata for the new session
        if let Ok(Some(mut new_session)) = self.coordinator.registry.get_session(&new_session_id).await {
            // Add transfer metadata to the session
            // This would require adding a metadata field to CallSession if not present
        }
        
        // Initiate the call
        self.coordinator
            .initiate_call(&new_session_id, None)
            .await?;
        
        Ok(new_session_id)
    }

    /// Spawn task to monitor transfer progress and send NOTIFYs
    async fn spawn_transfer_monitor(
        &self,
        dialog_id: DialogId,
        original_session_id: SessionId,
        new_session_id: SessionId,
        event_id: String,
    ) {
        let coordinator = self.coordinator.clone();
        let handler = self.clone();
        
        tokio::spawn(async move {
            let mut last_state = CallState::Initiated;
            let mut attempt_count = 0;
            let max_attempts = 30; // 30 seconds timeout
            
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                attempt_count += 1;
                
                // Check new call state
                if let Ok(Some(session)) = coordinator.registry.get_session(&new_session_id).await {
                    let current_state = session.state().clone();
                    
                    // Only send NOTIFY if state changed
                    if current_state != last_state {
                        match current_state {
                            CallState::Ringing => {
                                // Send 180 Ringing NOTIFY
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 180 Ringing\r\n",
                                    false,
                                ).await;
                            }
                            CallState::Active => {
                                // Transfer succeeded - send 200 OK NOTIFY
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 200 OK\r\n",
                                    true, // terminate subscription
                                ).await;
                                
                                // Terminate original call
                                let _ = coordinator.terminate_session(&original_session_id).await;
                                info!("Transfer completed successfully");
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                                break;
                            }
                            CallState::Failed(reason) => {
                                // Transfer failed - send error NOTIFY
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 487 Request Terminated\r\n",
                                    true, // terminate subscription
                                ).await;
                                error!("Transfer failed: {}", reason);
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                                break;
                            }
                            CallState::Terminated => {
                                // Call ended before connecting
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 487 Request Terminated\r\n",
                                    true,
                                ).await;
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                                break;
                            }
                            _ => {
                                // Still in progress
                            }
                        }
                        last_state = current_state;
                    }
                }
                
                if attempt_count >= max_attempts {
                    // Timeout - send error NOTIFY
                    let _ = handler.send_transfer_notify(
                        &dialog_id,
                        &event_id,
                        "SIP/2.0 408 Request Timeout\r\n",
                        true,
                    ).await;
                    error!("Transfer timed out");
                    
                    // Clean up subscription
                    handler.remove_subscription(&event_id).await;
                    break;
                }
            }
            
            // Cleanup any expired subscriptions periodically
            handler.cleanup_expired_subscriptions().await;
        });
    }

    /// Clean up expired subscriptions (older than 5 minutes)
    async fn cleanup_expired_subscriptions(&self) {
        let mut subs = self.subscriptions.write().await;
        let now = std::time::Instant::now();
        let expiry = std::time::Duration::from_secs(300); // 5 minutes
        
        subs.retain(|_, sub| {
            now.duration_since(sub.created_at) < expiry
        });
    }
}

// Implement Clone manually to share between tasks
impl Clone for TransferHandler {
    fn clone(&self) -> Self {
        Self {
            coordinator: self.coordinator.clone(),
            subscriptions: self.subscriptions.clone(),
        }
    }
}
```

#### File: `/crates/session-core/src/coordinator/mod.rs`

**Add transfer module:**
```rust
pub mod coordinator;
pub mod registry;
pub mod event_handler;
pub mod session_ops;
pub mod bridge_ops;
pub mod sip_client;
pub mod server_manager;
pub mod transfer;  // ADD THIS

pub use coordinator::SessionCoordinator;
pub use registry::SessionRegistry;
pub use event_handler::EventHandler;
pub use transfer::TransferHandler;  // ADD THIS
```

#### File: `/crates/session-core/src/coordinator/event_handler.rs`

**Add TransferRequest handling to existing EventHandler:**
```rust
// Add import at top of file
use crate::coordinator::transfer::TransferHandler;

// In the EventHandler struct, add new field:
pub struct EventHandler {
    // ... existing fields ...
    transfer_handler: Arc<TransferHandler>,
}

// In EventHandler::new(), initialize the transfer_handler:
impl EventHandler {
    pub fn new(coordinator: Arc<SessionCoordinator>, /* other params */) -> Self {
        let transfer_handler = Arc::new(TransferHandler::new(coordinator.clone()));
        Self {
            // ... existing fields ...
            transfer_handler,
        }
    }
}

// In handle_session_coordination_event method, add new match arm for TransferRequest:
match event {
    // ... existing cases ...
    
    SessionCoordinationEvent::TransferRequest {
        dialog_id,
        transaction_id,
        refer_to,
        referred_by,
        replaces,
    } => {
        tracing::info!("Received TransferRequest event for dialog {}", dialog_id);
        self.transfer_handler.handle_refer_request(
            dialog_id,
            transaction_id,
            refer_to,
            referred_by,
            replaces,
        ).await?;
    }
    
    // ... rest of existing cases ...
}
```

## Testing Plan

### Unit Tests

#### Test Suite 1: REFER Request Parsing
**File:** `/crates/session-core/tests/transfer_parsing_test.rs`
- Test parsing ReferTo headers using sip-core
- Test extraction of Referred-By headers
- Test extraction of Replaces headers
- Test invalid REFER requests

#### Test Suite 2: Transfer Response Flow
**File:** `/crates/session-core/tests/transfer_response_test.rs`
- Test 202 Accepted response for valid REFER
- Test error response for REFER without active session
- Test subscription creation
- Test initial NOTIFY generation

#### Test Suite 3: Transfer Call Initiation
**File:** `/crates/session-core/tests/transfer_initiation_test.rs`
- Test new session creation with correct From/To
- Test Referred-By header propagation
- Test metadata preservation
- Test error handling for invalid targets

#### Test Suite 4: NOTIFY Progress Updates
**File:** `/crates/session-core/tests/transfer_notify_test.rs`
- Test NOTIFY sequence (100 → 180 → 200)
- Test failure NOTIFY generation
- Test timeout handling
- Test subscription cleanup

#### Test Suite 5: End-to-End Transfer
**File:** `/crates/sip-client/tests/transfer_integration_test.rs`
- Test complete blind transfer flow
- Test transfer with busy target
- Test transfer cancellation
- Test bidirectional transfer attempts

## Implementation Timeline

### Phase 1: Core Infrastructure (2-3 hours)
- Add TransferRequest variant to SessionCoordinationEvent enum in dialog-core
- Update dialog-core REFER handler to use new TransferRequest event
- Create transfer.rs module in session-core/src/coordinator
- Integrate TransferHandler with EventHandler

### Phase 2: Transfer Logic (3-4 hours)
- Implement handle_refer_request
- Implement subscription management
- Implement NOTIFY generation
- Implement transfer monitoring

### Phase 3: Testing (2-3 hours)
- Write unit tests for each component
- Integration tests for complete flow
- Manual testing with SIP clients

### Phase 4: Attended Transfer (Future - 2-3 hours)
- Parse Replaces header
- Implement dialog replacement
- Test attended transfer scenarios

## Total Estimated Time: 7-10 hours (excluding attended transfer)

## Benefits of This Approach

1. **Clean Separation**: Transfer logic isolated in dedicated module
2. **Type Safety**: Dedicated event with pre-parsed headers
3. **Reusability**: Uses existing sip-core parsing infrastructure
4. **Maintainability**: Clear module boundaries and responsibilities
5. **Extensibility**: Easy to add attended transfer support later
6. **Performance**: Pre-parsing headers in dialog-core avoids duplicate work

## Risk Mitigation

1. **Subscription Management**: Use timeout-based cleanup to prevent memory leaks
2. **Concurrent Transfers**: Check for existing transfers before accepting new ones
3. **Error Recovery**: Always send final NOTIFY even on errors
4. **Resource Cleanup**: Ensure subscriptions are removed after completion

## Implementation Order

1. **Dialog-Core Changes First**:
   - Add `TransferRequest` to `SessionCoordinationEvent` enum
   - Update `handle_refer_method` to parse headers and create `TransferRequest` event
   - Compile and verify dialog-core builds

2. **Session-Core Transfer Module**:
   - Create `transfer.rs` in `session-core/src/coordinator/`
   - Implement `TransferHandler` with all methods
   - Add module exports to `mod.rs`

3. **Integration**:
   - Update `EventHandler` to include `TransferHandler` field
   - Add `TransferRequest` case to event handling
   - Wire up initialization in `EventHandler::new()`

4. **Testing**:
   - Unit tests for each component
   - Integration test for complete flow

## Library Assessment Results

### sip-client Library - PARTIAL SUPPORT ✅❌
**Implemented (Transferor Role):**
- ✅ `transfer_call()` method in both Simple and Advanced clients
- ✅ `CallTransferred` event when initiating transfers
- ✅ Proper error handling for transfer failures

**Missing (Transferee Role):**
- ❌ `IncomingTransferRequest` event type for receiving REFER
- ❌ Handler for incoming REFER requests
- ❌ Cannot act as transferee (Bob receiving REFER from Alice)

### client-core Library - FULL TRANSFEROR SUPPORT ✅
**Implemented:**
- ✅ `transfer_call()` method with proper validation
- ✅ Call state validation before transfer
- ✅ URI validation for transfer targets
- ✅ `attended_transfer()` method for consultative transfers
- ✅ Proper session mapping and state management

**Missing:**
- ❌ Handling for incoming REFER requests (transferee role)
- ❌ Events for transfer progress notifications

### session-core Library - MOSTLY COMPLETE ✅
**Implemented:**
- ✅ `transfer_session()` method in SessionControl API
- ✅ `TransferHandler` module with RFC-compliant implementation
- ✅ `TransferRequest` event in SessionCoordinationEvent enum
- ✅ REFER subscription management
- ✅ NOTIFY sending capability
- ✅ Proper 202 Accepted response logic
- ✅ Transfer monitoring with progress updates

**Needs Verification:**
- ⚠️ Dialog-core API methods for sending NOTIFY
- ⚠️ Integration testing of complete flow

## Critical Gap Analysis

The main issue preventing Bob from completing transfers: **rvoip_sip_client cannot act as a transferee**. When Bob receives a REFER from Alice:

1. ❌ No `IncomingTransferRequest` event in SipClientEvent enum
2. ❌ No handling in rvoip_sip_client for transfer requests  
3. ❌ REFER not responded to with 202 Accepted
4. ❌ No NOTIFY messages sent for progress
5. ❌ No new INVITE sent to Charlie

## Implementation Status

### Completed Components
1. ✅ Transferor implementation (Alice sending REFER)
2. ✅ session-core TransferHandler with RFC compliance
3. ✅ client-core transfer_call method
4. ✅ sip-client transfer_call method
5. ✅ TransferRequest event in SessionCoordinationEvent
6. ✅ Tests for blind transfer at session-core level
7. ✅ Instance methods pattern (no static functions)

## Detailed Implementation Plan for Transferee Functionality

### Phase 1: Add IncomingTransferRequest Event (30 minutes)

#### File: `/crates/sip-client/src/events.rs`

**Add new event variant:**
```rust
pub enum SipClientEvent {
    // ... existing events ...
    
    /// Incoming transfer request received
    IncomingTransferRequest {
        /// The call being transferred
        call: std::sync::Arc<Call>,
        /// Target URI to transfer to
        target_uri: String,
        /// Who initiated the transfer (optional)
        referred_by: Option<String>,
        /// Whether this is attended transfer (has Replaces)
        is_attended: bool,
    },
    
    /// Transfer progress notification
    TransferProgress {
        /// Call ID of the original call
        call_id: CallId,
        /// Transfer status
        status: TransferStatus,
        /// Optional message
        message: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum TransferStatus {
    /// Transfer accepted, attempting to call target
    Accepted,
    /// Target is ringing
    Ringing,
    /// Transfer completed successfully
    Completed,
    /// Transfer failed
    Failed(String),
}
```

### Phase 2: Wire Session Events to Client Events (1 hour)

#### File: `/crates/sip-client/src/simple.rs`

**Add handler in event processing loop:**
```rust
// In the session event handler match statement, add:
SessionEvent::IncomingTransferRequest { 
    session_id, 
    target_uri, 
    referred_by,
    replaces 
} => {
    // Find the call for this session
    if let Some(call) = self.find_call_by_session(&session_id) {
        // Emit IncomingTransferRequest event
        self.inner.events.emit(SipClientEvent::IncomingTransferRequest {
            call: call.clone(),
            target_uri: target_uri.clone(),
            referred_by,
            is_attended: replaces.is_some(),
        });
        
        // Automatically accept and process the transfer
        tokio::spawn({
            let inner = self.inner.clone();
            let call_id = call.id;
            async move {
                // The session-core TransferHandler will handle the actual transfer
                // We just need to track progress and update UI
                tracing::info!("Processing incoming transfer request for call {}", call_id);
            }
        });
    }
}

SessionEvent::TransferProgress { session_id, status } => {
    if let Some(call) = self.find_call_by_session(&session_id) {
        let transfer_status = match status {
            SessionTransferStatus::Trying => TransferStatus::Accepted,
            SessionTransferStatus::Ringing => TransferStatus::Ringing,
            SessionTransferStatus::Success => TransferStatus::Completed,
            SessionTransferStatus::Failed(reason) => TransferStatus::Failed(reason),
        };
        
        self.inner.events.emit(SipClientEvent::TransferProgress {
            call_id: call.id,
            status: transfer_status,
            message: None,
        });
    }
}
```

### Phase 3: Add Session Events for Transfer (1 hour)

#### File: `/crates/session-core/src/events.rs`

**Add new session events:**
```rust
pub enum SessionEvent {
    // ... existing events ...
    
    /// Incoming transfer request
    IncomingTransferRequest {
        session_id: SessionId,
        target_uri: String,
        referred_by: Option<String>,
        replaces: Option<String>,
    },
    
    /// Transfer progress update
    TransferProgress {
        session_id: SessionId,
        status: SessionTransferStatus,
    },
}

pub enum SessionTransferStatus {
    Trying,
    Ringing,
    Success,
    Failed(String),
}
```

### Phase 4: Connect TransferHandler to Event System (1.5 hours)

#### File: `/crates/session-core/src/coordinator/transfer.rs`

**Modify to emit events:**
```rust
impl TransferHandler {
    /// Handle incoming REFER request and emit events
    pub async fn handle_refer_request(
        &self,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        refer_to: ReferTo,
        referred_by: Option<String>,
        replaces: Option<String>,
    ) -> SessionResult<()> {
        // ... existing validation ...
        
        // Emit IncomingTransferRequest event
        if let Some(session_id) = self.get_session_id_for_dialog(&dialog_id).await.ok() {
            self.coordinator.emit_event(SessionEvent::IncomingTransferRequest {
                session_id: session_id.clone(),
                target_uri: target_uri.clone(),
                referred_by: referred_by.clone(),
                replaces: replaces.clone(),
            }).await;
        }
        
        // ... rest of existing implementation ...
    }
    
    /// Send transfer progress events
    async fn emit_progress(&self, session_id: &SessionId, status: SessionTransferStatus) {
        self.coordinator.emit_event(SessionEvent::TransferProgress {
            session_id: session_id.clone(),
            status,
        }).await;
    }
}
```

### Phase 5: Handle Transfer in UI (1 hour)

#### File: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip_sip_client/src/components/app.rs`

**Add transfer handling to coroutine:**
```rust
// In the event processing match statement:
SipClientEvent::IncomingTransferRequest { 
    call, 
    target_uri, 
    referred_by, 
    is_attended 
} => {
    info!("Incoming transfer request: {} wants to transfer call {} to {}", 
          referred_by.as_deref().unwrap_or("Remote party"),
          call.id,
          target_uri);
    
    // Update UI to show transfer in progress
    if let Some(mut call_info) = call_info_mut() {
        call_info.state = CallState::Transferring;
        call_info.transfer_target = Some(target_uri.clone());
    }
    
    // The actual transfer is handled automatically by session-core
    // We just track the progress
}

SipClientEvent::TransferProgress { call_id, status, message } => {
    match status {
        TransferStatus::Accepted => {
            info!("Transfer accepted, calling target...");
        }
        TransferStatus::Ringing => {
            info!("Transfer target is ringing...");
        }
        TransferStatus::Completed => {
            info!("Transfer completed successfully");
            // Call will be terminated automatically
        }
        TransferStatus::Failed(reason) => {
            error!("Transfer failed: {}", reason);
            // Revert to previous state
            if let Some(mut call_info) = call_info_mut() {
                call_info.state = CallState::Connected;
                call_info.transfer_target = None;
            }
        }
    }
}
```

### Phase 6: Update Call State Display (30 minutes)

#### File: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip_sip_client/src/components/call_interface_screen.rs`

**Add transfer status display:**
```rust
// In the render method, add transfer status:
if call_state == Some(CallState::Transferring) {
    ui.label("Transfer in progress...");
    if let Some(target) = &call_info.transfer_target {
        ui.label(format!("Transferring to: {}", target));
    }
}
```

### Phase 7: Testing (2 hours)

#### Test Scenarios:
1. **Basic Transfer Test:**
   - Alice calls Bob
   - Alice transfers Bob to Charlie
   - Verify Bob receives IncomingTransferRequest
   - Verify Bob sends 202 Accepted
   - Verify Bob calls Charlie
   - Verify NOTIFY sent to Alice
   - Verify Alice's call terminates

2. **Transfer Failure Test:**
   - Alice transfers Bob to invalid URI
   - Verify Bob sends failure NOTIFY
   - Verify original call preserved

3. **Concurrent Transfer Test:**
   - Multiple transfers happening simultaneously
   - Verify each handled independently

## Implementation Order

1. **Start with Session-Core** (Already complete)
   - ✅ TransferHandler implementation
   - ✅ Event emission hooks

2. **Add Events to sip-client** (30 min)
   - Add IncomingTransferRequest event
   - Add TransferProgress event
   - Add TransferStatus enum

3. **Wire Events in Simple Client** (1 hour)
   - Handle session events
   - Emit client events
   - Track transfer state

4. **Update UI Components** (1 hour)
   - Handle new events in app.rs
   - Update call interface display
   - Show transfer progress

5. **Integration Testing** (2 hours)
   - End-to-end transfer scenarios
   - Error cases
   - UI responsiveness

## Total Estimated Time: 5.5 hours

## Success Criteria

1. ✅ `TransferRequest` event properly added to `SessionCoordinationEvent` enum
2. ⏳ Bob responds with 202 Accepted when receiving REFER
3. ⏳ Bob initiates new call to Charlie
4. ⏳ Bob sends NOTIFY progress updates to Alice
5. ⏳ Successful transfer terminates original call
6. ⏳ Failed transfer preserves original call
7. ⏳ UI shows transfer progress
8. ⏳ All integration tests pass
9. ⏳ Works with real SIP clients (Linphone, etc.)