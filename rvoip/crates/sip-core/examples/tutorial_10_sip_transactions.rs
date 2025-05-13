// Example code for Tutorial 10: SIP Transactions
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::transaction::{
    ClientTransaction, ServerTransaction, TransactionManager, TransactionState
};
use std::time::{Duration, Instant};
use std::thread::sleep;

fn main() -> Result<()> {
    println!("Tutorial 10: SIP Transactions\n");
    
    // Example 1: Client Transaction State Machine
    println!("Example 1: Client Transaction State Machine\n");
    demonstrate_client_transaction()?;
    
    // Example 2: Server Transaction State Machine
    println!("\nExample 2: Server Transaction State Machine\n");
    demonstrate_server_transaction()?;
    
    // Example 3: INVITE Transaction
    println!("\nExample 3: INVITE Transaction\n");
    demonstrate_invite_client_transaction()?;
    
    // Example 4: Transaction Manager
    println!("\nExample 4: Transaction Manager\n");
    demonstrate_transaction_manager()?;
    
    // Example 5: Complete Transaction Flow
    println!("\nExample 5: Complete Transaction Flow\n");
    run_complete_transaction_example()?;
    
    Ok(())
}

// Example 1: Demonstrate a basic client transaction state machine
fn demonstrate_client_transaction() -> Result<()> {
    // Create a client transaction for a non-INVITE request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
    
    let message = Message::Request(request);
    let branch = "z9hG4bK776asdhds";  // Branch parameter from Via header
    
    // Create a client transaction
    let mut transaction = ClientTransaction::new(
        branch.to_string(),
        message.clone(),
        TransactionState::Trying,
        Instant::now(),
    );
    
    println!("Initial state: {:?}", transaction.state());
    
    // Simulate receiving a provisional response
    let provisional_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    transaction.handle_response(&Message::Response(provisional_response));
    println!("After 100 Trying: {:?}", transaction.state());
    
    // Simulate receiving a final response
    let final_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    transaction.handle_response(&Message::Response(final_response));
    println!("After 200 OK: {:?}", transaction.state());
    
    // Transactions should be terminated after a final response
    if transaction.is_completed() {
        println!("Transaction is completed, will terminate after timeout");
    }
    
    Ok(())
}

// Example 2: Demonstrate a basic server transaction state machine
fn demonstrate_server_transaction() -> Result<()> {
    // Simulate receiving a request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
    
    let message = Message::Request(request);
    let branch = "z9hG4bK776asdhds";  // Branch parameter from Via header
    
    // Create a server transaction
    let mut transaction = ServerTransaction::new(
        branch.to_string(),
        message.clone(),
        TransactionState::Trying,
        Instant::now(),
    );
    
    println!("Initial state: {:?}", transaction.state());
    
    // Send a provisional response
    let provisional_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    transaction.send_response(&Message::Response(provisional_response));
    println!("After sending 100 Trying: {:?}", transaction.state());
    
    // Send a final response
    let final_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    transaction.send_response(&Message::Response(final_response));
    println!("After sending 200 OK: {:?}", transaction.state());
    
    // Simulate receiving a retransmission of the original request
    println!("Receiving retransmission of original request...");
    let retransmitted_response = transaction.handle_request(&message);
    
    if let Some(response) = retransmitted_response {
        println!("Retransmitting the last response: {}", response);
    } else {
        println!("No response to retransmit");
    }
    
    Ok(())
}

// Example 3: Demonstrate an INVITE client transaction
fn demonstrate_invite_client_transaction() -> Result<()> {
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
    
    let message = Message::Request(request);
    let branch = "z9hG4bK776abdhds";  // Branch parameter from Via header
    
    // Create an INVITE client transaction
    let mut transaction = ClientTransaction::new(
        branch.to_string(),
        message.clone(),
        TransactionState::Calling,  // Initial state for INVITE is Calling
        Instant::now(),
    );
    
    println!("Initial state: {:?}", transaction.state());
    
    // Simulate receiving a 100 Trying response
    let trying_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Invite)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .build();
    
    transaction.handle_response(&Message::Response(trying_response));
    println!("After 100 Trying: {:?}", transaction.state());
    
    // Simulate receiving a 180 Ringing response
    let ringing_response = ResponseBuilder::new(StatusCode::Ringing, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Invite)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .build();
    
    transaction.handle_response(&Message::Response(ringing_response));
    println!("After 180 Ringing: {:?}", transaction.state());
    
    // Simulate receiving a 200 OK final response
    let ok_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Invite)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776abdhds"))
        .contact("sip:bob@biloxi.example.com", None)
        .build();
    
    transaction.handle_response(&Message::Response(ok_response));
    println!("After 200 OK: {:?}", transaction.state());
    
    // For INVITE, ACK is sent by the transaction user (outside the transaction)
    let ack = RequestBuilder::new(Method::Ack, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)  // Same CSeq as the INVITE
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776ccdhds"))  // New branch for ACK
        .max_forwards(70)
        .build();
    
    println!("ACK sent outside of transaction: {}", Message::Request(ack));
    
    Ok(())
}

// Example 4: Demonstrate a transaction manager
fn demonstrate_transaction_manager() -> Result<()> {
    // Create a transaction manager
    let mut manager = TransactionManager::new();
    
    // Create a REGISTER request
    let request = RequestBuilder::new(Method::Register, "sip:example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
    
    let register_message = Message::Request(request);
    
    // Create a client transaction and add it to the manager
    let branch = "z9hG4bK776asdhds";
    manager.add_client_transaction(
        branch.to_string(),
        register_message.clone(),
        TransactionState::Trying
    );
    
    println!("Added client transaction for REGISTER. Active transactions: {}", 
             manager.active_transactions());
    
    // Simulate receiving a response and find matching client transaction
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("1234567890@atlanta.example.com")
        .cseq(1, Method::Register)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    let response_message = Message::Response(response);
    
    // Process the response in the manager
    if let Some(transaction) = manager.find_transaction_for_response(&response_message) {
        println!("Found matching transaction with branch: {}", transaction.branch());
        manager.handle_response(&response_message);
    } else {
        println!("No matching transaction found for response");
    }
    
    // Clean up completed transactions
    manager.clean_completed_transactions();
    println!("After cleanup. Active transactions: {}", manager.active_transactions());
    
    Ok(())
}

// Example 5: Complete transaction flow with timers and network simulation
fn run_complete_transaction_example() -> Result<()> {
    // Create a transaction manager
    let mut manager = TransactionManager::new();
    
    // Create an OPTIONS request (lightweight way to test connectivity)
    let request = RequestBuilder::new(Method::Options, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("options-1234567890@atlanta.example.com")
        .cseq(1)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776opthds"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        .build();
    
    let options_message = Message::Request(request);
    
    // In a real application, you would send this message over the network
    println!("Sending OPTIONS request: {}", options_message);
    
    // Add the transaction to the manager
    let branch = "z9hG4bK776opthds";
    manager.add_client_transaction(
        branch.to_string(),
        options_message.clone(),
        TransactionState::Trying
    );
    
    println!("Active transactions: {}", manager.active_transactions());
    
    // Simulate receiving a response from the network
    println!("Simulating network delay...");
    sleep(Duration::from_millis(500));
    
    // Process timers while waiting (would be done in a real event loop)
    manager.process_timers();
    
    // Simulate a 200 OK response from the server
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("b73kszlfl"))
        .call_id("options-1234567890@atlanta.example.com")
        .cseq(1, Method::Options)
        .via("atlanta.example.com", "UDP", Some("z9hG4bK776opthds"))
        .allow(&[Method::Invite, Method::Ack, Method::Cancel, Method::Options, Method::Bye])
        .supported(&["path", "gruu"])
        .build();
    
    let response_message = Message::Response(response);
    println!("Received 200 OK response: {}", response_message);
    
    // Handle the response in the transaction manager
    if let Some(transaction) = manager.find_transaction_for_response(&response_message) {
        println!("Found matching transaction with branch: {}", transaction.branch());
        manager.handle_response(&response_message);
    } else {
        println!("No matching transaction found for response");
    }
    
    // Simulate the passage of time for transaction cleanup
    println!("Waiting for transaction timeout...");
    sleep(Duration::from_secs(1));
    
    // Process timers again
    manager.process_timers();
    
    // Clean up completed transactions
    manager.clean_completed_transactions();
    println!("Active transactions after cleanup: {}", manager.active_transactions());
    
    println!("\nAll examples completed successfully!");
    
    Ok(())
} 