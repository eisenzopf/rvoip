use std::collections::VecDeque;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::{timeout, Duration};
use async_trait::async_trait;
use uuid::Uuid;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_transport::{Transport, Error as TransportError, TransportEvent};

use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
use rvoip_transaction_core::transaction::TransactionState;
use rvoip_transaction_core::timer::TimerSettings;
use rvoip_transaction_core::builders::{client_quick, server_quick};

/// Enhanced mock transport for integration testing that tracks sent messages and allows
/// injecting transport events.
#[derive(Debug, Clone)]
pub struct MockTransport {
    /// Queue of messages sent through this transport
    pub sent_messages: Arc<Mutex<VecDeque<(Message, SocketAddr)>>>,
    /// Channel for injecting transport events
    pub event_tx: mpsc::Sender<TransportEvent>,
    /// Local transport address
    local_addr: SocketAddr,
    /// Flag to indicate if transport is closed
    is_closed: Arc<Mutex<bool>>,
    /// Notifier for when a message is sent
    message_sent_notifier: Arc<Notify>,
    /// Optional linked transport for auto-forwarding messages (simulating network)
    linked_transport: Arc<Mutex<Option<(Arc<MockTransport>, SocketAddr)>>>,
}

impl MockTransport {
    /// Create a new mock transport
    pub fn new(event_tx: mpsc::Sender<TransportEvent>, local_addr_str: &str) -> Self {
        Self {
            sent_messages: Arc::new(Mutex::new(VecDeque::new())),
            event_tx,
            local_addr: SocketAddr::from_str(local_addr_str)
                .unwrap_or_else(|_| SocketAddr::from_str("127.0.0.1:5060").unwrap()),
            is_closed: Arc::new(Mutex::new(false)),
            message_sent_notifier: Arc::new(Notify::new()),
            linked_transport: Arc::new(Mutex::new(None)),
        }
    }
    
    /// Link this transport to another transport for automatic message forwarding
    /// This creates a virtual network connection between the two transports
    pub async fn link_with(&self, other: Arc<MockTransport>, other_addr: SocketAddr) {
        let mut linked = self.linked_transport.lock().await;
        *linked = Some((other, other_addr));
    }

    /// Retrieve the next sent message, if any
    pub async fn get_sent_message(&self) -> Option<(Message, SocketAddr)> {
        let mut queue = self.sent_messages.lock().await;
        queue.pop_front()
    }
    
    /// Clear all messages from the queue
    pub async fn clear_message_queue(&self) {
        let mut queue = self.sent_messages.lock().await;
        queue.clear();
        println!("Cleared transport message queue");
    }
    
    /// Get all sent messages and clear the queue
    pub async fn get_all_sent_messages(&self) -> Vec<(Message, SocketAddr)> {
        let mut queue = self.sent_messages.lock().await;
        let messages: Vec<(Message, SocketAddr)> = queue.drain(..).collect();
        println!("Retrieved {} messages from queue", messages.len());
        messages
    }
    
    /// Find a message with a specific method and return it
    pub async fn find_message_with_method(&self, method: Method) -> Option<(Message, SocketAddr)> {
        let queue = self.sent_messages.lock().await;
        for msg in queue.iter() {
            if let (Message::Request(req), addr) = msg {
                if req.method() == method {
                    return Some((Message::Request(req.clone()), *addr));
                }
            }
        }
        None
    }

    /// Check if a message with the given method was sent
    pub async fn was_message_sent(&self, method: Option<Method>) -> bool {
        let queue = self.sent_messages.lock().await;
        queue.iter().any(|(msg, _)| msg.method() == method)
    }

    /// Wait for a message to be sent, with timeout
    pub async fn wait_for_message_sent(&self, duration: Duration) -> std::result::Result<(), tokio::time::error::Elapsed> {
        timeout(duration, self.message_sent_notifier.notified()).await
    }

    /// Count the sent messages
    pub async fn sent_message_count(&self) -> usize {
        let queue = self.sent_messages.lock().await;
        queue.len()
    }

    /// Inject an event into the transport event channel
    pub async fn inject_event(&self, event: TransportEvent) -> std::result::Result<(), String> {
        self.event_tx.send(event).await.map_err(|e| e.to_string())
    }
    
    /// Forwards a message to the linked transport if one exists
    async fn forward_message(&self, message: Message, source: SocketAddr) -> std::result::Result<(), String> {
        let linked = self.linked_transport.lock().await;
        
        if let Some((other_transport, other_addr)) = linked.as_ref() {
            // Forward the message to the linked transport
            let event = TransportEvent::MessageReceived {
                message: message.clone(),
                source,
                destination: *other_addr,
            };
            
            other_transport.inject_event(event).await?;
            println!("Message automatically forwarded from {} to {}", source, other_addr);
        }
        
        Ok(())
    }
}

#[async_trait]
impl Transport for MockTransport {
    fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
        Ok(self.local_addr)
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), TransportError> {
        // Check if transport is closed
        if *self.is_closed.lock().await {
            return Err(TransportError::TransportClosed);
        }
        
        // Store the message in our sent queue
        {
            let mut queue = self.sent_messages.lock().await;
            queue.push_back((message.clone(), destination));
            self.message_sent_notifier.notify_one();
        }
        
        // Debug message type
        let msg_type = match &message {
            Message::Request(req) => format!("Request ({})", req.method()),
            Message::Response(resp) => format!("Response ({})", resp.status()),
        };
        println!("Transport {} sent {} to {}", self.local_addr, msg_type, destination);
        
        // Forward the message to the linked transport (if any)
        if let Err(e) = self.forward_message(message, self.local_addr).await {
            println!("Warning: Failed to forward message: {}", e);
            // Don't return an error here, we still sent the message successfully
        }
        
        Ok(())
    }

    async fn close(&self) -> std::result::Result<(), TransportError> {
        let mut closed = self.is_closed.lock().await;
        *closed = true;
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.is_closed.try_lock().map_or(false, |guard| *guard)
    }
}

/// Test environment containing all necessary components for transaction testing
pub struct TestEnvironment {
    pub client_manager: TransactionManager,
    pub server_manager: TransactionManager,
    // Only used internally, not needed to expose as struct fields
    client_events_rx: mpsc::Receiver<TransactionEvent>,
    server_events_rx: mpsc::Receiver<TransactionEvent>,
    pub client_transport: Arc<MockTransport>,
    pub server_transport: Arc<MockTransport>,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
}

impl TestEnvironment {
    /// Create a new test environment with two transaction managers (client and server)
    pub async fn new() -> Self {
        // Set test environment variable
        std::env::set_var("RVOIP_TEST", "1");
        
        // Client side setup
        let client_addr_str = "127.0.0.1:5070";
        let client_addr = SocketAddr::from_str(client_addr_str).unwrap();
        let (client_event_injector_tx, client_event_injector_rx) = mpsc::channel(100);
        let client_transport = Arc::new(MockTransport::new(client_event_injector_tx, client_addr_str));
        
        // Server side setup
        let server_addr_str = "127.0.0.1:5080";
        let server_addr = SocketAddr::from_str(server_addr_str).unwrap();
        let (server_event_injector_tx, server_event_injector_rx) = mpsc::channel(100);
        let server_transport = Arc::new(MockTransport::new(server_event_injector_tx, server_addr_str));
        
        // Link the transports together to simulate a network
        client_transport.link_with(server_transport.clone(), server_addr).await;
        server_transport.link_with(client_transport.clone(), client_addr).await;
        
        // Create fast timer settings for testing
        let timer_settings = TimerSettings {
            t1: Duration::from_millis(100),        // Slightly longer T1 (normally 500ms)
            t2: Duration::from_millis(400),        // Longer T2 (normally 4s)
            t4: Duration::from_secs(5),            // Default T4 value
            timer_100_interval: Duration::from_millis(200), // Default Timer 100 interval
            transaction_timeout: Duration::from_millis(20000), // Much longer timeout (20 seconds)
            wait_time_j: Duration::from_millis(120),  // Longer Timer J
            wait_time_k: Duration::from_millis(120),  // Longer Timer K
            wait_time_h: Duration::from_millis(120),  // Longer Timer H
            wait_time_i: Duration::from_millis(120),  // Longer Timer I
            wait_time_d: Duration::from_millis(120),  // Longer Timer D
        };
        
        // Create transaction managers
        let (client_manager, client_events_rx) = TransactionManager::new_with_config(
            client_transport.clone() as Arc<dyn Transport>,
            client_event_injector_rx,
            Some(100),
            Some(timer_settings.clone()),
        ).await.expect("Failed to create client transaction manager");
        
        let (server_manager, server_events_rx) = TransactionManager::new_with_config(
            server_transport.clone() as Arc<dyn Transport>,
            server_event_injector_rx,
            Some(100),
            Some(timer_settings),
        ).await.expect("Failed to create server transaction manager");
        
        Self {
            client_manager,
            server_manager,
            client_events_rx,
            server_events_rx,
            client_transport,
            server_transport,
            client_addr,
            server_addr,
        }
    }
    
    /// Process a request for a server transaction using the public API
    /// This is the preferred method to process ACK and other requests for server transactions
    pub async fn process_server_request(&self, tx_id: &TransactionKey, request: Request) -> std::result::Result<(), String> {
        self.server_manager.process_request(tx_id, request).await
            .map_err(|e| e.to_string())
    }
    
    /// Create a SIP request of the specified method
    pub fn create_request(&self, method: Method, to_uri: &str) -> Request {
        match method {
            Method::Invite => {
                // Use the new INVITE builder
                let from_uri = format!("sip:client@{}", self.client_addr);
                client_quick::invite(&from_uri, to_uri, self.client_addr, None)
                    .expect("Failed to create INVITE request")
            },
            Method::Bye => {
                // For BYE, we need dialog information - fallback to manual construction for now
                // In real usage, dialog-core would provide this information
                let mut builder = SimpleRequestBuilder::new(method, to_uri)
                    .expect("Failed to create request builder");
                    
                let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().replace("-", ""));
                let call_id = format!("test-call-{}", Uuid::new_v4().to_string().replace("-", ""));
                let from_tag = format!("from-tag-{}", Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>());
                let to_tag = format!("to-tag-{}", Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>());
                
                let from_uri = format!("sip:client@{}", self.client_addr);
                
                builder = builder
                    .from("Client UA", &from_uri, Some(&from_tag))
                    .to("Server UA", to_uri, Some(&to_tag))
                    .call_id(&call_id)
                    .cseq(2) // BYE typically has a higher CSeq
                    .via(&self.client_addr.to_string(), "UDP", Some(&branch))
                    .max_forwards(70)
                    .header(TypedHeader::ContentLength(ContentLength::new(0)));
                    
                builder.build()
            },
            Method::Register => {
                // Use the new REGISTER builder
                let user_uri = format!("sip:client@{}", self.client_addr.ip());
                client_quick::register(to_uri, &user_uri, "Client UA", self.client_addr, Some(3600))
                    .expect("Failed to create REGISTER request")
            },
            _ => {
                // For other methods, use manual construction
                let mut builder = SimpleRequestBuilder::new(method, to_uri)
                    .expect("Failed to create request builder");
                    
                let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().replace("-", ""));
                let call_id = format!("test-call-{}", Uuid::new_v4().to_string().replace("-", ""));
                let from_tag = format!("from-tag-{}", Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>());
                
                let from_uri = format!("sip:client@{}", self.client_addr);
                
                builder = builder
                    .from("Client UA", &from_uri, Some(&from_tag))
                    .to("Server UA", to_uri, None)
                    .call_id(&call_id)
                    .cseq(1)
                    .via(&self.client_addr.to_string(), "UDP", Some(&branch))
                    .max_forwards(70)
                    .header(TypedHeader::ContentLength(ContentLength::new(0)));
                    
                builder.build()
            }
        }
    }
    
    /// Create a response for the given request
    pub fn create_response(&self, request: &Request, status_code: StatusCode, reason: Option<&str>) -> Response {
        match status_code {
            StatusCode::Trying => {
                // Use the new server quick builders
                server_quick::trying(request)
                    .expect("Failed to create 100 Trying response")
            },
            StatusCode::Ringing => {
                // Use the new server quick builders
                let contact = format!("sip:server@{}", self.server_addr);
                server_quick::ringing(request, Some(contact))
                    .expect("Failed to create 180 Ringing response")
            },
            StatusCode::Ok => {
                // Use the new server quick builders for different request types
                match request.method() {
                    Method::Invite => {
                        let contact = format!("sip:server@{}", self.server_addr);
                        server_quick::ok_invite(request, None, contact)
                            .expect("Failed to create 200 OK for INVITE")
                    },
                    Method::Bye => {
                        server_quick::ok_bye(request)
                            .expect("Failed to create 200 OK for BYE")
                    },
                    Method::Register => {
                        let contact = format!("sip:client@{}", self.client_addr);
                        server_quick::ok_register(request, 3600, vec![contact])
                            .expect("Failed to create 200 OK for REGISTER")
                    },
                    Method::Options => {
                        let allow_methods = vec![Method::Invite, Method::Ack, Method::Bye, Method::Cancel, Method::Options];
                        server_quick::ok_options(request, allow_methods)
                            .expect("Failed to create 200 OK for OPTIONS")
                    },
                    _ => {
                        // Fallback to manual construction for other methods
                        SimpleResponseBuilder::response_from_request(request, status_code, reason)
                            .header(TypedHeader::ContentLength(ContentLength::new(0)))
                            .build()
                    }
                }
            },
            StatusCode::BusyHere => {
                server_quick::busy_here(request)
                    .expect("Failed to create 486 Busy Here response")
            },
            StatusCode::RequestTerminated => {
                server_quick::request_terminated(request)
                    .expect("Failed to create 487 Request Terminated response")
            },
            StatusCode::NotFound => {
                server_quick::not_found(request)
                    .expect("Failed to create 404 Not Found response")
            },
            StatusCode::ServerInternalError => {
                server_quick::server_error(request, reason.map(|s| s.to_string()))
                    .expect("Failed to create 500 Server Internal Error response")
            },
            _ => {
                // Fallback to manual construction for other status codes
                SimpleResponseBuilder::response_from_request(request, status_code, reason)
                    .header(TypedHeader::ContentLength(ContentLength::new(0)))
                    .build()
            }
        }
    }
    
    /// Create a CANCEL request for an INVITE request
    /// 
    /// According to RFC 3261 Section 9.1:
    /// - CANCEL has the same Call-ID, To, From as the INVITE
    /// - Same CSeq sequence number, but different method
    /// - Same Route header set as the INVITE (if any)
    /// - Same Request-URI as the INVITE
    pub fn create_cancel_request(&self, invite_request: &Request) -> Request {
        // Verify this is an INVITE we're canceling
        assert_eq!(invite_request.method(), Method::Invite, "Can only cancel INVITE requests");
        
        // Extract needed headers from INVITE
        let request_uri = invite_request.uri().clone();
        let from = invite_request.from().unwrap().clone();
        let to = invite_request.to().unwrap().clone();
        let call_id = invite_request.call_id().unwrap().clone();
        let cseq_num = invite_request.cseq().unwrap().seq;
        
        // Get route set if it exists
        let mut route_headers: Vec<TypedHeader> = vec![];
        
        // We need to get the Route header if it exists
        if let Some(route_header) = invite_request.header(&HeaderName::Route) {
            route_headers.push(route_header.clone());
        }
        
        // Generate a new branch parameter (CANCEL creates a new transaction)
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().replace("-", ""));
        
        // Create the CANCEL request builder
        let mut builder = SimpleRequestBuilder::new(Method::Cancel, &request_uri.to_string())
            .expect("Failed to create CANCEL builder")
            .header(TypedHeader::From(from))
            .header(TypedHeader::To(to))
            .header(TypedHeader::CallId(call_id))
            .header(TypedHeader::CSeq(CSeq::new(cseq_num, Method::Cancel)))
            .via(&self.client_addr.to_string(), "UDP", Some(&branch))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)));
            
        // Add route headers if any
        for route in route_headers {
            builder = builder.header(route);
        }
        
        builder.build()
    }
    
    /// Inject a request message from client to server
    pub async fn inject_request_c2s(&self, request: Request) -> std::result::Result<(), String> {
        // Clone the request to avoid ownership issues
        let cloned_request = request.clone();
        
        println!("Injecting request with method {}", cloned_request.method());
        
        self.server_transport.inject_event(TransportEvent::MessageReceived {
            message: Message::Request(cloned_request),
            source: self.client_addr,
            destination: self.server_addr,
        }).await
    }
    
    /// Inject a response message from server to client
    pub async fn inject_response_s2c(&self, response: Response) -> std::result::Result<(), String> {
        self.client_transport.inject_event(TransportEvent::MessageReceived {
            message: Message::Response(response),
            source: self.server_addr,
            destination: self.client_addr,
        }).await
    }
    
    /// Wait for a specific event type from the client
    pub async fn wait_for_client_event<T, F>(&mut self, timeout_duration: Duration, matcher: F) -> Option<T> 
    where 
        F: Fn(&TransactionEvent) -> Option<T>,
    {
        let timeout_fut = tokio::time::sleep(timeout_duration);
        tokio::pin!(timeout_fut);
        
        loop {
            tokio::select! {
                Some(event) = self.client_events_rx.recv() => {
                    if let Some(result) = matcher(&event) {
                        return Some(result);
                    }
                }
                _ = &mut timeout_fut => {
                    return None;
                }
            }
        }
    }
    
    /// Wait for a specific event type from the server
    pub async fn wait_for_server_event<T, F>(&mut self, timeout_duration: Duration, matcher: F) -> Option<T> 
    where 
        F: Fn(&TransactionEvent) -> Option<T>,
    {
        let timeout_fut = tokio::time::sleep(timeout_duration);
        tokio::pin!(timeout_fut);
        
        loop {
            tokio::select! {
                Some(event) = self.server_events_rx.recv() => {
                    if let Some(result) = matcher(&event) {
                        return Some(result);
                    }
                }
                _ = &mut timeout_fut => {
                    return None;
                }
            }
        }
    }
    
    /// Shutdown both transaction managers
    pub async fn shutdown(&self) {
        self.client_manager.shutdown().await;
        self.server_manager.shutdown().await;
        
        // Reset environment variable
        std::env::remove_var("RVOIP_TEST");
    }
}

/// Match an InviteRequest event
pub fn match_invite_request(event: &TransactionEvent) -> Option<(TransactionKey, Request, SocketAddr)> {
    if let TransactionEvent::InviteRequest { transaction_id, request, source, .. } = event {
        Some((transaction_id.clone(), request.clone(), *source))
    } else {
        None
    }
}

/// Match a NonInviteRequest event
pub fn match_non_invite_request(event: &TransactionEvent) -> Option<(TransactionKey, Request, SocketAddr)> {
    if let TransactionEvent::NonInviteRequest { transaction_id, request, source, .. } = event {
        Some((transaction_id.clone(), request.clone(), *source))
    } else {
        None
    }
}

/// Match a ProvisionalResponse event
pub fn match_provisional_response(event: &TransactionEvent) -> Option<(TransactionKey, Response)> {
    if let TransactionEvent::ProvisionalResponse { transaction_id, response, .. } = event {
        Some((transaction_id.clone(), response.clone()))
    } else {
        None
    }
}

/// Match a SuccessResponse event
pub fn match_success_response(event: &TransactionEvent) -> Option<(TransactionKey, Response)> {
    if let TransactionEvent::SuccessResponse { transaction_id, response, .. } = event {
        Some((transaction_id.clone(), response.clone()))
    } else {
        None
    }
}

/// Match a FailureResponse event
pub fn match_failure_response(event: &TransactionEvent) -> Option<(TransactionKey, Response)> {
    if let TransactionEvent::FailureResponse { transaction_id, response, .. } = event {
        Some((transaction_id.clone(), response.clone()))
    } else {
        None
    }
}

/// Match an AckReceived event
pub fn match_ack_received(event: &TransactionEvent) -> Option<(TransactionKey, Request)> {
    if let TransactionEvent::AckReceived { transaction_id, request, .. } = event {
        Some((transaction_id.clone(), request.clone()))
    } else {
        None
    }
}

/// Match a CancelReceived event
pub fn match_cancel_received(event: &TransactionEvent) -> Option<(TransactionKey, Request)> {
    if let TransactionEvent::CancelReceived { transaction_id, cancel_request, .. } = event {
        Some((transaction_id.clone(), cancel_request.clone()))
    } else {
        None
    }
}

/// Match a state changed event
pub fn match_state_changed(event: &TransactionEvent) -> Option<(TransactionKey, TransactionState, TransactionState)> {
    if let TransactionEvent::StateChanged { transaction_id, previous_state, new_state, .. } = event {
        Some((transaction_id.clone(), *previous_state, *new_state))
    } else {
        None
    }
}

/// Match a transaction terminated event
pub fn match_transaction_terminated(event: &TransactionEvent) -> Option<TransactionKey> {
    if let TransactionEvent::TransactionTerminated { transaction_id, .. } = event {
        Some(transaction_id.clone())
    } else {
        None
    }
}

/// Match a transaction timeout event
pub fn match_transaction_timeout(event: &TransactionEvent) -> Option<TransactionKey> {
    if let TransactionEvent::TransactionTimeout { transaction_id, .. } = event {
        Some(transaction_id.clone())
    } else {
        None
    }
}

/// Match a timer triggered event
pub fn match_timer_triggered(event: &TransactionEvent) -> Option<(TransactionKey, String)> {
    if let TransactionEvent::TimerTriggered { transaction_id, timer, .. } = event {
        Some((transaction_id.clone(), timer.clone()))
    } else {
        None
    }
}

/// Create an ACK request for a 2xx response
pub fn create_ack_for_response(response: &Response, original_request: &Request, client_addr: SocketAddr) -> Request {
    // Get the destination URI from the Contact header of the response if present,
    // otherwise use the To header URI
    let target_uri = response.header(&HeaderName::Contact)
        .and_then(|h| {
            if let TypedHeader::Contact(contact) = h {
                contact.addresses().next().map(|addr| addr.uri.clone())
            } else {
                None
            }
        })
        .or_else(|| {
            response.to().map(|to| to.address().uri.clone())
        })
        .unwrap_or_else(|| original_request.uri().clone());
        
    // Get headers from original request and response
    let from = original_request.from().unwrap().clone();
    let to = response.to().unwrap().clone();
    let call_id = original_request.call_id().unwrap().clone();
    let cseq_num = original_request.cseq().unwrap().seq;
    
    // Generate a new branch parameter
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().replace("-", ""));
    
    // Build the ACK request
    SimpleRequestBuilder::new(Method::Ack, &target_uri.to_string())
        .expect("Failed to create ACK builder")
        .header(TypedHeader::From(from))
        .header(TypedHeader::To(to))
        .header(TypedHeader::CallId(call_id))
        .header(TypedHeader::CSeq(CSeq::new(cseq_num, Method::Ack)))
        .via(&client_addr.to_string(), "UDP", Some(&branch))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
} 