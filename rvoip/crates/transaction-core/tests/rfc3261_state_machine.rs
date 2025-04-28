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
    transaction::{
        TransactionState, 
        TransactionEvent,
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
#[tokio::test]
async fn test_invite_client_transaction_success() {
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
    
    // Verify transition to Calling state
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    assert_eq!(state, TransactionState::Calling, "Client INVITE transaction should transition to Calling state after sending");
    
    // Check events - drain timer events first
    let mut event_found = false;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            TransactionEvent::NewRequest { .. } => {
                event_found = true;
                break;
            },
            TransactionEvent::TimerTriggered { .. } => {
                // Ignore timer events
                continue;
            },
            other => panic!("Unexpected event: {:?}", other),
        }
    }
    
    if !event_found {
        // Wait for NewRequest event
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::NewRequest { .. } => {
                // Expected
            },
            _ => panic!("Expected NewRequest event, got {:?}", event),
        }
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
    
    // Drain any timer events first
    while let Ok(event) = events_rx.try_recv() {
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id: tx_id, .. } => {
                assert_eq!(tx_id, transaction_id);
                event_found = true;
                break;
            },
            TransactionEvent::TimerTriggered { .. } => {
                // Ignore timer events
                continue;
            },
            other => {
                panic!("Expected ProvisionalResponse event, got {:?}", other);
            }
        }
    }
    
    if !event_found {
        // Wait for ProvisionalResponse event
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id: tx_id, .. } => {
                assert_eq!(tx_id, transaction_id);
            },
            _ => panic!("Expected ProvisionalResponse event, got {:?}", event),
        }
    }
    
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    assert_eq!(state, TransactionState::Proceeding, "Client INVITE transaction should transition to Proceeding after 1xx response");
    
    // Simulate 200 OK response
    let ok_response = create_test_response(&invite_request, StatusCode::Ok);
    let transport_event = TransportEvent::MessageReceived {
        message: Message::Response(ok_response),
        source: remote_addr,
        destination: local_addr,
    };
    tx.send(transport_event).await.unwrap();
    
    // Allow time for processing
    sleep(Duration::from_millis(50)).await;
    
    // Check for SuccessResponse, ignoring any timer events
    event_found = false;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            TransactionEvent::SuccessResponse { transaction_id: tx_id, .. } => {
                assert_eq!(tx_id, transaction_id);
                event_found = true;
                break;
            },
            TransactionEvent::TimerTriggered { .. } => {
                // Ignore timer events
                continue;
            },
            other => {
                panic!("Expected SuccessResponse event, got {:?}", other);
            }
        }
    }
    
    if !event_found {
        // Wait for SuccessResponse event
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::SuccessResponse { transaction_id: tx_id, .. } => {
                assert_eq!(tx_id, transaction_id);
            },
            _ => panic!("Expected SuccessResponse event, got {:?}", event),
        }
    }
    
    // Allow time for termination
    sleep(Duration::from_millis(100)).await;
    
    // Verify state is Terminated for 2xx response (per RFC 3261 section 17.1.1.2)
    let state = manager.transaction_state(&transaction_id).await.unwrap_or(TransactionState::Terminated);
    assert_eq!(state, TransactionState::Terminated, "Client INVITE transaction should terminate after receiving 2xx response");
}

// Test INVITE client transaction state transitions for a failure response
#[tokio::test]
async fn test_invite_client_transaction_failure() {
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
    
    // Check event for request - ignore timer events
    let mut found_new_request = false;
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
    
    if !found_new_request {
        // Wait for NewRequest event if we haven't found it yet
        while let Some(event) = events_rx.recv().await {
            match event {
                TransactionEvent::NewRequest { .. } => {
                    break;
                },
                TransactionEvent::TimerTriggered { .. } => {
                    // Ignore timer events
                    continue;
                },
                _ => panic!("Expected NewRequest or TimerTriggered event, got {:?}", event),
            }
        }
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
    
    // Get event and verify state transition to Proceeding - ignore timer events
    let mut found_provisional = false;
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
    
    if !found_provisional {
        // Wait for ProvisionalResponse event if we haven't found it yet
        while let Some(event) = events_rx.recv().await {
            match event {
                TransactionEvent::ProvisionalResponse { .. } => {
                    break;
                },
                TransactionEvent::TimerTriggered { .. } => {
                    // Ignore timer events
                    continue;
                },
                _ => panic!("Expected ProvisionalResponse or TimerTriggered event, got {:?}", event),
            }
        }
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
    
    // Get event for 4xx response - ignore timer events
    let mut found_failure = false;
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
    
    if !found_failure {
        // Wait for FailureResponse event if we haven't found it yet
        while let Some(event) = events_rx.recv().await {
            match event {
                TransactionEvent::FailureResponse { transaction_id: tx_id, .. } => {
                    assert_eq!(tx_id, transaction_id);
                    break;
                },
                TransactionEvent::TimerTriggered { .. } => {
                    // Ignore timer events
                    continue;
                },
                _ => panic!("Expected FailureResponse or TimerTriggered event, got {:?}", event),
            }
        }
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
}

// Test non-INVITE client transaction state transitions
#[tokio::test]
async fn test_non_invite_client_transaction_states() {
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
    assert_eq!(state, TransactionState::Initial, "Non-INVITE client transaction should start in Initial state");
    
    // Send request
    manager.send_request(&transaction_id).await.unwrap();
    
    // Verify transition to Trying state (different from INVITE which goes to Calling)
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    assert_eq!(state, TransactionState::Trying, "Non-INVITE client transaction should transition to Trying state after sending");
    
    // Check for NewRequest, ignoring timer events
    let mut event_found = false;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            TransactionEvent::NewRequest { .. } => {
                event_found = true;
                break;
            },
            TransactionEvent::TimerTriggered { .. } => {
                // Ignore timer events
                continue;
            },
            other => {
                panic!("Unexpected event: {:?}", other);
            }
        }
    }
    
    if !event_found {
        // Wait for NewRequest event
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::NewRequest { .. } => {},
            _ => panic!("Expected NewRequest event, got {:?}", event),
        }
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
    sleep(Duration::from_millis(50)).await;
    
    // Check for ProvisionalResponse, ignoring timer events
    event_found = false;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            TransactionEvent::ProvisionalResponse { .. } => {
                event_found = true;
                break;
            },
            TransactionEvent::TimerTriggered { .. } => {
                // Ignore timer events
                continue;
            },
            other => {
                panic!("Unexpected event: {:?}", other);
            }
        }
    }
    
    if !event_found {
        // Wait for ProvisionalResponse event
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::ProvisionalResponse { .. } => {},
            _ => panic!("Expected ProvisionalResponse event, got {:?}", event),
        }
    }
    
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    assert_eq!(state, TransactionState::Proceeding, "Non-INVITE client transaction should transition to Proceeding after 1xx response");
    
    // Simulate 200 OK response
    let ok_response = create_test_response(&register_request, StatusCode::Ok);
    let transport_event = TransportEvent::MessageReceived {
        message: Message::Response(ok_response),
        source: remote_addr,
        destination: local_addr,
    };
    tx.send(transport_event).await.unwrap();
    
    // Allow time for processing
    sleep(Duration::from_millis(50)).await;
    
    // Check for SuccessResponse, ignoring timer events
    event_found = false;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            TransactionEvent::SuccessResponse { transaction_id: tx_id, .. } => {
                assert_eq!(tx_id, transaction_id);
                event_found = true;
                break;
            },
            TransactionEvent::TimerTriggered { .. } => {
                // Ignore timer events
                continue;
            },
            other => {
                panic!("Expected SuccessResponse event, got {:?}", other);
            }
        }
    }
    
    if !event_found {
        // Wait for SuccessResponse event
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::SuccessResponse { transaction_id: tx_id, .. } => {
                assert_eq!(tx_id, transaction_id);
            },
            _ => panic!("Expected SuccessResponse event, got {:?}", event),
        }
    }
    
    // Verify state is Completed after final response
    let state = manager.transaction_state(&transaction_id).await.unwrap();
    assert_eq!(state, TransactionState::Completed, "Non-INVITE client transaction should be in Completed state after receiving final response");
    
    // Wait for Timer K to expire - use a longer wait time to ensure it expires
    sleep(Duration::from_millis(1000)).await;
    
    // Verify state is now Terminated after Timer K
    let state = manager.transaction_state(&transaction_id).await.unwrap_or(TransactionState::Terminated);
    assert_eq!(state, TransactionState::Terminated, "Non-INVITE client transaction should terminate after Timer K");
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
#[tokio::test]
async fn test_server_invite_transaction_failure_states() {
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
    sleep(Duration::from_millis(50)).await;
    
    // Verify server transaction transitions to Completed for non-2xx response
    let state = manager.transaction_state(&server_tx_id).await.unwrap();
    assert_eq!(state, TransactionState::Completed, "Server INVITE transaction should transition to Completed after sending 4xx response");
    
    // Simulate receiving ACK from client
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
    sleep(Duration::from_millis(100)).await;
    
    // Verify server transaction transitions to Confirmed after receiving ACK
    let state = manager.transaction_state(&server_tx_id).await.unwrap();
    assert_eq!(state, TransactionState::Confirmed, "Server INVITE transaction should transition to Confirmed after receiving ACK");
    
    // Wait for Timer I to expire (using a short value for tests)
    sleep(Duration::from_millis(500)).await;
    
    // Verify server transaction transitions to Terminated after Timer I
    let state = manager.transaction_state(&server_tx_id).await.unwrap_or(TransactionState::Terminated);
    assert_eq!(state, TransactionState::Terminated, "Server INVITE transaction should terminate after Timer I");
}

// Test the server non-INVITE transaction state flow
#[tokio::test]
async fn test_server_non_invite_transaction_states() {
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
    sleep(Duration::from_millis(50)).await;
    
    // Verify server transaction moves to Completed after sending final response
    let state = manager.transaction_state(&server_tx_id).await.unwrap();
    assert_eq!(state, TransactionState::Completed, "Server non-INVITE transaction should transition to Completed after sending final response");
    
    // Wait for Timer J to expire - use a longer wait time to ensure it expires
    sleep(Duration::from_millis(1000)).await;
    
    // Verify server transaction terminates after Timer J
    let state = manager.transaction_state(&server_tx_id).await.unwrap_or(TransactionState::Terminated);
    assert_eq!(state, TransactionState::Terminated, "Server non-INVITE transaction should terminate after Timer J");
} 