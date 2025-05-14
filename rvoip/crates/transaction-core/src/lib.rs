mod error;
pub mod transaction;
pub mod client;
pub mod server;
pub mod manager;
pub mod timer;
pub mod utils;

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
///    based on SIP headers (Via, CSeq, etc.).
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
///     tokio::time::sleep(Duration::from_millis(50)).await; // Allow manager to send the message
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
/// #               local_socket_addr: local_addr_str.parse().expect("Invalid local_addr_str"),
/// #               is_transport_closed: Arc::new(Mutex::new(false)),
/// #           }
/// #       }
/// #       #[allow(dead_code)] pub async fn inject_event(&self, event: TransportLayerEvent) -> std::result::Result<(), String> {
/// #           self.event_injector.send(event).await.map_err(|e| e.to_string())
/// #       }
/// #   }
///
/// #   #[async_trait]
/// #   impl Transport for DocMockTransport {
/// #       fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
/// #           Ok(self.local_socket_addr)
/// #       }
/// #       async fn send_message(&self, message: SipMessage, destination: SocketAddr) -> std::result::Result<(), TransportError> {
/// #           if *self.is_transport_closed.lock().await { return Err(TransportError::TransportClosed); }
/// #           self.sent_messages.lock().await.push_back((message, destination));
/// #           Ok(())
/// #       }
/// #       async fn close(&self) -> std::result::Result<(), TransportError> {
/// #           *self.is_transport_closed.lock().await = true; Ok(())
/// #       }
/// #       fn is_closed(&self) -> bool { self.is_transport_closed.try_lock().map_or(false, |g| *g) }
/// #   }
///
/// #   pub fn build_incoming_invite(from_client_addr_str: &str, to_server_uri_str: &str) -> std::result::Result<SipCoreRequest, Box<dyn std::error::Error>> {
/// #       let from_uri_str = format!("sip:caller@{}", from_client_addr_str.split(':').next().unwrap_or("client.com"));
/// #       let request = SimpleRequestBuilder::new(Method::Invite, to_server_uri_str)?
/// #           .from("Caller", &from_uri_str, Some("fromtagServer1"))
/// #           .to("Server", to_server_uri_str, None)
/// #           .call_id(&format!("callserver1-{}", Uuid::new_v4()))
/// #           .cseq(1)
/// #           .via(from_client_addr_str, "UDP", Some(&format!("z9hG4bK{}", Uuid::new_v4().simple())))
/// #           .contact(&format!("sip:caller@{}", from_client_addr_str), Some("Caller Contact"))
/// #           .header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)))
/// #           .build();
/// #       Ok(request)
/// #   }
/// # }
/// # use doctest_helpers::*;
/// use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
/// use rvoip_sip_core::{Method, Message as SipMessage, Request as SipCoreRequest, Response as SipCoreResponse, types::status::StatusCode};
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::header::TypedHeader;
/// use rvoip_sip_core::types::content_length::ContentLength as ContentLengthHeaderType;
/// use rvoip_sip_transport::{TransportEvent as TransportLayerEvent, Transport};
/// use std::net::SocketAddr;
/// use std::sync::Arc;
/// use tokio::sync::mpsc;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
///     let (event_injector_tx, event_injector_rx_for_manager) = mpsc::channel(100);
///     let mock_server_addr_str = "127.0.0.1:5070";
///     let mock_transport = Arc::new(DocMockTransport::new(event_injector_tx.clone(), mock_server_addr_str));
///
///     let (manager, mut server_events_rx) = TransactionManager::new(
///         mock_transport.clone() as Arc<dyn Transport>,
///         event_injector_rx_for_manager,
///         Some(10)
///     ).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///
///     let client_addr_str = "127.0.0.1:6000";
///     let client_socket_addr: SocketAddr = client_addr_str.parse()?;
///     let server_uri_str = format!("sip:server@{}", mock_server_addr_str);
///     let incoming_invite = build_incoming_invite(client_addr_str, &server_uri_str)?;
///
///     event_injector_tx.send(TransportLayerEvent::MessageReceived {
///         message: SipMessage::Request(incoming_invite.clone()),
///         source: client_socket_addr,
///         destination: mock_transport.local_addr()?,
///     }).await?;
///
///     let received_tx_id: TransactionKey;
///     let original_request_for_response: SipCoreRequest;
///
///     tokio::select! {
///         Some(event) = server_events_rx.recv() => {
///             match event {
///                 TransactionEvent::NewRequest { transaction_id, request, source, .. } => {
///                     println!("Received new INVITE with ID: {} from {}", transaction_id, source);
///                     assert_eq!(request.method(), Method::Invite);
///                     assert_eq!(source, client_socket_addr);
///                     received_tx_id = transaction_id;
///                     original_request_for_response = request;
///
///                     let ringing_response = SimpleResponseBuilder::response_from_request(&original_request_for_response, StatusCode::Ringing, Some("Ringing"))
///                         .header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)))
///                         .build();
///
///                     tokio::time::sleep(Duration::from_millis(10)).await; // Small delay before sending response
///                     manager.send_response(&received_tx_id, ringing_response).await
///                         .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///                     println!("Sent 180 Ringing");
///                 },
///                 other_event => return Err(format!("Unexpected event instead of NewRequest: {:?}", other_event).into()),
///             }
///         },
///         _ = tokio::time::sleep(Duration::from_secs(2)) => return Err("Timeout waiting for NewRequest".into()),
///     }
///
///     tokio::time::sleep(Duration::from_millis(50)).await; // Allow manager to send the message
///
///     let sent_messages = mock_transport.sent_messages.lock().await;
///     assert_eq!(sent_messages.len(), 1, "Ringing response should have been sent");
///     if let Some((msg, dest)) = sent_messages.front() {
///         assert!(msg.is_response(), "Sent message should be a response");
///         if let SipMessage::Response(ref resp_msg) = msg {
///             assert_eq!(resp_msg.status_code(), StatusCode::Ringing.as_u16());
///         } else {
///             panic!("Sent message was not a response type as expected");
///         }
///         assert_eq!(*dest, client_socket_addr);
///     } else {
///         panic!("No message found in sent_messages for ringing");
///     }
///
///     manager.shutdown().await;
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
/// # mod doctest_helpers {
/// #   use rvoip_sip_core::{Method, Message as SipMessage, Request as SipCoreRequest, Response as SipCoreResponse, Uri};
/// #   use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// #   use rvoip_sip_core::types::{
/// #       header::TypedHeader,
/// #       content_length::ContentLength as ContentLengthHeaderType,
/// #       status::StatusCode,
/// #       param::Param,
/// #       via::Via,
/// #       cseq::CSeq,
/// #       address::Address,
/// #       call_id::CallId as CallIdHeader,
/// #       from::From as FromHeader,
/// #       to::To as ToHeader,
/// #       contact::Contact as ContactHeader, // For TypedHeader::Contact(ContactHeader)
/// #   };
/// #   use rvoip_sip_transport::{Transport, Error as TransportError, TransportEvent as TransportLayerEvent};
/// #   use rvoip_sip_core::json::ext::SipMessageJson; // May not be needed if direct access works
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
/// #               local_socket_addr: local_addr_str.parse().expect("Invalid local_addr_str"),
/// #               is_transport_closed: Arc::new(Mutex::new(false)),
/// #           }
/// #       }
/// #       #[allow(dead_code)] pub async fn inject_event(&self, event: TransportLayerEvent) -> std::result::Result<(), String> {
/// #           self.event_injector.send(event).await.map_err(|e| e.to_string())
/// #       }
/// #   }
///
/// #   #[async_trait]
/// #   impl Transport for DocMockTransport {
/// #       fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> { Ok(self.local_socket_addr) }
/// #       async fn send_message(&self, message: SipMessage, destination: SocketAddr) -> std::result::Result<(), TransportError> {
/// #           if *self.is_transport_closed.lock().await { return Err(TransportError::TransportClosed); }
/// #           self.sent_messages.lock().await.push_back((message, destination)); Ok(())
/// #       }
/// #       async fn close(&self) -> std::result::Result<(), TransportError> { *self.is_transport_closed.lock().await = true; Ok(()) }
/// #       fn is_closed(&self) -> bool { self.is_transport_closed.try_lock().map_or(false, |g| *g) }
/// #   }
///
/// #   pub fn build_invite_for_ack_test(client_addr_str: &str, server_uri_str: &str) -> std::result::Result<SipCoreRequest, Box<dyn std::error::Error>> {
/// #       let from_uri_str = format!("sip:ackclient@{}", client_addr_str.split(':').next().unwrap_or("client.com"));
/// #       let request = SimpleRequestBuilder::new(Method::Invite, server_uri_str)?
/// #           .from("AckClient", &from_uri_str, Some("fromtagAckClient"))
/// #           .to("AckServer", server_uri_str, None)
/// #           .call_id(&format!("callack-{}", Uuid::new_v4()))
/// #           .cseq(1)
/// #           .via(client_addr_str, "UDP", Some(&format!("z9hG4bK{}", Uuid::new_v4().simple())))
/// #           .contact(&format!("sip:ackclient@{}", client_addr_str), Some("AckClient Contact"))
/// #           .header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)))
/// #           .build();
/// #       Ok(request)
/// #   }
///
/// #   pub fn build_ok_for_invite(invite_req: &SipCoreRequest, server_contact_str: &str, to_tag: &str) -> std::result::Result<SipCoreResponse, Box<dyn std::error::Error>> {
/// #       let invite_to_header = invite_req.to().ok_or_else(|| Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "Invite missing To header")) as Box<dyn std::error::Error>)?;
/// #       let response = SimpleResponseBuilder::response_from_request(invite_req, StatusCode::Ok, Some("OK"))
/// #           .to( // Rebuild To header with the new tag
/// #               invite_to_header.address().display_name().unwrap_or_default(),
/// #               &invite_to_header.address().uri.to_string(),
/// #               Some(to_tag)
/// #           )
/// #           .contact(server_contact_str, None)?
/// #           .header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)))
/// #           .build();
/// #       Ok(response)
/// #   }
///
/// #   pub async fn create_ack_for_2xx_invite(
/// #       ok_response: &SipCoreResponse,
/// #       original_invite: &SipCoreRequest,
/// #       client_via_addr_str: &str
/// #   ) -> std::result::Result<(SipCoreRequest, SocketAddr), Box<dyn std::error::Error>> {
/// #
/// #       let ack_request_uri_string = ok_response.header(&rvoip_sip_core::types::header::HeaderName::Contact) // Use full path for HeaderName
/// #           .and_then(|h| if let TypedHeader::Contact(c_header) = h { // c_header is ContactHeader
/// #               c_header.address().map(|addr| addr.uri.to_string()) // Access Contact -> Option<Address> -> uri
/// #           } else { None })
/// #           .or_else(|| ok_response.to().map(|t| t.address().uri.to_string()))
/// #           .ok_or_else(|| "Could not determine ACK Request-URI from 200 OK".to_string())?;
/// #
/// #       let from_hdr_val = original_invite.from().ok_or("Missing From in original INVITE")?.clone();
/// #       let to_hdr_val = ok_response.to().ok_or("Missing To in 2xx response")?.clone();
/// #       let call_id_hdr_val = original_invite.call_id().ok_or("Missing Call-ID in original INVITE")?.clone();
/// #       let original_cseq_val = original_invite.cseq().ok_or("Missing CSeq in original INVITE")?;
/// #       let ack_cseq_hdr_val = CSeq::new(original_cseq_val.seq, Method::Ack);
/// #
/// #       let mut ack_builder = SimpleRequestBuilder::new(Method::Ack, &ack_request_uri_string)?;
/// #
/// #       ack_builder = ack_builder
/// #           .header(TypedHeader::From(from_hdr_val))
/// #           .header(TypedHeader::To(to_hdr_val))
/// #           .header(TypedHeader::CallId(call_id_hdr_val))
/// #           .header(TypedHeader::CSeq(ack_cseq_hdr_val));
/// #
/// #       let ack_branch = format!("z9hG4bK{}", Uuid::new_v4().simple());
/// #       ack_builder = ack_builder.via(client_via_addr_str, "UDP", Some(&ack_branch));
/// #
/// #       ack_builder = ack_builder.header(TypedHeader::ContentLength(ContentLengthHeaderType::new(0)));
/// #       let ack_request = ack_builder.build();
/// #
/// #       let ack_request_uri_parsed = Uri::from_str(&ack_request_uri_string)?;
/// #       let host = ack_request_uri_parsed.host.as_str();
/// #       let port = ack_request_uri_parsed.port.unwrap_or(5060);
/// #       let ack_destination: SocketAddr = format!("{}:{}", host, port).parse()?;
/// #
/// #       Ok((ack_request, ack_destination))
/// #   }
/// # }
/// # use doctest_helpers::*;
/// use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
/// use rvoip_sip_core::{
///     Method, Message as SipMessage, Uri, Request as SipCoreRequest, Response as SipCoreResponse,
///     types::status::StatusCode,
///     types::address::Address,
///     types::header::HeaderName, // For ok_response.header(&HeaderName::Contact)
/// };
/// use rvoip_sip_transport::{TransportEvent as TransportLayerEvent, Transport};
/// use std::net::SocketAddr;
/// use std::sync::Arc;
/// use tokio::sync::mpsc;
/// use std::time::Duration;
/// use std::str::FromStr;
/// use uuid::Uuid;
///
/// #[tokio::main]
/// async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
///     let (event_injector_tx, event_injector_rx_for_manager) = mpsc::channel(100);
///     let client_addr_str = "127.0.0.1:5088";
///     let mock_transport = Arc::new(DocMockTransport::new(event_injector_tx.clone(), client_addr_str));
///
///     let (manager, mut client_events_rx) = TransactionManager::new(
///         mock_transport.clone() as Arc<dyn Transport>,
///         event_injector_rx_for_manager,
///         Some(10)
///     ).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///
///     let server_uri_str = "sip:ackserver@server.com:5098";
///     let server_socket_addr: SocketAddr = { 
///         let uri = Uri::from_str(server_uri_str)?;
///         let host = uri.host.as_str(); 
///         let port = uri.port.unwrap_or(5060); 
///         format!("{}:{}", host, port).parse()?
///     };
///
///     let invite_request = build_invite_for_ack_test(client_addr_str, server_uri_str)?;
///
///     let tx_id = manager.create_client_transaction(invite_request.clone(), server_socket_addr)
///         .await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
///     println!("Client INVITE for ACK test created: {}", tx_id);
/// 
///     tokio::time::sleep(Duration::from_millis(50)).await; // Allow manager to send INVITE
///
///     let server_contact_str = format!("sip:contact@{}", server_socket_addr);
///     let ok_response_to_invite = build_ok_for_invite(&invite_request, &server_contact_str, "totagForAckDialog1")?;
///
///     mock_transport.inject_event(TransportLayerEvent::MessageReceived {
///         message: SipMessage::Response(ok_response_to_invite.clone()),
///         source: server_socket_addr,
///         destination: mock_transport.local_addr()?,
///     }).await?;
///
///     tokio::select! {
///         Some(event) = client_events_rx.recv() => {
///             match event {
///                 TransactionEvent::SuccessResponse { transaction_id, response, .. } if transaction_id == tx_id => {
///                     println!("Received Success Response: {} {}", response.status_code(), response.reason_phrase());
///                     assert!(StatusCode::from_u16(response.status_code())?.is_success());
///
///                     let (ack_request, ack_destination) = create_ack_for_2xx_invite(
///                         &response,
///                         &invite_request,
///                         client_addr_str
///                     ).await?;
///
///                     println!("TU sending ACK to {} for 2xx INVITE response", ack_destination);
///                     mock_transport.send_message(SipMessage::Request(ack_request), ack_destination).await?;
///
///                 },
///                 other_event => return Err(format!("Unexpected event: {:?}, expected SuccessResponse", other_event).into()),
///             }
///         },
///         _ = tokio::time::sleep(Duration::from_secs(2)) => return Err("Timeout waiting for SuccessResponse".into()),
///     }
///
///     tokio::time::sleep(Duration::from_millis(100)).await;
///     let sent_messages = mock_transport.sent_messages.lock().await;
///     assert_eq!(sent_messages.len(), 2, "Expected INVITE and ACK to be sent. Found: {:?}",
///         sent_messages.iter().map(|(m,_)| {
///             match m {
///                 SipMessage::Request(req) => format!("Method: {:?}", req.method()),
///                 SipMessage::Response(res) => format!("Status: {}", res.status_code()),
///             }
///         }).collect::<Vec<_>>()
///     );
///
///     let invite_sent = sent_messages.iter().any(|(m, _)| m.method() == Some(Method::Invite));
///     let ack_sent = sent_messages.iter().any(|(m, _)| m.method() == Some(Method::Ack));
///     assert!(invite_sent, "INVITE not found in sent messages");
///     assert!(ack_sent, "ACK not found in sent messages");
///
///     manager.shutdown().await;
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
    pub use crate::error::{Error, Result};
    pub use crate::manager::TransactionManager;
    pub use crate::transaction::{
        Transaction, TransactionAsync, TransactionEvent, TransactionKey, TransactionKind, TransactionState,
        InternalTransactionCommand,
    };
    pub use crate::client::{ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction, TransactionExt as ClientTransactionExt};
    pub use crate::server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction, TransactionExt as ServerTransactionExt};
    pub use crate::timer::{Timer, TimerManager, TimerFactory, TimerSettings, TimerType};
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
        assert!(AtomicTransactionState::validate_transition(initial, calling, TransactionKind::InviteClient).is_ok());
        assert!(AtomicTransactionState::validate_transition(calling, proceeding, TransactionKind::InviteClient).is_ok());
        assert!(AtomicTransactionState::validate_transition(proceeding, completed, TransactionKind::InviteClient).is_ok());
        assert!(AtomicTransactionState::validate_transition(completed, terminated, TransactionKind::InviteClient).is_ok());
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