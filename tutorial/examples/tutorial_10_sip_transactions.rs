// Example code for Tutorial 10: SIP Transactions
//
// This tutorial demonstrates SIP transaction state machines as defined in RFC 3261.
// SIP transactions are crucial for reliable message delivery and proper dialog management.
// The tutorial covers:
// - Client and server transaction state machines for both INVITE and non-INVITE requests
// - Transaction timers (Timer A, B, C, D, E, F, G, H, I, J, K) and their role in state transitions
// - Proper event handling required for transaction termination
// - Complete transaction flows with proper Contact header handling for routing
//
// UNDERSTANDING SIP TRANSACTIONS:
//
// 1. Purpose: SIP transactions ensure reliable message delivery and maintain proper state
//    during SIP request/response exchanges, even over unreliable transports like UDP.
//
// 2. Types of Transactions:
//    - INVITE Client Transaction: Initiates session establishment
//    - Non-INVITE Client Transaction: For registration, options, message, etc.
//    - INVITE Server Transaction: Handles incoming session establishment
//    - Non-INVITE Server Transaction: Handles other incoming requests
//
// 3. Transaction Timers:
//    - Timer A: Retransmission interval for INVITE requests (exponential backoff)
//    - Timer B: Transaction timeout for INVITE requests (typically 64*T1)
//    - Timer C: Proxy timeout for INVITE transactions
//    - Timer D: Wait time for response retransmissions, INVITE client transaction
//    - Timer E: Retransmission interval for non-INVITE requests (exponential backoff)
//    - Timer F: Transaction timeout for non-INVITE client transactions
//    - Timer G: Retransmission interval for INVITE responses (exponential backoff)
//    - Timer H: Wait time for ACK receipt
//    - Timer I: Wait time for ACK retransmissions
//    - Timer J: Wait time for request retransmissions, non-INVITE server transaction
//    - Timer K: Wait time for response retransmissions, non-INVITE client transaction
//
// 4. CRITICAL IMPLEMENTATION DETAIL: For proper transaction termination, applications MUST
//    process transaction timer events. If timer events aren't processed, transactions won't
//    transition to the Terminated state, leading to resource leaks.
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::headers::SupportedBuilderExt; // For supported headers
use rvoip_sip_core::json::ext::SipMessageJson; // Add this for header access methods
use rvoip_sip_transport::Transport; // Add this for transport trait methods
use rvoip_transaction_core::prelude::*;
use rvoip_transaction_core::TransactionManager;
use rvoip_transaction_core::transaction::{
    TransactionState,
    TransactionKey,
    TransactionKind,
    TransactionEvent
};
use rvoip_transaction_core::timer::TimerSettings;

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tokio::time::sleep;
use tokio::runtime::Runtime;
use rvoip_sip_core::error::Result;
use rvoip_sip_core::error::Error;

// Mock Transport for examples that doesn't actually send messages over the network
// Enhanced to track contacts for better routing
#[derive(Debug)]
struct MockTransport {
    local_addr: SocketAddr,
    sent_messages: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
    // Add contact mapping for better ACK routing
    contacts: Arc<Mutex<HashMap<String, (Option<Uri>, SocketAddr)>>>,
}

impl MockTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            sent_messages: Arc::new(Mutex::new(Vec::new())),
            contacts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    async fn store_contact_info(&self, message: &Message, source: SocketAddr) {
        if let Message::Response(response) = message {
            if let Some(call_id) = response.call_id() {
                let call_id_str = call_id.to_string();
                
                // Get contact URI from response
                let contact_uri = if let Some(contact_str) = response.contact_uri() {
                    // Try to parse it to Uri
                    match Uri::from_str(&contact_str) {
                        Ok(uri) => Some(uri),
                        Err(e) => {
                            println!("Failed to parse contact URI '{}': {}", contact_str, e);
                            None
                        }
                    }
                } else {
                    None
                };
                
                if let Some(uri) = &contact_uri {
                    println!("Storing routing info for {}: {:?} -> {}", call_id_str, uri, source);
                }
                
                let mut contacts = self.contacts.lock().await;
                contacts.insert(call_id_str, (contact_uri, source));
            }
        }
    }
    
    async fn get_routing_info(&self, call_id: &str) -> Option<(Option<Uri>, SocketAddr)> {
        let contacts = self.contacts.lock().await;
        contacts.get(call_id).cloned()
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::Error> {
        println!("MockTransport sending to {}: {:?}", destination, message);
        let mut messages = self.sent_messages.lock().await;
        messages.push((message, destination));
        Ok(())
    }

    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

// This is a simplified tutorial implementation that demonstrates transaction usage
// In a real application, you'd have a proper event loop and proper error handling
fn main() -> Result<()> {
    println!("Tutorial 10: SIP Transactions\n");
    
    // Create a tokio runtime for async functionality
    let rt = Runtime::new().unwrap();
    
    // Examples run in the async runtime
    rt.block_on(async {
        // Example 1: Client Transaction State Machine
        println!("Example 1: Client Transaction State Machine\n");
        demonstrate_client_transaction().await?;
        
        // Example 2: Server Transaction State Machine
        println!("\nExample 2: Server Transaction State Machine\n");
        demonstrate_server_transaction().await?;
        
        // Example 3: INVITE Transaction
        println!("\nExample 3: INVITE Transaction\n");
        demonstrate_invite_client_transaction().await?;
        
        // Example 4: Transaction Manager
        println!("\nExample 4: Transaction Manager\n");
        demonstrate_transaction_manager().await?;
        
        // Example 5: Complete Transaction Flow
        println!("\nExample 5: Complete Transaction Flow\n");
        run_complete_transaction_example().await?;
        
        Ok::<(), Error>(())
    })?;
    
    Ok(())
}

// Process transaction events for a specified duration, returning count of timer events
// and last important event received
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
            Ok(None) => {
                println!("Event channel closed");
                break;
            },
            Err(_) => {
                // Timeout occurred, continue the loop
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    
    // Check remaining time if we should wait longer
    let remaining = duration.checked_sub(start.elapsed()).unwrap_or(Duration::from_millis(0));
    if !remaining.is_zero() {
        tokio::time::sleep(remaining).await;
    }
    
    (timer_events, last_important_event)
}

// Ensure transaction is terminated, either by receiving termination event or by checking manager
async fn ensure_transaction_terminated(
    events_rx: &mut mpsc::Receiver<TransactionEvent>,
    manager: &TransactionManager,
    transaction_id: &TransactionKey,
    max_wait_ms: u64
) -> bool {
    let start = std::time::Instant::now();
    let max_duration = Duration::from_millis(max_wait_ms);
    
    // First check transaction state directly
    let pre_check = match manager.transaction_state(transaction_id).await {
        Ok(state) => {
            if state == TransactionState::Terminated {
                println!("Transaction {} already in terminated state", transaction_id);
                return true;
            }
            false
        },
        Err(_) => {
            // Error means transaction not found, which effectively means it's terminated
            println!("Transaction {} not found - already terminated", transaction_id);
            return true;
        }
    };
    
    if pre_check {
        return true;
    }
    
    // Wait for termination event or timeout
    while start.elapsed() < max_duration {
        match tokio::time::timeout(
            Duration::from_millis(50),
            events_rx.recv()
        ).await {
            Ok(Some(TransactionEvent::TransactionTerminated { transaction_id: id })) => {
                if id == *transaction_id {
                    println!("Received termination event for transaction {}", transaction_id);
                    return true;
                }
            },
            Ok(Some(_)) => {
                // Other event received, ignore
            },
            Ok(None) => {
                println!("Event channel closed while waiting for termination");
                break;
            },
            Err(_) => {
                // Timeout, check manager directly
                match manager.transaction_state(transaction_id).await {
                    Ok(state) => {
                        if state == TransactionState::Terminated {
                            println!("Transaction {} now in terminated state", transaction_id);
                            return true;
                        }
                    },
                    Err(_) => {
                        // Error means transaction not found, which is successful termination
                        println!("Transaction {} removed from manager", transaction_id);
                        return true;
                    }
                }
                
                // Continue waiting
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    
    // Final check
    match manager.transaction_state(transaction_id).await {
        Ok(state) => {
            println!("Transaction {} final state: {:?}", transaction_id, state);
            state == TransactionState::Terminated
        },
        Err(_) => {
            println!("Transaction {} successfully removed from manager", transaction_id);
            true
        }
    }
}

// Example 1: Demonstrate a basic client non-INVITE transaction
//
// Non-INVITE client transaction state machine (RFC 3261 Section 17.1.2):
// - Initial state: Trying
// - After sending request: Trying
// - After receiving 1xx response: Proceeding
// - After receiving 2xx-6xx response: Completed
// - After Timer K expires (in Completed state): Terminated
async fn demonstrate_client_transaction() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Create a transaction manager with faster timer settings for the example
        let timer_settings = TimerSettings {
        t1: Duration::from_millis(500),      // Base retransmission interval (default 500ms)
        t2: Duration::from_millis(4000),     // Maximum retransmission interval (default 4s)
        t4: Duration::from_millis(2500),     // Maximum duration for messages to remain in network
        timer_100_interval: Duration::from_millis(200), // Automatic 100 Trying response timeout
        // Use shorter timer values for the example
        transaction_timeout: Duration::from_millis(5000),  
        wait_time_j: Duration::from_millis(500),  // Shorter Timer J
        wait_time_k: Duration::from_millis(500),  // Shorter Timer K
        wait_time_h: Duration::from_millis(500),  // Shorter Timer H
        wait_time_i: Duration::from_millis(500),  // Shorter Timer I
        wait_time_d: Duration::from_millis(500),  // Shorter Timer D
    };

    let (manager, mut events_rx) = TransactionManager::new_with_config(
        transport.clone(),
        transport_rx,
        Some(100),
        Some(timer_settings),
    ).await.unwrap();

    // Create a client transaction for a non-INVITE request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
    
    // Create and start the transaction through the manager
    println!("Creating transaction for REGISTER");
    let transaction_id = manager.create_client_transaction(
        request,
        remote_addr
    ).await.unwrap();
    
    // Send the request - in transaction-core, this initiates the transaction
    println!("Initiating transaction send");
    manager.send_request(&transaction_id).await.unwrap();
    
    // Check current state - should be in Trying state
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Initial state: {:?}", state);
    
    // Simulate receiving a provisional response
    println!("Creating 100 Trying response");
    let provisional_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    // Manually inject the response through transport_tx
    println!("Simulating receipt of 100 Trying");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(provisional_response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check state again - should be in Proceeding
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 100 Trying: {:?}", state);
    
    // Simulate receiving a final response
    println!("Creating 200 OK response");
    let final_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:alice@atlanta.example.com", None) // Add contact for better routing
        .build();
    
    // Store contact information in the mock transport
    transport.store_contact_info(&Message::Response(final_response.clone()), remote_addr).await;
    
    // Manually inject the response
    println!("Simulating receipt of 200 OK");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(final_response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check state again - should be Completed
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 200 OK: {:?}", state);
    
    // Wait for transition to Terminated state
    // IMPORTANT: Timer K transitions from Completed to Terminated state
    // For UDP, RFC 3261 sets it to 64*T1 (64*500ms = 32s)
    // In this example we use a shorter timeout for demonstration purposes
    println!("Waiting for transaction termination (Timer K)...");
    
    // Keep processing events while waiting for termination
    let (timer_events, _) = process_events_for_duration(&mut events_rx, &manager, 1000).await;
    println!("Processed {} timer events during Timer K period", timer_events);
    
    // Ensure transaction is fully terminated - this is important!
    let terminated = ensure_transaction_terminated(&mut events_rx, &manager, &transaction_id, 500).await;
    
    // Check before/after state
    let (before_client_txs, before_server_txs) = manager.active_transactions().await;
    println!("After termination check: {} client transactions, {} server transactions", 
             before_client_txs.len(), before_server_txs.len());
    
    // Check state - should eventually be Terminated
    let state = match manager.transaction_state(&transaction_id).await {
        Ok(s) => s,
        Err(_) => {
            println!("Transaction no longer found (correctly terminated)");
            TransactionState::Terminated
        }
    };
    println!("Final state: {:?}", state);
    println!("Transaction properly terminated: {}", terminated);
    
    Ok(())
}

// Example 2: Demonstrate a server transaction state machine
//
// Non-INVITE server transaction state machine (RFC 3261 Section 17.2.2):
// - Initial state: Trying
// - After receiving request: Trying
// - After sending 1xx response: Proceeding
// - After sending 2xx-6xx response: Completed
// - After Timer J expires (in Completed state): Terminated
async fn demonstrate_server_transaction() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Create a transaction manager with faster timer settings for the example
        let timer_settings = TimerSettings {
        t1: Duration::from_millis(500),      // Base retransmission interval (default 500ms)
        t2: Duration::from_millis(4000),     // Maximum retransmission interval (default 4s)
        t4: Duration::from_millis(2500),     // Maximum duration for messages to remain in network
        timer_100_interval: Duration::from_millis(200), // Automatic 100 Trying response timeout
        // Use shorter timer values for the example
        transaction_timeout: Duration::from_millis(5000),  
        wait_time_j: Duration::from_millis(500),  // Shorter Timer J
        wait_time_k: Duration::from_millis(500),  // Shorter Timer K
        wait_time_h: Duration::from_millis(500),  // Shorter Timer H
        wait_time_i: Duration::from_millis(500),  // Shorter Timer I
        wait_time_d: Duration::from_millis(500),  // Shorter Timer D
    };

    let (manager, mut events_rx) = TransactionManager::new_with_config(
        transport.clone(),
        transport_rx,
        Some(100),
        Some(timer_settings),
    ).await.unwrap();

    // Prepare a request to be "received"
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
    
    // Directly create a server transaction with the request
    println!("Creating server transaction");
    let server_tx = manager.create_server_transaction(
        request.clone(), 
        remote_addr
    ).await.expect("Failed to create server transaction");
    
    // Get the transaction ID
    let transaction_id = server_tx.id().clone();
    println!("Server transaction created with ID: {}", transaction_id);
    
    // Check initial state
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Initial state: {:?}", state);
    
    // Send a provisional response
    println!("Sending 100 Trying response");
    let provisional_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    // Send response through the transaction manager
    manager.send_response(&transaction_id, provisional_response).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check state again - should be Proceeding
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After sending 100 Trying: {:?}", state);
    
    // Send a final response
    println!("Sending 200 OK response");
    let final_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:bob@biloxi.example.com", None) // Add contact for better routing
        .build();
    
    // Send through manager
    manager.send_response(&transaction_id, final_response.clone()).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check state again - should be Completed
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After sending 200 OK: {:?}", state);
    
    // Simulate receiving a retransmission of the original request
    println!("Simulating retransmission of original request");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Request(request.clone()),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Transaction should still be in Completed and manager should have retransmitted the response
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After request retransmission: {:?}", state);
    
    // Wait for transaction termination
    // IMPORTANT: Timer J transitions from Completed to Terminated state
    // For UDP, RFC 3261 sets it to 64*T1 (64*500ms = 32s)
    // For TCP/SCTP, Timer J is 0 seconds (immediate termination)
    println!("Waiting for transaction termination (Timer J)...");
    
    // Keep processing events while waiting for termination
    let (timer_events, _) = process_events_for_duration(&mut events_rx, &manager, 1000).await;
    println!("Processed {} timer events during Timer J period", timer_events);
    
    // Ensure transaction is fully terminated - this is important!
    let terminated = ensure_transaction_terminated(&mut events_rx, &manager, &transaction_id, 500).await;
    
    // Check if transaction has been terminated
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("After termination check: {} client transactions, {} server transactions", 
             client_txs.len(), server_txs.len());
    
    // Check final state if transaction is still available
    match manager.transaction_state(&transaction_id).await {
        Ok(state) => println!("Final transaction state: {:?}", state),
        Err(_) => println!("Transaction has been terminated and removed from manager")
    }
    
    println!("Transaction properly terminated: {}", terminated);
    
    Ok(())
}

// Example 3: Demonstrate an INVITE client transaction
//
// INVITE client transaction state machine (RFC 3261 Section 17.1.1):
// - Initial state: Calling
// - After sending INVITE: Calling
// - After receiving 1xx response: Proceeding
// - After receiving 2xx response: Terminated (handled by TU, not transaction)
// - After receiving 3xx-6xx response: Completed
// - After sending ACK for 3xx-6xx: Completed
// - After Timer D expires (in Completed state): Terminated
async fn demonstrate_invite_client_transaction() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Create a transaction manager with faster timer settings for the example
        let timer_settings = TimerSettings {
        t1: Duration::from_millis(500),      // Base retransmission interval (default 500ms)
        t2: Duration::from_millis(4000),     // Maximum retransmission interval (default 4s)
        t4: Duration::from_millis(2500),     // Maximum duration for messages to remain in network
        timer_100_interval: Duration::from_millis(200), // Automatic 100 Trying response timeout
        // Use shorter timer values for the example
        transaction_timeout: Duration::from_millis(5000),  
        wait_time_j: Duration::from_millis(500),  // Shorter Timer J
        wait_time_k: Duration::from_millis(500),  // Shorter Timer K
        wait_time_h: Duration::from_millis(500),  // Shorter Timer H
        wait_time_i: Duration::from_millis(500),  // Shorter Timer I
        wait_time_d: Duration::from_millis(500),  // Shorter Timer D
    };

    let (manager, mut events_rx) = TransactionManager::new_with_config(
        transport.clone(),
        transport_rx,
        Some(100),
        Some(timer_settings),
    ).await.unwrap();

    // Create an INVITE request
    let request = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        .build();
    
    // Create and start transaction
    println!("Creating transaction for INVITE");
    let transaction_id = manager.create_invite_client_transaction(
        request, 
        remote_addr
    ).await.unwrap();
    
    // Send the request
    println!("Initiating INVITE send");
    manager.send_request(&transaction_id).await.unwrap();
    
    // Check current state - should be in Calling state
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Initial state: {:?}", state);
    
    // Simulate receiving a 100 Trying response
    println!("Creating 100 Trying response");
    let trying_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Invite)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .build();
    
    // Inject the response
    println!("Simulating receipt of 100 Trying");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(trying_response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 50).await;
    
    // Check state - should be Proceeding
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 100 Trying: {:?}", state);
    
    // Simulate receiving a 180 Ringing response
    println!("Creating 180 Ringing response");
    let ringing_response = ResponseBuilder::new(StatusCode::Ringing, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Invite)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .build();
    
    // Inject the response
    println!("Simulating receipt of 180 Ringing");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(ringing_response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 50).await;
    
    // Check state - should still be Proceeding
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 180 Ringing: {:?}", state);
    
    // Simulate receiving a 200 OK final response
    println!("Creating 200 OK response");
    let ok_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Invite)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .contact("sip:bob@biloxi.example.com", None)
        .build();
    
    // Store contact information for ACK routing
    transport.store_contact_info(&Message::Response(ok_response.clone()), remote_addr).await;
    
    // Inject the response
    println!("Simulating receipt of 200 OK");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(ok_response.clone()),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 50).await;
    
    // Check state - should be Terminated for 2xx responses to INVITE
    // IMPORTANT: For 2xx responses to INVITE, the transaction moves directly to Terminated
    // This is because the ACK for 2xx is sent by TU directly as a separate transaction
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 200 OK: {:?}", state);
    
    // For INVITE, ACK to 2xx is sent by transaction user directly, outside the transaction
    println!("Sending ACK for 200 OK response (outside transaction)");
    
    // Send ACK using the transaction manager's utility method
    let ack_result = manager.send_ack_for_2xx(&transaction_id, &ok_response).await;
    println!("Transaction manager ACK Result: {:?}", ack_result);
    
    // Process timer events that might occur after ACK
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check final transaction state
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("After ACK: {} client transactions, {} server transactions", 
             client_txs.len(), server_txs.len());
    
    Ok(())
}

// Example 4: Demonstrate a transaction manager
async fn demonstrate_transaction_manager() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Create a transaction manager with faster timer settings for the example
        let timer_settings = TimerSettings {
        t1: Duration::from_millis(500),      // Base retransmission interval (default 500ms)
        t2: Duration::from_millis(4000),     // Maximum retransmission interval (default 4s)
        t4: Duration::from_millis(2500),     // Maximum duration for messages to remain in network
        timer_100_interval: Duration::from_millis(200), // Automatic 100 Trying response timeout
        // Use shorter timer values for the example
        transaction_timeout: Duration::from_millis(5000),  
        wait_time_j: Duration::from_millis(500),  // Shorter Timer J
        wait_time_k: Duration::from_millis(500),  // Shorter Timer K
        wait_time_h: Duration::from_millis(500),  // Shorter Timer H
        wait_time_i: Duration::from_millis(500),  // Shorter Timer I
        wait_time_d: Duration::from_millis(500),  // Shorter Timer D
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
    println!("Creating transaction for REGISTER");
    let transaction_id = manager.create_client_transaction(
        request,
        remote_addr
    ).await.unwrap();
    
    // Check active transactions
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("Added client transaction for REGISTER. Active client transactions: {}", client_txs.len());
    println!("Active server transactions: {}", server_txs.len());
    
    // Initiate the transaction
    println!("Sending request through transaction");
    manager.send_request(&transaction_id).await.unwrap();
    
    // Simulate receiving a response and have the transaction handle it
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:bob@biloxi.example.com", None) // Add contact
        .build();
    
    // Store contact info for possible later use
    transport.store_contact_info(&Message::Response(response.clone()), remote_addr).await;
    
    // Inject the response through transport
    println!("Simulating receipt of 200 OK response");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly
    process_events_for_duration(&mut events_rx, &manager, 50).await;
    
    // Check the transaction state
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Transaction state after 200 OK: {:?}", state);
    
    // Allow time for transaction termination with timer processing
    println!("Waiting for transaction timeout and processing timer events...");
    let (timer_events, _) = process_events_for_duration(&mut events_rx, &manager, 1000).await;
    println!("Processed {} timer events during timeout period", timer_events);
    
    // Check active transactions again after timer processing
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("After transaction processing: {} client transactions, {} server transactions", 
             client_txs.len(), server_txs.len());
    
    // If transactions still exist, explain why
    if !client_txs.is_empty() {
        println!("Transactions still active after timeout. This might be due to:");
        println!("1. Longer timeout values in the transaction-core library");
        println!("2. Transactions awaiting explicit termination calls");
        println!("3. The transaction-core library might preserve transaction records for reporting");
    }
    
    Ok(())
}

// Example 5: Complete transaction flow with timers and network simulation
//
// This example demonstrates a complete OPTIONS transaction flow from start to termination,
// showing proper handling of timer events throughout the transaction lifecycle.
async fn run_complete_transaction_example() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Create a transaction manager with faster timer settings for the example
    let timer_settings = TimerSettings {
        t1: Duration::from_millis(500),      // Base retransmission interval (default 500ms)
        t2: Duration::from_millis(4000),     // Maximum retransmission interval (default 4s)
        t4: Duration::from_millis(2500),     // Maximum duration for messages to remain in network
        timer_100_interval: Duration::from_millis(200), // Automatic 100 Trying response timeout
        // Use shorter timer values for the example
        transaction_timeout: Duration::from_millis(5000),  
        wait_time_j: Duration::from_millis(500),  // Shorter Timer J
        wait_time_k: Duration::from_millis(500),  // Shorter Timer K
        wait_time_h: Duration::from_millis(500),  // Shorter Timer H
        wait_time_i: Duration::from_millis(500),  // Shorter Timer I
        wait_time_d: Duration::from_millis(500),  // Shorter Timer D
    };
    
    let (manager, mut events_rx) = TransactionManager::new_with_config(
        transport.clone(),
        transport_rx,
        Some(100), 
        Some(timer_settings),
    ).await.unwrap();
    
    // Create an OPTIONS request
    let request = RequestBuilder::new(Method::Options, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("options-1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776opthds"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        .build();
    
    // In a real application, you would send this message over the network
    println!("Creating OPTIONS transaction");
    let transaction_id = manager.create_client_transaction(
        request, 
        remote_addr
    ).await.unwrap();
    
    // Send the request
    println!("Sending OPTIONS request");
    manager.send_request(&transaction_id).await.unwrap();
    
    // Check active transactions
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("Active client transactions: {}", client_txs.len());
    println!("Active server transactions: {}", server_txs.len());
    
    // Simulate network delay while processing timer events 
    println!("Simulating network delay while processing timer events...");
    let (timer_events, _) = process_events_for_duration(&mut events_rx, &manager, 500).await;
    println!("Processed {} timer events during network delay", timer_events);
    
    // Simulate a 200 OK response from the server
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("options-1234567890@atlanta.example.com")
        .cseq(1, Method::Options)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776opthds"))
        // Add Allow header listing supported methods
        .header(TypedHeader::Allow(Allow(vec![Method::Invite, Method::Ack, Method::Cancel, Method::Options, Method::Bye])))
        // Add Supported header one tag at a time
        .supported_tag("path")
        .supported_tag("gruu")
        .contact("sip:bob@biloxi.example.com", None) // Add contact
        .build();
    
    // Store contact info
    transport.store_contact_info(&Message::Response(response.clone()), remote_addr).await;
    
    // Inject the response
    println!("Simulating receipt of 200 OK response");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Process events briefly to handle the response
    process_events_for_duration(&mut events_rx, &manager, 100).await;
    
    // Check transaction state after receiving 200 OK
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("Transaction state after 200 OK: {:?}", state);
    
    // Simulate the passage of time for transaction cleanup
    println!("Waiting for transaction timeout with timer processing...");
    let (timer_events, _) = process_events_for_duration(&mut events_rx, &manager, 1000).await;
    println!("Processed {} timer events during timeout period", timer_events);
    
    // Explicitly ensure the transaction terminates properly
    let terminated = ensure_transaction_terminated(&mut events_rx, &manager, &transaction_id, 500).await;
    println!("Transaction properly terminated: {}", terminated);
    
    // Check active transactions after waiting for timeout
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("After termination check: {} client transactions, {} server transactions", 
             client_txs.len(), server_txs.len());
    
    // If no more active transactions, we've successfully demonstrated complete transaction lifecycle
    if client_txs.is_empty() && server_txs.is_empty() {
        println!("Success: All transactions have properly terminated");
    } else {
        println!("Note: Some transactions still active. In production code you need to:");
        println!("      1. Process ALL timer events to ensure proper termination");
        println!("      2. Consider using explicit transaction cleanup for long-lived applications");
        println!("      3. For this example, the transaction may still be in manager's internal storage");
    }
    
    println!("\nAll examples completed successfully!");

    println!("\nKEY LESSONS FROM THIS TUTORIAL:");
    println!("-------------------------------");
    println!("1. SIP transactions follow specific state machines defined in RFC 3261");
    println!("2. Transaction termination requires proper timer event processing");
    println!("3. INVITE and non-INVITE transactions have different state machines");
    println!("4. Client and server transactions handle retransmissions differently");
    println!("5. ACK for 2xx responses is handled outside the INVITE transaction");
    println!("6. Contact headers are critical for proper message routing");
    println!("7. In real-world applications, implement a dedicated event loop to process transaction events");
    println!("8. Transactions automatically handle retransmissions providing reliability over unreliable transports");
    
    Ok(())
} 