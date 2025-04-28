use std::net::SocketAddr;
use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::sleep;
use async_trait::async_trait;

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, TransportEvent, Error as TransportError};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState};

// In-memory transport for local testing without any actual network I/O
// This allows us to simulate direct client-server communication
#[derive(Debug, Clone)]
pub struct MemoryTransport {
    local_addr: SocketAddr,
    outgoing_tx: Sender<(Message, SocketAddr)>,
    transport_tx: Sender<TransportEvent>,
    sent_messages: Arc<tokio::sync::Mutex<Vec<(Message, SocketAddr)>>>,
    closed: Arc<tokio::sync::Mutex<bool>>,
}

impl MemoryTransport {
    pub fn new(
        local_addr: SocketAddr,
        outgoing_tx: Sender<(Message, SocketAddr)>,
        transport_tx: Sender<TransportEvent>,
    ) -> Self {
        Self {
            local_addr,
            outgoing_tx,
            transport_tx,
            sent_messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            closed: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    // Simulate receiving a message from the network
    pub async fn receive_message(&self, message: Message, source: SocketAddr) -> std::result::Result<(), TransportError> {
        if *self.closed.lock().await {
            return Err(TransportError::Other("Transport closed".into()));
        }

        self.transport_tx.send(TransportEvent::MessageReceived {
            message,
            source,
            destination: self.local_addr,
        }).await.map_err(|_| TransportError::Other("Failed to send event".into()))
    }

    pub async fn get_sent_messages(&self) -> Vec<(Message, SocketAddr)> {
        self.sent_messages.lock().await.clone()
    }

    pub async fn clear_sent_messages(&self) {
        self.sent_messages.lock().await.clear();
    }
}

#[async_trait]
impl Transport for MemoryTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), TransportError> {
        if *self.closed.lock().await {
            return Err(TransportError::Other("Transport closed".into()));
        }

        // Record the message
        self.sent_messages.lock().await.push((message.clone(), destination));
        
        // Forward to the recipient's transport
        self.outgoing_tx.send((message, destination)).await
            .map_err(|_| TransportError::Other("Failed to forward message".into()))
    }

    fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> std::result::Result<(), TransportError> {
        let mut closed = self.closed.lock().await;
        *closed = true;
        Ok(())
    }

    fn is_closed(&self) -> bool {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let closed_ref = self.closed.clone();
                let result = handle.block_on(async move {
                    *closed_ref.lock().await
                });
                result
            }
            Err(_) => false,
        }
    }
}

// Pair of connected transports for client and server
pub struct TransportPair {
    pub client_transport: Arc<MemoryTransport>,
    pub server_transport: Arc<MemoryTransport>,
    pub client_events_rx: Receiver<TransportEvent>,
    pub server_events_rx: Receiver<TransportEvent>,
}

impl TransportPair {
    pub fn new(client_addr: SocketAddr, server_addr: SocketAddr) -> Self {
        // Channels for relaying messages between transports
        let (client_to_server_tx, mut client_to_server_rx) = mpsc::channel(100);
        let (server_to_client_tx, mut server_to_client_rx) = mpsc::channel(100);
        
        // Channels for transport events
        let (client_transport_tx, client_events_rx) = mpsc::channel(100);
        let (server_transport_tx, server_events_rx) = mpsc::channel(100);
        
        // Create transports
        let client_transport = Arc::new(MemoryTransport::new(
            client_addr,
            client_to_server_tx,
            client_transport_tx,
        ));
        
        let server_transport = Arc::new(MemoryTransport::new(
            server_addr,
            server_to_client_tx,
            server_transport_tx,
        ));
        
        // Set up message forwarding for client -> server
        let server_transport_for_task = server_transport.clone();
        tokio::spawn(async move {
            while let Some((message, destination)) = client_to_server_rx.recv().await {
                // Only forward if destination matches server address
                if destination == server_transport_for_task.local_addr().unwrap() {
                    let _ = server_transport_for_task.receive_message(message, client_addr).await;
                }
            }
        });
        
        // Set up message forwarding for server -> client
        let client_transport_for_task = client_transport.clone();
        tokio::spawn(async move {
            while let Some((message, destination)) = server_to_client_rx.recv().await {
                // Only forward if destination matches client address
                if destination == client_transport_for_task.local_addr().unwrap() {
                    let _ = client_transport_for_task.receive_message(message, server_addr).await;
                }
            }
        });
        
        Self {
            client_transport,
            server_transport,
            client_events_rx,
            server_events_rx,
        }
    }
}

// Helper functions for creating test messages
pub fn create_test_invite() -> Request {
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

pub fn create_test_register() -> Request {
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

// Helper to create an ACK request for a given INVITE and response
pub fn create_test_ack(invite_request: &Request, response: &Response) -> Request {
    let uri = invite_request.uri().to_string();
    
    let mut builder = RequestBuilder::new(Method::Ack, &uri).unwrap();
    
    // Copy essential headers from the INVITE
    if let Some(header) = invite_request.header(&HeaderName::From) {
        builder = builder.header(header.clone());
    }

    // But use the To header from the response (may have a tag)
    if let Some(header) = response.header(&HeaderName::To) {
        builder = builder.header(header.clone());
    }

    if let Some(header) = invite_request.header(&HeaderName::CallId) {
        builder = builder.header(header.clone());
    }
    
    // Via headers
    if let Some(header) = invite_request.header(&HeaderName::Via) {
        builder = builder.header(header.clone());
    }
    
    // Create CSeq header with same sequence but ACK method
    if let Some(TypedHeader::CSeq(cseq)) = invite_request.header(&HeaderName::CSeq) {
        builder = builder.header(TypedHeader::CSeq(CSeq::new(cseq.sequence(), Method::Ack)));
    }
    
    builder
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

// Search for specific event types in an event receiver
pub async fn find_event<F, T>(events_rx: &mut Receiver<TransactionEvent>, predicate: F, timeout_ms: u64) -> Option<T>
where
    F: Fn(&TransactionEvent) -> Option<T>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(
            deadline - tokio::time::Instant::now(),
            events_rx.recv()
        ).await {
            Ok(Some(event)) => {
                if let Some(result) = predicate(&event) {
                    return Some(result);
                }
            }
            _ => break,
        }
    }
    
    None
}

// Waits for a particular transaction state
pub async fn wait_for_transaction_state(
    manager: &TransactionManager, 
    transaction_id: &str, 
    target_state: TransactionState,
    timeout_ms: u64
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let transaction_id_str = transaction_id.to_string();
    
    while tokio::time::Instant::now() < deadline {
        match manager.transaction_state(&transaction_id_str).await {
            Ok(state) if state == target_state => return true,
            Ok(_) => {
                sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(_) => return false, // Transaction not found or other error
        }
    }
    
    false // Timeout
} 