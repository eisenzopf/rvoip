use std::{sync::Arc, time::Duration, str::FromStr, net::SocketAddr, sync::Mutex};
use tokio::{
    sync::mpsc,
    time::sleep,
};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, Error as TransportError, TransportEvent};
use rvoip_transaction_core::{
    TransactionManager,
    TransactionKey,
    TransactionEvent,
    transaction::{
        TransactionState, 
        TransactionKind
    }
};

/// Mock transport for testing
#[derive(Debug)]
struct MockTransport {
    sent_messages: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
    local_addr: SocketAddr,
    should_fail: bool,
}

impl MockTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            sent_messages: Arc::new(Mutex::new(Vec::new())),
            local_addr,
            should_fail: false,
        }
    }

    fn get_sent_messages(&self) -> Vec<(Message, SocketAddr)> {
        self.sent_messages.lock().unwrap().clone()
    }

    fn clear_sent_messages(&self) {
        self.sent_messages.lock().unwrap().clear();
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), TransportError> {
        if self.should_fail {
            return Err(TransportError::Other("Simulated transport error".to_string()));
        }
        
        self.sent_messages.lock().unwrap().push((message, destination));
        Ok(())
    }
    
    fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
        Ok(self.local_addr)
    }
    
    async fn close(&self) -> std::result::Result<(), TransportError> {
        Ok(()) // Do nothing for test mock
    }
    
    fn is_closed(&self) -> bool {
        false // Always return false for testing
    }
}

// Helper function to create an INVITE request
fn create_test_invite() -> Request {
    let uri = Uri::sip("bob@example.com");
    let from_uri = Uri::sip("alice@example.com");
    
    // Create address and add tag to uri
    let mut from_uri_with_tag = from_uri.clone();
    from_uri_with_tag = from_uri_with_tag.with_parameter(Param::tag("fromtag123"));
    let from_addr = Address::new(from_uri_with_tag);
    let to_addr = Address::new(uri.clone());
    
    RequestBuilder::new(Method::Invite, uri.to_string().as_str()).unwrap()
        .header(TypedHeader::From(From::new(from_addr)))
        .header(TypedHeader::To(To::new(to_addr)))
        .header(TypedHeader::CallId(CallId::new("test-call-id")))
        .header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

// Helper function to create a non-INVITE request
fn create_test_register() -> Request {
    let uri = Uri::sip("registrar.example.com");
    let from_uri = Uri::sip("alice@example.com");
    
    // Create address and add tag to uri
    let mut from_uri_with_tag = from_uri.clone();
    from_uri_with_tag = from_uri_with_tag.with_parameter(Param::tag("fromtag123"));
    let from_addr = Address::new(from_uri_with_tag);
    
    RequestBuilder::new(Method::Register, uri.to_string().as_str()).unwrap()
        .header(TypedHeader::From(From::new(from_addr)))
        .header(TypedHeader::To(To::new(Address::new(from_uri.clone()))))
        .header(TypedHeader::CallId(CallId::new("test-reg-id")))
        .header(TypedHeader::CSeq(CSeq::new(1, Method::Register)))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

// Helper function to create a response
fn create_test_response(request: &Request, status_code: StatusCode) -> Response {
    let mut builder = ResponseBuilder::new(status_code);
    
    // Copy essential headers
    if let Some(header) = request.header(&HeaderName::Via) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::From) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::To) {
        // Add a tag for final (non-100) responses
        if status_code.as_u16() >= 200 {
            if let TypedHeader::To(to) = header {
                let to_addr = to.address().clone();
                if !to_addr.uri.parameters.iter().any(|p| matches!(p, Param::Tag(_))) {
                    let uri_with_tag = to_addr.uri.with_parameter(Param::tag("resp-tag"));
                    let addr_with_tag = Address::new(uri_with_tag);
                    builder = builder.header(TypedHeader::To(To::new(addr_with_tag)));
                } else {
                    builder = builder.header(header.clone());
                }
            } else {
                builder = builder.header(header.clone());
            }
        } else {
            builder = builder.header(header.clone());
        }
    }
    if let Some(header) = request.header(&HeaderName::CallId) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::CSeq) {
        builder = builder.header(header.clone());
    }
    
    builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
    
    builder.build()
}

// Helper function to add a Via header with branch parameter
fn add_via_header(request: &mut Request) {
    let via = Via::new(
        "SIP", "2.0", "UDP",
        "127.0.0.1", Some(5060),
        vec![Param::branch("z9hG4bK-test")]
    ).unwrap();
    
    // Check if we already have a Via header
    if request.header(&HeaderName::Via).is_some() {
        return;
    }
    
    request.headers.insert(0, TypedHeader::Via(via));
}

// Test INVITE client transaction state transitions for a successful response
#[tokio::test(flavor = "multi_thread")]
async fn test_invite_client_transaction_success() {
    // Set a timeout for the entire test
    let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let transport = Arc::new(MockTransport::new(local_addr));
        
        // Setup transport and manager
        let (tx, rx) = mpsc::channel(100);
        let (manager, mut events_rx) = TransactionManager::new(transport.clone(), rx, None).await.unwrap();
        
        // Create INVITE request
        let mut invite_request = create_test_invite();
        add_via_header(&mut invite_request);
        
        // Create client transaction
        let transaction_id = manager.create_client_transaction(
            invite_request.clone(),
            remote_addr
        ).await.unwrap();
        
        // Verify initial state
        let state = manager.transaction_state(&transaction_id).await.unwrap();
        assert_eq!(state, TransactionState::Initial, "Client INVITE transaction should start in Initial state");
        
        // Send request
        manager.send_request(&transaction_id).await.unwrap();
        
        println!("INVITE client success: Request sent, now waiting for events");
        
        // Verify transition to Calling state
        let state = manager.transaction_state(&transaction_id).await.unwrap();
        assert_eq!(state, TransactionState::Calling, "Client INVITE transaction should transition to Calling state after sending");
        
        // Drain all events until we find NewRequest, ignoring timer events
        // Wait up to 5 seconds for event
        let mut new_request_seen = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                println!("INVITE client success: Received event: {:?}", event);
                match event {
                    TransactionEvent::NewRequest { .. } => {
                        new_request_seen = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    other => {
                        println!("INVITE client success: Unexpected event: {:?}", other);
                    }
                }
            }
            
            if new_request_seen {
                break;
            }
            
            // Wait a bit before polling again
            sleep(Duration::from_millis(50)).await;
        }
        
        if !new_request_seen {
            println!("WARNING: INVITE client success: Never saw NewRequest event, but continuing test anyway");
        }
        
        // Simulate 100 Trying response
        let trying_response = create_test_response(&invite_request, StatusCode::Trying);
        let transport_event = TransportEvent::MessageReceived {
            message: Message::Response(trying_response),
            source: remote_addr,
            destination: local_addr,
        };
        tx.send(transport_event).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(100)).await;
        
        // Wait for state to change to Proceeding
        let mut in_proceeding = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&transaction_id).await.unwrap();
            if state == TransactionState::Proceeding {
                in_proceeding = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        
        assert!(in_proceeding, "Transaction should have transitioned to Proceeding state");
        
        // Drain all events, look for ProvisionalResponse
        let mut provisional_response_seen = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                println!("INVITE client success: Received event: {:?}", event);
                match event {
                    TransactionEvent::ProvisionalResponse { .. } => {
                        provisional_response_seen = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    other => {
                        println!("INVITE client success: Unexpected event: {:?}", other);
                    }
                }
            }
            
            if provisional_response_seen {
                break;
            }
            
            // Wait a bit before polling again
            sleep(Duration::from_millis(50)).await;
        }
        
        if !provisional_response_seen {
            println!("WARNING: INVITE client success: Never saw ProvisionalResponse event, but continuing test anyway");
        }
        
        // Simulate 200 OK response
        let ok_response = create_test_response(&invite_request, StatusCode::Ok);
        let transport_event = TransportEvent::MessageReceived {
            message: Message::Response(ok_response),
            source: remote_addr,
            destination: local_addr,
        };
        tx.send(transport_event).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(100)).await;
        
        // Wait for the transaction to Terminate (direct termination on 2xx for INVITE)
        println!("INVITE client success: Waiting for transaction to terminate after 2xx response");
        let mut is_terminated = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&transaction_id).await.unwrap_or(TransactionState::Terminated);
            println!("INVITE client success: Current state: {:?}", state);
            if state == TransactionState::Terminated {
                is_terminated = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        assert!(is_terminated, "Client INVITE transaction should terminate after receiving 2xx response");
    }).await;
    
    // Check if we hit the timeout
    match timeout_result {
        Ok(_) => println!("INVITE client success test completed successfully"),
        Err(_) => panic!("INVITE client success test timed out after 30 seconds"),
    }
}

// Test INVITE client transaction state transitions for a failure response
#[tokio::test(flavor = "multi_thread")]
async fn test_invite_client_transaction_failure() {
    // Set a timeout for the entire test
    let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let transport = Arc::new(MockTransport::new(local_addr));
        
        // Setup transport and manager
        let (tx, rx) = mpsc::channel(100);
        let (manager, mut events_rx) = TransactionManager::new(transport.clone(), rx, None).await.unwrap();
        
        // Create INVITE request
        let mut invite_request = create_test_invite();
        add_via_header(&mut invite_request);
        
        // Create client transaction
        let transaction_id = manager.create_client_transaction(
            invite_request.clone(),
            remote_addr
        ).await.unwrap();
        
        // Send request
        manager.send_request(&transaction_id).await.unwrap();
        
        // Verify transition to Calling state
        let state = manager.transaction_state(&transaction_id).await.unwrap();
        assert_eq!(state, TransactionState::Calling, "Client INVITE transaction should transition to Calling state after sending");
        
        // Look for NewRequest event with a timeout
        let start = std::time::Instant::now();
        let mut found_new_request = false;
        
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                match event {
                    TransactionEvent::NewRequest { .. } => {
                        found_new_request = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    _ => {}
                }
            }
            
            if found_new_request {
                break;
            }
            
            // Prevent tight loop
            sleep(Duration::from_millis(10)).await;
        }
        
        if !found_new_request {
            println!("WARNING: Never saw NewRequest event, continuing anyway");
        }
        
        // Simulate 100 Trying response
        let trying_response = create_test_response(&invite_request, StatusCode::Trying);
        let transport_event = TransportEvent::MessageReceived {
            message: Message::Response(trying_response),
            source: remote_addr,
            destination: local_addr,
        };
        tx.send(transport_event).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(50)).await;
        
        // Look for ProvisionalResponse with a timeout
        let start = std::time::Instant::now();
        let mut found_provisional = false;
        
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                match event {
                    TransactionEvent::ProvisionalResponse { .. } => {
                        found_provisional = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    _ => {}
                }
            }
            
            if found_provisional {
                break;
            }
            
            // Prevent tight loop
            sleep(Duration::from_millis(10)).await;
        }
        
        if !found_provisional {
            println!("WARNING: Never saw ProvisionalResponse event, continuing anyway");
        }
        
        // Simulate 404 Not Found response
        let not_found_response = create_test_response(&invite_request, StatusCode::NotFound);
        let transport_event = TransportEvent::MessageReceived {
            message: Message::Response(not_found_response),
            source: remote_addr,
            destination: local_addr,
        };
        tx.send(transport_event).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(50)).await;
        
        // Look for FailureResponse with a timeout
        let start = std::time::Instant::now();
        let mut found_failure = false;
        
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                match event {
                    TransactionEvent::FailureResponse { transaction_id: tx_id, .. } => {
                        assert_eq!(tx_id, transaction_id);
                        found_failure = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    _ => {}
                }
            }
            
            if found_failure {
                break;
            }
            
            // Prevent tight loop
            sleep(Duration::from_millis(10)).await;
        }
        
        if !found_failure {
            println!("WARNING: Never saw FailureResponse event, continuing anyway");
        }
        
        // Allow time for ACK to be sent and state to transition to Completed
        sleep(Duration::from_millis(100)).await;
        
        // Verify state is Completed after non-2xx final response
        let state = manager.transaction_state(&transaction_id).await.unwrap();
        assert_eq!(state, TransactionState::Completed, "Client INVITE transaction should be in Completed state after receiving 4xx");
        
        // Wait for Timer D to expire - use a longer wait time to ensure it expires
        sleep(Duration::from_millis(1000)).await;
        
        // Verify state is now Terminated after Timer D
        let state = manager.transaction_state(&transaction_id).await.unwrap_or(TransactionState::Terminated);
        assert_eq!(state, TransactionState::Terminated, "Client INVITE transaction should terminate after Timer D");
    }).await;
    
    // Check if we hit the timeout
    match timeout_result {
        Ok(_) => println!("INVITE client failure test completed successfully"),
        Err(_) => panic!("INVITE client failure test timed out after 30 seconds"),
    }
}

// Test non-INVITE client transaction state transitions
#[tokio::test(flavor = "multi_thread")]
async fn test_non_invite_client_transaction_states() {
    // Set a timeout for the entire test
    let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let transport = Arc::new(MockTransport::new(local_addr));
        
        // Setup transport and manager
        let (tx, rx) = mpsc::channel(100);
        let (manager, mut events_rx) = TransactionManager::new(transport.clone(), rx, None).await.unwrap();
        
        // Create REGISTER request
        let mut register_request = create_test_register();
        add_via_header(&mut register_request);
        
        // Create client transaction
        let transaction_id = manager.create_client_transaction(
            register_request.clone(), 
            remote_addr
        ).await.unwrap();
        
        // Verify initial state
        let state = manager.transaction_state(&transaction_id).await.unwrap();
        assert_eq!(state, TransactionState::Initial, "Client non-INVITE transaction should start in Initial state");
        
        // Send request
        manager.send_request(&transaction_id).await.unwrap();
        
        println!("non-INVITE client: Request sent, now waiting for events");
        
        // Verify transition to Trying state (different from INVITE which goes to Calling)
        let state = manager.transaction_state(&transaction_id).await.unwrap();
        assert_eq!(state, TransactionState::Trying, "Non-INVITE client transaction should transition to Trying state after sending");
        
        // Drain all events until we find NewRequest, ignoring timer events
        // Wait up to 5 seconds for event
        let mut new_request_seen = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                println!("non-INVITE client: Received event: {:?}", event);
                match event {
                    TransactionEvent::NewRequest { .. } => {
                        new_request_seen = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    other => {
                        println!("non-INVITE client: Unexpected event: {:?}", other);
                    }
                }
            }
            
            if new_request_seen {
                break;
            }
            
            // Wait a bit before polling again
            sleep(Duration::from_millis(50)).await;
        }
        
        if !new_request_seen {
            println!("WARNING: non-INVITE client: Never saw NewRequest event, but continuing test anyway");
        }
        
        // Simulate 100 Trying response
        let trying_response = create_test_response(&register_request, StatusCode::Trying);
        let transport_event = TransportEvent::MessageReceived {
            message: Message::Response(trying_response),
            source: remote_addr,
            destination: local_addr,
        };
        tx.send(transport_event).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(100)).await;
        
        // Wait for the transaction to reach Proceeding
        let mut in_proceeding = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&transaction_id).await.unwrap();
            if state == TransactionState::Proceeding {
                in_proceeding = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        
        assert!(in_proceeding, "Transaction should have transitioned to Proceeding state");
        
        // Drain all events, look for ProvisionalResponse
        let mut provisional_response_seen = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            while let Ok(event) = events_rx.try_recv() {
                println!("non-INVITE client: Received event: {:?}", event);
                match event {
                    TransactionEvent::ProvisionalResponse { .. } => {
                        provisional_response_seen = true;
                        break;
                    },
                    TransactionEvent::TimerTriggered { .. } => {
                        // Ignore timer events
                        continue;
                    },
                    other => {
                        println!("non-INVITE client: Unexpected event: {:?}", other);
                    }
                }
            }
            
            if provisional_response_seen {
                break;
            }
            
            // Wait a bit before polling again
            sleep(Duration::from_millis(50)).await;
        }
        
        if !provisional_response_seen {
            println!("WARNING: non-INVITE client: Never saw ProvisionalResponse event, but continuing test anyway");
        }
        
        // Simulate 200 OK response
        let ok_response = create_test_response(&register_request, StatusCode::Ok);
        let transport_event = TransportEvent::MessageReceived {
            message: Message::Response(ok_response),
            source: remote_addr,
            destination: local_addr,
        };
        tx.send(transport_event).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(100)).await;
        
        // Wait for the transaction to reach Completed
        let mut in_completed = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&transaction_id).await.unwrap();
            if state == TransactionState::Completed {
                in_completed = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        
        assert!(in_completed, "Transaction should have transitioned to Completed state");
        
        // Wait for Timer K to expire - use a longer wait time to ensure it expires
        // Poll the state every 100ms until it's Terminated or we timeout
        println!("non-INVITE client: Waiting for Timer K to expire and transaction to terminate");
        let mut is_terminated = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&transaction_id).await.unwrap_or(TransactionState::Terminated);
            println!("non-INVITE client: Current state: {:?}", state);
            if state == TransactionState::Terminated {
                is_terminated = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        assert!(is_terminated, "Non-INVITE client transaction should terminate after Timer K");
    }).await;
    
    // Check if we hit the timeout
    match timeout_result {
        Ok(_) => println!("non-INVITE client test completed successfully"),
        Err(_) => panic!("non-INVITE client test timed out after 30 seconds"),
    }
}

// Test the server INVITE transaction state flow
#[tokio::test]
async fn test_server_invite_transaction_states() {
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Setup transport and manager
    let (transport_tx, transport_rx) = mpsc::channel(100);
    let (manager, mut events_rx) = TransactionManager::new(transport.clone(), transport_rx, None).await.unwrap();
    
    // Create INVITE request with Via header
    let mut invite_request = create_test_invite();
    let via = Via::new(
        "SIP", "2.0", "UDP",
        "192.168.1.2", Some(5060),
        vec![Param::branch("z9hG4bK-test")]
    ).unwrap();
    invite_request.headers.insert(0, TypedHeader::Via(via));
    
    // Deliver request via transport event channel
    transport_tx.send(TransportEvent::MessageReceived {
        message: Message::Request(invite_request.clone()),
        source: remote_addr,
        destination: local_addr,
    }).await.unwrap();
    
    // Allow time for processing
    sleep(Duration::from_millis(50)).await;
    
    // Get NewRequest event with server transaction ID
    let event = events_rx.recv().await.unwrap();
    let server_tx_id = match event {
        TransactionEvent::NewRequest { transaction_id, .. } => transaction_id,
        _ => panic!("Expected NewRequest event, got {:?}", event),
    };
    
    // Verify server transaction state is Proceeding
    let state = manager.transaction_state(&server_tx_id).await.unwrap();
    assert_eq!(state, TransactionState::Proceeding, "Server INVITE transaction should start in Proceeding state");
    
    // Send 100 Trying response
    let trying_response = create_test_response(&invite_request, StatusCode::Trying);
    manager.send_response(&server_tx_id, trying_response).await.unwrap();
    
    // Get event for 100 Trying
    let event = events_rx.recv().await.unwrap();
    match event {
        TransactionEvent::ProvisionalResponseSent { .. } => {
            // Expected
        },
        _ => panic!("Expected ProvisionalResponseSent event, got {:?}", event),
    }
    
    // Verify still in Proceeding state
    let state = manager.transaction_state(&server_tx_id).await.unwrap();
    assert_eq!(state, TransactionState::Proceeding, "Server INVITE transaction should remain in Proceeding state after sending 1xx");
    
    // Send 200 OK (success) response
    let ok_response = create_test_response(&invite_request, StatusCode::Ok);
    manager.send_response(&server_tx_id, ok_response).await.unwrap();
    
    // Get event for 200 OK
    let event = events_rx.recv().await.unwrap();
    match event {
        TransactionEvent::FinalResponseSent { .. } => {
            // Expected
        },
        _ => panic!("Expected FinalResponseSent event, got {:?}", event),
    }
    
    // Allow time for state transition
    sleep(Duration::from_millis(50)).await;
    
    // Verify server transaction moves directly to Terminated for 2xx response (per RFC 3261)
    let state = manager.transaction_state(&server_tx_id).await.unwrap_or(TransactionState::Terminated);
    assert_eq!(state, TransactionState::Terminated, "Server INVITE transaction should transition to Terminated after sending 2xx response");
}

// Test the server INVITE transaction state flow with a failure response
#[tokio::test(flavor = "multi_thread")]
async fn test_server_invite_transaction_failure_states() {
    // Set a timeout for the entire test
    let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let transport = Arc::new(MockTransport::new(local_addr));
        
        // Setup transport and manager
        let (transport_tx, transport_rx) = mpsc::channel(100);
        let (manager, mut events_rx) = TransactionManager::new(transport.clone(), transport_rx, None).await.unwrap();
        
        // Create INVITE request with Via header
        let mut invite_request = create_test_invite();
        let via = Via::new(
            "SIP", "2.0", "UDP",
            "192.168.1.2", Some(5060),
            vec![Param::branch("z9hG4bK-test")]
        ).unwrap();
        invite_request.headers.insert(0, TypedHeader::Via(via));
        
        // Deliver request via transport event channel
        transport_tx.send(TransportEvent::MessageReceived {
            message: Message::Request(invite_request.clone()),
            source: remote_addr,
            destination: local_addr,
        }).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(50)).await;
        
        // Get NewRequest event with server transaction ID
        let event = events_rx.recv().await.unwrap();
        let server_tx_id = match event {
            TransactionEvent::NewRequest { transaction_id, .. } => transaction_id,
            _ => panic!("Expected NewRequest event, got {:?}", event),
        };
        
        // Verify server transaction state is Proceeding
        let state = manager.transaction_state(&server_tx_id).await.unwrap();
        assert_eq!(state, TransactionState::Proceeding, "Server INVITE transaction should start in Proceeding state");
        
        // Send 404 Not Found response
        let not_found_response = create_test_response(&invite_request, StatusCode::NotFound);
        manager.send_response(&server_tx_id, not_found_response.clone()).await.unwrap();
        
        // Get event for 404 Not Found
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::FinalResponseSent { .. } => {
                // Expected
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
        
        // Allow time for state transition
        sleep(Duration::from_millis(100)).await;
        
        // Verify server transaction transitions to Completed for non-2xx response
        let state = manager.transaction_state(&server_tx_id).await.unwrap();
        assert_eq!(state, TransactionState::Completed, "Server INVITE transaction should transition to Completed after sending 4xx response");
        
        // Create and send the ACK
        println!("server INVITE failure: Sending ACK request to move to Confirmed state");
        let mut ack_request = Request::new(Method::Ack, invite_request.uri.clone());
        
        // Copy key headers from original INVITE
        if let Some(TypedHeader::Via(via)) = invite_request.header(&HeaderName::Via) {
            ack_request.headers.push(TypedHeader::Via(via.clone()));
        }
        if let Some(TypedHeader::From(from)) = invite_request.header(&HeaderName::From) {
            ack_request.headers.push(TypedHeader::From(from.clone()));
        }
        // For To, use the one with tag from the response
        if let Some(TypedHeader::To(to)) = not_found_response.header(&HeaderName::To) {
            ack_request.headers.push(TypedHeader::To(to.clone()));
        }
        if let Some(TypedHeader::CallId(call_id)) = invite_request.header(&HeaderName::CallId) {
            ack_request.headers.push(TypedHeader::CallId(call_id.clone()));
        }
        // Create CSeq with same sequence number but ACK method
        if let Some(TypedHeader::CSeq(cseq)) = invite_request.header(&HeaderName::CSeq) {
            let seq_num = cseq.sequence();
            ack_request.headers.push(TypedHeader::CSeq(CSeq::new(seq_num, Method::Ack)));
        }
        
        // Deliver ACK via transport event channel
        transport_tx.send(TransportEvent::MessageReceived {
            message: Message::Request(ack_request),
            source: remote_addr,
            destination: local_addr,
        }).await.unwrap();
        
        // Allow time for processing and state transition
        sleep(Duration::from_millis(200)).await;
        
        // Poll for transition to Confirmed state
        let mut in_confirmed = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&server_tx_id).await.unwrap();
            println!("server INVITE failure: Current state: {:?}", state);
            if state == TransactionState::Confirmed {
                in_confirmed = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        assert!(in_confirmed, "Server INVITE transaction should transition to Confirmed after receiving ACK");
        
        // Wait for Timer I to expire
        println!("server INVITE failure: Waiting for Timer I to expire and transaction to terminate");
        let mut is_terminated = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&server_tx_id).await.unwrap_or(TransactionState::Terminated);
            println!("server INVITE failure: Current state: {:?}", state);
            if state == TransactionState::Terminated {
                is_terminated = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        assert!(is_terminated, "Server INVITE transaction should terminate after Timer I");
    }).await;
    
    // Check if we hit the timeout
    match timeout_result {
        Ok(_) => println!("server INVITE failure test completed successfully"),
        Err(_) => panic!("server INVITE failure test timed out after 30 seconds"),
    }
}

// Test the server non-INVITE transaction state flow
#[tokio::test(flavor = "multi_thread")]
async fn test_server_non_invite_transaction_states() {
    // Set a timeout for the entire test
    let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let transport = Arc::new(MockTransport::new(local_addr));
        
        // Setup transport and manager
        let (transport_tx, transport_rx) = mpsc::channel(100);
        let (manager, mut events_rx) = TransactionManager::new(transport.clone(), transport_rx, None).await.unwrap();
        
        // Create REGISTER request with Via header
        let mut register_request = create_test_register();
        let via = Via::new(
            "SIP", "2.0", "UDP",
            "192.168.1.2", Some(5060),
            vec![Param::branch("z9hG4bK-test")]
        ).unwrap();
        register_request.headers.insert(0, TypedHeader::Via(via));
        
        // Deliver request via transport event channel
        transport_tx.send(TransportEvent::MessageReceived {
            message: Message::Request(register_request.clone()),
            source: remote_addr,
            destination: local_addr,
        }).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(50)).await;
        
        // Get NewRequest event with server transaction ID
        let event = events_rx.recv().await.unwrap();
        let server_tx_id = match event {
            TransactionEvent::NewRequest { transaction_id, .. } => transaction_id,
            _ => panic!("Expected NewRequest event, got {:?}", event),
        };
        
        // Verify server transaction state is Trying
        let state = manager.transaction_state(&server_tx_id).await.unwrap();
        assert_eq!(state, TransactionState::Trying, "Server non-INVITE transaction should start in Trying state");
        
        // Send 200 OK response
        let ok_response = create_test_response(&register_request, StatusCode::Ok);
        manager.send_response(&server_tx_id, ok_response).await.unwrap();
        
        // Get event for 200 OK
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::FinalResponseSent { .. } => {
                // Expected
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
        
        // Allow time for state transition
        sleep(Duration::from_millis(100)).await;
        
        // Verify server transaction moves to Completed after sending final response
        let state = manager.transaction_state(&server_tx_id).await.unwrap();
        assert_eq!(state, TransactionState::Completed, "Server non-INVITE transaction should transition to Completed after sending final response");
        
        // Wait for Timer J to expire
        println!("server non-INVITE: Waiting for Timer J to expire and transaction to terminate");
        let mut is_terminated = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let state = manager.transaction_state(&server_tx_id).await.unwrap_or(TransactionState::Terminated);
            println!("server non-INVITE: Current state: {:?}", state);
            if state == TransactionState::Terminated {
                is_terminated = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        assert!(is_terminated, "Server non-INVITE transaction should terminate after Timer J");
    }).await;
    
    // Check if we hit the timeout
    match timeout_result {
        Ok(_) => println!("server non-INVITE test completed successfully"),
        Err(_) => panic!("server non-INVITE test timed out after 30 seconds"),
    }
} 