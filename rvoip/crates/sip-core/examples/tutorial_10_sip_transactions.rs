// Example code for Tutorial 10: SIP Transactions
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::headers::SupportedBuilderExt; // For supported headers
use rvoip_sip_core::json::ext::SipMessageJson; // Add this for header access methods
use rvoip_sip_transport::Transport; // Add this for transport trait methods
use rvoip_transaction_core::{
    TransactionManager,
    transaction::{
        TransactionState,
        client::{ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction},
        server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction}
    }
};
use std::time::{Duration, Instant};
use std::thread::sleep;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::str::FromStr;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::runtime::Runtime;

// Mock Transport for examples that doesn't actually send messages over the network
// Enhanced to track contacts for better routing
#[derive(Debug)]
struct MockTransport {
    local_addr: SocketAddr,
    sent_messages: Vec<(Message, SocketAddr)>,
    // Add contact mapping for better ACK routing
    contacts: Mutex<HashMap<String, (Option<Uri>, SocketAddr)>>,
}

impl MockTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            sent_messages: Vec::new(),
            contacts: Mutex::new(HashMap::new()),
        }
    }
    
    // Store contact information from responses to use for ACK routing
    fn store_contact_info(&self, message: &Message, source: SocketAddr) {
        if let Message::Response(response) = message {
            if let Some(call_id) = response.call_id() {
                // Extract first contact URI from the response
                let contact_uri = if let Some(contact) = response.headers.iter().find_map(|h| {
                    if let TypedHeader::Contact(contact) = h {
                        Some(contact)
                    } else {
                        None
                    }
                }) {
                    // Get the first address from the contact if it exists
                    contact.address().map(|addr| addr.uri.clone())
                } else {
                    None
                };
                
                let mut contacts = self.contacts.lock().unwrap();
                contacts.insert(call_id.to_string(), (contact_uri, source));
            }
        }
    }
    
    // Get routing information for a call ID
    fn get_routing_info(&self, call_id: &str) -> Option<(Option<Uri>, SocketAddr)> {
        let contacts = self.contacts.lock().unwrap();
        contacts.get(call_id).cloned()
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::Error> {
        println!("Sent message to {}: {}", destination, message);
        
        // Store contact information if this is a response
        if let Message::Response(_) = &message {
            // We wouldn't store here in real life, but this helps for simulation
        }
        
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

// Example 1: Demonstrate a basic client non-INVITE transaction
async fn demonstrate_client_transaction() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    let (manager, mut events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(100)
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
    let transaction_id = manager.create_client_transaction(request, remote_addr).await.unwrap();
    
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
    
    // Brief pause to allow processing
    sleep(Duration::from_millis(50));
    
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
    transport.store_contact_info(&Message::Response(final_response.clone()), remote_addr);
    
    // Manually inject the response
    println!("Simulating receipt of 200 OK");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(final_response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Brief pause to allow processing
    sleep(Duration::from_millis(50));
    
    // Check state again - should be Completed
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 200 OK: {:?}", state);
    
    // Wait for transition to Terminated state
    // Note: In production, Timer K would be 5 seconds per RFC 3261
    // We use a longer delay here to ensure the transaction has time to terminate
    println!("Waiting for transaction termination (Timer K)...");
    sleep(Duration::from_secs(3)); // Increased from 200ms to give time for termination
    
    // Explicitly request transaction cleanup
    println!("Cleaning up completed transactions");
    let (before_client_txs, before_server_txs) = manager.active_transactions().await;
    println!("Before cleanup: {} client transactions, {} server transactions", 
             before_client_txs.len(), before_server_txs.len());
    
    // Check state - should eventually be Terminated
    let state = match manager.transaction_state(&transaction_id).await {
        Ok(s) => s,
        Err(_) => {
            println!("Transaction no longer found (likely terminated)");
            TransactionState::Terminated
        }
    };
    println!("Final state: {:?}", state);
    
    // Check transactions after attempted cleanup
    let (after_client_txs, after_server_txs) = manager.active_transactions().await;
    println!("After cleanup: {} client transactions, {} server transactions", 
             after_client_txs.len(), after_server_txs.len());
    
    Ok(())
}

// Example 2: Demonstrate a server transaction state machine
async fn demonstrate_server_transaction() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    let (manager, mut events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(100)
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
    
    // Simulate receiving this request
    println!("Simulating receipt of REGISTER request");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Request(request.clone()),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Brief pause to allow processing
    sleep(Duration::from_millis(50));
    
    // Check for NewRequest event
    let event = events_rx.try_recv().ok();
    println!("Received event: {:?}", event);
    
    // Extract transaction ID from event
    let transaction_id = match &event {
        Some(rvoip_transaction_core::transaction::TransactionEvent::NewRequest { transaction_id, .. }) => {
            transaction_id.clone()
        }
        _ => {
            println!("Did not receive expected NewRequest event");
            // Fallback ID for demo purposes
            "unknown".to_string()
        }
    };
    
    // Check current state
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
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
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
    manager.send_response(&transaction_id, final_response).await.unwrap();
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
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
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
    // Transaction should still be in Completed and manager should have retransmitted the response
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After request retransmission: {:?}", state);
    
    // Wait for transaction termination
    // Note: In production, Timer J would be 32 seconds per RFC 3261
    println!("Waiting for transaction termination (Timer J)...");
    sleep(Duration::from_secs(3)); // Increased from 200ms to give time for termination
    
    // Check if transaction has been terminated
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("After waiting for termination: {} client transactions, {} server transactions", 
             client_txs.len(), server_txs.len());
    
    Ok(())
}

// Example 3: Demonstrate an INVITE client transaction
async fn demonstrate_invite_client_transaction() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    let (manager, mut events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(100)
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
    let transaction_id = manager.create_client_transaction(request, remote_addr).await.unwrap();
    
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
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
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
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
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
    transport.store_contact_info(&Message::Response(ok_response.clone()), remote_addr);
    
    // Inject the response
    println!("Simulating receipt of 200 OK");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(ok_response.clone()),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
    // Check state - should be Terminated for 2xx responses to INVITE
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    println!("After 200 OK: {:?}", state);
    
    // For INVITE, ACK to 2xx is sent by transaction user directly, outside the transaction
    println!("Sending ACK for 200 OK response (outside transaction)");
    
    // Get call ID to look up routing info
    let call_id = ok_response.call_id()
                             .map(|cid| cid.to_string())
                             .unwrap_or_else(|| "unknown".to_string());
    
    // Build and send a manual ACK with proper routing
    if let Some((contact_uri, dest_addr)) = transport.get_routing_info(&call_id) {
        println!("Found contact info for ACK routing: {:?} -> {}", contact_uri, dest_addr);
        
        // Create an ACK request directly
        let ack_request = RequestBuilder::new(Method::Ack, "sip:bob@biloxi.example.com")?
            .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
            .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
            .call_id(&call_id)
            .cseq(1)
            .via("atlanta.example.com", "UDP", Some("z9hG4bKnewbranch"))
            .max_forwards(70)
            .build();
            
        // Send the ACK directly through transport
        transport.send_message(Message::Request(ack_request), dest_addr).await.unwrap();
        println!("ACK sent successfully");
    } else {
        // Try using the transaction manager's method
        let ack_result = manager.send_2xx_ack(&ok_response).await;
        println!("Transaction manager ACK Result: {:?}", ack_result);
    }
    
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
    let (manager, mut events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(100)
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
    let transaction_id = manager.create_client_transaction(request, remote_addr).await.unwrap();
    
    // Check active transactions
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("Added client transaction for REGISTER. Active client transactions: {}", client_txs.len());
    println!("Active server transactions: {}", server_txs.len());
    
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
    transport.store_contact_info(&Message::Response(response.clone()), remote_addr);
    
    // Inject the response through transport
    println!("Simulating receipt of 200 OK response");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
    // Check events
    let evt = events_rx.try_recv().ok();
    println!("Event received: {:?}", evt);
    
    // Allow time for transaction termination (longer than before)
    println!("Waiting for transaction timeout...");
    sleep(Duration::from_secs(3)); // Increased from 200ms
    
    // Check active transactions again
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("After waiting for completion. Active client transactions: {}", client_txs.len());
    println!("Active server transactions: {}", server_txs.len());
    
    // If transactions still exist, try to determine why
    if !client_txs.is_empty() {
        println!("Transactions still active after timeout. This might be due to:");
        println!("1. Longer timeout values in the transaction-core library");
        println!("2. Transactions awaiting explicit termination calls");
        println!("3. The transaction-core library might preserve transaction records for reporting");
    }
    
    Ok(())
}

// Example 5: Complete transaction flow with timers and network simulation
async fn run_complete_transaction_example() -> Result<()> {
    // Create transport and transaction manager setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
    
    // Create a transport
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup channels for transaction events and transport events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    let (manager, mut events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(100)
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
    let transaction_id = manager.create_client_transaction(request, remote_addr).await.unwrap();
    
    // Send the request
    println!("Sending OPTIONS request");
    manager.send_request(&transaction_id).await.unwrap();
    
    // Check active transactions
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("Active client transactions: {}", client_txs.len());
    println!("Active server transactions: {}", server_txs.len());
    
    // Simulate network delay
    println!("Simulating network delay...");
    sleep(Duration::from_millis(500));
    
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
    transport.store_contact_info(&Message::Response(response.clone()), remote_addr);
    
    // Inject the response
    println!("Simulating receipt of 200 OK response");
    transport_tx.send(rvoip_sip_transport::TransportEvent::MessageReceived {
        message: Message::Response(response),
        source: remote_addr,
        destination: local_addr
    }).await.unwrap();
    
    // Brief pause
    sleep(Duration::from_millis(50));
    
    // Check events
    let evt = events_rx.try_recv().ok();
    println!("Event received: {:?}", evt);
    
    // Simulate the passage of time for transaction cleanup
    println!("Waiting for transaction timeout...");
    sleep(Duration::from_secs(3)); // Increased from 1 second
    
    // Check active transactions after waiting for timeout
    let (client_txs, server_txs) = manager.active_transactions().await;
    println!("Active client transactions after timeout: {}", client_txs.len());
    println!("Active server transactions after timeout: {}", server_txs.len());
    
    if !client_txs.is_empty() {
        println!("Note: In a complete implementation with a proper event loop,");
        println!("      we would continually check for and process timer events");
        println!("      which would help ensure proper transaction termination.");
    }
    
    println!("\nAll examples completed successfully!");
    
    Ok(())
} 