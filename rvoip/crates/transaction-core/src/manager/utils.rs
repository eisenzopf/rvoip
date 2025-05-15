use std::net::SocketAddr;
use std::str::FromStr;
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};

use crate::error::{self, Error, Result};
use crate::transaction::{TransactionKey, Transaction};
use crate::client::ClientTransaction;
use crate::client::TransactionExt as ClientTransactionExt;
use crate::server::TransactionExt as ServerTransactionExt;

/// ResponseBuilderExt trait - Use specific accessors and wrap headers
pub trait ResponseBuilderExt {
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

/// Extract the socket address from a SIP URI if possible
/// Returns None if the host part is not an IP address or if port is missing and no default provided
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

/// Extract CSeq header from a SIP message
/// Returns a tuple of (sequence number, method)
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

/// Utility to determine the destination for an ACK to a 2xx response
/// Follows the RFC 3261 Section 13.2.2.4 rules
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

/// Get the original request from a transaction
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