# rVOIP Transaction-Core Redesign Plan

## Overview

This document outlines the plan to redesign the transaction-core library to better align with RFC 3261's transaction model. The redesign will:

1. Consolidate to the four core transaction types defined in RFC 3261
2. Provide a clear, comprehensive API for the session layer
3. Improve handling of special cases like CANCEL and ACK for 2xx responses
4. Simplify the developer experience 

## Architectural Changes

### Current Structure
```
src/
├── client/
│   ├── invite.rs      # INVITE Client Transaction
│   ├── non_invite.rs  # Non-INVITE Client Transaction
│   ├── cancel.rs      # CANCEL Client Transaction
│   ├── update.rs      # UPDATE Client Transaction
│   ├── data.rs        # Common client data structures
│   └── mod.rs
├── server/
│   ├── invite.rs      # INVITE Server Transaction
│   ├── non_invite.rs  # Non-INVITE Server Transaction
│   ├── cancel.rs      # CANCEL Server Transaction  
│   ├── update.rs      # UPDATE Server Transaction
│   ├── data.rs        # Common server data structures
│   └── mod.rs
├── transaction/       # Common transaction functionality
└── manager/           # Transaction manager
```

### Target Structure
```
src/
├── client/
│   ├── invite.rs      # ICT (INVITE Client Transaction)
│   ├── non_invite.rs  # NICT (Non-INVITE Client Transaction)
│   ├── data.rs        # Common client data structures
│   └── mod.rs
├── server/
│   ├── invite.rs      # IST (INVITE Server Transaction)
│   ├── non_invite.rs  # NIST (Non-INVITE Server Transaction)
│   ├── data.rs        # Common server data structures
│   └── mod.rs
├── method/            # New module for method-specific behavior
│   ├── cancel.rs      # CANCEL-specific helper functions
│   ├── update.rs      # UPDATE-specific helper functions
│   ├── ack.rs         # ACK-specific helper functions
│   └── mod.rs
├── transaction/       # Common transaction functionality
└── manager/           # Enhanced transaction manager
```

## Public API for Session Layer

The public API will be implemented in the TransactionManager and provides all functionality needed by the session layer:

### Core Transaction Creation & Control

```rust
// Transaction creation
pub async fn create_invite_client_transaction(&self, request: Request, destination: SocketAddr) -> Result<TransactionKey>;
pub async fn create_non_invite_client_transaction(&self, request: Request, destination: SocketAddr) -> Result<TransactionKey>;
pub async fn create_server_transaction(&self, request: Request, source: SocketAddr) -> Result<TransactionKey>;

// Special transaction operations
pub async fn cancel_invite_transaction(&self, invite_tx_id: &TransactionKey) -> Result<TransactionKey>;
pub async fn send_ack_for_2xx(&self, invite_tx_id: &TransactionKey, response: &Response) -> Result<()>;
pub async fn create_ack_for_2xx(&self, invite_tx_id: &TransactionKey, response: &Response) -> Result<Request>;

// Request/response operations
pub async fn send_request(&self, tx_id: &TransactionKey) -> Result<()>;
pub async fn send_response(&self, tx_id: &TransactionKey, response: Response) -> Result<()>;
pub async fn retry_request(&self, tx_id: &TransactionKey) -> Result<()>;
```

### Transaction State & Information

```rust
// State information
pub async fn transaction_state(&self, tx_id: &TransactionKey) -> Result<TransactionState>;
pub async fn transaction_kind(&self, tx_id: &TransactionKey) -> Result<TransactionKind>;
pub async fn transaction_exists(&self, tx_id: &TransactionKey) -> async -> bool;

// Transaction data access
pub async fn original_request(&self, tx_id: &TransactionKey) -> Result<Option<Request>>;
pub async fn last_response(&self, tx_id: &TransactionKey) -> Result<Option<Response>>;
pub async fn remote_addr(&self, tx_id: &TransactionKey) -> Result<SocketAddr>;

// Transaction matching
pub async fn find_transaction_by_message(&self, message: &Message) -> Result<Option<TransactionKey>>;
pub async fn find_related_transactions(&self, tx_id: &TransactionKey) -> Result<Vec<TransactionKey>>;

// Special lookups
pub async fn find_invite_transaction_for_cancel(&self, cancel_request: &Request) -> Result<Option<TransactionKey>>;
```

### Transaction Events & Notifications

```rust
// Event subscription
pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent>;

// Specific event filtering helpers
pub async fn wait_for_transaction_state(&self, tx_id: &TransactionKey, 
                                      state: TransactionState,
                                      timeout: Duration) -> Result<bool>;
pub async fn wait_for_final_response(&self, tx_id: &TransactionKey, 
                                   timeout: Duration) -> Result<Option<Response>>;
```

### Management Functions

```rust
// Transaction management
pub async fn terminate_transaction(&self, tx_id: &TransactionKey) -> Result<()>;
pub async fn cleanup_terminated_transactions(&self) -> Result<usize>;

// Monitoring
pub async fn active_transactions(&self) -> (Vec<TransactionKey>, Vec<TransactionKey>);
pub async fn transaction_count(&self) -> usize;
```

## Implementation Plan

### Phase 1: Public API Enhancement
- [x] Add Display trait for TransactionKind enum
- [x] Create utility functions in method/mod.rs, method/cancel.rs, method/ack.rs and method/update.rs
- [x] Implement original_request() function
- [x] Implement last_response() function
- [x] Implement remote_addr() function
- [x] Implement wait_for_transaction_state() function
- [x] Implement wait_for_final_response() function
- [x] Implement transaction_count() function
- [x] Implement terminate_transaction() function
- [x] Implement cleanup_terminated_transactions() function
- [x] Implement find_related_transactions() function
- [x] Implement retry_request() function

### Phase 2: Internal Refactoring
- [x] Create method/ directory structure
- [x] Move specialized CANCEL logic to method/cancel.rs
- [x] Move specialized UPDATE logic to method/update.rs
- [x] Add ACK handling in method/ack.rs
- [x] Modify TransactionManager to use method-specific modules

### Phase 3: Transaction Consolidation
- [x] Refactor to use only four core transaction types
- [x] Update ClientCancelTransaction to use NonInviteClientTransaction internally
- [x] Update ClientUpdateTransaction to use NonInviteClientTransaction internally
- [x] Update ServerCancelTransaction to use NonInviteServerTransaction internally
- [x] Update ServerUpdateTransaction to use NonInviteServerTransaction internally

### Phase 4: Testing & Validation
- [x] Update existing tests to use new API methods
- [x] Add test coverage for special cases (CANCEL, ACK)
- [x] Verify all RFC 3261 requirements are met
- [ ] Performance testing

## Required Improvements for Production Readiness

### 1. Event Broadcasting Reliability
- [x] Fix event broadcasting to ensure all subscribers receive events
- [x] Add robust error handling for event channel failures
- [ ] Implement better channel capacity management to prevent backpressure
- [ ] Add diagnostics for tracking dropped or missed events

### 2. State Management Robustness
- [x] Improve synchronization of state transitions across threads
- [x] Add more detailed logging for state transitions
- [x] Implement more comprehensive state validation
- [ ] Add recovery mechanisms for invalid states

### 3. Transaction Lifecycle Management
- [x] Enhance transaction termination process to guarantee cleanup
- [x] Implement periodic health checks for transactions
- [x] Add timeout detection for stalled transactions
- [ ] Create monitoring hooks for transaction metrics

### 4. Error Handling and Recovery
- [x] Improve error categorization and reporting
- [ ] Add circuit breakers for failing remote endpoints
- [x] Implement retry mechanisms with exponential backoff
- [ ] Add graceful degradation for partial system failures

### 5. Performance Optimizations
- [x] Implement more efficient transaction lookup mechanisms
- [ ] Add connection pooling for underlying transport
- [ ] Optimize memory usage in transaction data structures
- [ ] Improve concurrency control to reduce lock contention

### 6. Testing and Validation
- [x] Add comprehensive integration test suite with network simulation
- [ ] Implement chaos testing to verify robustness
- [ ] Create benchmark suite for performance validation
- [x] Add conformance tests against RFC 3261 requirements

## Special Notes

### CANCEL Handling
CANCEL is given special treatment with its own API method because it has unique requirements:
- Must target an existing INVITE transaction
- Must copy specific headers from original INVITE (Call-ID, From, To, CSeq number)
- Can only be sent if original INVITE hasn't received a final response
- Must follow specific rules in RFC 3261 Section 9.1

### ACK for 2xx Responses
ACK for 2xx responses is a special case:
- ACK for non-2xx is part of the original INVITE transaction
- ACK for 2xx is a separate transaction in its own right
- Requires special handling according to RFC 3261 