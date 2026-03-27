//! TransactionManager transaction creation and transport capability methods

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::str::FromStr;
use std::fmt;

use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};
use rvoip_sip_transport::Transport;
use rvoip_sip_transport::transport::TransportType;

use crate::transaction::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
use crate::transaction::client::{
    ClientTransaction,
    ClientInviteTransaction,
    ClientNonInviteTransaction,
    CommonClientTransaction,
};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction, CommonServerTransaction};
use crate::transaction::timer::{Timer, TimerSettings};
use crate::transaction::method::{cancel, update, ack};
use crate::transaction::utils::{generate_branch, create_ack_from_invite};
use crate::transaction::transport::{
    TransportCapabilities, TransportInfo,
    NetworkInfoForSdp, WebSocketStatus, TransportCapabilitiesExt
};

use super::{TransactionManager, RFC3261_BRANCH_MAGIC_COOKIE, handlers, utils};

impl TransactionManager {
    /// Create a client transaction for sending a SIP request
    /// The caller is responsible for calling send_request() to initiate the transaction.
    pub async fn create_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        debug!(method=%request.method(), destination=%destination, "Creating client transaction");
        
        // Debug the Via headers in the request
        tracing::trace!("Request Via headers before transaction creation:");
        for (i, via) in request.via_headers().iter().enumerate() {
            tracing::trace!("  Via[{}]: {}", i, via);
        }
        
        // Extract branch parameter from the top Via header or generate a new one
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) => b.to_string(),
                    None => {
                        // Generate a branch parameter if none exists
                        format!("{}{}", RFC3261_BRANCH_MAGIC_COOKIE, uuid::Uuid::new_v4().as_simple())
                    }
                }
            },
            None => {
                // No Via header - should not happen, but we'll handle it by generating a branch
                // and a Via header will be added by the transaction
                format!("{}{}", RFC3261_BRANCH_MAGIC_COOKIE, uuid::Uuid::new_v4().as_simple())
            }
        };
        
        // We'll create the transaction key directly
        let key = TransactionKey::new(branch.clone(), request.method().clone(), false);
        
        // For CANCEL method, make sure we don't add a new Via header if one already exists
        // This is already checked in create_cancel_request, but we'll verify here as well
        let mut modified_request = request.clone();
        
        // For CANCEL requests, the Via header should be preserved exactly as it was created
        // No need to add or modify it
        if request.method() == Method::Cancel {
            // Since CANCEL already has a Via header with the correct branch from create_cancel_request,
            // we don't need to modify it further
            tracing::trace!("CANCEL request detected - not adding Via header");
        } else {
            // For other methods, ensure the request has a Via header with our branch
            // Create a Via header with the branch parameter
            let local_addr = self.transport.local_addr()
                .map_err(|e| Error::transport_error(e, "Failed to get local address for Via header"))?;
            
            let via_header = handlers::create_via_header(&local_addr, &branch)?;
            
            // Check if there's already a Via header
            if request.first_via().is_some() {
                // Replace it to ensure the branch is correct
                modified_request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
                modified_request = modified_request.with_header(via_header);
            } else {
                // Add a new Via header
                modified_request = modified_request.with_header(via_header);
            }
        }
        
        tracing::trace!("Request Via headers after potential modification:");
        for (i, via) in modified_request.via_headers().iter().enumerate() {
            tracing::trace!("  Via[{}]: {}", i, via);
        }
        
        // Select the best transport for this destination (prefer WS/TCP if available)
        let tx_transport = if let Some(ref tm) = self.transport_manager {
            tm.get_transport_for_destination(destination).await
                .unwrap_or_else(|| self.transport.clone())
        } else {
            self.transport.clone()
        };

        // Create the appropriate transaction based on the request method
        let transaction: Arc<dyn ClientTransaction + Send + Sync> = match modified_request.method() {
            Method::Invite => {
                tracing::trace!("Creating ClientInviteTransaction: {}", key);
                let tx = ClientInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                tracing::trace!("Created ClientInviteTransaction: {}", key);
                Arc::new(tx)
            },
            Method::Cancel => {
                // Validate the CANCEL request
                if let Err(e) = cancel::validate_cancel_request(&modified_request) {
                    warn!(method = %modified_request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                }

                let tx = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                Arc::new(tx)
            },
            Method::Update => {
                // Validate the UPDATE request
                if let Err(e) = update::validate_update_request(&modified_request) {
                    warn!(method = %modified_request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                }

                let tx = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                Arc::new(tx)
            },
            _ => {
                let tx = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                Arc::new(tx)
            }
        };
        
        // Store the transaction
        {
            let mut client_txs = self.client_transactions.lock().await;
            client_txs.insert(key.clone(), transaction);
        }
        
        // Store the destination
        {
            let mut dest_map = self.transaction_destinations.lock().await;
            dest_map.insert(key.clone(), destination);
        }
        
        debug!(id=%key, "Created client transaction");
        
        if request.method() == Method::Cancel {
            debug!(id=%key, original_id=%branch, "Created CANCEL transaction");
        }
        
        Ok(key)
    }

    /// Creates and sends an ACK request for a 2xx response to an INVITE.
    pub async fn send_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<()> {
        // Create the ACK request
        let ack_request = self.create_ack_for_2xx(invite_tx_id, response).await?;
        
        // Try to get a destination from the Contact header first
        let destination = if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
            if let Some(contact_addr) = contact.addresses().next() {
                // Try to parse the URI as a socket address
                if let Some(addr) = utils::socket_addr_from_uri(&contact_addr.uri) {
                    Some(addr)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        
        // If we couldn't get a destination from the Contact header, use the original destination
        let destination = if let Some(dest) = destination {
            dest
        } else {
            // Fall back to the original destination
            let dest_map = self.transaction_destinations.lock().await;
            match dest_map.get(invite_tx_id) {
                Some(addr) => *addr,
                None => return Err(Error::Other(format!("Destination for transaction {:?} not found", invite_tx_id))),
            }
        };
        
        // Send the ACK directly without creating a transaction, using transport routing.
        self.send_with_routing(Message::Request(ack_request), destination)
            .await?;
        
        Ok(())
    }
    
    /// Find transaction by message.
    ///
    /// This method tries to find a transaction that matches the given message.
    /// For requests, it looks for server transactions.
    /// For responses, it looks for client transactions.
    ///
    /// # Arguments
    /// * `message` - The message to match
    ///
    /// # Returns
    /// * `Result<Option<TransactionKey>>` - The matching transaction key if found
    pub async fn find_transaction_by_message(&self, message: &Message) -> Result<Option<TransactionKey>> {
        match message {
            Message::Request(req) => {
                // For requests, look for server transactions
                let server_txs = self.server_transactions.lock().await;
                for (tx_id, tx) in server_txs.iter() {
                    if tx.matches(message) {
                        return Ok(Some(tx_id.clone()));
                    }
                }
                Ok(None)
            },
            Message::Response(resp) => {
                // For responses, look for client transactions
                let client_txs = self.client_transactions.lock().await;
                for (tx_id, tx) in client_txs.iter() {
                    if tx.matches(message) {
                        return Ok(Some(tx_id.clone()));
                    }
                }
                Ok(None)
            }
        }
    }
    
    /// Find the matching INVITE transaction for a CANCEL request.
    ///
    /// # Arguments
    /// * `cancel_request` - The CANCEL request
    ///
    /// # Returns
    /// * `Result<Option<TransactionKey>>` - The matching INVITE transaction key if found
    pub async fn find_invite_transaction_for_cancel(&self, cancel_request: &Request) -> Result<Option<TransactionKey>> {
        if cancel_request.method() != Method::Cancel {
            return Err(Error::Other("Not a CANCEL request".to_string()));
        }
        
        // Get all client transactions
        let client_txs = self.client_transactions.lock().await;
        let invite_tx_keys: Vec<TransactionKey> = client_txs.keys()
            .filter(|k| *k.method() == Method::Invite && !k.is_server)
            .cloned()
            .collect();
        drop(client_txs);
        
        // Use the utility to find the matching INVITE transaction
        let tx_id = crate::transaction::method::cancel::find_invite_transaction_for_cancel(
            cancel_request, 
            invite_tx_keys
        );
        
        Ok(tx_id)
    }

    /// Creates an ACK request for a 2xx response to an INVITE.
    pub async fn create_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<Request> {
        // Verify this is an INVITE client transaction
        if *invite_tx_id.method() != Method::Invite || invite_tx_id.is_server {
            return Err(Error::Other("Can only create ACK for INVITE client transactions".to_string()));
        }
        
        // Get the original INVITE request
        let invite_request = utils::get_transaction_request(
            &self.client_transactions, 
            invite_tx_id
        ).await?;
        
        // Get the local address for the Via header
        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
        
        // Create the ACK request using our utility
        let ack_request = crate::transaction::method::ack::create_ack_for_2xx(&invite_request, response, &local_addr)?;
        
        Ok(ack_request)
    }

    /// Create a server transaction from an incoming request.
    /// 
    /// This is called when a new request is received from the transport layer.
    /// It creates an appropriate transaction based on the request method.
    pub async fn create_server_transaction(
        &self,
        request: Request,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ServerTransaction>> {
        // Extract branch parameter from the top Via header
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) => b.to_string(),
                    None => return Err(Error::Other("Missing branch parameter in Via header".to_string())),
                }
            },
            None => return Err(Error::Other("Missing Via header in request".to_string())),
        };
        
        // Create the transaction key directly with is_server: true
        let key = TransactionKey::new(branch, request.method().clone(), true);
        
        // Check if this is a retransmission of an existing transaction
        {
            let server_txs = self.server_transactions.lock().await;
            if let Some(transaction) = server_txs.get(&key).cloned() {
                // This is a retransmission, get the existing transaction
                drop(server_txs); // Release lock
                
                // Process the request in the existing transaction
                transaction.process_request(request.clone()).await?;
                
                debug!(id=%key, method=%request.method(), "Processed retransmitted request in existing transaction");
                return Ok(transaction);
            }
        }
        
        // Select the best transport for this peer — prefer the one that received the message
        let tx_transport = if let Some(ref tm) = self.transport_manager {
            if let Some(transport) = tm.get_transport_for_destination(remote_addr).await {
                info!(
                    remote_addr = %remote_addr,
                    transport_type = ?transport.default_transport_type(),
                    "Selected transport for server transaction from transport_manager"
                );
                transport
            } else {
                let fallback = self.transport.clone();
                info!(
                    remote_addr = %remote_addr,
                    transport_type = ?fallback.default_transport_type(),
                    "Selected fallback default transport for server transaction (no mapped transport)"
                );
                fallback
            }
        } else {
            let fallback = self.transport.clone();
            info!(
                remote_addr = %remote_addr,
                transport_type = ?fallback.default_transport_type(),
                "Selected default transport for server transaction (transport_manager unavailable)"
            );
            fallback
        };

        // Create a new transaction based on the request method
        let transaction: Arc<dyn ServerTransaction> = match request.method() {
            Method::Invite => {
                let tx = Arc::new(ServerInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerInviteTransaction");
                tx
            },
            Method::Cancel => {
                // Validate the CANCEL request
                if let Err(e) = cancel::validate_cancel_request(&request) {
                    warn!(method = %request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                }
                
                // For CANCEL, try to find the target INVITE transaction
                let mut target_invite_tx_id = None;
                
                // Look for a matching INVITE transaction using the method utility
                let client_txs = self.client_transactions.lock().await;
                let invite_tx_keys: Vec<TransactionKey> = client_txs.keys()
                    .filter(|k| k.method() == &Method::Invite && !k.is_server)
                    .cloned()
                    .collect();
                drop(client_txs);
                
                if let Some(invite_tx_id) = cancel::find_matching_invite_transaction(&request, invite_tx_keys) {
                    target_invite_tx_id = Some(invite_tx_id);
                    debug!(method=%request.method(), "Found matching INVITE transaction for CANCEL");
                } else {
                    debug!(method=%request.method(), "No matching INVITE transaction found for CANCEL");
                }
                
                // Create a non-INVITE server transaction for CANCEL
                let tx = Arc::new(ServerNonInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction for CANCEL");
                
                // If we found a matching INVITE transaction, notify the TU
                if let Some(invite_tx_id) = target_invite_tx_id {
                    self.events_tx.send(TransactionEvent::CancelRequest {
                        transaction_id: tx.id().clone(),
                        target_transaction_id: invite_tx_id,
                        request: request.clone(),
                        source: remote_addr,
                    }).await.ok();
                }
                
                tx
            },
            Method::Update => {
                // Validate the UPDATE request
                if let Err(e) = update::validate_update_request(&request) {
                    warn!(method = %request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                }
                
                // Create a non-INVITE server transaction for UPDATE
                let tx = Arc::new(ServerNonInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction for UPDATE");
                tx
            },
            _ => {
                let tx = Arc::new(ServerNonInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    tx_transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction");
                tx
            }
        };
        
        // Store the transaction
        {
            let mut server_txs = self.server_transactions.lock().await;
            server_txs.insert(transaction.id().clone(), transaction.clone());
        }
        
        // Start the transaction in Trying state (for non-INVITE) or Proceeding (for INVITE)
        let initial_state = match transaction.kind() {
            TransactionKind::InviteServer => TransactionState::Proceeding,
            _ => TransactionState::Trying,
        };
        
        // Transition to the initial active state
        if let Err(e) = transaction.send_command(InternalTransactionCommand::TransitionTo(initial_state)).await {
            error!(id=%transaction.id(), error=%e, "Failed to initialize new server transaction");
            return Err(e);
        }
        
        Ok(transaction)
    }

    /// Cancel an active INVITE client transaction
    ///
    /// Creates a CANCEL request based on the original INVITE and creates
    /// a new client transaction to send it.
    ///
    /// Returns the transaction ID of the new CANCEL transaction.
    pub async fn cancel_invite_transaction(
        &self,
        invite_tx_id: &TransactionKey,
    ) -> Result<TransactionKey> {
        debug!(id=%invite_tx_id, "Canceling invite transaction");
        
        // Check that this is an INVITE client transaction
        if invite_tx_id.method() != &Method::Invite || invite_tx_id.is_server() {
            return Err(Error::Other(format!(
                "Transaction {} is not an INVITE client transaction", invite_tx_id
            )));
        }
        
        // Get the original INVITE request 
        let invite_request = utils::get_transaction_request(
            &self.client_transactions,
            invite_tx_id
        ).await?;
        
        debug!(id=%invite_tx_id, "Got INVITE request for cancellation");
        
        // Create a CANCEL request from the INVITE
        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
        
        // Use the method utility to create the CANCEL request
        let cancel_request = cancel::create_cancel_request(&invite_request, &local_addr)?;
        
        // Log and validate the CANCEL request to help with debugging
        if let Err(e) = cancel::validate_cancel_request(&cancel_request) {
            warn!(method = %cancel_request.method(), error = %e, "CANCEL request validation issue - proceeding anyway");
        }
        
        // Get the destination for the CANCEL request (same as the INVITE)
        let destination = {
            let dest_map = self.transaction_destinations.lock().await;
            match dest_map.get(invite_tx_id) {
                Some(addr) => *addr,
                None => return Err(Error::Other(format!(
                    "No destination found for transaction {}", invite_tx_id
                ))),
            }
        };
        
        // Create a transaction for the CANCEL request
        let cancel_tx_id = self.create_client_transaction(
            cancel_request,
            destination,
        ).await?;
        
        debug!(id=%cancel_tx_id, original_id=%invite_tx_id, "Created CANCEL transaction");
        
        // Send the CANCEL request immediately
        self.send_request(&cancel_tx_id).await?;
        
        Ok(cancel_tx_id)
    }

    /// Creates a client transaction for a non-INVITE request.
    ///
    /// # Arguments
    /// * `request` - The non-INVITE request to send
    /// * `destination` - The destination address to send the request to
    ///
    /// # Returns
    /// * `Result<TransactionKey>` - The transaction ID on success, or an error
    pub async fn create_non_invite_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        if request.method() == Method::Invite {
            return Err(Error::Other("Cannot create non-INVITE transaction for INVITE request".to_string()));
        }
        
        self.create_client_transaction(request, destination).await
    }

    /// Creates a client transaction for an INVITE request.
    ///
    /// # Arguments
    /// * `request` - The INVITE request to send
    /// * `destination` - The destination address to send the request to
    ///
    /// # Returns
    /// * `Result<TransactionKey>` - The transaction ID on success, or an error 
    pub async fn create_invite_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Cannot create INVITE transaction for non-INVITE request".to_string()));
        }
        
        self.create_client_transaction(request, destination).await
    }

    /// Get information about available transport types and their capabilities
    /// 
    /// This method returns information about which transport types are available
    /// and their capabilities. This is useful for session-level components that
    /// need to know what transport options are available.
    pub fn get_transport_capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            supports_udp: self.transport.supports_udp(),
            supports_tcp: self.transport.supports_tcp(),
            supports_tls: self.transport.supports_tls(),
            supports_ws: self.transport.supports_ws(),
            supports_wss: self.transport.supports_wss(),
            local_addr: self.transport.local_addr().ok(),
            default_transport: self.transport.default_transport_type(),
        }
    }

    /// Get detailed information about a specific transport type
    /// 
    /// This method returns detailed information about a specific transport type,
    /// such as connection status, local address, etc.
    pub fn get_transport_info(&self, transport_type: TransportType) -> Option<TransportInfo> {
        if !self.transport.supports_transport(transport_type) {
            return None;
        }

        Some(TransportInfo {
            transport_type,
            is_connected: self.transport.is_transport_connected(transport_type),
            local_addr: self.transport.get_transport_local_addr(transport_type).ok(),
            connection_count: self.transport.get_connection_count(transport_type),
        })
    }

    /// Check if a specific transport type is available
    pub fn is_transport_available(&self, transport_type: TransportType) -> bool {
        self.transport.supports_transport(transport_type)
    }

    /// Get network information for SDP generation
    /// 
    /// This method returns network information that can be used for SDP generation,
    /// such as the local IP address and ports for different media types.
    pub fn get_network_info_for_sdp(&self) -> NetworkInfoForSdp {
        NetworkInfoForSdp {
            local_ip: self.transport.local_addr()
                .map(|addr| addr.ip())
                .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
            rtp_port_range: (10000, 20000), // Default port range, could be configurable
        }
    }

    /// Get the best transport type for a given URI
    /// 
    /// This method analyzes a URI and returns the best transport type to use
    /// based on the URI scheme and available transports.
    pub fn get_best_transport_for_uri(&self, uri: &rvoip_sip_core::Uri) -> TransportType {
        // Determine the best transport based on the URI scheme
        let scheme = uri.scheme().to_string();
        
        match scheme.as_str() {
            "sips" => {
                if self.transport.supports_tls() {
                    TransportType::Tls
                } else {
                    // Fallback to another secure transport if TLS is not available
                    if self.transport.supports_wss() {
                        TransportType::Wss
                    } else {
                        // Last resort: use any available transport
                        self.transport.default_transport_type()
                    }
                }
            },
            "ws" => {
                if self.transport.supports_ws() {
                    TransportType::Ws
                } else {
                    self.transport.default_transport_type()
                }
            },
            "wss" => {
                if self.transport.supports_wss() {
                    TransportType::Wss
                } else if self.transport.supports_tls() {
                    TransportType::Tls
                } else {
                    self.transport.default_transport_type()
                }
            },
            // Default for "sip:" and any other schemes
            _ => self.transport.default_transport_type()
        }
    }

    /// Get WebSocket connection status if available
    /// 
    /// This method returns information about WebSocket connections if WebSocket
    /// transport is supported and enabled.
    pub fn get_websocket_status(&self) -> Option<WebSocketStatus> {
        if !self.transport.supports_ws() && !self.transport.supports_wss() {
            return None;
        }

        Some(WebSocketStatus {
            ws_connections: self.transport.get_connection_count(TransportType::Ws),
            wss_connections: self.transport.get_connection_count(TransportType::Wss),
            has_active_connection: self.transport.is_transport_connected(TransportType::Ws) || 
                                   self.transport.is_transport_connected(TransportType::Wss),
        })
    }
}

impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Avoid trying to print the Mutex contents directly or requiring Debug on contents
        f.debug_struct("TransactionManager")
            .field("transport", &"Arc<dyn Transport>")
            .field("client_transactions", &"Arc<Mutex<HashMap<...>>>") // Indicate map exists
            .field("server_transactions", &"Arc<Mutex<HashMap<...>>>")
            .field("transaction_destinations", &"Arc<Mutex<HashMap<...>>>")
            .field("events_tx", &self.events_tx) // Sender might be Debug
            .field("event_subscribers", &"Arc<Mutex<Vec<Sender>>>")
            .field("transport_rx", &"Arc<Mutex<Receiver>>")
            .field("running", &self.running)
            .field("timer_settings", &self.timer_settings)
            .field("timer_manager", &"Arc<TimerManager>")
            .field("timer_factory", &"TimerFactory")
            .finish()
    } 
}
