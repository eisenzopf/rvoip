use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex, RwLock, oneshot};
use tokio::time;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Uri, Header, HeaderName, HeaderValue
};
use rvoip_transaction_core::TransactionManager;

use crate::config::ClientConfig;
use crate::error::{Error, Result};
use super::events::SipClientEvent;
use super::registration::Registration;

/// Lightweight client for use in detached tasks
pub(crate) struct LightweightClient {
    pub transaction_manager: Arc<TransactionManager>,
    pub config: ClientConfig,
    pub cseq: Arc<Mutex<u32>>,
    pub registration: Arc<RwLock<Option<Registration>>>,
    pub event_tx: mpsc::Sender<SipClientEvent>,
}

impl LightweightClient {
    /// Register with a SIP server (used for refreshing registration)
    pub async fn register(&self, server_addr: SocketAddr) -> Result<()> {
        // Create request URI for REGISTER (domain)
        let request_uri: Uri = format!("sip:{}", self.config.domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid domain URI: {}", e)))?;
        
        // Create REGISTER request - simplified version
        let mut request = Request::new(Method::Register, request_uri.clone());
        
        // Add From header with user information
        request.headers.push(Header::text(
            HeaderName::From,
            format!("<sip:{}@{}>", self.config.username, self.config.domain)
        ));
        
        // Add To header (same as From for REGISTER)
        request.headers.push(Header::text(
            HeaderName::To,
            format!("<sip:{}@{}>", self.config.username, self.config.domain)
        ));
        
        // Add CSeq header
        let cseq = {
            let mut lock = self.cseq.lock().await;
            *lock += 1;
            *lock
        };
        request.headers.push(Header::text(
            HeaderName::CSeq,
            format!("{} REGISTER", cseq)
        ));
        
        // Add Call-ID header
        request.headers.push(Header::text(
            HeaderName::CallId,
            format!("register-{}", uuid::Uuid::new_v4().to_string())
        ));
        
        // Add Max-Forwards header
        request.headers.push(Header::text(
            HeaderName::MaxForwards,
            "70"
        ));
        
        // Add Expires header
        request.headers.push(Header::text(
            HeaderName::Expires, 
            self.config.register_expires.to_string()
        ));
        
        // Add Contact header with expires parameter
        let contact = format!(
            "<sip:{}@{};transport=udp>;expires={}",
            self.config.username,
            self.config.local_addr.unwrap(),
            self.config.register_expires
        );
        request.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Create a transaction
        let transaction_id = self.transaction_manager.create_client_transaction(
            request.clone(), 
            server_addr
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Create a oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        
        // Set up a separate task to listen for the response
        let transaction_manager = self.transaction_manager.clone();
        let event_tx = self.event_tx.clone();
        let tx_id_for_task = transaction_id.clone(); // Clone for the task
        
        tokio::spawn(async move {
            // Listen for transaction events
            let mut events_rx = transaction_manager.subscribe();
            
            while let Some(event) = events_rx.recv().await {
                if let rvoip_transaction_core::TransactionEvent::ResponseReceived { 
                    message, 
                    transaction_id: event_tx_id, 
                    .. 
                } = &event {
                    if event_tx_id == &tx_id_for_task {
                        if let Message::Response(response) = message {
                            // Send response to our oneshot channel
                            let _ = tx.send(response.clone());
                            break;
                        }
                    }
                }
            }
        });
        
        // Send the request
        self.transaction_manager.send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Wait for the response with timeout
        let response = match time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(Error::Transport("Response channel closed".into())),
            Err(_) => return Err(Error::Timeout("Timeout waiting for response".into())),
        };
            
        if response.status == StatusCode::Ok {
            info!("Registration successful (refresh)");
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: true,
                server: server_addr.to_string(),
                expires: Some(self.config.register_expires),
                error: None,
            }).await;
            
            Ok(())
        } else {
            // Registration failed
            error!("Registration refresh failed: {}", response.status);
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: false,
                server: server_addr.to_string(),
                expires: None,
                error: Some(format!("Registration failed: {}", response.status)),
            }).await;
            
            Err(Error::Registration(format!("Registration failed: {}", response.status)))
        }
    }
} 