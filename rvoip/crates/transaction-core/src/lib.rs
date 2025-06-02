pub mod client;
pub mod error;
pub mod server;
pub mod manager;
pub mod transaction;
pub mod timer;
pub mod utils;
pub mod method;
pub mod transport;

// Re-export core types
pub use error::{Error, Result};
pub use manager::TransactionManager;
pub use transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
pub use client::{ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction, TransactionExt as ClientTransactionExt};
pub use server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction, TransactionExt as ServerTransactionExt};
pub use timer::{Timer, TimerManager, TimerFactory, TimerSettings, TimerType};
pub use transport::TransportManager;
pub use rvoip_sip_transport::transport::TransportType;
pub use transport::{
    TransportCapabilities, TransportInfo, NetworkInfoForSdp, 
    WebSocketStatus, TransportCapabilitiesExt
};

/// Convenient re-exports for request and response builders
pub mod builders {
    /// Client-side request builders for common SIP operations
    pub use crate::client::builders::{
        InviteBuilder, ByeBuilder, RegisterBuilder,
        quick as client_quick
    };
    
    /// Server-side response builders for common SIP operations
    pub use crate::server::builders::{
        ResponseBuilder, InviteResponseBuilder, RegisterResponseBuilder,
        quick as server_quick
    };
}

/// # SIP Transaction Layer
/// 
/// This crate implements the SIP transaction layer as defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261),
/// providing reliable message delivery and state management for SIP request-response exchanges.
///
/// ## Purpose and Responsibilities
/// 
/// The transaction layer in a SIP stack has several key responsibilities:
/// 
/// 1. **Message Reliability**: Ensuring messages are delivered reliably over unreliable transports (e.g., UDP)
///    by handling retransmissions and timeouts.
/// 
/// 2. **State Management**: Implementing the state machines defined in RFC 3261 for handling different types
///    of SIP transactions (INVITE client/server, non-INVITE client/server).
/// 
/// 3. **Transaction Matching**: Correctly matching requests and responses to their corresponding transactions
///    based on SIP headers (Via, CSeq, etc.) according to the rules defined in RFC 3261 Section 17.1.3 and 17.2.3.
/// 
/// 4. **Timer Management**: Managing various timers required by the SIP protocol for retransmissions,
///    transaction timeouts, and cleanup operations.
/// 
/// 5. **ACK Handling**: Special processing for ACK messages, which are treated differently depending on
///    whether they acknowledge 2xx or non-2xx responses.
/// 
/// 6. **Transaction User Interface**: Providing a clean API for the Transaction User (TU) layer to
///    send requests, receive responses, and handle events.
///
/// ## Architecture
///
/// The transaction layer sits between the transport layer and the transaction user (TU) layer in the SIP stack:
///
/// ```text
/// +--------------------------------------+
/// |        Transaction User (TU)         |
/// |  (dialog management, call control)   |
/// +--------------------------------------+
///                    |
///                    v
/// +--------------------------------------+
/// |        Transaction Layer             |
/// |  (this crate: transaction-core)      |
/// +--------------------------------------+
///                    |
///                    v
/// +--------------------------------------+
/// |        Transport Layer               |
/// |  (UDP, TCP, TLS, WebSocket)          |
/// +--------------------------------------+
///                    |
///                    v
/// +--------------------------------------+
/// |          Network                     |
/// +--------------------------------------+
/// ```
///
/// ### Transaction vs. Dialog Layer
/// 
/// It's important to understand the separation between the transaction layer and dialog layer in SIP:
/// 
/// - **Transaction Layer** (this library): Handles individual request-response exchanges and ensures
///   reliable message delivery. Transactions are short-lived with well-defined lifecycles.
/// 
/// - **Dialog Layer** (implemented in session-core): Maintains long-lived application state across
///   multiple transactions. Dialogs track the relationship between endpoints using Call-ID, tags,
///   and sequence numbers.
/// 
/// This separation allows the transaction layer to focus solely on message reliability and state management,
/// while the dialog layer handles higher-level application logic.
///
/// ### Relationship to Other Libraries
/// 
/// In the RVOIP project, transaction-core interacts with:
/// 
/// - **sip-core**: Provides SIP message parsing, construction, and basic types
/// - **sip-transport**: Handles the actual sending and receiving of SIP messages
/// - **session-core**: Consumer of transaction services, implementing dialog management
/// 
/// The transaction layer isolates the transport details from higher layers while providing 
/// transaction state management that higher layers don't need to implement.
///
/// #### Transaction Core and Session Core Relationship
/// 
/// The `transaction-core` and `session-core` libraries are designed to work together while maintaining
/// clear separation of concerns:
/// 
/// - **transaction-core** handles individual message exchanges with reliability and retransmission
///   according to RFC 3261 Section 17, ensuring messages are delivered and properly tracked.
/// 
/// - **session-core** builds on top of transaction-core to implement dialog management (RFC 3261 Section 12)
///   and higher-level session concepts like calls and registrations.
/// 
/// Typically, an application would:
/// 
/// 1. Create a `TransactionManager` from transaction-core
/// 2. Pass it to a `SessionManager` from session-core 
/// 3. Work primarily with the SessionManager's higher-level API
/// 4. Receive events from both layers (transaction events and session events)
/// 
/// This layered architecture allows each component to focus on its specific responsibilities
/// while providing clean integration points between layers.
///
/// ## Library Organization
/// 
/// The codebase is organized into several modules:
/// 
/// - **manager**: Contains the `TransactionManager`, the main entry point for the library
/// - **client**: Implements client transaction types (INVITE and non-INVITE)
/// - **server**: Implements server transaction types (INVITE and non-INVITE)
/// - **transaction**: Defines common transaction traits, states, and events
/// - **timer**: Implements timer management for retransmissions and timeouts
/// - **method**: Handles special method-specific behavior (CANCEL, ACK, etc.)
/// - **utils**: Utility functions for transaction processing
/// - **error**: Error types and results for the library
/// 
/// Most users will primarily interact with the `TransactionManager` class, which provides
/// the public API for creating and managing transactions.
///
/// ## Key Components
///
/// * [`TransactionManager`]: Central coordinator for all transactions. Responsible for:
///   - Creating and tracking transactions
///   - Routing incoming messages to the right transaction
///   - Handling "stray" messages that don't match any transaction
///   - Providing a unified interface to the Transaction User
///
/// * Transaction Types:
///   * [`ClientInviteTransaction`]: Implements RFC 3261 section 17.1.1 state machine
///   * [`ClientNonInviteTransaction`]: Implements RFC 3261 section 17.1.2 state machine
///   * [`ServerInviteTransaction`]: Implements RFC 3261 section 17.2.1 state machine
///   * [`ServerNonInviteTransaction`]: Implements RFC 3261 section 17.2.2 state machine
///
/// * Timer Management:
///   * [`TimerManager`]: Manages transaction timers as per RFC 3261
///   * [`TimerFactory`]: Creates appropriate timers for different transaction types
///   * [`TimerSettings`]: Configures timer durations (T1, T2, etc.)
///
/// ## Transaction Matching
/// 
/// The transaction layer needs to match incoming messages to the right transaction. According to RFC 3261:
/// 
/// - **For Responses**: Matched using the branch parameter, sent-by value in the top Via header, 
///   and CSeq method.
/// 
/// - **For ACK to non-2xx**: Matched to the original INVITE transaction. The branch parameter and other
///   identifiers remain the same as the INVITE.
/// 
/// - **For ACK to 2xx**: Not matched to any transaction - handled by the TU (dialog layer).
/// 
/// - **For CANCEL**: Creates a new transaction but matches to an existing INVITE transaction
///   with the same identifiers (except method).
/// 
/// The `TransactionManager` implements these matching rules to route messages to the appropriate
/// transaction instance.
///
/// ## Usage Examples
///
/// ### 1. Basic Client Transaction
///
/// ```
/// # mod doctest_helpers {
/// #   use rvoip_sip_core::{Method, Message as SipMessage, Request as SipCoreRequest, Response as SipCoreResponse, Uri};
/// #   use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// #   use rvoip_sip_core::types::{
/// #       header::TypedHeader,
/// #       content_length::ContentLength as ContentLengthHeaderType,
/// #       status::StatusCode,
/// #       param::Param,
/// #       via::Via,
/// #   };
/// #   use rvoip_sip_transport::{Transport, Error as TransportError, TransportEvent as TransportLayerEvent};
/// #   use std::net::SocketAddr;
/// #   use std::sync::Arc;
/// #   use tokio::sync::{Mutex, mpsc};
/// #   use std::collections::VecDeque;
/// #   use async_trait::async_trait;
/// #   use std::str::FromStr;
/// #   use uuid::Uuid;
///
/// #   #[derive(Debug, Clone)]
/// #   pub struct DocMockTransport {
/// #       pub sent_messages: Arc<Mutex<VecDeque<(SipMessage, SocketAddr)>>>,
/// #       pub event_injector: mpsc::Sender<TransportLayerEvent>,
/// #       local_socket_addr: SocketAddr,
/// #       is_transport_closed: Arc<Mutex<bool>>,
/// #   }
///
/// #   impl DocMockTransport {
/// #       pub fn new(event_injector: mpsc::Sender<TransportLayerEvent>, local_addr_str: &str) -> Self {
/// #           DocMockTransport {
/// #               sent_messages: Arc::new(Mutex::new(VecDeque::new())),
/// #               event_injector,
/// #               local_socket_addr: local_addr_str.parse().expect("Invalid local_addr_str for DocMockTransport"),
/// #               is_transport_closed: Arc::new(Mutex::new(false)),
/// #           }
/// #       }
/// #       pub async fn inject_event(&self, event: TransportLayerEvent) -> std::result::Result<(), String> {
/// #           self.event_injector.send(event).await.map_err(|e| e.to_string())
/// #       }
/// #   }
///
/// #   #[async_trait]
/// #   impl Transport for DocMockTransport {
/// #       fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
/// #           Ok(self.local_socket_addr)
/// #       }
/// #
/// #       async fn send_message(&self, message: SipMessage, destination: SocketAddr) -> std::result::Result<(), TransportError> {
/// #           if *self.is_transport_closed.lock().await {
/// #               return Err(TransportError::TransportClosed);
/// #           }
/// #           let mut sent = self.sent_messages.lock().await;
/// #           sent.push_back((message, destination));
/// #           Ok(())
/// #       }
/// #
/// #       async fn close(&self) -> std::result::Result<(), TransportError> {
/// #           let mut closed_status = self.is_transport_closed.lock().await;
/// #           *closed_status = true;
/// #           Ok(())
/// #       }
/// #
/// #       fn is_closed(&self) -> bool {
/// #           self.is_transport_closed.try_lock().map_or(false, |guard| *guard)
/// #       }
/// #   }
///
/// #   pub fn build_invite_request(from_domain: &str, to_domain: &str, local_contact_host: &str) -> std::result::Result<SipCoreRequest, Box<dyn std::error::Error>> {
/// #       let from_uri_str = format!("sip:alice@{}", from_domain);
/// #       let to_uri_str = format!("sip:bob@{}", to_domain);
/// #       let contact_uri_str = format!("sip:alice@{}", local_contact_host);
/// #
/// #       let request = SimpleRequestBuilder::new(Method::Invite, &to_uri_str)?
/// #           .from("Alice", &from_uri_str, Some("fromtagClient1"))
/// #           .to("Bob", &to_uri_str, None)
/// #           .call_id(&format!("callclient1-{}", Uuid::new_v4()))
/// #           .cseq(1)
/// #           .contact(&contact_uri_str, Some("Alice Contact"))
/// #           .via(&format!("{}:5060", local_contact_host), "UDP", Some(&format!("z9hG4bK{}", Uuid::new_v4().simple())))
/// #           .header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)))
/// #           .build();
/// #       Ok(request)
/// #   }
/// #
/// #   pub fn build_trying_response(invite_req: &SipCoreRequest) -> std::result::Result<SipCoreResponse, Box<dyn std::error::Error>> {
/// #       let response = SimpleResponseBuilder::response_from_request(invite_req, StatusCode::Trying, Some("Trying"))
/// #           .header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)))
/// #           .build();
/// #       Ok(response)
/// #   }
/// # }
/// # use doctest_helpers::*;
/// use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
/// use rvoip_sip_core::{Method, Message as SipMessage, Request as SipCoreRequest, Response as SipCoreResponse, types::status::StatusCode};
/// use rvoip_sip_transport::{TransportEvent as TransportLayerEvent, Transport};
/// use std::net::SocketAddr;
/// use std::sync::Arc;
/// use tokio::sync::mpsc;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
///     let (event_injector_tx, event_injector_rx_for_manager) = mpsc::channel(100);
///     let mock_client_addr = "127.0.0.1:5080";
///     let mock_transport = Arc::new(DocMockTransport::new(event_injector_tx, mock_client_addr));
///
///     let (manager, mut client_events_rx) = TransactionManager::new(
///         mock_transport.clone() as Arc<dyn Transport>,
///         event_injector_rx_for_manager,
///         Some(10)
///     ).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///
///     let invite_req = build_invite_request("client.com", "server.com", mock_client_addr)?;
///     let destination_server_addr: SocketAddr = "127.0.0.1:5090".parse()?;
///
///     let tx_id = manager.create_client_transaction(invite_req.clone(), destination_server_addr)
///         .await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///     println!("Client transaction created with ID: {}", tx_id);
///     
///     // Send the request after creating the transaction
///     manager.send_request(&tx_id).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///
///     // Allow more time for the transaction manager to send the message
///     tokio::time::sleep(Duration::from_millis(100)).await;
///
///     // Process any state change events before checking messages
///     tokio::select! {
///         Some(event) = client_events_rx.recv() => {
///             match event {
///                 TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
///                     if transaction_id == tx_id => {
///                     println!("Transaction state changed from {:?} to {:?}", previous_state, new_state);
///                     // This is the expected StateChanged event, continue with the test
///                 },
///                 _ => {} // Ignore other events
///             }
///         },
///         _ = tokio::time::sleep(Duration::from_millis(50)) => {
///             // Timeout waiting for event, continue anyway
///         }
///     }
///
///     let sent_messages = mock_transport.sent_messages.lock().await;
///     assert_eq!(sent_messages.len(), 1, "INVITE should have been sent");
///     if let Some((msg, dest)) = sent_messages.front() {
///         assert!(msg.is_request(), "Sent message should be a request");
///         assert_eq!(msg.method(), Some(Method::Invite));
///         assert_eq!(*dest, destination_server_addr);
///     } else {
///         panic!("No message found in sent_messages");
///     }
///     drop(sent_messages);
///
///     let trying_response_msg = build_trying_response(&invite_req)?;
///     mock_transport.inject_event(TransportLayerEvent::MessageReceived {
///         message: SipMessage::Response(trying_response_msg.clone()),
///         source: destination_server_addr,
///         destination: mock_transport.local_addr()?,
///     }).await?;
///
///     tokio::select! {
///         Some(event) = client_events_rx.recv() => {
///             match event {
///                 TransactionEvent::ProvisionalResponse { transaction_id, response, .. } if transaction_id == tx_id => {
///                     println!("Received Provisional Response: {} {}", response.status_code(), response.reason_phrase());
///                     assert_eq!(response.status_code(), StatusCode::Trying.as_u16());
///                 },
///                 TransactionEvent::TransportError { transaction_id, .. } if transaction_id == tx_id => {
///                     eprintln!("Transport error for transaction {}", transaction_id);
///                     return Err("Transport error".into());
///                 },
///                 TransactionEvent::TransactionTimeout { transaction_id, .. } if transaction_id == tx_id => {
///                     eprintln!("Transaction {} timed out", transaction_id);
///                     return Err("Transaction timeout".into());
///                 },
///                 other_event => {
///                     eprintln!("Received unexpected event: {:?}", other_event);
///                     return Err("Unexpected event".into());
///                 }
///             }
///         },
///         _ = tokio::time::sleep(Duration::from_secs(2)) => {
///             eprintln!("Timeout waiting for transaction event");
///             return Err("Timeout waiting for event".into());
///         }
///     }
///
///     manager.shutdown().await;
///     Ok(())
/// }
/// ```
///
/// ### 2. Basic Server Transaction
///
/// ```
/// use rvoip_transaction_core::{TransactionManager, TransactionEvent};
/// use rvoip_transaction_core::builders::{client_quick, server_quick};
/// use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
/// use rvoip_sip_core::{Method, StatusCode};
/// use std::net::SocketAddr;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create server transport
///     let server_config = TransportManagerConfig {
///         enable_udp: true,
///         bind_addresses: vec!["127.0.0.1:5060".parse()?],
///         ..Default::default()
///     };
///     
///     let (mut server_transport, server_transport_rx) = TransportManager::new(server_config).await?;
///     server_transport.initialize().await?;
///     let server_addr = server_transport.default_transport().await
///         .ok_or("No default transport")?.local_addr()?;
///     
///     // Create transaction manager
///     let (server_tm, mut server_events) = TransactionManager::with_transport_manager(
///         server_transport,
///         server_transport_rx,
///         Some(10),
///     ).await?;
///     
///     // Clone for use in spawn (to avoid ownership issues)
///     let server_tm_clone = server_tm.clone();
///     
///     // Handle incoming requests
///     tokio::spawn(async move {
///         while let Some(event) = server_events.recv().await {
///             match event {
///                 TransactionEvent::NonInviteRequest { transaction_id, request, source, .. } => {
///                     println!("Received {} from {}", request.method(), source);
///                     
///                     // Send appropriate response using builders
///                     match request.method() {
///                         Method::Register => {
///                             let ok = server_quick::ok_register(
///                                 &request, 
///                                 3600, 
///                                 vec![format!("sip:user@{}", source.ip())]
///                             ).expect("Failed to create REGISTER response");
///                             
///                             let _ = server_tm_clone.send_response(&transaction_id, ok).await;
///                         },
///                         Method::Options => {
///                             let ok = server_quick::ok_options(
///                                 &request, 
///                                 vec![Method::Invite, Method::Register, Method::Options]
///                             ).expect("Failed to create OPTIONS response");
///                             
///                             let _ = server_tm_clone.send_response(&transaction_id, ok).await;
///                         },
///                         _ => {
///                             let ok = server_quick::ok_bye(&request)
///                                 .expect("Failed to create OK response");
///                             let _ = server_tm_clone.send_response(&transaction_id, ok).await;
///                         }
///                     }
///                 },
///                 TransactionEvent::InviteRequest { transaction_id, request, source, .. } => {
///                     println!("Received INVITE from {}", source);
///                     
///                     // Send INVITE response
///                     let ok = server_quick::ok_invite(
///                         &request, 
///                         Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n...".to_string()),
///                         format!("sip:server@{}", source.ip())
///                     ).expect("Failed to create INVITE response");
///                     
///                     let _ = server_tm_clone.send_response(&transaction_id, ok).await;
///                 },
///                 _ => {}
///             }
///         }
///     });
///     
///     // Keep server running for a bit
///     tokio::time::sleep(Duration::from_millis(100)).await;
///     server_tm.shutdown().await;
///     Ok(())
/// }
/// ```
///
/// ### 3. Client INVITE with ACK Handling
///
/// This example demonstrates a client INVITE transaction, receiving a 2xx response,
/// and then the Transaction User (TU) constructing and sending an ACK.
///
/// ```
/// use rvoip_transaction_core::{TransactionManager, TransactionEvent};
/// use rvoip_transaction_core::builders::client_quick;
/// use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
/// use rvoip_sip_core::{Method, StatusCode};
/// use std::net::SocketAddr;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create client transport
///     let client_config = TransportManagerConfig {
///         enable_udp: true,
///         bind_addresses: vec!["127.0.0.1:0".parse()?], // Ephemeral port
///         ..Default::default()
///     };
///     
///     let (mut client_transport, client_transport_rx) = TransportManager::new(client_config).await?;
///     client_transport.initialize().await?;
///     let client_addr = client_transport.default_transport().await
///         .ok_or("No default transport")?.local_addr()?;
///     
///     // Create transaction manager
///     let (client_tm, mut client_events) = TransactionManager::with_transport_manager(
///         client_transport,
///         client_transport_rx,
///         Some(10),
///     ).await?;
///     
///     // Create INVITE using builders
///     let server_addr: SocketAddr = "127.0.0.1:5060".parse()?;
///     let invite_request = client_quick::invite(
///         "sip:alice@example.com",
///         "sip:bob@example.com", 
///         client_addr,
///         Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n")
///     ).expect("Failed to create INVITE");
///     
///     // Create client transaction
///     let tx_id = client_tm.create_client_transaction(invite_request, server_addr).await?;
///     println!("Created INVITE transaction: {}", tx_id);
///     
///     // Send the INVITE
///     client_tm.send_request(&tx_id).await?;
///     println!("Sent INVITE request");
///     
///     // Handle events
///     tokio::spawn(async move {
///         while let Some(event) = client_events.recv().await {
///             match event {
///                 TransactionEvent::SuccessResponse { transaction_id, response, .. } => {
///                     println!("Received {} {}", response.status_code(), response.reason_phrase());
///                     
///                     // For 2xx responses to INVITE, the TU must send ACK
///                     // (This would normally be done by the dialog layer)
///                     if response.status_code() >= 200 && response.status_code() < 300 {
///                         println!("Would send ACK for 2xx response (handled by TU/Dialog layer)");
///                     }
///                 },
///                 TransactionEvent::FailureResponse { transaction_id, response } => {
///                     println!("Received error: {} {}", response.status_code(), response.reason_phrase());
///                     // ACK for non-2xx responses is handled automatically by transaction layer
///                 },
///                 TransactionEvent::StateChanged { transaction_id, new_state, .. } => {
///                     println!("Transaction {} state: {:?}", transaction_id, new_state);
///                 },
///                 _ => {}
///             }
///         }
///     });
///     
///     // Keep client running for a bit
///     tokio::time::sleep(Duration::from_millis(100)).await;
///     client_tm.shutdown().await;
///     Ok(())
/// }
/// ```
///
/// ## Transactional vs. Non-Transactional Behavior
///
/// SIP distinguishes between:
///
/// 1. **Transactional Messages**: Initial requests and their responses
///    - Handled by this transaction layer
///    - Examples: REGISTER, INVITE, BYE, etc.
///
/// 2. **Non-Transactional Messages**: Messages that don't create a transaction
///    - ACK for 2xx responses (separate from the INVITE transaction)
///    - CANCEL (creates its own transaction but refers to another)
///    - In-dialog requests (handled at the dialog layer, but still use transactions)
///
/// ## RFC 3261 Compliance
///
/// This implementation follows the transaction state machines defined in RFC 3261:
///
/// * Section 17.1.1: INVITE client transactions
/// * Section 17.1.2: Non-INVITE client transactions
/// * Section 17.2.1: INVITE server transactions
/// * Section 17.2.2: Non-INVITE server transactions
///
/// ## Error Handling
///
/// The library provides a comprehensive error system via the [`Error`] type, enabling
/// detailed error information propagation for various failure scenarios. The transaction
/// layer focuses on handling protocol-level errors such as timeouts, transport failures,
/// and improper message sequences.
/// # Example
///
/// ```
/// use rvoip_transaction_core::prelude::*;
/// use rvoip_transaction_core::transaction::AtomicTransactionState;
/// use std::time::Duration;
/// ```
pub mod prelude {
    pub use crate::transaction::{TransactionKey, TransactionEvent, TransactionState, TransactionKind};
    pub use crate::manager::TransactionManager;
    pub use rvoip_sip_transport::transport::TransportType;
    pub use crate::transport::{
        TransportCapabilities, TransportInfo,
        NetworkInfoForSdp, WebSocketStatus, TransportCapabilitiesExt
    };
    pub use crate::error::{Error, Result};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::AtomicTransactionState;
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use std::time::Duration;

    /// Mock Transport implementation for testing
    #[derive(Debug)]
    struct MockTransport {
        local_addr: SocketAddr,
    }

    impl MockTransport {
        fn new(addr: &str) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
            }
        }
    }

    #[async_trait::async_trait]
    impl rvoip_sip_transport::Transport for MockTransport {
        async fn send_message(
            &self,
            _message: rvoip_sip_core::Message,
            _destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
            Ok(()) // Just pretend we sent it
        }

        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::error::Error> {
            Ok(self.local_addr)
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_transaction_manager_creation() {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_, transport_rx) = mpsc::channel(10);
        
        let result = TransactionManager::new(
            transport,
            transport_rx,
            Some(100)
        ).await;
        
        assert!(result.is_ok(), "Should create TransactionManager without error");
    }

    #[tokio::test]
    async fn test_transaction_key_creation() {
        let key = TransactionKey::new(
            "z9hG4bK1234".to_string(),
            rvoip_sip_core::Method::Invite,
            false
        );
        
        assert_eq!(key.branch, "z9hG4bK1234");
        assert_eq!(key.method, rvoip_sip_core::Method::Invite);
        assert_eq!(key.is_server, false);
    }

    #[tokio::test]
    async fn test_transaction_state_transitions() {
        // Instead of comparing with <, verify the state machine flow by checking
        // that states are different and represent the correct sequence
        let initial = TransactionState::Initial;
        let calling = TransactionState::Calling;
        let proceeding = TransactionState::Proceeding;
        let completed = TransactionState::Completed;
        let terminated = TransactionState::Terminated;
        
        // Verify states are different
        assert_ne!(initial, calling);
        assert_ne!(calling, proceeding);
        assert_ne!(proceeding, completed);
        assert_ne!(completed, terminated);
        
        // Verify some valid transitions for InviteClient
        assert!(AtomicTransactionState::validate_transition(TransactionKind::InviteClient, initial, calling).is_ok());
        assert!(AtomicTransactionState::validate_transition(TransactionKind::InviteClient, calling, proceeding).is_ok());
        assert!(AtomicTransactionState::validate_transition(TransactionKind::InviteClient, proceeding, completed).is_ok());
        assert!(AtomicTransactionState::validate_transition(TransactionKind::InviteClient, completed, terminated).is_ok());
    }

    #[tokio::test]
    async fn test_timer_settings() {
        // Test the timer settings defaults
        let settings = TimerSettings::default();
        
        assert_eq!(settings.t1, Duration::from_millis(500), "T1 should be 500ms");
        assert_eq!(settings.t2, Duration::from_secs(4), "T2 should be 4s");
        
        // Create a custom timer settings
        let custom_settings = TimerSettings {
            t1: Duration::from_millis(200),
            t2: Duration::from_secs(2),
            t4: Duration::from_secs(5),
            timer_100_interval: Duration::from_millis(200),
            transaction_timeout: Duration::from_secs(16),
            wait_time_d: Duration::from_secs(16),
            wait_time_h: Duration::from_secs(16),
            wait_time_i: Duration::from_secs(2),
            wait_time_j: Duration::from_secs(16),
            wait_time_k: Duration::from_secs(2),
        };
        
        assert_eq!(custom_settings.t1, Duration::from_millis(200));
        assert_eq!(custom_settings.transaction_timeout, Duration::from_secs(16));
    }
} 