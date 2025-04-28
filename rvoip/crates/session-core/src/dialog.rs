use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex};
use tracing::{debug, info, warn, error};
use dashmap::DashMap;
use std::net::{IpAddr, SocketAddr, Ipv4Addr, Ipv6Addr};

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, 
    Uri, Header, HeaderName, Message, CSeq, CallId
};

// Import SIP message types using the prelude pattern
use rvoip_sip_core::prelude::*;
// Import proper types from rvoip_sip_core::types
use rvoip_sip_core::types::route::Route;
use rvoip_sip_core::types::contact::Contact;
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::from::From as FromHeader;
use rvoip_sip_core::types::to::To as ToHeader;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::parser::headers::route::RouteEntry;

// Import transaction types
use rvoip_transaction_core::{
    TransactionManager,
    TransactionEvent,
    TransactionState,
    TransactionKey,
    TransactionKind,
};

use crate::errors::{Error, self};
// Add this line to define a Result type alias
type Result<T> = std::result::Result<T, Error>;

use crate::events::{EventBus, SessionEvent};
use crate::dialog_state::DialogState;

/// Unique identifier for a SIP dialog
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DialogId(pub Uuid);

impl DialogId {
    /// Create a new dialog ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for DialogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for DialogId {
    fn default() -> Self {
        Self::new()
    }
}

/// A SIP dialog as defined in RFC 3261
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dialog {
    /// Unique identifier for this dialog
    pub id: DialogId,
    
    /// Current state of the dialog
    pub state: DialogState,
    
    /// Call-ID for this dialog
    pub call_id: String,
    
    /// Local URI
    pub local_uri: Uri,
    
    /// Remote URI
    pub remote_uri: Uri,
    
    /// Local tag
    pub local_tag: Option<String>,
    
    /// Remote tag
    pub remote_tag: Option<String>,
    
    /// Local sequence number
    pub local_seq: u32,
    
    /// Remote sequence number
    pub remote_seq: u32,
    
    /// Remote target URI (where to send requests)
    pub remote_target: Uri,
    
    /// Route set for this dialog
    pub route_set: Vec<Uri>,
    
    /// Whether this dialog was created by local UA (true) or remote UA (false)
    pub is_initiator: bool,
}

impl Dialog {
    /// Create a dialog from a 2xx response to an INVITE
    pub fn from_2xx_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        use tracing::debug;
        
        if !matches!(response.status, StatusCode::Ok | StatusCode::Accepted) {
            debug!("Dialog creation failed: Response status is not 200 OK or 202 Accepted ({})", response.status);
            return None;
        }
        
        if request.method != Method::Invite {
            debug!("Dialog creation failed: Request method is not INVITE ({})", request.method);
            return None;
        }
        
        // Extract Call-ID using TypedHeader pattern
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => {
                debug!("Dialog creation failed: Missing or invalid Call-ID header");
                return None;
            }
        };
        
        // Extract CSeq using TypedHeader pattern
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => {
                debug!("Dialog creation failed: Missing or invalid CSeq header in request");
                return None;
            }
        };
        
        // Extract To using TypedHeader pattern
        let (to_tag, to_value) = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => {
                let to_tag = to.tag();
                let to_str = to.to_string();
                if to_tag.is_none() {
                    debug!("Dialog creation failed: Missing tag in To header");
                }
                (to_tag, to_str)
            },
            _ => {
                debug!("Dialog creation failed: Missing or invalid To header");
                return None;
            }
        };
        
        // Extract From using TypedHeader pattern
        let (from_tag, from_value) = match response.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => {
                let from_tag = from.tag();
                let from_str = from.to_string();
                if from_tag.is_none() {
                    debug!("Dialog creation failed: Missing tag in From header");
                }
                (from_tag, from_str)
            },
            _ => {
                debug!("Dialog creation failed: Missing or invalid From header");
                return None;
            }
        };
        
        debug!("Tags - From: {:?}, To: {:?}", from_tag, to_tag);
        
        // From RFC 3261: The to-tag and from-tag are required for dialog
        if to_tag.is_none() {
            debug!("Dialog creation failed: Missing required To tag");
            return None;
        }
        
        let remote_tag = if is_initiator { 
            to_tag.map(|s| s.to_string()) 
        } else { 
            from_tag.map(|s| s.to_string()) 
        };
        
        let local_tag = if is_initiator { 
            from_tag.map(|s| s.to_string()) 
        } else { 
            to_tag.map(|s| s.to_string()) 
        };
        
        let local_uri_result = extract_uri(if is_initiator { &from_value } else { &to_value });
        if local_uri_result.is_none() {
            debug!("Dialog creation failed: Could not extract local URI from {}", 
                if is_initiator { "From" } else { "To" });
            return None;
        }
        let local_uri = local_uri_result.unwrap();
        
        let remote_uri_result = extract_uri(if is_initiator { &to_value } else { &from_value });
        if remote_uri_result.is_none() {
            debug!("Dialog creation failed: Could not extract remote URI from {}", 
                if is_initiator { "To" } else { "From" });
            return None;
        }
        let remote_uri = remote_uri_result.unwrap();
        
        // Extract Contact using TypedHeader pattern
        let contact_uri = match response.header(&HeaderName::Contact) {
            Some(TypedHeader::Contact(contact)) => {
                // Contact may have multiple values, use the first one
                if let Some(address) = contact.addresses().next() {
                    let uri = address.uri.clone();
                    debug!("Using contact URI: {}", uri);
                    uri
                } else {
                    debug!("Dialog creation failed: Empty Contact header");
                    return None;
                }
            },
            _ => {
                debug!("Dialog creation failed: Missing or invalid Contact header");
                return None;
            }
        };
        
        // Extract route set from Record-Route headers
        let mut route_set = Vec::new();
        // Record-Route headers may not exist, so this part is optional
        if let Some(record_route) = response.header(&HeaderName::RecordRoute) {
            if let TypedHeader::RecordRoute(rr) = record_route {
                for entry in rr.reversed() {
                    route_set.push(entry.uri().clone());
                }
            }
        }
        
        debug!("Dialog created successfully");
        
        Some(Self {
            id: DialogId::new(),
            state: DialogState::Confirmed,
            call_id,
            local_uri,
            remote_uri,
            local_tag,
            remote_tag,
            local_seq: if is_initiator { cseq_number } else { 0 },
            remote_seq: if is_initiator { 0 } else { cseq_number },
            remote_target: contact_uri,
            route_set,
            is_initiator,
        })
    }
    
    /// Create a dialog from an early (1xx) response to an INVITE
    pub fn from_provisional_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        // Only create early dialogs from reliable provisional responses (e.g., 183)
        let status_code = response.status.as_u16();
        if !(100..200).contains(&status_code) {
            return None;
        }
        
        if request.method != Method::Invite {
            return None;
        }
        
        // Extract To header using TypedHeader pattern
        let (to_tag, to_value) = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => {
                let to_tag = to.tag();
                if to_tag.is_none() {
                    return None;
                }
                (to_tag, to.to_string())
            },
            _ => return None,
        };
        
        // Extract Call-ID using TypedHeader pattern
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => return None,
        };
        
        // Extract CSeq using TypedHeader pattern
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => return None,
        };
        
        // Extract From using TypedHeader pattern
        let (from_tag, from_value) = match response.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => {
                let from_tag = from.tag();
                (from_tag, from.to_string())
            },
            _ => return None,
        };
        
        let remote_tag = if is_initiator { 
            to_tag.map(|s| s.to_string()) 
        } else { 
            from_tag.map(|s| s.to_string()) 
        };
        
        let local_tag = if is_initiator { 
            from_tag.map(|s| s.to_string()) 
        } else { 
            to_tag.map(|s| s.to_string()) 
        };
        
        let local_uri = extract_uri(if is_initiator { &from_value } else { &to_value })?;
        let remote_uri = extract_uri(if is_initiator { &to_value } else { &from_value })?;
        
        // Extract Contact using TypedHeader pattern
        let contact_uri = match response.header(&HeaderName::Contact) {
            Some(TypedHeader::Contact(contact)) => {
                // Contact may have multiple values, use the first one
                if let Some(address) = contact.addresses().next() {
                    address.uri.clone()
                } else {
                    return None;
                }
            },
            _ => return None,
        };
        
        // Extract route set from Record-Route headers
        let mut route_set = Vec::new();
        if let Some(record_route) = response.header(&HeaderName::RecordRoute) {
            if let TypedHeader::RecordRoute(rr) = record_route {
                for entry in rr.reversed() {
                    route_set.push(entry.uri().clone());
                }
            }
        }
        
        Some(Self {
            id: DialogId::new(),
            state: DialogState::Early,
            call_id,
            local_uri,
            remote_uri,
            local_tag,
            remote_tag,
            local_seq: if is_initiator { cseq_number } else { 0 },
            remote_seq: if is_initiator { 0 } else { cseq_number },
            remote_target: contact_uri,
            route_set,
            is_initiator,
        })
    }
    
    /// Update an early dialog to a confirmed dialog from a 2xx response
    pub fn update_from_2xx(&mut self, response: &Response) -> bool {
        if !matches!(response.status, StatusCode::Ok | StatusCode::Accepted) {
            return false;
        }
        
        if self.state != DialogState::Early {
            return false;
        }
        
        // Update the remote target URI if a contact header is present
        if let Some(contact) = response.header(&HeaderName::Contact) {
            if let TypedHeader::Contact(contact_header) = contact {
                if let Some(address) = contact_header.address() {
                    let contact_uri = address.uri.clone();
                    self.remote_target = contact_uri;
                }
            }
        }
        
        // Update remote tag if needed
        if let Some(to) = response.header(&HeaderName::To) {
            if let TypedHeader::To(to_header) = to {
                let to_value = to_header.to_string();
                if self.is_initiator {
                    self.remote_tag = extract_tag(&to_value);
                }
            }
        }
        
        // Update state to confirmed
        self.state = DialogState::Confirmed;
        
        true
    }
    
    /// Create a new request within this dialog
    pub fn create_request(&mut self, method: Method) -> Request {
        use tracing::debug;
        
        debug!("Creating request for method {} in dialog {}", method, self.id);
        
        // Increment local sequence number for new requests
        if method != Method::Ack {
            self.local_seq += 1;
        }
        
        // Ensure local and remote tags are set
        if self.local_tag.is_none() {
            debug!("Warning: Local tag is missing, generating a random tag");
            self.local_tag = Some(format!("autogen-{}", uuid::Uuid::new_v4()));
        }
        
        let mut request = Request::new(method.clone(), self.remote_target.clone());
        
        // Create typed headers
        let call_id = TypedHeader::CallId(CallId(self.call_id.clone()));
        
        // Add From header with local tag
        let local_tag_value = self.local_tag.as_ref().unwrap_or(&"".to_string()).clone();
        let from_value = format!("<{}>;tag={}", self.local_uri, local_tag_value);
        let from = TypedHeader::From(
            FromHeader::from_str(&from_value).unwrap_or_else(|_| {
                debug!("Error parsing From header: {}", from_value);
                FromHeader::new(Address::new(Uri::sip("unknown")))
            })
        );
        
        // Add To header with remote tag
        let mut to_value = format!("<{}>", self.remote_uri);
        if let Some(tag) = &self.remote_tag {
            to_value.push_str(&format!(";tag={}", tag));
        } else {
            debug!("Warning: Remote tag is missing in dialog");
        }
        let to = TypedHeader::To(
            ToHeader::from_str(&to_value).unwrap_or_else(|_| {
                debug!("Error parsing To header: {}", to_value);
                ToHeader::new(Address::new(Uri::sip("unknown")))
            })
        );
        
        // Add CSeq
        let cseq_value = CSeq::new(self.local_seq, request.method.clone());
        let cseq = TypedHeader::CSeq(cseq_value);
        
        // Insert the headers at the beginning of the request
        let mut headers = Vec::new();
        headers.push(call_id);
        headers.push(from);
        headers.push(to);
        headers.push(cseq);
        
        // Add route set if present
        if !self.route_set.is_empty() {
            // Create a single Route header with all URIs
            let mut route_entries = Vec::new();
            for uri in &self.route_set {
                let address = Address::new(uri.clone());
                let route_entry = RouteEntry(address);
                route_entries.push(route_entry);
            }
            
            let route = Route::new(route_entries);
            headers.push(TypedHeader::Route(route));
        }
        
        // Replace the headers in the request with our new ones
        request.headers = headers;
        
        debug!("Created {} request for dialog {}", method, self.id);
        
        request
    }
    
    /// Update dialog state to terminated
    pub fn terminate(&mut self) {
        self.state = DialogState::Terminated;
    }
    
    /// Check if the dialog is terminated
    pub fn is_terminated(&self) -> bool {
        self.state == DialogState::Terminated
    }
    
    /// Get the dialog ID tuple (call-id, local-tag, remote-tag)
    pub fn dialog_id_tuple(&self) -> Option<(String, String, String)> {
        match (&self.call_id, &self.local_tag, &self.remote_tag) {
            (call_id, Some(local_tag), Some(remote_tag)) => {
                Some((call_id.clone(), local_tag.clone(), remote_tag.clone()))
            }
            _ => None,
        }
    }
}

/// Extract a tag parameter from a SIP header value
pub fn extract_tag(header_value: &str) -> Option<String> {
    if let Some(tag_pos) = header_value.find(";tag=") {
        let tag_start = tag_pos + 5; // ";tag=" length
        let tag_end = header_value[tag_start..]
            .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
            .map(|pos| tag_start + pos)
            .unwrap_or(header_value.len());
        Some(header_value[tag_start..tag_end].to_string())
    } else {
        None
    }
}

/// Extract a URI from a SIP header value 
/// (typically from Contact, From, or To headers)
pub fn extract_uri(header_value: &str) -> Option<Uri> {
    use tracing::debug;

    // Check for URI enclosed in < >
    if let Some(uri_start) = header_value.find('<') {
        let uri_start = uri_start + 1;
        if let Some(uri_end) = header_value[uri_start..].find('>') {
            let uri_str = &header_value[uri_start..(uri_start + uri_end)];
            let uri_result = Uri::from_str(uri_str);
            match &uri_result {
                Ok(uri) => debug!("Extracted URI from <...>: {}", uri),
                Err(e) => debug!("Failed to parse URI from <{}>: {}", uri_str, e),
            }
            return uri_result.ok();
        } else {
            debug!("Found opening < but no closing > in: {}", header_value);
        }
    }
    
    // If no < > found, try to extract URI directly
    // Look for scheme:user@host or just scheme:host
    if let Some(scheme_end) = header_value.find(':') {
        let scheme = &header_value[0..scheme_end];
        if scheme == "sip" || scheme == "sips" || scheme == "tel" {
            // Find end of URI (whitespace, comma, semicolon)
            let uri_end = header_value[scheme_end..]
                .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
                .map(|pos| scheme_end + pos)
                .unwrap_or(header_value.len());
            
            let uri_str = &header_value[0..uri_end];
            let uri_result = Uri::from_str(uri_str);
            match &uri_result {
                Ok(uri) => debug!("Extracted URI from scheme:host: {}", uri),
                Err(e) => debug!("Failed to parse URI from {}: {}", uri_str, e),
            }
            return uri_result.ok();
        }
    }
    
    // Try to extract from display-name <...> format - get everything before ;tag= if present
    let without_params = if let Some(param_pos) = header_value.find(';') {
        &header_value[0..param_pos]
    } else {
        header_value
    };
    
    // Try to extract just the domain part and make a SIP URI
    let host_part = without_params
        .trim_start_matches("sip:")
        .trim_start_matches("sips:")
        .trim_start_matches("tel:")
        .split('@')
        .last()
        .unwrap_or(without_params)
        .trim();
    
    // Handle domain with port
    let host_port_parts: Vec<&str> = host_part.split(':').collect();
    let host_only = host_port_parts[0];
    
    // Try to make a SIP URI from the host
    if !host_only.is_empty() {
        let uri_str = format!("sip:{}", host_only);
        let uri_result = Uri::from_str(&uri_str);
        match &uri_result {
            Ok(uri) => {
                debug!("Constructed URI from host part: {}", uri);
                return uri_result.ok();
            },
            Err(e) => debug!("Failed final URI construction attempt: {}", e),
        }
    }
    
    debug!("All URI extraction attempts failed for: {}", header_value);
    None
} 

/// Manager for SIP dialogs that integrates with the transaction layer
pub struct DialogManager {
    /// Active dialogs by ID
    dialogs: DashMap<DialogId, Dialog>,
    
    /// Dialog lookup by SIP dialog identifier tuple (call-id, local-tag, remote-tag)
    dialog_lookup: DashMap<(String, String, String), DialogId>,
    
    /// DialogId mapped to SessionId for session references
    dialog_to_session: DashMap<DialogId, crate::session::SessionId>,
    
    /// Transaction manager reference
    transaction_manager: Arc<TransactionManager>,
    
    /// Transaction to Dialog mapping
    transaction_to_dialog: DashMap<TransactionKey, DialogId>,
    
    /// Event bus for dialog events
    event_bus: EventBus,
}

impl DialogManager {
    /// Create a new dialog manager
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
    ) -> Self {
        Self {
            dialogs: DashMap::new(),
            dialog_lookup: DashMap::new(),
            dialog_to_session: DashMap::new(),
            transaction_manager,
            transaction_to_dialog: DashMap::new(),
            event_bus,
        }
    }
    
    /// Subscribe to transaction events and start processing them
    pub async fn start(&self) -> tokio::sync::mpsc::Receiver<TransactionEvent> {
        // Subscribe to transaction events
        let mut events_rx = self.transaction_manager.subscribe();
        
        // Clone references for the task
        let dialog_manager = self.clone();
        
        // Spawn a task to process transaction events
        tokio::spawn(async move {
            while let Some(event) = events_rx.recv().await {
                match &event {
                    // Handle transaction timeout specially
                    TransactionEvent::TransactionTimeout { transaction_id } => {
                        debug!("Transaction timeout event received for {}", transaction_id);
                        
                        // Check if this transaction is associated with a dialog
                        if let Some(dialog_id) = dialog_manager.transaction_to_dialog.get(transaction_id) {
                            let dialog_id = dialog_id.clone();
                            if let Some(mut dialog) = dialog_manager.dialogs.get_mut(&dialog_id) {
                                if dialog.state != DialogState::Terminated {
                                    debug!("Terminating dialog {} due to transaction timeout", dialog_id);
                                    dialog.terminate();
                                    
                                    // Publish termination event
                                    if let Some(session_id) = dialog_manager.dialog_to_session.get(&dialog_id) {
                                        dialog_manager.event_bus.publish(SessionEvent::Terminated {
                                            session_id: session_id.clone(),
                                            reason: "Transaction timeout".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    },
                    // ACK timeout is similar to transaction timeout for INVITE server transactions
                    TransactionEvent::AckTimeout { transaction_id } => {
                        debug!("ACK timeout event received for {}", transaction_id);
                        
                        // Handle similar to transaction timeout - dialog may transition to terminated
                        // if the application layer decides that missing ACK should terminate the dialog
                        if let Some(dialog_id) = dialog_manager.transaction_to_dialog.get(transaction_id) {
                            // For now we log but don't terminate the dialog - RFC 3261 allows dialog to continue
                            // even if ACK is not received for the final response
                            debug!("Dialog {} associated with transaction that timed out waiting for ACK", 
                                dialog_id);
                        }
                    },
                    // Handle transport errors that should terminate dialogs
                    TransactionEvent::TransportError { transaction_id } => {
                        debug!("Transport error event received for {}", transaction_id);
                        
                        if let Some(dialog_id) = dialog_manager.transaction_to_dialog.get(transaction_id) {
                            let dialog_id = dialog_id.clone();
                            if let Some(mut dialog) = dialog_manager.dialogs.get_mut(&dialog_id) {
                                debug!("Terminating dialog {} due to transport error", dialog_id);
                                dialog.terminate();
                                
                                // Publish termination event
                                if let Some(session_id) = dialog_manager.dialog_to_session.get(&dialog_id) {
                                    dialog_manager.event_bus.publish(SessionEvent::Terminated {
                                        session_id: session_id.clone(),
                                        reason: "Transport error".to_string(),
                                    });
                                }
                            }
                        }
                    },
                    // Process all other events normally
                    _ => dialog_manager.process_transaction_event(event).await,
                }
            }
        });
        
        // Return a copy of the subscription for the caller to use if needed
        self.transaction_manager.subscribe()
    }
    
    /// Process a transaction event and update dialogs accordingly
    async fn process_transaction_event(&self, event: TransactionEvent) {
        match event {
            // Handle new INVITE request
            TransactionEvent::NewRequest { 
                transaction_id, 
                request, 
                source 
            } if request.method() == Method::Invite => {
                debug!("New INVITE request received for transaction: {}", transaction_id);
                // Dialog will be created when a response with to-tag is sent
                // No action needed at this point
            },
            
            // Handle a provisional response which may create an early dialog
            TransactionEvent::ProvisionalResponse { 
                transaction_id, 
                response 
            } => {
                self.handle_provisional_response(&transaction_id, response).await;
            },
            
            // Handle a success response which will create or confirm a dialog
            TransactionEvent::SuccessResponse { 
                transaction_id, 
                response 
            } => {
                self.handle_success_response(&transaction_id, response).await;
            },
            
            // Handle failure responses (potentially terminate dialogs)
            TransactionEvent::FailureResponse { 
                transaction_id, 
                response 
            } => {
                debug!("Failure response for transaction: {}", transaction_id);
                // If we have an associated dialog, we may want to terminate it
                if let Some(dialog_id) = self.transaction_to_dialog.get(&transaction_id) {
                    let dialog_id = dialog_id.clone();
                    if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
                        debug!("Terminating dialog {} due to failure response", dialog_id);
                        dialog.terminate();
                        
                        // Emit dialog terminated event
                        let session_id = self.dialog_to_session.get(&dialog_id).map(|id| id.clone());
                        if let Some(session_id) = session_id {
                            self.event_bus.publish(SessionEvent::Terminated {
                                session_id,
                                reason: format!("Failure response: {}", response.status()),
                            });
                        }
                    }
                }
            },
            
            // Handle provisional response sent by us
            TransactionEvent::ProvisionalResponseSent { 
                transaction_id, 
                response 
            } => {
                self.handle_provisional_response_sent(&transaction_id, response).await;
            },
            
            // Handle final response sent by us
            TransactionEvent::FinalResponseSent { 
                transaction_id, 
                response 
            } => {
                self.handle_final_response_sent(&transaction_id, response).await;
            },
            
            // Handle ACK received
            TransactionEvent::AckReceived { 
                transaction_id, 
                ack_request 
            } => {
                self.handle_ack_received(&transaction_id, ack_request).await;
            },
            
            // Handle CANCEL received (potentially terminate a dialog)
            TransactionEvent::CancelReceived {
                transaction_id,
                cancel_request
            } => {
                debug!("CANCEL received for transaction: {}", transaction_id);
                if let Some(dialog_id) = self.transaction_to_dialog.get(&transaction_id) {
                    let dialog_id = dialog_id.clone();
                    if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
                        // Only terminate if in early state
                        if dialog.state == DialogState::Early {
                            debug!("Terminating early dialog {} due to CANCEL", dialog_id);
                            dialog.terminate();
                            
                            // Emit dialog terminated event
                            let session_id = self.dialog_to_session.get(&dialog_id).map(|id| id.clone());
                            if let Some(session_id) = session_id {
                                self.event_bus.publish(SessionEvent::Terminated {
                                    session_id,
                                    reason: "CANCEL received".to_string(),
                                });
                            }
                        }
                    }
                }
            },
            
            // Handle BYE request to terminate a dialog
            TransactionEvent::NewRequest { 
                transaction_id, 
                request, 
                source 
            } if request.method() == Method::Bye => {
                self.handle_bye_request(&transaction_id, request).await;
            },
            
            // Handle transaction timeout
            TransactionEvent::TransactionTimeout {
                transaction_id
            } => {
                debug!("Transaction timeout for: {}", transaction_id);
                if let Some(dialog_id) = self.transaction_to_dialog.get(&transaction_id) {
                    let dialog_id = dialog_id.clone();
                    if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
                        // Only terminate if not already terminated
                        if dialog.state != DialogState::Terminated {
                            debug!("Terminating dialog {} due to transaction timeout", dialog_id);
                            dialog.terminate();
                            
                            // Emit dialog terminated event
                            let session_id = self.dialog_to_session.get(&dialog_id).map(|id| id.clone());
                            if let Some(session_id) = session_id {
                                self.event_bus.publish(SessionEvent::Terminated {
                                    session_id,
                                    reason: "Transaction timeout".to_string(),
                                });
                            }
                        }
                    }
                }
            },
            
            // Ignore other events
            _ => {}
        }
    }
    
    /// Handle an incoming provisional response which may create an early dialog
    async fn handle_provisional_response(&self, transaction_id: &TransactionKey, response: Response) {
        debug!("Provisional response for transaction: {}", transaction_id);
        
        // Only interested in responses > 100 with to-tag for dialog creation
        if response.status().as_u16() <= 100 || !self.has_to_tag(&response) {
            return;
        }
        
        // Get the original request
        let request = match self.get_transaction_request(transaction_id).await {
            Ok(Some(req)) if req.method() == Method::Invite => req,
            _ => return,
        };
        
        // Create early dialog using the new method
        if let Some(dialog_id) = self.create_dialog_from_transaction(transaction_id, &request, &response, true).await {
            debug!("Created early dialog {} from provisional response", dialog_id);
            
            // Emit dialog updated event if associated with a session
            if let Some(session_id) = self.find_session_for_transaction(transaction_id) {
                debug!("Associating dialog {} with session {}", dialog_id, session_id);
                let _ = self.associate_with_session(&dialog_id, &session_id);
                
                // Emit dialog updated event
                self.event_bus.publish(SessionEvent::DialogUpdated {
                    session_id,
                    dialog_id,
                });
            }
        }
    }
    
    /// Handle an incoming success response which will create or confirm a dialog
    async fn handle_success_response(&self, transaction_id: &TransactionKey, response: Response) {
        debug!("Success response for transaction: {}", transaction_id);
        
        // Get the original request
        let request = match self.get_transaction_request(transaction_id).await {
            Ok(Some(req)) if req.method() == Method::Invite => req,
            _ => return,
        };
        
        // Check if we already have an early dialog for this transaction
        let existing_dialog_id = self.transaction_to_dialog.get(transaction_id).map(|id| id.clone());
        
        if let Some(dialog_id) = existing_dialog_id {
            // Try to get mutable access to the dialog
            if let Some(mut dialog_entry) = self.dialogs.get_mut(&dialog_id) {
                debug!("Updating existing dialog {:?} with final response", dialog_id);
                
                // Update early dialog to confirmed
                if dialog_entry.update_from_2xx(&response) {
                    // Get dialog tuple
                    if let Some(tuple) = dialog_entry.dialog_id_tuple() {
                        drop(dialog_entry); // Release the reference before modifying other maps
                        
                        // Update dialog tuple mapping
                        self.dialog_lookup.insert(tuple, dialog_id.clone());
                        
                        // Publish event
                        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                            let session_id = session_id_ref.clone();
                            drop(session_id_ref); // Release the reference
                            
                            // Emit dialog updated event
                            self.event_bus.publish(SessionEvent::DialogUpdated {
                                session_id,
                                dialog_id: dialog_id.clone(),
                            });
                        }
                    }
                }
                return;
            }
        }
        
        // No existing dialog, create a new one using the new method
        if let Some(dialog_id) = self.create_dialog_from_transaction(transaction_id, &request, &response, true).await {
            debug!("Created confirmed dialog {} from 2xx response", dialog_id);
            
            // Emit dialog updated event if associated with a session
            if let Some(session_id) = self.find_session_for_transaction(transaction_id) {
                debug!("Associating dialog {} with session {}", dialog_id, session_id);
                let _ = self.associate_with_session(&dialog_id, &session_id);
                
                // Emit dialog updated event
                self.event_bus.publish(SessionEvent::DialogUpdated {
                    session_id,
                    dialog_id,
                });
            }
        }
    }
    
    /// Handle a provisional response sent by us
    async fn handle_provisional_response_sent(&self, transaction_id: &TransactionKey, response: Response) {
        debug!("Sent provisional response for transaction: {}", transaction_id);
        
        // Only interested in responses > 100 with to-tag for dialog creation
        if response.status().as_u16() <= 100 || !self.has_to_tag(&response) {
            return;
        }
        
        // Get the original request
        let request = match self.get_transaction_request(transaction_id).await {
            Ok(Some(req)) if req.method() == Method::Invite => req,
            _ => return,
        };
        
        // Create early dialog
        self.create_dialog_from_provisional(transaction_id, &request, &response, false).await;
    }
    
    /// Handle a final response sent by us
    async fn handle_final_response_sent(&self, transaction_id: &TransactionKey, response: Response) {
        debug!("Sent final response for transaction: {}", transaction_id);
        
        // Only interested in success responses for INVITE
        if !response.status().is_success() {
            return;
        }
        
        // Get the original request
        let request = match self.get_transaction_request(transaction_id).await {
            Ok(Some(req)) if req.method() == Method::Invite => req,
            _ => return,
        };
        
        // Create or update dialog
        self.create_or_update_dialog_from_final(transaction_id, &request, &response, false).await;
    }
    
    /// Handle an ACK received which may confirm a dialog
    async fn handle_ack_received(&self, transaction_id: &TransactionKey, ack_request: Request) {
        debug!("ACK received for transaction: {}", transaction_id);
        
        // Find the associated dialog ID
        let dialog_id = match self.transaction_to_dialog.get(transaction_id) {
            Some(id_ref) => {
                let id = id_ref.clone();
                drop(id_ref); // Release the reference
                id
            },
            None => return,
        };
        
        // Check if this is an INVITE server transaction in Completed state
        match self.transaction_manager.transaction_state(transaction_id).await {
            Ok(TransactionState::Completed) => {
                // Get the transaction kind to verify it's an INVITE server transaction
                match self.transaction_manager.transaction_kind(transaction_id).await {
                    Ok(TransactionKind::InviteServer) => {
                        // Correct transaction type for ACK handling
                        debug!("ACK received for INVITE server transaction in Completed state");
                        
                        // Get mutable access to the dialog
                        if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
                            // Check if dialog is in early state and should be confirmed
                            if dialog.state == DialogState::Early {
                                debug!("Confirming dialog {} after receiving ACK", dialog_id);
                                dialog.state = DialogState::Confirmed;
                                
                                // Emit dialog updated event
                                let session_id = match self.dialog_to_session.get(&dialog_id) {
                                    Some(id_ref) => {
                                        let id = id_ref.clone();
                                        drop(id_ref);
                                        Some(id)
                                    },
                                    None => None,
                                };
                                
                                if let Some(session_id) = session_id {
                                    self.event_bus.publish(SessionEvent::DialogUpdated {
                                        session_id,
                                        dialog_id: dialog_id.clone(),
                                    });
                                }
                            }
                        }
                    },
                    _ => debug!("ACK received for non-INVITE server transaction, ignoring"),
                }
            },
            Ok(state) => debug!("ACK received for transaction in {:?} state, ignoring", state),
            Err(e) => error!("Failed to get transaction state: {}", e),
        }
    }
    
    /// Handle a BYE request which terminates a dialog
    async fn handle_bye_request(&self, transaction_id: &TransactionKey, request: Request) {
        debug!("BYE request received for transaction: {}", transaction_id);
        
        // Try to find the associated dialog based on the request headers
        let dialog_id = match self.find_dialog_for_request(&request) {
            Some(id) => id,
            None => {
                debug!("No dialog found for BYE request");
                return;
            },
        };
        
        debug!("Found dialog {} for BYE request", dialog_id);
        
        // Update dialog state to Terminated
        if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
            dialog.state = DialogState::Terminated;
            drop(dialog); // Release the lock
            
            // Associate this transaction with the dialog for subsequent events
            self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
            
            // Emit dialog terminated event
            let session_id = match self.dialog_to_session.get(&dialog_id) {
                Some(id_ref) => {
                    let id = id_ref.clone();
                    drop(id_ref);
                    Some(id)
                },
                None => None,
            };
            
            if let Some(session_id) = session_id {
                self.event_bus.publish(SessionEvent::Terminated {
                    session_id,
                    reason: "BYE received".to_string(),
                });
            }
        }
    }
    
    /// Check if a response has a to-tag
    fn has_to_tag(&self, response: &Response) -> bool {
        // Get the To header
        if let Some(header) = response.header(&HeaderName::To) {
            // Extract the header text and check for tag
            match header {
                TypedHeader::To(to) => to.tag().is_some(),
                _ => false
            }
        } else {
            false
        }
    }
    
    /// Create a dialog from a provisional response
    async fn create_dialog_from_provisional(
        &self, 
        transaction_id: &TransactionKey, 
        request: &Request, 
        response: &Response,
        is_initiator: bool
    ) {
        match Dialog::from_provisional_response(request, response, is_initiator) {
            Some(dialog) => {
                debug!("Created early dialog {} from provisional response", dialog.id);
                
                // Get dialog ID and dialog tuple
                let dialog_id = dialog.id.clone();
                let dialog_tuple = dialog.dialog_id_tuple();
                
                // Store the dialog first
                self.dialogs.insert(dialog_id.clone(), dialog);
                
                // Associate the transaction with this dialog
                self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
                
                // Save dialog tuple mapping if available
                if let Some(tuple) = dialog_tuple {
                    self.dialog_lookup.insert(tuple, dialog_id);
                }
            },
            None => {
                debug!("Failed to create early dialog from provisional response");
            }
        }
    }
    
    /// Create or update a dialog from a final response
    async fn create_or_update_dialog_from_final(
        &self, 
        transaction_id: &TransactionKey, 
        request: &Request, 
        response: &Response,
        is_initiator: bool
    ) {
        // Check if we already have an early dialog for this transaction
        let existing_dialog_id = match self.transaction_to_dialog.get(transaction_id) {
            Some(id_ref) => {
                let id = id_ref.clone();
                drop(id_ref); // Release the reference before getting a mutable one
                Some(id)
            },
            None => None,
        };
        
        if let Some(dialog_id) = existing_dialog_id {
            // Try to get mutable access to the dialog
            if let Some(mut dialog_entry) = self.dialogs.get_mut(&dialog_id) {
                debug!("Updating existing dialog {:?} with final response", dialog_id);
                
                // Update early dialog to confirmed
                if dialog_entry.update_from_2xx(response) {
                    // Get dialog tuple
                    if let Some(tuple) = dialog_entry.dialog_id_tuple() {
                        drop(dialog_entry); // Release the reference before modifying other maps
                        
                        // Update dialog tuple mapping
                        self.dialog_lookup.insert(tuple, dialog_id.clone());
                        
                        // Publish event
                        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                            let session_id = session_id_ref.clone();
                            drop(session_id_ref); // Release the reference
                            
                            // Emit dialog updated event
                            self.event_bus.publish(SessionEvent::DialogUpdated {
                                session_id,
                                dialog_id: dialog_id.clone(),
                            });
                        }
                    }
                }
                return;
            }
        }
        
        // No existing dialog, create a new one
        match Dialog::from_2xx_response(request, response, is_initiator) {
            Some(dialog) => {
                debug!("Created new dialog {} from 2xx response", dialog.id);
                
                // Get dialog ID and tuple
                let dialog_id = dialog.id.clone();
                let dialog_tuple = dialog.dialog_id_tuple();
                
                // Store the dialog
                self.dialogs.insert(dialog_id.clone(), dialog);
                
                // Make additional associations
                self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
                
                // Add tuple mapping if available
                if let Some(tuple) = dialog_tuple {
                    self.dialog_lookup.insert(tuple, dialog_id);
                }
            },
            None => {
                debug!("Failed to create dialog from 2xx response");
            }
        }
    }
    
    /// Get the original request from a transaction
    async fn get_transaction_request(&self, transaction_id: &TransactionKey) -> Result<Option<Request>> {
        // Since we don't have direct access to get_transaction, we'll need to use a different approach.
        // We'll query active transactions and pull from them if found.
        
        // If we have a cached request for this transaction, we could use that
        // For now, just return None to indicate we can't retrieve it
        debug!("Unable to directly retrieve transaction request - transaction API has changed");
        
        // TODO: Implement a proper caching mechanism for transaction requests
        Ok(None)
    }
    
    /// Find the dialog for an in-dialog request
    fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        // Extract call-id
        let call_id = match request.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => return None
        };
        
        // Extract tags
        let from_tag = match request.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => from.tag().map(|s| s.to_string()),
            _ => None
        };
        
        let to_tag = match request.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => to.tag().map(|s| s.to_string()),
            _ => None
        };
        
        // Both tags are required for dialog lookup
        if from_tag.is_none() || to_tag.is_none() {
            return None;
        }
        
        let from_tag = from_tag.unwrap();
        let to_tag = to_tag.unwrap();
        
        // Try to find a matching dialog
        // We need to check both UAC (local=from, remote=to) and UAS (local=to, remote=from) scenarios
        
        // Scenario 1: Local is From, Remote is To
        let id_tuple1 = (call_id.clone(), from_tag.clone(), to_tag.clone());
        if let Some(dialog_id_ref) = self.dialog_lookup.get(&id_tuple1) {
            let dialog_id = dialog_id_ref.clone();
            drop(dialog_id_ref);
            return Some(dialog_id);
        }
        
        // Scenario 2: Local is To, Remote is From
        let id_tuple2 = (call_id, to_tag, from_tag);
        if let Some(dialog_id_ref) = self.dialog_lookup.get(&id_tuple2) {
            let dialog_id = dialog_id_ref.clone();
            drop(dialog_id_ref);
            return Some(dialog_id);
        }
        
        // No matching dialog found
        None
    }
    
    /// Create a new request in a dialog
    pub async fn create_request(
        &self, 
        dialog_id: &DialogId, 
        method: Method
    ) -> Result<Request> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(dialog_id.to_string()))?;
            
        // Create the request
        let request = dialog.create_request(method);
        Ok(request)
    }
    
    /// Associate a dialog with a session
    pub fn associate_with_session(
        &self, 
        dialog_id: &DialogId, 
        session_id: &crate::session::SessionId
    ) -> Result<()> {
        if !self.dialogs.contains_key(dialog_id) {
            return Err(Error::DialogNotFoundWithId(dialog_id.to_string()));
        }
        
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        Ok(())
    }
    
    /// Get a dialog by ID
    pub fn get_dialog(&self, dialog_id: &DialogId) -> Result<Dialog> {
        self.dialogs.get(dialog_id)
            .map(|d| d.clone())
            .ok_or_else(|| Error::DialogNotFoundWithId(dialog_id.to_string()))
    }
    
    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> Result<()> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(dialog_id.to_string()))?;
            
        dialog.terminate();
        Ok(())
    }
    
    /// Remove terminated dialogs
    pub fn cleanup_terminated(&self) -> usize {
        let mut count = 0;
        
        let terminated_dialogs: Vec<_> = self.dialogs.iter()
            .filter(|d| d.is_terminated())
            .map(|d| d.id.clone())
            .collect();
        
        for dialog_id in terminated_dialogs {
            if let Some((_, dialog)) = self.dialogs.remove(&dialog_id) {
                count += 1;
                
                // Remove from the lookup tables
                // Get the dialog tuple directly from the dialog
                let call_id = &dialog.call_id;
                if let (Some(local_tag), Some(remote_tag)) = (&dialog.local_tag, &dialog.remote_tag) {
                    let tuple = (call_id.clone(), local_tag.clone(), remote_tag.clone());
                    self.dialog_lookup.remove(&tuple);
                }
                
                self.dialog_to_session.remove(&dialog_id);
                
                // Remove transaction associations
                let txs_to_remove: Vec<_> = self.transaction_to_dialog.iter()
                    .filter(|e| e.value().clone() == dialog_id)
                    .map(|e| e.key().clone())
                    .collect();
                
                for tx_id in txs_to_remove {
                    self.transaction_to_dialog.remove(&tx_id);
                }
            }
        }
        
        count
    }
    
    /// Create a dialog directly from transaction events
    pub async fn create_dialog_from_transaction(
        &self, 
        transaction_id: &TransactionKey, 
        request: &Request, 
        response: &Response,
        is_initiator: bool
    ) -> Option<DialogId> {
        debug!("Creating dialog from transaction: {}", transaction_id);
        
        // Create dialog based on response type
        let dialog = if response.status().is_success() {
            debug!("Creating confirmed dialog from 2xx response");
            Dialog::from_2xx_response(request, response, is_initiator)
        } else if (100..200).contains(&response.status().as_u16()) && response.status().as_u16() > 100 {
            debug!("Creating early dialog from 1xx response");
            Dialog::from_provisional_response(request, response, is_initiator)
        } else {
            debug!("Response status {} not appropriate for dialog creation", response.status());
            None
        };
        
        if let Some(dialog) = dialog {
            let dialog_id = dialog.id.clone();
            debug!("Created dialog with ID: {}", dialog_id);
            
            // Store the dialog
            self.dialogs.insert(dialog_id.clone(), dialog.clone());
            
            // Associate the transaction with this dialog
            self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
            
            // Save dialog tuple mapping if available
            if let Some(tuple) = dialog.dialog_id_tuple() {
                debug!("Mapping dialog tuple to dialog ID: {:?} -> {}", tuple, dialog_id.to_string());
                self.dialog_lookup.insert(tuple, dialog_id.clone());
            }
            
            // Return the created dialog ID
            Some(dialog_id)
        } else {
            debug!("Failed to create dialog from transaction event");
            None
        }
    }

    // Helper method to find a session associated with a transaction
    fn find_session_for_transaction(&self, transaction_id: &TransactionKey) -> Option<crate::session::SessionId> {
        // First, look up the dialog ID
        let dialog_id = match self.transaction_to_dialog.get(transaction_id) {
            Some(ref_val) => {
                // Clone the value to avoid issues with Display formatting
                let dialog_id = ref_val.clone();
                // Explicitly drop the reference
                drop(ref_val);
                dialog_id
            },
            None => return None
        };
        
        // Now look up the session ID for this dialog
        match self.dialog_to_session.get(&dialog_id) {
            Some(ref_val) => {
                // Clone the value to avoid issues with Display formatting
                let session_id = ref_val.clone();
                // Explicitly drop the reference
                drop(ref_val);
                Some(session_id)
            },
            None => None
        }
    }

    /// Get the current transaction state for a dialog
    pub async fn get_transaction_state(&self, dialog_id: &DialogId) -> Result<TransactionState> {
        // Find the transaction ID associated with this dialog
        let transaction_id = self.find_transaction_for_dialog(dialog_id)?;
        
        // Get the transaction state from the transaction manager
        self.transaction_manager.transaction_state(&transaction_id).await
            .map_err(|e| Error::Other(format!("Failed to get transaction state: {}", e)))
    }

    /// Get the transaction kind for a dialog
    pub async fn get_transaction_kind(&self, dialog_id: &DialogId) -> Result<TransactionKind> {
        // Find the transaction ID associated with this dialog
        let transaction_id = self.find_transaction_for_dialog(dialog_id)?;
        
        // Get the transaction kind from the transaction manager
        self.transaction_manager.transaction_kind(&transaction_id).await
            .map_err(|e| Error::Other(format!("Failed to get transaction kind: {}", e)))
    }

    /// Helper method to find the transaction ID for a dialog
    fn find_transaction_for_dialog(&self, dialog_id: &DialogId) -> Result<TransactionKey> {
        for entry in self.transaction_to_dialog.iter() {
            if entry.value() == dialog_id {
                return Ok(entry.key().clone());
            }
        }
        Err(Error::DialogNotFoundWithId(dialog_id.to_string()))
    }

    /// Synchronize dialog state with transaction state
    pub async fn sync_dialog_with_transaction(&self, dialog_id: &DialogId) -> Result<()> {
        let transaction_state = self.get_transaction_state(dialog_id).await?;
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(dialog_id.to_string()))?;
        
        // Update dialog state based on transaction state
        match transaction_state {
            TransactionState::Confirmed => {
                if dialog.state == DialogState::Early {
                    dialog.state = DialogState::Confirmed;
                    
                    // Emit event for state change
                    if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
                        self.event_bus.publish(SessionEvent::DialogUpdated {
                            session_id: session_id.clone(),
                            dialog_id: dialog_id.clone(),
                        });
                    }
                }
            },
            TransactionState::Terminated => {
                if dialog.state != DialogState::Terminated {
                    dialog.terminate();
                    
                    // Emit event for state change
                    if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
                        self.event_bus.publish(SessionEvent::Terminated {
                            session_id: session_id.clone(),
                            reason: "Transaction terminated".to_string(),
                        });
                    }
                }
            },
            TransactionState::Completed => {
                // Some INVITE transactions may remain in Completed for a while
                // We don't need to change dialog state here unless it's terminating
            },
            _ => {
                // For other states, no action needed
            }
        }
        
        Ok(())
    }

    /// Send a request through this dialog and create a client transaction
    pub async fn send_dialog_request(
        &self,
        dialog_id: &DialogId,
        method: Method,
    ) -> Result<TransactionKey> {
        // Get the dialog
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(dialog_id.to_string()))?;
        
        // Create the request within the dialog
        let request = dialog.create_request(method);
        
        // Get the destination for this dialog (stored in remote_target)
        let destination = match utils::resolve_uri_to_socketaddr(&dialog.remote_target).await {
            Some(addr) => addr,
            None => return Err(Error::Other(format!("Failed to resolve remote target: {}", dialog.remote_target))),
        };
        
        // Create a client transaction for this request
        let transaction_id = self.transaction_manager.create_client_transaction(request, destination)
            .await
            .map_err(|e| Error::Other(format!("Failed to create transaction: {}", e)))?;
        
        // Associate this transaction with the dialog
        self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
        
        // Send the request
        self.transaction_manager.send_request(&transaction_id)
            .await
            .map_err(|e| Error::Other(format!("Failed to send request: {}", e)))?;
        
        Ok(transaction_id)
    }
}

/// Helper for resolving a URI to a socket address
mod utils {
    use super::*;
    use std::net::{IpAddr, SocketAddr};
    
    pub async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
        // Get the host from the URI
        let host = uri.host.clone();
        
        // Get the port, defaulting to 5060 for SIP
        let port = uri.port.unwrap_or(5060);
        
        // Resolve the host to an IP address (simplified version)
        // In a real implementation, this would use DNS resolution
        let ip = match host {
            // Match based on the correct Host enum variants
            Host::Address(ip_addr) => ip_addr,
            Host::Domain(_) => {
                // For domain names, we'd need proper DNS resolution
                // For now, just return None
                return None;
            }
        };
        
        Some(SocketAddr::new(ip, port))
    }
}

// Implement Clone for DialogManager (needed for async functions)
impl Clone for DialogManager {
    fn clone(&self) -> Self {
        Self {
            dialogs: self.dialogs.clone(),
            dialog_lookup: self.dialog_lookup.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            transaction_manager: self.transaction_manager.clone(),
            transaction_to_dialog: self.transaction_to_dialog.clone(),
            event_bus: self.event_bus.clone(),
        }
    }
} 