# rVOIP Transaction-Core Redesign Plan

## Overview

This document outlines the plan to redesign the transaction-core library to better align with RFC 3261's transaction model. The redesign will:

1. Consolidate to the four core transaction types defined in RFC 3261
2. Provide a clear, comprehensive API for the session layer
3. Improve handling of special cases like CANCEL and ACK for 2xx responses
4. Simplify the developer experience 

## Standardized Event Bus Implementation

Integrate with the infra-common high-performance event bus for transaction processing:

### Transaction Events Architecture

1. **Static Event Implementation (High-Throughput Protocol Events)**
   - [ ] Implement `StaticEvent` trait for all transaction state events
     - [ ] Create `TransactionStateEvent` with StaticEvent fast path
     - [ ] Implement `MessageEvent` for transaction message processing
     - [ ] Optimize critical transaction event types like timeout events
   - [ ] Add specialized event types for transaction types
     - [ ] `InviteClientTransactionEvent`
     - [ ] `InviteServerTransactionEvent`
     - [ ] `NonInviteClientTransactionEvent`
     - [ ] `NonInviteServerTransactionEvent`

2. **Priority-Based Processing**
   - [ ] Use `EventPriority::Critical` for critical transaction operations
     - [ ] Transaction terminations
     - [ ] Error conditions affecting call state
     - [ ] INVITE transaction completion events
   - [ ] Use `EventPriority::High` for important transaction events
     - [ ] Final response processing
     - [ ] Transaction timeouts
     - [ ] Retransmission handling
   - [ ] Use `EventPriority::Normal` for regular transaction processing
     - [ ] Provisional responses
     - [ ] Standard message flow
     - [ ] Normal transaction progression
   - [ ] Use `EventPriority::Low` for monitoring and diagnostics
     - [ ] Transaction metrics
     - [ ] Performance statistics
     - [ ] Debug information

3. **Implementation Components**
   - [ ] Create `TransactionEventPublisher` for common transaction events
     - [ ] Implement per-transaction type publishers
     - [ ] Add specialized publishers for transaction management events
   - [ ] Implement efficient transaction event subscription model
     - [ ] Create typed subscribers for different transaction events
     - [ ] Add filtering capabilities for specific transaction types
     - [ ] Implement correlation between related transaction events

4. **Performance Optimizations**
   - [ ] Configure event bus for optimal transaction performance:
     ```rust
     EventBusConfig {
         max_concurrent_dispatches: 20000,
         broadcast_capacity: 16384,
         enable_priority: true,
         enable_zero_copy: true,
         batch_size: 50,  // Transaction events have moderate batch value
         shard_count: 32,
     }
     ```
   - [ ] Implement event batching for high-throughput scenarios
   - [ ] Optimize memory usage for transaction event propagation
   - [ ] Create efficient memory pools for common event types

5. **Integration with Session Layer**
   - [ ] Create seamless event propagation to session-core
     - [ ] Implement transaction-to-session event mapping
     - [ ] Add event correlation IDs across layers
     - [ ] Create consistent event taxonomies between layers
   - [ ] Create compatibility with transport layer events
     - [ ] Add transport-to-transaction event mapping
     - [ ] Implement transaction event extraction from transport events
     - [ ] Create unified event model across all three layers

6. **Scaling Optimizations**
   - [ ] Test transaction event bus with 100,000+ concurrent transactions
   - [ ] Optimize transaction event memory footprint
   - [ ] Implement adaptive event propagation based on system load
   - [ ] Create automated performance monitoring framework

## Critical Issues Discovered in Benchmark Testing

These issues were identified through benchmark testing and require immediate attention:

### 1. Transaction Event Broadcasting Issues
- [x] Fix indiscriminate event broadcasting that sends all events to all subscribers
- [x] Implement proper event filtering mechanism to direct events only to interested subscribers
- [x] Add transaction-to-subscriber mapping to ensure events are routed correctly
- [x] Add proper synchronization to prevent race conditions in event delivery

### 2. Server Transaction Management Issues
- [x] Fix server transaction lookup to properly identify retransmitted requests
- [x] Ensure retransmitted requests are associated with existing transactions per RFC 3261
- [x] Implement correct branch parameter matching for server transactions
- [x] Add transaction matching diagnostic tools for easier debugging

### 3. Transaction Lifecycle Management Issues
- [x] Fix premature command channel closing in transaction runners
- [x] Ensure transactions remain active long enough to process ACKs for non-2xx responses
- [x] Add proper coordination between transaction termination and related cleanup tasks
- [x] Implement graceful handling of commands to terminated transactions
- [x] Keep transactions alive until all pending operations complete

### 4. Request Processing Issues
- [x] Fix race conditions in concurrent transaction processing
- [x] Implement proper locking for shared transaction state
- [x] Add robust error handling for all edge cases in message processing
- [x] Ensure transactions properly follow RFC 3261 state machine definitions

### 5. Test Improvements
- [x] Create more realistic network simulation tests
- [x] Add tests that explicitly verify retransmission handling
- [x] Implement tests for race conditions and concurrency issues
- [ ] Add benchmarks that detect performance regressions

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
pub async fn subscribe_to_transactions(&self, tx_ids: &[TransactionKey]) -> Result<mpsc::Receiver<TransactionEvent>>;

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

### 7. Tokio Async Runtime Best Practices
- [ ] Replace polling-based subscription tracking with event-driven mechanisms
- [ ] Use task multiplication more efficiently by using shared event handling tasks
- [ ] Replace tokio::sync::Mutex with async-aware alternatives where appropriate
- [ ] Implement proper backpressure mechanisms in event channels
- [ ] Optimize lock granularity to reduce contention
- [ ] Use tokio::select! for efficient multiplexing of multiple event sources
- [ ] Reduce the number of spawned tasks by consolidating related functionality
- [ ] Optimize channel buffer sizes based on expected throughput

## Integration with Session Layer

### 1. Dialog to Transaction Integration
- [x] Fix dialog-to-transaction mapping for proper event routing
- [x] Implement transaction-specific subscriptions for dialogs
- [x] Add proper handling of ACK requests in dialog manager
- [x] Ensure transaction events are processed correctly by dialog layer
- [ ] Optimize event propagation from transaction layer to session layer
- [ ] Add metrics for transaction-to-dialog interactions

### 2. Session Layer Event Processing
- [x] Update session layer to use improved transaction subscription API
- [x] Fix handling of retransmitted requests in dialog manager
- [x] Implement proper error recovery for dialog-transaction interactions
- [ ] Add transaction event batching for more efficient processing

## Integration with Transport Layer

### 1. Dependency Setup
- [x] Add sip-transport dependency to Cargo.toml
- [x] Configure appropriate feature flags (udp, tcp, ws)
- [x] Verify dependency version alignment

### 2. Transport Manager Implementation
- [x] Create TransportManager to manage multiple transport instances
- [x] Implement transport lifecycle management (init, shutdown)
- [x] Add configuration options for transport parameters
- [x] Use TransportFactory for creating transport instances
- [x] Support multiple transport types simultaneously
- [x] Add URI scheme to transport type mapping

### 3. Event Handling and Message Routing
- [x] Implement handler for sip-transport's TransportEvent
- [x] Route incoming messages to appropriate transactions
- [x] Process transport errors correctly in transaction layer
- [x] Update transaction send mechanism to use real transports
- [x] Implement transport selection logic based on message properties

### 4. Connection Management
- [x] Add connection tracking to associate transactions with connections
- [ ] Handle connection failures with appropriate transaction notifications
- [ ] Implement reconnection logic for persistent connections
- [ ] Add transport failover for when primary transport fails
- [ ] Create backoff/retry policies for connection failures

### 5. Testing and Validation
- [x] Update unit tests to use real transport or suitable mocks
- [x] Create integration tests for full network stack
- [ ] Test various transport combinations and failure scenarios
- [x] Add example showing transaction-core with real transport
- [ ] Create benchmarks for measuring performance

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
- Requires special handling according to RFC 3261 Section 13

### Process Request Method
- [x] Implement process_request method for handling in-transaction requests
- [x] Add special handling for ACK requests in INVITE server transactions
- [x] Add proper validation of requests against transaction state
- [x] Ensure retransmitted requests are properly identified and handled 

## Integration Progress (May 17, 2025)

The integration between sip-transport and transaction-core is now complete and working correctly. We have:

1. Successfully implemented the transport layer integration through TransportManager
2. Created comprehensive working examples that demonstrate the full SIP flow:
   - `integrated_transport.rs`: Basic request-response flow with REGISTER
   - `invite_example.rs`: Complete INVITE dialog with call setup, 180 Ringing, 200 OK, ACK, and BYE
   - `non_invite_example.rs`: OPTIONS and MESSAGE request flows with proper response handling 
   - `cancel_example.rs`: INVITE request cancellation with 487 Request Terminated (fixed and working)

3. Fixed issues with timer management during transaction processing
4. Ensured proper resource cleanup after transactions complete

### CANCEL Request Fix (COMPLETED)

We identified and fixed a major issue in the CANCEL request handling:

1. ✅ Fixed `create_cancel_request` in `method/cancel.rs` to preserve the branch parameter from the original INVITE request instead of generating a new one. This is required for RFC 3261 compliance, specifically Section 9.1 that states the CANCEL must have the same branch parameter as the request it is canceling.

2. ✅ Updated `find_invite_transaction_for_cancel` to match CANCEL requests to INVITE transactions using the branch parameter.

3. ✅ Fixed an issue in the TransactionManager where a CANCEL request would get a duplicate Via header added during transaction creation. We now skip adding Via headers for CANCEL requests since they already have the correct Via header from create_cancel_request.

4. ✅ Added extensive debug logging to help track Via headers throughout the transaction process.

5. ✅ Fixed server-side CANCEL handling to properly correlate CANCEL requests with their corresponding INVITE server transactions.

6. ✅ All tests now pass, including the comprehensive integration tests that validate full message flows.

7. ✅ Fixed an issue in the cancel_example.rs that was causing it to hang indefinitely by adding a timeout mechanism to properly terminate the example.

The CANCEL handling now fully complies with RFC 3261 Section 9.1 and 9.2 requirements. This ensures that:
- CANCEL requests have the correct branch parameter
- Server can properly match CANCEL requests to INVITE transactions  
- Proper 487 Request Terminated responses are sent for the original INVITE
- Transaction state management works correctly across both client and server sides

Next priorities:
- Complete the transport failover capabilities
- Add client-side WebSocket connection support
- Implement reconnection logic for persistent connections 