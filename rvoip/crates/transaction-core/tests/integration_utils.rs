// Integration test utilities for transaction-core tests
//
// This module contains shared utilities for the integration tests
// including memory transport and test message creation functions.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use std::sync::atomic::{AtomicBool, Ordering};
use async_trait::async_trait;

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, TransportEvent};
use rvoip_transaction_core::{TransactionEvent, TransactionManager, TransactionKey};
use rvoip_sip_transport::error::{Error as TransportError, Result as TransportResult};

// Mock memory transport for testing
#[derive(Debug)]
pub struct MemoryTransport {
    local_addr: SocketAddr,
    events_tx: mpsc::Sender<TransportEvent>,
    received_messages: Mutex<Vec<(Message, SocketAddr)>>,
    connected_transport: Mutex<Option<Arc<MemoryTransport>>>,
    closed: AtomicBool,
}

impl MemoryTransport {
    pub fn new(
        local_addr: SocketAddr,
        events_tx: mpsc::Sender<TransportEvent>,
    ) -> Self {
        Self {
            local_addr,
            events_tx,
            received_messages: Mutex::new(Vec::new()),
            connected_transport: Mutex::new(None),
            closed: AtomicBool::new(false),
        }
    }
    
    pub fn connect(&self, other: Arc<MemoryTransport>) {
        let mut guard = self.connected_transport.lock().unwrap();
        *guard = Some(other);
    }
    
    // Helper for receiving a message from another transport (internal use)
    pub async fn receive_message(&self, message: Message, source: SocketAddr) -> TransportResult<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(TransportError::TransportClosed);
        }
        
        // Try sending the event to registered event handler
        self.events_tx.send(TransportEvent::MessageReceived {
            message,
            source,
            destination: self.local_addr,
        }).await.map_err(|_| TransportError::ChannelClosed)?;
        
        Ok(())
    }
    
    pub fn get_received_messages(&self) -> Vec<(Message, SocketAddr)> {
        let guard = self.received_messages.lock().unwrap();
        guard.clone()
    }
}

#[async_trait]
impl Transport for MemoryTransport {
    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.local_addr)
    }
    
    async fn send_message(&self, message: Message, dest: SocketAddr) -> TransportResult<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(TransportError::TransportClosed);
        }
        
        let connected = {
            let guard = self.connected_transport.lock().unwrap();
            guard.clone()
        };
        
        if let Some(remote) = connected {
            // Record message
            {
                let mut messages = self.received_messages.lock().unwrap();
                messages.push((message.clone(), dest));
            }
            
            // Deliver to remote
            remote.receive_message(message, self.local_addr).await?;
            Ok(())
        } else {
            Err(TransportError::Other("Transport not connected".into()))
        }
    }
    
    async fn close(&self) -> TransportResult<()> {
        self.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

// A pair of connected memory transports
pub struct TransportPair {
    pub client_transport: Arc<MemoryTransport>,
    pub client_events_rx: mpsc::Receiver<TransportEvent>,
    pub server_transport: Arc<MemoryTransport>,
    pub server_events_rx: mpsc::Receiver<TransportEvent>,
}

impl TransportPair {
    pub fn new(client_addr: SocketAddr, server_addr: SocketAddr) -> Self {
        // Create channels
        let (client_events_tx, client_events_rx) = mpsc::channel(100);
        let (server_events_tx, server_events_rx) = mpsc::channel(100);
        
        // Create transports
        let client_transport = Arc::new(MemoryTransport::new(
            client_addr,
            client_events_tx,
        ));
        
        let server_transport = Arc::new(MemoryTransport::new(
            server_addr,
            server_events_tx,
        ));
        
        // Connect them
        client_transport.connect(server_transport.clone());
        server_transport.connect(client_transport.clone());
        
        Self {
            client_transport,
            client_events_rx,
            server_transport,
            server_events_rx,
        }
    }
}

// Helper function to find a specific event in an event stream
pub async fn find_event<T, F>(
    events: &mut mpsc::Receiver<TransactionEvent>,
    predicate: F,
    timeout_ms: u64,
) -> Option<T>
where
    F: Fn(&TransactionEvent) -> Option<T>,
{
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    
    while start.elapsed() < timeout {
        if let Ok(Some(event)) = tokio::time::timeout(
            Duration::from_millis(100),
            events.recv()
        ).await {
            println!("Event: {:?}", event);
            if let Some(result) = predicate(&event) {
                return Some(result);
            }
        }
        
        sleep(Duration::from_millis(10)).await;
    }
    
    None
}

// Helper to wait for a transaction to reach a specific state
pub async fn wait_for_transaction_state(
    manager: &TransactionManager,
    tx_id: &TransactionKey,
    expected_state: rvoip_transaction_core::TransactionState,
    timeout_ms: u64,
) -> bool {
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    
    while start.elapsed() < timeout {
        if let Ok(state) = manager.transaction_state(tx_id).await {
            if state == expected_state {
                return true;
            }
        }
        
        sleep(Duration::from_millis(50)).await;
    }
    
    false
}

// Set up test environment with client and server transaction managers
pub async fn setup_test_environment() -> (
    TransactionManager,  // Client manager
    mpsc::Receiver<TransactionEvent>,  // Client events
    TransactionManager,  // Server manager
    mpsc::Receiver<TransactionEvent>,  // Server events
    SocketAddr,  // Client address
    SocketAddr,  // Server address
    Arc<MemoryTransport>, // Client transport
    Arc<MemoryTransport>  // Server transport
) {
    let client_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let server_addr = SocketAddr::from_str("127.0.0.1:5061").unwrap();
    
    // Create connected transport pair
    let transport_pair = TransportPair::new(client_addr, server_addr);
    
    // Create client transaction manager
    let (client_manager, client_events_rx) = TransactionManager::new(
        transport_pair.client_transport.clone(),
        transport_pair.client_events_rx,
        None
    ).await.unwrap();
    
    // Create server transaction manager
    let (server_manager, server_events_rx) = TransactionManager::new(
        transport_pair.server_transport.clone(),
        transport_pair.server_events_rx,
        None
    ).await.unwrap();
    
    (
        client_manager,
        client_events_rx,
        server_manager,
        server_events_rx,
        client_addr,
        server_addr,
        transport_pair.client_transport.clone(),
        transport_pair.server_transport.clone(),
    )
}

// Helper function to add Via header to a request with proper branch parameter
pub fn add_via_header(request: &mut Request, addr: SocketAddr) {
    let via = Via::new(
        "SIP", 
        "2.0", 
        "UDP", 
        &addr.ip().to_string(), 
        Some(addr.port()),
        vec![Param::branch("z9hG4bK1234")]
    ).unwrap();
    request.headers.insert(0, TypedHeader::Via(via));
}

// Helper functions to create test messages

pub fn create_test_invite() -> Request {
    let mut request = Request::new(
        Method::Invite,
        Uri::from_str("sip:bob@example.com").unwrap(),
    );
    
    // Add necessary headers
    request.headers.push(TypedHeader::From(From::new(
        Address::from_str("sip:alice@example.com;tag=123").unwrap(),
    )));
    
    request.headers.push(TypedHeader::To(To::new(
        Address::from_str("sip:bob@example.com").unwrap(),
    )));
    
    request.headers.push(TypedHeader::CallId(CallId::new("test-call-id")));
    
    request.headers.push(TypedHeader::CSeq(CSeq::new(1, Method::Invite)));
    
    request.headers.push(TypedHeader::ContentLength(ContentLength::new(0)));
    
    request
}

pub fn create_test_ack(invite: &Request, response: &Response) -> Request {
    let mut ack = Request::new(
        Method::Ack,
        invite.uri().clone(),
    );
    
    // Copy relevant headers from INVITE
    for header in &invite.headers {
        match header {
            TypedHeader::From(_) | TypedHeader::CallId(_) => {
                ack.headers.push(header.clone());
            }
            _ => {}
        }
    }
    
    // Copy To header from response (it has tag)
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        ack.headers.push(TypedHeader::To(to.clone()));
    }
    
    // Copy Via from INVITE
    if let Some(TypedHeader::Via(via)) = invite.header(&HeaderName::Via) {
        ack.headers.push(TypedHeader::Via(via.clone()));
    }
    
    // Create CSeq with same number but ACK method
    if let Some(TypedHeader::CSeq(cseq)) = invite.header(&HeaderName::CSeq) {
        ack.headers.push(TypedHeader::CSeq(CSeq::new(cseq.sequence(), Method::Ack)));
    }
    
    ack.headers.push(TypedHeader::ContentLength(ContentLength::new(0)));
    
    ack
}

pub fn create_test_register() -> Request {
    let mut request = Request::new(
        Method::Register,
        Uri::from_str("sip:example.com").unwrap(),
    );
    
    // Add necessary headers
    request.headers.push(TypedHeader::From(From::new(
        Address::from_str("sip:alice@example.com;tag=456").unwrap(),
    )));
    
    request.headers.push(TypedHeader::To(To::new(
        Address::from_str("sip:alice@example.com").unwrap(),
    )));
    
    request.headers.push(TypedHeader::CallId(CallId::new("test-register-id")));
    
    request.headers.push(TypedHeader::CSeq(CSeq::new(1, Method::Register)));
    
    request.headers.push(TypedHeader::ContentLength(ContentLength::new(0)));
    
    request
} 