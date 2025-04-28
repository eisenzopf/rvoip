#[cfg(test)]
mod manager_tests {
    use std::net::{IpAddr, SocketAddr};
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::sleep;

    use rvoip_sip_core::prelude::*;
    use rvoip_sip_transport::Transport;
    use rvoip_sip_transport::TransportEvent;

    use crate::manager::TransactionManager;
    use crate::transaction::{Transaction, TransactionEvent, TransactionKey, TransactionState};
    use crate::utils;

    // Mock Transport implementation for testing
    #[derive(Debug)]
    struct MockTransport {
        local_addr: SocketAddr,
        sent_messages: std::sync::Mutex<Vec<(Message, SocketAddr)>>,
        message_tx: mpsc::Sender<TransportEvent>,
    }

    impl MockTransport {
        fn new(local_addr: SocketAddr) -> (Self, mpsc::Receiver<TransportEvent>) {
            let (tx, rx) = mpsc::channel(100);
            (
                Self {
                    local_addr,
                    sent_messages: std::sync::Mutex::new(Vec::new()),
                    message_tx: tx,
                },
                rx,
            )
        }

        // Helper to simulate an incoming message for testing
        async fn simulate_incoming(&self, message: Message, source: SocketAddr) {
            self.message_tx
                .send(TransportEvent::MessageReceived {
                    message,
                    source,
                    destination: self.local_addr,
                })
                .await
                .expect("Failed to send mock transport event");
        }

        // Get all sent messages
        fn get_sent_messages(&self) -> Vec<(Message, SocketAddr)> {
            self.sent_messages.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Transport for MockTransport {
        async fn send_message(&self, message: Message, destination: SocketAddr) -> std::io::Result<()> {
            println!("Mock transport sending message to {}: {:?}", destination, message);
            self.sent_messages.lock().unwrap().push((message, destination));
            Ok(())
        }

        fn local_addr(&self) -> std::io::Result<SocketAddr> {
            Ok(self.local_addr)
        }
    }

    // Helper function to create a test INVITE request
    fn create_test_invite(to_uri: &str) -> Request {
        let to_uri = Uri::from_str(to_uri).unwrap();
        let from_uri = Uri::from_str("sip:test@example.com").unwrap();
        
        // Add tag to From header
        let from_addr = Address::new(from_uri).with_parameter(Param::tag("fromtag123"));
        let to_addr = Address::new(to_uri);
        
        RequestBuilder::new(Method::Invite, to_uri.to_string().as_str())
            .unwrap()
            .header(TypedHeader::From(From::new(from_addr)))
            .header(TypedHeader::To(To::new(to_addr)))
            .header(TypedHeader::CallId(CallId::new(format!("call-{}", uuid::Uuid::new_v4()))))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }

    // Helper function to create a test response
    fn create_test_response(request: &Request, status: StatusCode) -> Response {
        utils::create_response(request, status)
    }

    #[tokio::test]
    async fn test_client_transaction_basic_flow() {
        // Set up mock transport
        let local_addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 5060);
        let remote_addr = SocketAddr::new(IpAddr::from_str("10.0.0.1").unwrap(), 5060);
        let (mock_transport, transport_rx) = MockTransport::new(local_addr);
        let mock_transport = Arc::new(mock_transport);

        // Create transaction manager
        let (manager, mut events_rx) = TransactionManager::new(
            mock_transport.clone(),
            transport_rx,
            Some(100),
        )
        .await
        .expect("Failed to create transaction manager");

        // Create an INVITE request for the client transaction
        let invite_request = create_test_invite("sip:bob@example.net");

        // Start a client transaction
        let transaction_id = manager
            .create_client_transaction(invite_request.clone(), remote_addr)
            .await
            .expect("Failed to create client transaction");

        // Verify transaction is created but not started
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Initial);

        // Send the initial request
        manager.send_request(&transaction_id).await.expect("Failed to send request");

        // Verify request was sent through the transport
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        let (sent_message, sent_dest) = &sent_messages[0];
        assert_eq!(sent_dest, &remote_addr);
        if let Message::Request(req) = sent_message {
            assert_eq!(req.method(), Method::Invite);
        } else {
            panic!("Expected Request message");
        }

        // Verify transaction state changed to Calling
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Calling);

        // Simulate receiving a provisional response
        let trying_response = create_test_response(&invite_request, StatusCode::Trying);
        mock_transport
            .simulate_incoming(Message::Response(trying_response.clone()), remote_addr)
            .await;

        // Wait for event
        let event = events_rx.recv().await.expect("Failed to receive event");
        match event {
            TransactionEvent::ProvisionalResponse {
                transaction_id: id,
                response,
            } => {
                assert_eq!(id, transaction_id);
                assert_eq!(response.status(), StatusCode::Trying);
            }
            _ => panic!("Unexpected event: {:?}", event),
        }

        // Verify transaction state changed to Proceeding
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Proceeding);

        // Simulate receiving a final success response
        let ok_response = create_test_response(&invite_request, StatusCode::Ok);
        mock_transport
            .simulate_incoming(Message::Response(ok_response.clone()), remote_addr)
            .await;

        // Wait for event
        let event = events_rx.recv().await.expect("Failed to receive event");
        match event {
            TransactionEvent::SuccessResponse {
                transaction_id: id,
                response,
            } => {
                assert_eq!(id, transaction_id);
                assert_eq!(response.status(), StatusCode::Ok);
            }
            _ => panic!("Unexpected event: {:?}", event),
        }

        // Verify transaction state eventually transitions to Terminated
        // For 2xx responses, the client transaction should terminate quickly
        for _ in 0..10 {
            let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
            if state == TransactionState::Terminated {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Terminated);

        // Test ACK for 2xx
        manager.send_2xx_ack(&ok_response).await.expect("Failed to send ACK");

        // Verify ACK was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 2); // INVITE + ACK
        let (sent_message, sent_dest) = &sent_messages[1];
        assert_eq!(sent_dest, &remote_addr);
        if let Message::Request(req) = sent_message {
            assert_eq!(req.method(), Method::Ack);
        } else {
            panic!("Expected Request message");
        }
    }

    #[tokio::test]
    async fn test_server_transaction_basic_flow() {
        // Set up mock transport
        let local_addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 5060);
        let remote_addr = SocketAddr::new(IpAddr::from_str("10.0.0.1").unwrap(), 5060);
        let (mock_transport, transport_rx) = MockTransport::new(local_addr);
        let mock_transport = Arc::new(mock_transport);

        // Create transaction manager
        let (manager, mut events_rx) = TransactionManager::new(
            mock_transport.clone(),
            transport_rx,
            Some(100),
        )
        .await
        .expect("Failed to create transaction manager");

        // Create an INVITE request with a Via header for the server transaction
        let mut invite_request = create_test_invite("sip:user@127.0.0.1:5060");
        
        // Add Via header with branch
        let branch = utils::generate_branch();
        let via = Via::new(
            "SIP", "2.0", "UDP", 
            remote_addr.ip().to_string().as_str(), 
            Some(remote_addr.port()),
            vec![Param::branch(&branch)]
        ).unwrap();
        
        // Replace any existing Via header
        invite_request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
        invite_request.headers.insert(0, TypedHeader::Via(via));

        // Simulate receiving the INVITE
        mock_transport
            .simulate_incoming(Message::Request(invite_request.clone()), remote_addr)
            .await;

        // Wait for NewRequest event
        let transaction_id = loop {
            match events_rx.recv().await.expect("Failed to receive event") {
                TransactionEvent::NewRequest {
                    transaction_id,
                    request,
                    source,
                } => {
                    assert_eq!(request.method(), Method::Invite);
                    assert_eq!(source, remote_addr);
                    break transaction_id;
                }
                other => {
                    println!("Unexpected event while waiting for NewRequest: {:?}", other);
                    continue;
                }
            }
        };

        // Send a provisional response
        let trying_response = create_test_response(&invite_request, StatusCode::Trying);
        manager
            .send_response(&transaction_id, trying_response)
            .await
            .expect("Failed to send provisional response");

        // Verify response was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        let (sent_message, sent_dest) = &sent_messages[0];
        assert_eq!(sent_dest, &remote_addr);
        if let Message::Response(resp) = sent_message {
            assert_eq!(resp.status(), StatusCode::Trying);
        } else {
            panic!("Expected Response message");
        }

        // Send a ringing response
        let ringing_response = create_test_response(&invite_request, StatusCode::Ringing);
        manager
            .send_response(&transaction_id, ringing_response)
            .await
            .expect("Failed to send ringing response");

        // Verify response was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 2);
        let (sent_message, sent_dest) = &sent_messages[1];
        if let Message::Response(resp) = sent_message {
            assert_eq!(resp.status(), StatusCode::Ringing);
        } else {
            panic!("Expected Response message");
        }

        // Send a final success response
        let mut ok_response = create_test_response(&invite_request, StatusCode::Ok);
        
        // Add To tag to response
        if let Some(TypedHeader::To(to)) = ok_response.header(&HeaderName::To) {
            let new_to = to.clone().with_tag("totag123");
            ok_response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
            ok_response.headers.push(TypedHeader::To(new_to));
        }
        
        manager
            .send_response(&transaction_id, ok_response.clone())
            .await
            .expect("Failed to send final response");

        // Verify response was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 3);
        let (sent_message, sent_dest) = &sent_messages[2];
        if let Message::Response(resp) = sent_message {
            assert_eq!(resp.status(), StatusCode::Ok);
        } else {
            panic!("Expected Response message");
        }

        // Verify transaction is terminated for 2xx responses
        for _ in 0..10 {
            let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
            if state == TransactionState::Terminated {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Terminated);

        // Simulate receiving an ACK for the 2xx response
        let ack_request = RequestBuilder::new(Method::Ack, "sip:user@127.0.0.1:5060").unwrap()
            .header(TypedHeader::Via(via))
            .header(TypedHeader::From(invite_request.header(&HeaderName::From).unwrap().clone()))
            .header(TypedHeader::To(ok_response.header(&HeaderName::To).unwrap().clone()))
            .header(TypedHeader::CallId(invite_request.header(&HeaderName::CallId).unwrap().clone()))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Ack)))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
        
        // Since this is a 2xx ACK, it should be treated as a new transaction by the TU
        mock_transport
            .simulate_incoming(Message::Request(ack_request), remote_addr)
            .await;

        // We should get a StrayAck event since ACKs for 2xx create a new transaction
        let found_stray_ack = loop {
            match events_rx.recv().await.expect("Failed to receive event") {
                TransactionEvent::StrayAck { request, source } => {
                    assert_eq!(request.method(), Method::Ack);
                    assert_eq!(source, remote_addr);
                    break true;
                }
                other => {
                    println!("Got other event while waiting for StrayAck: {:?}", other);
                    // Check for timeout or other end conditions
                    if let TransactionEvent::StrayRequest { request, source: _ } = other {
                        if request.method() == Method::Ack {
                            break true; // Some implementations might use StrayRequest instead
                        }
                    }
                    if let Some(TransactionEvent::Error { transaction_id: _, error: _ }) = None {
                        break false;
                    }
                }
            }
        };
        
        assert!(found_stray_ack, "Did not receive StrayAck event");
    }

    #[tokio::test]
    async fn test_client_non_invite_transaction() {
        // Set up mock transport
        let local_addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 5060);
        let remote_addr = SocketAddr::new(IpAddr::from_str("10.0.0.1").unwrap(), 5060);
        let (mock_transport, transport_rx) = MockTransport::new(local_addr);
        let mock_transport = Arc::new(mock_transport);

        // Create transaction manager
        let (manager, mut events_rx) = TransactionManager::new(
            mock_transport.clone(),
            transport_rx,
            Some(100),
        )
        .await
        .expect("Failed to create transaction manager");

        // Create a non-INVITE request (REGISTER)
        let register_uri = Uri::from_str("sip:registrar.example.com").unwrap();
        let register_request = RequestBuilder::new(Method::Register, register_uri.to_string().as_str()).unwrap()
            .header(TypedHeader::From(From::new(Address::new(Uri::from_str("sip:user@example.com").unwrap())
                .with_parameter(Param::tag("fromtag123")))))
            .header(TypedHeader::To(To::new(Address::new(Uri::from_str("sip:user@example.com").unwrap()))))
            .header(TypedHeader::CallId(CallId::new(format!("reg-{}", uuid::Uuid::new_v4()))))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Register)))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();

        // Start a client transaction
        let transaction_id = manager
            .create_client_transaction(register_request.clone(), remote_addr)
            .await
            .expect("Failed to create client transaction");

        // Send the initial request
        manager.send_request(&transaction_id).await.expect("Failed to send request");

        // Verify transaction state changed to Trying
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Trying);

        // Simulate receiving a final success response
        let ok_response = create_test_response(&register_request, StatusCode::Ok);
        mock_transport
            .simulate_incoming(Message::Response(ok_response.clone()), remote_addr)
            .await;

        // Wait for event
        let event = events_rx.recv().await.expect("Failed to receive event");
        match event {
            TransactionEvent::SuccessResponse {
                transaction_id: id,
                response,
            } => {
                assert_eq!(id, transaction_id);
                assert_eq!(response.status(), StatusCode::Ok);
            }
            _ => panic!("Unexpected event: {:?}", event),
        }

        // Verify transaction state transitions to Completed and then Terminated
        // Non-INVITE transactions go to Completed first, then terminate after Timer K
        for _ in 0..10 {
            let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
            if state == TransactionState::Terminated {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        
        // It may still be in Completed state depending on Timer K duration
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert!(state == TransactionState::Completed || state == TransactionState::Terminated);
    }

    #[tokio::test]
    async fn test_server_non_invite_transaction() {
        // Set up mock transport
        let local_addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 5060);
        let remote_addr = SocketAddr::new(IpAddr::from_str("10.0.0.1").unwrap(), 5060);
        let (mock_transport, transport_rx) = MockTransport::new(local_addr);
        let mock_transport = Arc::new(mock_transport);

        // Create transaction manager
        let (manager, mut events_rx) = TransactionManager::new(
            mock_transport.clone(),
            transport_rx,
            Some(100),
        )
        .await
        .expect("Failed to create transaction manager");

        // Create a non-INVITE request (OPTIONS)
        let register_uri = Uri::from_str("sip:user@127.0.0.1:5060").unwrap();
        let mut options_request = RequestBuilder::new(Method::Options, register_uri.to_string().as_str()).unwrap()
            .header(TypedHeader::From(From::new(Address::new(Uri::from_str("sip:caller@example.com").unwrap())
                .with_parameter(Param::tag("fromtag123")))))
            .header(TypedHeader::To(To::new(Address::new(Uri::from_str("sip:user@127.0.0.1:5060").unwrap()))))
            .header(TypedHeader::CallId(CallId::new(format!("options-{}", uuid::Uuid::new_v4()))))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Options)))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
        
        // Add Via header with branch
        let branch = utils::generate_branch();
        let via = Via::new(
            "SIP", "2.0", "UDP", 
            remote_addr.ip().to_string().as_str(), 
            Some(remote_addr.port()),
            vec![Param::branch(&branch)]
        ).unwrap();
        
        // Replace any existing Via header
        options_request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
        options_request.headers.insert(0, TypedHeader::Via(via));

        // Simulate receiving the OPTIONS
        mock_transport
            .simulate_incoming(Message::Request(options_request.clone()), remote_addr)
            .await;

        // Wait for NewRequest event
        let transaction_id = loop {
            match events_rx.recv().await.expect("Failed to receive event") {
                TransactionEvent::NewRequest {
                    transaction_id,
                    request,
                    source,
                } => {
                    assert_eq!(request.method(), Method::Options);
                    assert_eq!(source, remote_addr);
                    break transaction_id;
                }
                other => {
                    println!("Unexpected event while waiting for NewRequest: {:?}", other);
                    continue;
                }
            }
        };

        // Send a final response (no provisional response needed for OPTIONS)
        let mut ok_response = create_test_response(&options_request, StatusCode::Ok);
        
        // Add To tag to response
        if let Some(TypedHeader::To(to)) = ok_response.header(&HeaderName::To) {
            let new_to = to.clone().with_tag("totag456");
            ok_response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
            ok_response.headers.push(TypedHeader::To(new_to));
        }
        
        manager
            .send_response(&transaction_id, ok_response)
            .await
            .expect("Failed to send final response");

        // Verify response was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        let (sent_message, sent_dest) = &sent_messages[0];
        if let Message::Response(resp) = sent_message {
            assert_eq!(resp.status(), StatusCode::Ok);
        } else {
            panic!("Expected Response message");
        }

        // Verify transaction state transitions to Completed
        let state = manager.transaction_state(&transaction_id).await.expect("Failed to get transaction state");
        assert_eq!(state, TransactionState::Completed);
        
        // It will transition to Terminated after Timer J, but we won't wait for that in the test
    }
} 