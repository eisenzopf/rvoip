# SIP Transactions

In the previous tutorials, we explored SIP messages and SDP media negotiation. Now we'll move to a higher level of abstraction and learn about SIP transactions - the fundamental building blocks of SIP communication.

## Understanding SIP Transactions

A SIP transaction consists of a client request and all server responses to that request. Transactions are crucial for reliable message delivery in an unreliable network environment like the Internet.

### Types of SIP Transactions

SIP defines four main types of transactions:

1. **INVITE Client Transactions (ICT)**: Used by clients to establish sessions
2. **Non-INVITE Client Transactions (NICT)**: Used by clients for requests other than INVITE
3. **INVITE Server Transactions (IST)**: Used by servers to handle incoming INVITE requests
4. **Non-INVITE Server Transactions (NIST)**: Used by servers to handle non-INVITE requests

Each transaction type follows a different state machine with specific behaviors defined in RFC 3261.

### Transaction Layer Architecture

In the SIP protocol stack, the transaction layer sits between the transport layer and the Transaction User (TU) layer:

```text
+---------------------------+
|  Transaction User (TU)    |  <- Dialogs, call control, etc.
|  (UAC, UAS, Proxy)        |
+---------------------------+
             ↑ ↓
             | |  Transaction events, requests, responses
             ↓ ↑
+---------------------------+
|  Transaction Layer        |  <- TransactionManager
|  (Manager + Transactions) |
+---------------------------+
             ↑ ↓
             | |  Messages, transport events
             ↓ ↑
+---------------------------+
|  Transport Layer          |  <- UDP, TCP, etc.
+---------------------------+
```

## Transaction Timers

SIP transaction reliability is ensured through a series of timers that control message retransmissions, timeouts, and cleanup. These timers (A through K) are critical for proper operation:

- **Timer A**: Controls INVITE request retransmissions
- **Timer B**: INVITE transaction timeout
- **Timer C**: Proxy INVITE transaction timeout
- **Timer D**: Wait time for response retransmissions (INVITE client)
- **Timer E**: Non-INVITE request retransmission interval
- **Timer F**: Non-INVITE transaction timeout
- **Timer G**: INVITE response retransmission interval
- **Timer H**: Wait time for ACK receipt
- **Timer I**: Wait time for ACK retransmissions
- **Timer J**: Wait time for non-INVITE request retransmissions
- **Timer K**: Wait time for response retransmissions (non-INVITE client)

## Transaction State Machines

SIP defines four distinct state machines for transaction processing. Let's examine each one in detail.

### Non-INVITE Client Transaction State Machine

```
Non-INVITE Client Transaction (Section 17.1.2)

           |Request
           V
+-------+
|Trying |------------+
+-------+            |
    |                |
    |1xx             |
    |                |
    V                |
+----------+         |
|Proceeding|         |
+----------+         |
    |                |
    |200-699         |
    |                |
    V                |
+---------+          |
|Completed|<---------+
+---------+
    |
    | Timer K
    V
+-----------+
|Terminated |
+-----------+
```

Here's an example demonstrating a non-INVITE client transaction:

```rust
async fn demonstrate_client_transaction() -> Result<()> {
    // Setup transport and transaction manager
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    let transport = Arc::new(MockTransport::new(local_addr));
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Configure faster timers for example purposes
    let timer_settings = TimerSettings {
        t1: Duration::from_millis(500),
        t2: Duration::from_millis(4000),
        transaction_timeout: Duration::from_millis(5000),
        wait_time_k: Duration::from_millis(500),
        // ... other timer settings ...
    };
    
    let (manager, mut events_rx) = TransactionManager::new_with_config(
        transport.clone(),
        transport_rx,
        Some(100),
        Some(timer_settings),
    ).await.unwrap();
    
    // Create a REGISTER request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
    
    // Create a client transaction
    let transaction_id = manager.create_client_transaction(
        request,
        remote_addr
    ).await.unwrap();
    
    // Initiate the transaction (sends the request)
    manager.send_request(&transaction_id).await.unwrap();
    
    // Check state - should be in Trying
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Initial state: {:?}", state);
    
    // Simulate receiving 100 Trying response
    let provisional_response = ResponseBuilder::new(StatusCode::Trying, None)
        // ... headers ...
        .build();
    
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(provisional_response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events to handle the response
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check state - should be Proceeding
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 100 Trying: {:?}", state);
    
    // Simulate final response and Timer K handling
    // ...
    
    Ok(())
}
```

### Non-INVITE Server Transaction State Machine

```
Non-INVITE Server Transaction (Section 17.2.2)
           |Request
           V
+----------+
|  Trying  |
+----------+
    |
    |1xx
    |
    v
+----------+   2xx-699
|Proceeding|------------+
+----------+            |
    |                   |
    |1xx                |
    |                   |
    v                   v
+----------+        +----------+
|Proceeding|        |Completed |
+----------+        +----------+
                        |
                        | Timer J
                        v
                    +-----------+
                    |Terminated |
                    +-----------+
```

Here's an example of a non-INVITE server transaction:

```rust
async fn demonstrate_server_transaction() -> Result<()> {
    // Setup transport and transaction manager
    // ... similar setup as client example ...
    
    // Create a REGISTER request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        // ... headers ...
        .build();
    
    // Create server transaction directly with the request
    let server_tx = manager.create_server_transaction(
        request.clone(), 
        remote_addr
    ).await.expect("Failed to create server transaction");
    
    // Get the transaction ID
    let transaction_id = server_tx.id().clone();
    
    // Check state - should be Trying
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Initial state: {:?}", state);
    
    // Send a provisional response
    let provisional_response = ResponseBuilder::new(StatusCode::Trying, None)
        // ... headers ...
        .build();
    
    manager.send_response(&transaction_id, provisional_response).await.unwrap();
    
    // Check state - should be Proceeding
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After sending 100 Trying: {:?}", state);
    
    // Simulate final response and retransmission handling
    // ...
    
    Ok(())
}
```

### INVITE Client Transaction State Machine

```
INVITE Client Transaction (Section 17.1.1)
           |INVITE
           V
+-------+
|Calling|-------------+
+-------+             |
    |                 |
    |1xx              |
    |                 |
    V                 |
+----------+          |
|Proceeding|          |
+----------+          |
    |                 |
    |200-699          |
    |                 |
    V                 |
+---------+           |
|Completed|<----------+
+---------+
    |
    | Timer D
    V
+-----------+
|Terminated |
+-----------+
```

INVITE clients have special handling for different response codes:

- For 2xx responses: Transaction transitions directly to Terminated (ACK is sent by the Transaction User)
- For 3xx-6xx responses: Transaction transitions to Completed, sends ACK automatically, then waits for Timer D

Let's see an INVITE client transaction example:

```rust
async fn demonstrate_invite_client_transaction() -> Result<()> {
    // Setup transport and transaction manager
    // ... similar setup as previous examples ...
    
    // Create an INVITE request
    let request = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")?
        // ... headers ...
        .build();
    
    // Create and start transaction
    let transaction_id = manager.create_invite_client_transaction(
        request, 
        remote_addr
    ).await.unwrap();
    
    // Send the request
    manager.send_request(&transaction_id).await.unwrap();
    
    // Check state - should be Calling
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Initial state: {:?}", state);
    
    // Simulate receiving 100 Trying, 180 Ringing, and 200 OK
    // ... processing responses ...
    
    // For 2xx responses to INVITE, send ACK outside the transaction
    println!("Sending ACK for 200 OK response (outside transaction)");
    let ack_result = manager.send_ack_for_2xx(&transaction_id, &ok_response).await;
    println!("Transaction manager ACK Result: {:?}", ack_result);
    
    // ... finish example ...
    
    Ok(())
}
```

### INVITE Server Transaction State Machine

```
INVITE Server Transaction (Section 17.2.1)
           |INVITE
           V
+----------+
|Proceeding|---+
+----------+   |
    |          |
    |1xx       |1xx
    |          |
    |          v
    |      +----------+
    |      |Proceeding|---+
    |      +----------+   |
    |          |          |
    |          |2xx       |2xx
    |          |          |
    v          v          |
+----------+   |          |
|Completed |<--+----------+
+----------+
    |
    | Timer H + Timer I
    V
+-----------+
|Terminated |
+-----------+
```

## Transaction Manager

The TransactionManager class provides a central interface for managing all SIP transactions. It handles transaction creation, message routing, and event distribution:

```rust
async fn demonstrate_transaction_manager() -> Result<()> {
    // Setup transport and transaction manager
    // ... similar setup as previous examples ...
    
    // Create a REGISTER request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        // ... headers ...
        .build();
    
    // Create a client transaction
    let transaction_id = manager.create_client_transaction(
        request,
        remote_addr
    ).await.unwrap();
    
    // Check active transactions
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("Active client transactions: {}", client_txs.len());
    println!("Active server transactions: {}", server_txs.len());
    
    // Initiate the transaction
    manager.send_request(&transaction_id).await.unwrap();
    
    // ... simulate response handling ...
    
    // Process timer events to ensure proper transaction termination
    println!("Waiting for transaction timeout and processing timer events...");
    process_events_for_duration(&mut events_rx, &manager, 1000).await;
    
    // Check final transaction state
    // ...
    
    Ok(())
}
```

## Event Processing and Timer Handling

A critical aspect of transaction processing is handling timer events. SIP transactions rely on timers for reliability, especially for retransmissions and cleanup. Here's a helper function to process transaction events:

```rust
async fn process_events_for_duration(
    events_rx: &mut mpsc::Receiver<TransactionEvent>,
    manager: &TransactionManager,
    duration_ms: u64
) -> (usize, Option<TransactionEvent>) {
    let start = std::time::Instant::now();
    let duration = Duration::from_millis(duration_ms);
    let mut timer_events = 0;
    let mut last_important_event = None;
    
    while start.elapsed() < duration {
        // Use tokio::time::timeout for async timeout
        match tokio::time::timeout(
            Duration::from_millis(50),  // Small timeout to check elapsed time
            events_rx.recv()
        ).await {
            Ok(Some(event)) => {
                match &event {
                    TransactionEvent::TransactionTerminated { transaction_id } => {
                        println!("Transaction terminated: {}", transaction_id);
                        last_important_event = Some(event.clone());
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        timer_events += 1;
                    },
                    _ => {
                        println!("Event received: {:?}", event);
                        last_important_event = Some(event.clone());
                    }
                }
            },
            // ... handling other cases ...
        }
    }
    
    (timer_events, last_important_event)
}
```

Failure to process timer events will prevent transactions from transitioning to the Terminated state, potentially causing resource leaks in your application.

## Complete Transaction Flow Example

To demonstrate a complete transaction lifecycle, let's examine a full transaction flow using the OPTIONS method:

```rust
async fn run_complete_transaction_example() -> Result<()> {
    // Setup transport and transaction manager
    // ... similar setup as previous examples ...
    
    // Create an OPTIONS request
    let request = RequestBuilder::new(Method::Options, "sip:bob@biloxi.example.com")?
        // ... headers ...
        .build();
    
    // Create and send the transaction
    let transaction_id = manager.create_client_transaction(
        request, 
        remote_addr
    ).await.unwrap();
    
    manager.send_request(&transaction_id).await.unwrap();
    
    // Simulate network delay while processing timer events 
    println!("Simulating network delay while processing timer events...");
    process_events_for_duration(&mut events_rx, &manager, 500).await;
    
    // Simulate server response with supported methods and extensions
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        // ... basic headers ...
        // Add Allow header listing supported methods
        .header(TypedHeader::Allow(Allow(vec![
            Method::Invite, Method::Ack, Method::Cancel, 
            Method::Options, Method::Bye
        ])))
        // Add Supported header tags
        .supported_tag("path")
        .supported_tag("gruu")
        .contact("sip:bob@biloxi.example.com", None)
        .build();
    
    // ... simulate receiving the response ...
    
    // Wait for transaction termination with timer processing
    process_events_for_duration(&mut events_rx, &manager, 1000).await;
    
    // ... verify transaction termination ...
    
    Ok(())
}
```

## Key Transaction API Methods

The `TransactionManager` offers several key methods for transaction handling:

- `create_client_transaction(request, destination)`: Creates a client transaction for any method
- `create_invite_client_transaction(request, destination)`: Creates a client transaction specifically for INVITE
- `create_non_invite_client_transaction(request, destination)`: Creates a client transaction for non-INVITE requests
- `create_server_transaction(request, source)`: Creates a server transaction for an incoming request
- `send_request(transaction_id)`: Initiates a client transaction by sending the request
- `send_response(transaction_id, response)`: Sends a response through a server transaction
- `transaction_exists(transaction_id)`: Checks if a transaction exists
- `transaction_state(transaction_id)`: Gets the current state of a transaction
- `active_transactions()`: Lists all active transactions
- `send_ack_for_2xx(transaction_id, response)`: Sends an ACK for a 2xx response to INVITE

## Best Practices for Transaction Handling

1. **Process All Timer Events**: Always process transaction timer events to ensure proper state transitions and cleanup.

2. **Handle Transaction Termination**: Monitor for `TransactionTerminated` events to clean up resources.

3. **Unique Branch Parameters**: Generate unique branch parameters for each new transaction (preferably starting with "z9hG4bK" to indicate RFC 3261 compliance).

4. **Proper ACK Handling**: Remember that 2xx responses to INVITE require ACK generation outside the transaction.

5. **Reliable Event Loop**: Implement a reliable event processing loop to handle transaction events.

6. **Error Handling**: Gracefully handle transaction errors and report them to higher layers.

7. **Transaction Timeouts**: Configure reasonable timer values based on your network environment.

8. **Contact Headers**: Always include Contact headers in responses for proper message routing.

## Conclusion

SIP transactions manage the reliable exchange of messages between SIP endpoints. By properly implementing the transaction state machines and timer handling described in RFC 3261, you can build robust SIP applications that work reliably even over unreliable transports like UDP.

In the next tutorial, we'll explore SIP dialogs, which build upon transactions to manage higher-level session state.
