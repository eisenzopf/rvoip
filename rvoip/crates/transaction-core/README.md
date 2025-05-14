# Transaction Core

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

### Key Components

1. **TransactionManager**: Central coordinator for all transactions
2. **Transaction Types**:
   - **ClientInviteTransaction**: For INVITE client transactions
   - **ClientNonInviteTransaction**: For non-INVITE client transactions
   - **ServerInviteTransaction**: For INVITE server transactions
   - **ServerNonInviteTransaction**: For non-INVITE server transactions
3. **Transaction States**:
   - Initial
   - Calling/Trying (client-side)
   - Proceeding
   - Completed
   - Confirmed (INVITE server only)
   - Terminated

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
        TransactionEvent::ProvisionalResponse { transaction_id, response } => {
            println!("Received 1xx response: {}", response.status());
        },
        TransactionEvent::SuccessResponse { transaction_id, response } => {
            println!("Received 2xx response: {}", response.status());
            // For INVITE, send ACK for 2xx responses
            if is_invite {
                manager.send_2xx_ack(&response).await?;
            }
        },
        TransactionEvent::FailureResponse { transaction_id, response } => {
            println!("Received 3xx-6xx response: {}", response.status());
        },
        TransactionEvent::TransactionTerminated { transaction_id } => {
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
let event = events_rx.recv().await.unwrap();
let tx_id = match event {
    TransactionEvent::NewRequest { transaction_id, request, source } => {
        println!("New request: {}", request.method());
        transaction_id
    },
    _ => panic!("Expected NewRequest event"),
};

// Send a response through the transaction
let response = ResponseBuilder::new(StatusCode::Ok, None)?
    // Add headers...
    .build();

manager.send_response(&tx_id, response).await?;
```

## Error Handling

The library provides specific error types for different failure scenarios:

- `Error::TransactionNotFound`: When a transaction ID doesn't exist
- `Error::InvalidStateTransition`: For illegal state transitions
- `Error::TransactionTimeout`: When a transaction times out
- `Error::TransportError`: For network-related failures

## Timer Configuration

Transaction timers can be configured through the `TimerConfig` struct:

```rust
let custom_timers = TimerConfig {
    t1: Duration::from_millis(500),   // Base timer value
    t2: Duration::from_secs(4),       // Max retransmit interval
    transaction_timeout: Duration::from_secs(32),  // Client transaction timeout
    // Other timer configuration...
};

let (manager, events_rx) = TransactionManager::new_with_config(
    transport,
    transport_rx,
    Some(custom_timers),
).await?;
```

## Best Practices

1. Always process all events from the `events_rx` channel
2. Properly handle `TransactionTerminated` events to avoid resource leaks
3. For INVITE transactions, send ACK for 2xx responses outside the transaction
4. Configure appropriate timer values based on network conditions
5. Implement proper error handling and retries at the application level 