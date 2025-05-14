use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::test;

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, TransportEvent, Error as TransportError};
use rvoip_sip_transport::error::Result as TransportResult;
use rvoip_transaction_core::prelude::*;

// A simple mock transport for testing purposes
#[derive(Debug)]
struct MockTransport {
    local_addr: SocketAddr,
}

impl MockTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self { local_addr }
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send_message(&self, _message: Message, _destination: SocketAddr) -> TransportResult<()> {
        // Just pretend to send
        Ok(())
    }

    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> TransportResult<()> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

#[test]
async fn test_transaction_creation() {
    // Setup
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let remote_addr = SocketAddr::from_str("192.168.1.1:5060").unwrap();
    let transport = Arc::new(MockTransport::new(local_addr));
    
    // Create channels for transaction events
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Create transaction manager
    let (manager, mut events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(100)
    ).await.unwrap();
    
    // Create a request
    let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("tag123"))
        .to("Registrar", "sip:registrar.example.com", None)
        .call_id("call123@atlanta.com")
        .cseq(1)
        .build();
    
    // Create a client transaction
    let tx_id = manager.create_client_transaction(request, remote_addr).await.unwrap();
    
    // Verify transaction exists
    assert!(manager.transaction_exists(&tx_id).await);
    
    // Verify transaction kind
    assert_eq!(manager.transaction_kind(&tx_id).await.unwrap(), TransactionKind::NonInviteClient);
    
    // Initiate the transaction
    manager.send_request(&tx_id).await.unwrap();
    
    // Verify transaction state
    assert_eq!(manager.transaction_state(&tx_id).await.unwrap(), TransactionState::Trying);
    
    println!("Transaction test passed!");
} 