/// # Transaction Manager Utilities
///
/// This module provides utility functions for the TransactionManager implementation.
/// These utilities help with common tasks required for SIP message processing and
/// transaction handling according to RFC 3261.
///
/// The utilities include:
/// - Extracting specific headers and values from SIP messages
/// - Resolving network addresses from SIP URIs
/// - Building standard SIP responses from requests
/// - Determining message routing based on SIP headers
///
/// These functions encapsulate low-level details, allowing the TransactionManager
/// to focus on higher-level transaction state management.

use std::net::SocketAddr;
use std::str::FromStr;
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};

use crate::transaction::error::{self, Error, Result};
use crate::transaction::{TransactionKey, Transaction};
use crate::transaction::client::ClientTransaction;
use crate::transaction::client::TransactionExt as ClientTransactionExt;
use crate::transaction::server::TransactionExt as ServerTransactionExt;

/// Extensions for response building to ensure RFC 3261 compliance.
///
/// RFC 3261 Section 8.2.6.2 specifies that responses need to copy specific 
/// headers from the request, including Via, From, To, Call-ID, and CSeq.
/// This trait simplifies that process.
pub trait ResponseBuilderExt {
    /// Copies essential headers from a request as required by RFC 3261.
    ///
    /// RFC 3261 Section 8.2.6.2 requires that certain headers be copied from
    /// the request to the response:
    /// - All Via headers (in same order)
    /// - From header
    /// - To header (optionally adding a tag)
    /// - Call-ID header
    /// - CSeq header
    ///
    /// # Arguments
    /// * `request` - The request to copy headers from
    ///
    /// # Returns
    /// * `Result<Self>` - The builder with headers added
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_dialog_core::transaction::manager::utils::ResponseBuilderExt;
    /// # fn example(request: &Request) -> rvoip_dialog_core::transaction::error::Result<Response> {
    /// let builder = ResponseBuilder::new(StatusCode::Ok, Some("OK"));
    /// let builder = builder.copy_essential_headers(request)?;
    /// let response = builder.build();
    /// # Ok(response)
    /// # }
    /// ```
    fn copy_essential_headers(self, request: &Request) -> Result<Self> where Self: Sized;
}

impl ResponseBuilderExt for ResponseBuilder {
    fn copy_essential_headers(mut self, request: &Request) -> Result<Self> {
        if let Some(via) = request.first_via() {
            self = self.header(TypedHeader::Via(via.clone()));
        }
        if let Some(to) = request.header(&HeaderName::To) {
            if let TypedHeader::To(to_val) = to {
                self = self.header(TypedHeader::To(to_val.clone()));
            }
        }
        if let Some(from) = request.header(&HeaderName::From) {
            if let TypedHeader::From(from_val) = from {
                self = self.header(TypedHeader::From(from_val.clone()));
            }
        }
        if let Some(call_id) = request.header(&HeaderName::CallId) {
            if let TypedHeader::CallId(call_id_val) = call_id {
                self = self.header(TypedHeader::CallId(call_id_val.clone()));
            }
        }
        if let Some(cseq) = request.header(&HeaderName::CSeq) {
            if let TypedHeader::CSeq(cseq_val) = cseq {
                self = self.header(TypedHeader::CSeq(cseq_val.clone()));
            }
        }
        self = self.header(TypedHeader::ContentLength(ContentLength::new(0)));
        Ok(self)
    }
}

/// Extract the socket address from a SIP URI if possible.
/// 
/// RFC 3261 Section 19.1.1 describes SIP URI syntax, which can include a host
/// and port. This function attempts to parse the host and port as a socket address,
/// which is needed for network transmission.
///
/// # Arguments
/// * `uri` - The SIP URI to extract address from
///
/// # Returns
/// * `Option<SocketAddr>` - Socket address if successful, None otherwise
///
/// # Example
/// ```
/// # use std::str::FromStr;
/// # use rvoip_sip_core::Uri;
/// # use rvoip_dialog_core::transaction::manager::utils::socket_addr_from_uri;
/// # fn example() {
/// let uri = Uri::from_str("sip:user@192.168.1.10:5060").unwrap();
/// if let Some(addr) = socket_addr_from_uri(&uri) {
///     println!("Socket address: {}", addr);  // 192.168.1.10:5060
/// }
/// # }
/// ```
pub fn socket_addr_from_uri(uri: &Uri) -> Option<SocketAddr> {
    let host = uri.host.to_string();
    let port = uri.port.unwrap_or(5060); // Default to 5060 if no port specified
    
    // Try to parse the host as an IP address
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        Some(SocketAddr::new(ip, port))
    } else {
        None
    }
}

/// Extract CSeq header from a SIP message.
///
/// The CSeq header is crucial for transaction matching and reliable request 
/// processing as defined in RFC 3261 Section 8.1.1.5.
///
/// # Arguments
/// * `message` - The SIP message to extract CSeq from
///
/// # Returns
/// * `Option<(u32, Method)>` - Tuple of sequence number and method if found
///
/// # Example
/// ```no_run
/// # use rvoip_sip_core::Message;
/// # use rvoip_dialog_core::transaction::manager::utils::extract_cseq;
/// # fn example(message: &Message) {
/// if let Some((seq_num, method)) = extract_cseq(message) {
///     println!("CSeq: {} {}", seq_num, method);
/// }
/// # }
/// ```
pub fn extract_cseq(message: &Message) -> Option<(u32, Method)> {
    match message {
        Message::Request(request) => {
            request.cseq().map(|cseq| (cseq.seq, cseq.method.clone()))
        },
        Message::Response(response) => {
            response.cseq().map(|cseq| (cseq.seq, cseq.method.clone()))
        }
    }
}

/// Determine the destination address for an ACK to a 2xx response.
///
/// According to RFC 3261 Section 13.2.2.4, ACK for 2xx responses should be sent to:
/// 1. The URI in the Contact header of the response, if present
/// 2. Otherwise, constructed by the procedures in Section 8.1.2
///
/// This function implements this logic to find the appropriate destination.
///
/// # Arguments
/// * `response` - The 2xx response to ACK
///
/// # Returns
/// * `Option<SocketAddr>` - Destination address for the ACK if it can be determined
///
/// # Example
/// ```no_run
/// # use rvoip_sip_core::Response;
/// # use rvoip_dialog_core::transaction::manager::utils::determine_ack_destination;
/// # async fn example(response: &Response) {
/// if let Some(dest) = determine_ack_destination(response).await {
///     println!("ACK destination: {}", dest);
/// }
/// # }
/// ```
pub async fn determine_ack_destination(response: &Response) -> Option<SocketAddr> {
    // Try to get destination from Contact header first
    if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
        if let Some(contact_addr) = contact.addresses().next() {
            debug!("Found Contact URI in response: {}", contact_addr.uri);
            
            // Try to parse the URI as a socket address
            if let Some(addr) = socket_addr_from_uri(&contact_addr.uri) {
                debug!("Parsed Contact URI to socket address: {}", addr);
                return Some(addr);
            }
        }
    }
    
    // Fall back to using the received/rport parameters in the top Via
    if let Some(via) = response.first_via() {
        // Extract host and port from Via header
        // Since we can't access the parameters directly, try to parse them from the Via string
        let via_str = via.to_string();
        
        // Look for received parameter
        if let Some(received_start) = via_str.find("received=") {
            let received_part = &via_str[received_start + 9..];
            if let Some(received_end) = received_part.find(';') {
                let received = &received_part[..received_end];
                if let Ok(ip) = received.parse::<std::net::IpAddr>() {
                    // Look for rport
                    let mut port = 5060;
                    if let Some(rport_start) = via_str.find("rport=") {
                        let rport_part = &via_str[rport_start + 6..];
                        if let Some(rport_end) = rport_part.find(';') {
                            if let Ok(rport) = rport_part[..rport_end].parse::<u16>() {
                                port = rport;
                            }
                        } else if let Ok(rport) = rport_part.parse::<u16>() {
                            port = rport;
                        }
                    }
                    
                    return Some(SocketAddr::new(ip, port));
                }
            }
        }
        
        // If we couldn't extract received/rport, try to use the sent-by part
        let host_start = via_str.find(' ').map(|pos| pos + 1).unwrap_or(0);
        let host_end = via_str[host_start..].find(';').map(|pos| host_start + pos).unwrap_or(via_str.len());
        let host_port = &via_str[host_start..host_end];
        
        if let Ok(addr) = host_port.parse::<SocketAddr>() {
            return Some(addr);
        } else if let Some(colon_pos) = host_port.find(':') {
            let host = &host_port[..colon_pos];
            let port = host_port[colon_pos+1..].parse::<u16>().unwrap_or(5060);
            
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                return Some(SocketAddr::new(ip, port));
            }
        } else if let Ok(ip) = host_port.parse::<std::net::IpAddr>() {
            return Some(SocketAddr::new(ip, 5060));
        }
    }
    
    None
}

/// Get the original request from a transaction.
///
/// Retrieves the original request that initiated a client transaction.
/// This is needed for many operations, including generating CANCEL and ACK requests.
///
/// ## Uses in SIP Transaction Layer
/// - Creating CANCEL requests based on an INVITE
/// - Creating ACK requests for final responses
/// - Validating incoming requests against stored requests
///
/// # Arguments
/// * `transactions` - Mutex-wrapped map of transactions
/// * `tx_id` - Transaction ID to look up
///
/// # Returns
/// * `Result<Request>` - The original request or an error
pub async fn get_transaction_request(
    transactions: &Mutex<HashMap<TransactionKey, Box<dyn ClientTransaction + Send>>>,
    tx_id: &TransactionKey
) -> Result<Request> {
    let transactions_lock = transactions.lock().await;
    
    if let Some(tx) = transactions_lock.get(tx_id) {
        if let Some(client_tx) = tx.as_client_transaction() {
            if let Some(request) = client_tx.original_request().await {
                return Ok(request);
            }
        }
    }
    
    Err(Error::transaction_not_found(tx_id.clone(), "get_transaction_request - transaction not found"))
}