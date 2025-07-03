# Transaction Core

[![Crates.io](https://img.shields.io/crates/v/rvoip-transaction-core.svg)](https://crates.io/crates/rvoip-transaction-core)
[![Documentation](https://docs.rs/rvoip-transaction-core/badge.svg)](https://docs.rs/rvoip-transaction-core)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

## Overview

The `transaction-core` library implements the SIP transaction layer as defined in RFC 3261. It provides reliable message delivery and proper state management for SIP request-response exchanges, even over unreliable transports like UDP.

## Architecture

This library follows a message-passing architecture using Tokio's async/await model:

```
┌───────────────────┐     ┌───────────────────┐
│  Transaction User │     │  SIP Transport    │
│  (Application)    │◄────┤  (Network Layer)  │
└─────────┬─────────┘     └─────────▲─────────┘
          │                         │
          ▼                         │
┌────────────────────────────────────────────┐
│                                            │
│             TransactionManager             │
│                                            │
└────────────┬───────────────────┬───────────┘
             │                   │
┌────────────▼────────┐ ┌────────▼────────────┐
│ Client Transactions │ │ Server Transactions │
└─────────────────────┘ └─────────────────────┘
```

### Transaction vs. Dialog Layer

One of the key architectural principles in SIP is the separation between transaction and dialog layers:

- **Transaction Layer** (this library): Handles individual request-response exchanges with defined lifecycles. Responsible for message reliability, retransmissions, and state tracking for a single exchange.

- **Dialog Layer** (implemented separately): Maintains long-lived application state across multiple transactions. Manages the relationship between endpoints using Call-ID, tags, and sequence numbers.

This separation allows the transaction layer to focus on protocol-level reliability while letting the dialog layer handle the application logic and session state.

### Integration with Session Core

In the RVOIP stack, the `transaction-core` library provides the foundation for the `session-core` library, which implements dialog and session management:

```
┌─────────────────────────┐
│ Application Layer       │
│ (SIP Client/Server)     │
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│ Session Core            │
│ (Dialog & Call Sessions)│
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│ Transaction Core        │
│ (Message Reliability)   │
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│ SIP Transport Layer     │
│ (UDP, TCP, TLS, etc.)   │
└─────────────────────────┘
```

The relationship between these libraries follows these principles:

1. **Ownership**: Session Core holds a reference to the Transaction Manager
2. **Event Flow**: Transaction events are propagated to Session Core, which maps them to dialogs
3. **Request Initiation**: Session Core uses Transaction Core to initiate requests
4. **Response Handling**: Transaction Core delivers responses back to Session Core
5. **Message Reliability**: Transaction Core handles all retransmissions transparently for Session Core

This layered approach allows each library to focus on its specific responsibilities while providing a clean API for higher-level application logic.

### Key Components

1. **TransactionManager**: Central coordinator for all transactions, providing the primary API for the application
2. **Transaction Types**:
   - **ClientInviteTransaction**: For INVITE client transactions
   - **ClientNonInviteTransaction**: For non-INVITE client transactions
   - **ServerInviteTransaction**: For INVITE server transactions
   - **ServerNonInviteTransaction**: For non-INVITE server transactions
3. **Method-specific Modules**:
   - **method/cancel.rs**: Specialized logic for CANCEL requests
   - **method/ack.rs**: Special handling for ACK requests
   - **method/update.rs**: Support for UPDATE method
4. **Transaction States**:
   - Initial
   - Calling/Trying (client-side)
   - Proceeding
   - Completed
   - Confirmed (INVITE server only)
   - Terminated

## Code Organization

The library code is organized into several modules:

- **manager/**: Contains the `TransactionManager` implementation, the main entry point for applications
- **client/**: Implements client transaction state machines
- **server/**: Implements server transaction state machines
- **transaction/**: Defines common transaction traits, states, and events
- **method/**: Contains method-specific logic for special SIP methods
- **timer/**: Implements timer management for retransmissions and timeouts
- **utils/**: Utility functions for transaction processing
- **error/**: Error types and results

## State Management and Event Flow

The transaction system manages state through a combination of state machines, command channels, and event broadcasting:

### Transaction Identification and Storage

Transactions are identified by a `TransactionKey` containing:
- Branch ID (from the Via header)
- SIP method type (INVITE, REGISTER, etc.)
- Flag indicating if it's a server-side transaction

The `TransactionManager` stores transactions in two separate HashMaps:
- `client_transactions`: For client-side transactions
- `server_transactions`: For server-side transactions

### Transaction Matching

According to RFC 3261 sections 17.1.3 and 17.2.3, transactions are matched as follows:

- **For Responses**: Matched using the branch parameter in the top Via header, the sent-by value, and the CSeq method
- **For Requests**: Server transactions are matched using the branch parameter, the request method, and sent-by value
- **For ACK to non-2xx**: Matched to the original INVITE transaction (same branch parameter)
- **For ACK to 2xx**: Not matched to any transaction - handled by the TU (dialog layer)
- **For CANCEL**: Creates a new transaction but targets an existing INVITE transaction with the same identifiers

The `TransactionManager` implements these rules to properly route incoming messages.

### State Machine Implementation

Each transaction implements a state machine according to RFC 3261:

1. **State Storage**: States are stored in `AtomicTransactionState` objects for thread-safe access
2. **State Transitions**: Transitions are validated according to RFC 3261 rules
3. **State-specific Logic**: Each state has specific behavior for message processing and timer management

### Command and Event Architecture

The system uses an actor-like pattern where each transaction runs in its own task:

1. **Command Flow**:
   - Application code calls methods on `TransactionManager`
   - `TransactionManager` sends commands to transactions via command channels
   - Each transaction's runner task processes commands and executes appropriate actions

2. **Event Broadcasting**:
   - State changes and other significant events generate `TransactionEvent` objects
   - Events are broadcast to the primary channel and any subscribers
   - Applications receive events via a channel obtained from `subscribe()`

3. **Transaction Runner**:
   - Each transaction runs in a dedicated async task
   - The runner receives commands from its command channel
   - It processes incoming SIP messages and timer events
   - It manages state transitions and executes state-specific logic

### Timers and Retransmissions

The library handles RFC 3261 timers automatically:

1. **Timer Factory**: Creates timers with appropriate intervals based on configuration
2. **Timer Registration**: Each transaction registers its timers with the timer manager
3. **Timer Events**: When timers expire, events are sent to the transaction's command channel
4. **Automatic Retransmissions**: Transactions automatically retransmit requests according to RFC 3261

### Handling Special Cases

The system provides specialized handling for certain SIP methods:

1. **CANCEL Handling**: 
   - Helper functions in `method/cancel.rs` validate and create CANCEL requests
   - CANCEL can only target INVITE transactions that haven't received a final response
   - CANCEL requests maintain the same Call-ID, From, To, and Request-URI as the original INVITE

2. **ACK Handling**:
   - For non-2xx responses: ACK is automatically generated by the transaction layer
   - For 2xx responses: ACK is treated as a separate transaction created by the dialog layer (TU)
   - The `create_ack_for_2xx` method helps TUs generate correct ACK requests

3. **UPDATE Support**:
   - Implements RFC 3311 UPDATE method for session modification without impact on dialog state
   - Uses non-INVITE transaction type for processing

## Test Suite

The library includes an extensive test suite covering various aspects of RFC 3261 compliance:

- **Real-world scenarios**: Tests for authentication flows, network failure recovery, concurrent transactions, etc.
- **INVITE transaction tests**: Tests for success and failure flows of INVITE transactions
- **Non-INVITE transaction tests**: Tests for different non-INVITE methods
- **CANCEL transaction tests**: Tests for cancellation of pending INVITE transactions
- **Integration tests**: End-to-end tests of client and server interaction

All tests run serially with comprehensive logging to aid in debugging and understanding the protocol flow.

## State Machines

### Client INVITE Transaction
```
              Timer A   Timer B
              (Resend)  (Timeout)
                 |         |
                 V         V
Initial ----> Calling ---------> Terminated
               |    \
          1xx  |     \ 2xx
               |      \
               V       \ 
           Proceeding --┘
               |
        3xx-6xx|
               V
            Completed ---------> Terminated
                       Timer D
```

### Client Non-INVITE Transaction
```
              Timer E    Timer F
              (Resend)   (Timeout)
                 |          |
                 V          V
Initial ----> Trying ----------> Terminated
               |   \
          1xx  |    \ 2xx-6xx
               |     \
               V      \
           Proceeding  \
               |       |
        2xx-6xx|       |
               V       V
            Completed ---------> Terminated
                       Timer K
```

### Server INVITE Transaction
```
                           INVITE
                             |
                             V
                        Proceeding --------> Terminated
                         /      \            (2xx sent)
                        /        \
                   2xx /          \ 3xx-6xx
                      /            \
                     V              V
               Terminated         Completed
                                /    |    \
                               /     |     \
                         ACK /   TimerG    \ Timer H
                           /   (Resend)     \(Timeout)
                          V                  V
                      Confirmed ----------> Terminated
                           \
                            \ Timer I
                             \
                              V
                          Terminated
```

### Server Non-INVITE Transaction
```
                         Request
                            |
                            V
                         Trying
                         /    \
                  1xx   /      \ 2xx-6xx
                       /        \
                      V          V
                 Proceeding     Completed
                     |          /   |
              2xx-6xx|         /    |
                     |        /     |
                     V       V      V
                  Completed        Terminated
                      |              ^
                Timer J|             |
                      |             /
                      V            /
                  Terminated  ----/
```

## Usage

### Creating a Transaction Manager

```rust
// Create a transport implementation
let transport = Arc::new(YourTransportImplementation::new());

// Create channels for transport events
let (transport_tx, transport_rx) = mpsc::channel(100);

// Initialize the transaction manager
let (manager, events_rx) = TransactionManager::new(
    transport,
    transport_rx,
    Some(100) // Event channel capacity
).await.unwrap();
```

### Client Transaction Example

```rust
// Create a SIP request
let request = RequestBuilder::new(Method::Register, "sip:example.com")?
    .from("Alice", "sip:alice@atlanta.com", Some("tag123"))
    .to("Bob", "sip:bob@biloxi.com", None)
    .call_id("callid123@atlanta.com")
    .cseq(1)
    .build();

// Create and initiate a client transaction
let tx_id = manager.create_client_transaction(request, remote_addr).await?;
manager.send_request(&tx_id).await?;

// Process events from the events_rx channel
while let Some(event) = events_rx.recv().await {
    match event {
        TransactionEvent::ProvisionalResponse { transaction_id, response, .. } => {
            println!("Received 1xx response: {}", response.status_code());
        },
        TransactionEvent::SuccessResponse { transaction_id, response, .. } => {
            println!("Received 2xx response: {}", response.status_code());
            // For INVITE, handle ACK for 2xx responses at the TU level
            if is_invite {
                let ack_request = manager.create_ack_for_2xx(&transaction_id, &response).await?;
                // Send the ACK (typically via a new transport method or direct send)
            }
        },
        TransactionEvent::FailureResponse { transaction_id, response, .. } => {
            println!("Received 3xx-6xx response: {}", response.status_code());
        },
        TransactionEvent::TransactionTerminated { transaction_id, .. } => {
            println!("Transaction terminated: {}", transaction_id);
        },
        // Handle other events
        _ => {}
    }
}
```

### Server Transaction Example

```rust
// Handle incoming requests via transport_tx events
transport_tx.send(TransportEvent::MessageReceived {
    message: Message::Request(request),
    source: remote_addr,
    destination: local_addr,
}).await?;

// Process events to get the transaction ID of the new server transaction
let tx_id = match events_rx.recv().await.unwrap() {
    TransactionEvent::NewRequest { transaction_id, request, source, .. } => {
        println!("New request: {}", request.method());
        
        // Create a server transaction
        let server_tx = manager.create_server_transaction(
            request.clone(), 
            source
        ).await.expect("Failed to create server transaction");
        
        server_tx.id().clone()
    },
    _ => panic!("Expected NewRequest event"),
};

// Send a response through the transaction
let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))?
    // Add headers...
    .build();

manager.send_response(&tx_id, response).await?;
```

### CANCEL Example

```rust
// To cancel an ongoing INVITE transaction:
let cancel_tx_id = manager.cancel_invite_transaction(&invite_tx_id).await?;

// The cancel_invite_transaction method handles:
// 1. Creating the correct CANCEL request
// 2. Creating a new transaction for the CANCEL
// 3. Sending the CANCEL request
// 4. Managing the relationship between the INVITE and CANCEL transactions
```

### Handling ACK for non-2xx Responses

```rust
// When the server transaction has sent a non-2xx response to an INVITE,
// the client will send an ACK directly to the server transaction.
// This ACK needs to be processed by the server transaction:

// Assuming you have the server transaction ID and the ACK request:
let server_invite_tx_id = /* ID of the INVITE server transaction */;
let ack_request = /* The ACK request received from the client */;

// Process the ACK in the server transaction
manager.process_request(&server_invite_tx_id, ack_request).await?;

// This will cause the server INVITE transaction to transition to Confirmed state,
// and eventually to Terminated after Timer I expires.
```

## Error Handling

The library provides specific error types for different failure scenarios:

- `Error::TransactionNotFound`: When a transaction ID doesn't exist
- `Error::InvalidStateTransition`: For illegal state transitions
- `Error::TransactionTimeout`: When a transaction times out
- `Error::TransportError`: For network-related failures

## Timer Configuration

Transaction timers can be configured through the `TimerSettings` struct:

```rust
let custom_timers = TimerSettings {
    t1: Duration::from_millis(500),   // Base timer value
    t2: Duration::from_secs(4),       // Max retransmit interval
    transaction_timeout: Duration::from_secs(32),  // Client transaction timeout
    wait_time_d: Duration::from_secs(32),  // Timer D duration
    wait_time_h: Duration::from_secs(32),  // Timer H duration
    wait_time_i: Duration::from_secs(5),   // Timer I duration
    wait_time_j: Duration::from_secs(32),  // Timer J duration
    wait_time_k: Duration::from_secs(5),   // Timer K duration
};

let (manager, events_rx) = TransactionManager::new_with_config(
    transport,
    transport_rx,
    Some(100),             // Event channel capacity
    Some(custom_timers),   // Custom timer settings
).await?;
```

## Best Practices

1. Always process all events from the `events_rx` channel
2. Properly handle `TransactionTerminated` events to avoid resource leaks
3. For INVITE transactions, handle ACK for 2xx responses at the dialog layer
4. Configure appropriate timer values based on network conditions
5. Implement proper error handling and retries at the application level
6. Use `cleanup_terminated_transactions` periodically for long-running applications
7. Subscribe to events using the `subscribe()` method when implementing multiple consumers
8. Verify transaction state with `transaction_state()` before critical operations

## Relationship to Other Crates

### Core Dependencies

- **`rvoip-sip-core`**: Provides SIP message types, parsing, and protocol definitions
- **`rvoip-sip-transport`**: Transport layer for message delivery and network I/O
- **`tokio`**: Async runtime for concurrent transaction processing
- **`async-trait`**: Async trait support for transport abstractions

### Optional Dependencies

- **`uuid`**: Transaction ID generation and uniqueness
- **`rand`**: Random number generation for timer variations
- **`tracing`**: Structured logging and debugging support

### Integration with rvoip Stack

```
┌─────────────────────────────────────────┐
│            Application Layer            │
├─────────────────────────────────────────┤
│          rvoip-session-core             │
├─────────────────────────────────────────┤
│       rvoip-transaction-core  ⬅️ YOU ARE HERE
├─────────────────────────────────────────┤
│         rvoip-sip-transport             │
├─────────────────────────────────────────┤
│            Network Layer                │
└─────────────────────────────────────────┘
```

The transaction layer serves as the reliability layer in the SIP stack, providing:

- **Upward Interface**: Delivers transaction events to session-core/dialog layer
- **Downward Interface**: Uses sip-transport for actual message transmission
- **State Management**: Maintains transaction state machines per RFC 3261
- **Timer Services**: Handles retransmissions and timeouts transparently

## Testing

Run the comprehensive test suite:

```bash
# Run all tests
cargo test -p rvoip-transaction-core

# Run with detailed logging
RUST_LOG=debug cargo test -p rvoip-transaction-core

# Run specific test categories
cargo test -p rvoip-transaction-core --test integration_tests
cargo test -p rvoip-transaction-core client_transaction
cargo test -p rvoip-transaction-core server_transaction
cargo test -p rvoip-transaction-core cancel_transaction

# Run tests serially (recommended for debugging)
cargo test -p rvoip-transaction-core -- --test-threads=1
```

### Test Coverage

The library includes extensive test coverage for:

- **RFC 3261 Compliance**: State machine transitions, timer behavior, message handling
- **Integration Scenarios**: End-to-end client-server transaction flows
- **Error Conditions**: Network failures, malformed messages, timeout scenarios
- **Concurrency**: Multiple simultaneous transactions, thread safety
- **Special Cases**: CANCEL handling, ACK processing, method-specific behavior

All tests are designed to run deterministically with comprehensive logging for debugging.

## Performance Characteristics

### Transaction Throughput
- **Concurrent Transactions**: Scales linearly with available CPU cores
- **Memory Usage**: ~2KB per active transaction (excluding message buffers)
- **State Transitions**: Sub-millisecond latency for most transitions
- **Timer Precision**: Configurable from millisecond to second granularity

### Scalability Factors
- **Client Transactions**: Limited primarily by network bandwidth and remote server capacity
- **Server Transactions**: Can handle thousands of concurrent transactions with proper system tuning
- **Memory Management**: Automatic cleanup of terminated transactions
- **Timer Efficiency**: Uses Tokio's timer wheel for efficient timeout management

### Optimization Recommendations
- **Timer Configuration**: Tune timer values based on network RTT characteristics
- **Event Processing**: Use dedicated tasks for event processing to avoid blocking
- **Connection Pooling**: Leverage transport layer connection reuse for TCP/TLS
- **Message Buffering**: Consider message size when planning memory allocation

## Future Improvements

See [TODO.md](./TODO.md) for a comprehensive roadmap including:

### Performance Enhancements
- Transaction pool management for reduced allocation overhead
- Zero-copy message processing where possible
- Batch operations for multiple transactions
- Memory usage optimization for long-running services

### Reliability Improvements  
- Transaction state persistence for crash recovery
- Enhanced retry mechanisms with exponential backoff
- Transport failure detection and automatic recovery
- Network condition adaptive timer adjustment

### Monitoring & Observability
- Detailed transaction metrics and performance monitoring  
- Integration with infra-common monitoring infrastructure
- Debug tracing for complex transaction scenarios
- Health check APIs for operational monitoring

### Integration Enhancements
- Direct integration with infra-common event bus
- Priority-based transaction processing
- Plugin architecture for custom transaction behavior
- Enhanced dialog layer integration

## Contributing

Contributions are welcome! Please see the main [rvoip contributing guidelines](../../README.md#contributing) for details on:

- Code style and formatting requirements
- Testing standards and coverage expectations  
- Pull request process and review criteria
- Development environment setup

For transaction-core specific contributions:
- Ensure RFC 3261 compliance for any state machine changes
- Add comprehensive tests for new transaction scenarios
- Update documentation for any API changes
- Consider backward compatibility for public interfaces

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option. 

## Features

### ✅ Completed Features

- **Transaction Management**
  - ✅ Complete RFC 3261 compliant transaction state machines
  - ✅ Client and server transaction support for all SIP methods
  - ✅ Automatic transaction identification and matching
  - ✅ Thread-safe transaction storage and lookup

- **INVITE Transaction Support**
  - ✅ Client INVITE transactions with proper state management
  - ✅ Server INVITE transactions with 2xx/non-2xx response handling
  - ✅ Automatic ACK generation for non-2xx responses
  - ✅ Dialog layer integration for 2xx ACK handling

- **Non-INVITE Transaction Support**
  - ✅ Client non-INVITE transactions (REGISTER, OPTIONS, etc.)
  - ✅ Server non-INVITE transactions with proper timeouts
  - ✅ All SIP methods supported (REGISTER, OPTIONS, INFO, etc.)

- **Special Method Handling**
  - ✅ CANCEL request validation and processing
  - ✅ UPDATE method support (RFC 3311)
  - ✅ ACK handling for both 2xx and non-2xx responses
  - ✅ Method-specific state machine optimizations

- **Timer Management**
  - ✅ RFC 3261 compliant timers (T1, T2, Timer A-K)
  - ✅ Configurable timer values for different network conditions
  - ✅ Automatic retransmissions and timeout handling
  - ✅ Adaptive timer behavior based on transport type

- **Error Handling & Reliability**
  - ✅ Comprehensive error types with detailed context
  - ✅ Transaction timeout detection and cleanup
  - ✅ Transport error propagation and handling
  - ✅ Graceful degradation on network failures

- **Event System**
  - ✅ Rich event notifications for transaction state changes
  - ✅ Event broadcasting to multiple subscribers
  - ✅ Detailed transaction lifecycle events
  - ✅ Integration-friendly event architecture

### 🚧 Planned Features

- **Performance Optimizations**
  - 🚧 Transaction pool management for high-throughput scenarios
  - 🚧 Zero-copy message handling optimizations
  - 🚧 Batch processing for multiple transactions
  - 🚧 Memory usage optimization for long-running applications

- **Advanced Reliability**
  - 🚧 Transaction recovery from transport failures
  - 🚧 Persistent transaction state for crash recovery
  - 🚧 Advanced retry mechanisms with backoff strategies
  - 🚧 Network condition adaptive timer adjustment

- **Monitoring & Diagnostics**
  - 🚧 Transaction metrics and performance monitoring
  - 🚧 Debug tracing for transaction state transitions
  - 🚧 Health check APIs for transaction manager
  - 🚧 Integration with infra-common monitoring

- **Enhanced Integration**
  - 🚧 Direct integration with infra-common event bus
  - 🚧 Priority-based transaction processing
  - 🚧 Plugin architecture for custom transaction handling 