use std::str::FromStr;
use serde::{Serialize, Deserialize};
use tracing::{debug, error};

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, 
    Uri, Header, HeaderName, TypedHeader
};

use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::from::From as FromHeader;
use rvoip_sip_core::types::to::To as ToHeader;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::parser::headers::route::RouteEntry;
use rvoip_sip_core::types::route::Route;
use rvoip_sip_core::types::content_type::ContentType;

use super::dialog_state::DialogState;
use super::dialog_id::DialogId;
use super::dialog_utils::{extract_tag, extract_uri_from_contact};
use crate::sdp::{SdpContext, SessionDescription};

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
    
    /// SDP context for this dialog
    #[serde(skip)]  // Skip serialization as it might be too large
    pub sdp_context: SdpContext,
    
    /// Last known good remote socket address
    pub last_known_remote_addr: Option<std::net::SocketAddr>,
    
    /// Time of the last successful transaction
    pub last_successful_transaction_time: Option<std::time::SystemTime>,
    
    /// Number of recovery attempts made
    pub recovery_attempts: u32,
    
    /// Reason for recovery (if in recovering state)
    pub recovery_reason: Option<String>,
    
    /// Time when the dialog was last successfully recovered
    #[serde(skip)] // Skip in serialization
    pub recovered_at: Option<std::time::SystemTime>,
    
    /// Time when recovery was started
    #[serde(skip)] // Skip in serialization
    pub recovery_start_time: Option<std::time::SystemTime>,
}

impl Dialog {
    /// Create a dialog from a 2xx response to an INVITE
    pub fn from_2xx_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        if !matches!(response.status, StatusCode::Ok | StatusCode::Accepted) {
            debug!("Dialog creation failed: Response status is not 200 OK or 202 Accepted ({})", response.status);
            println!("Dialog creation failed: Response status is not 200 OK or 202 Accepted ({})", response.status);
            return None;
        }
        
        if request.method != Method::Invite {
            debug!("Dialog creation failed: Request method is not INVITE ({})", request.method);
            println!("Dialog creation failed: Request method is not INVITE ({})", request.method);
            return None;
        }
        
        // Extract Call-ID using TypedHeader pattern
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => {
                debug!("Dialog creation failed: Missing or invalid Call-ID header");
                println!("Dialog creation failed: Missing or invalid Call-ID header");
                return None;
            }
        };
        
        // Extract CSeq using TypedHeader pattern
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => {
                debug!("Dialog creation failed: Missing or invalid CSeq header in request");
                println!("Dialog creation failed: Missing or invalid CSeq header in request");
                return None;
            }
        };
        
        // Extract To using TypedHeader pattern
        let to_header = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => to,
            _ => {
                debug!("Dialog creation failed: Missing or invalid To header");
                println!("Dialog creation failed: Missing or invalid To header");
                return None;
            }
        };
        
        // Extract From using TypedHeader pattern
        let from_header = match response.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => from,
            _ => {
                debug!("Dialog creation failed: Missing or invalid From header");
                println!("Dialog creation failed: Missing or invalid From header");
                return None;
            }
        };
        
        // Extract tags from headers
        let to_tag = to_header.tag();
        let from_tag = from_header.tag();
        
        // Set local and remote tags and URIs based on initiator status
        let (local_tag, remote_tag, local_uri, remote_uri) = if is_initiator {
            // Local UA initiated, so local is From, remote is To
            (from_tag.map(|s| s.to_string()), 
             to_tag.map(|s| s.to_string()),
             from_header.uri().clone(), 
             to_header.uri().clone())
        } else {
            // Remote UA initiated, so local is To, remote is From
            (to_tag.map(|s| s.to_string()), 
             from_tag.map(|s| s.to_string()),
             to_header.uri().clone(), 
             from_header.uri().clone())
        };
        
        // Contact is required in responses that establish dialogs
        if let Some(contact_header) = response.header(&HeaderName::Contact) {
            match contact_header {
                TypedHeader::Contact(contacts) => {
                    if let Some(contact) = contacts.0.first() {
                        // Extract the URI from contact
                        let uri = extract_uri_from_contact(contact).expect("Failed to extract URI from contact");
                        
                        // Extract Route set from Record-Route headers
                        let route_set = if is_initiator {
                            // For initiator, the Record-Route is used in reverse order
                            response.headers.iter()
                                .filter_map(|h| {
                                    if h.name() == HeaderName::RecordRoute {
                                        match h {
                                            TypedHeader::RecordRoute(routes) => {
                                                // Access the routes directly as a field
                                                Some(routes.0.iter()
                                                    .map(|route| route.uri().clone())
                                                    .collect::<Vec<Uri>>())
                                            },
                                            _ => None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .flatten()
                                .rev() // Reverse for initiator
                                .collect()
                        } else {
                            // For recipient, the Record-Route is used in the same order
                            response.headers.iter()
                                .filter_map(|h| {
                                    if h.name() == HeaderName::RecordRoute {
                                        match h {
                                            TypedHeader::RecordRoute(routes) => {
                                                // Access the routes directly as a field
                                                Some(routes.0.iter()
                                                    .map(|route| route.uri().clone())
                                                    .collect::<Vec<Uri>>())
                                            },
                                            _ => None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .flatten()
                                .collect()
                        };
                        
                        // Create SDP context
                        let mut sdp_context = SdpContext::new();
                        
                        // Extract SDP from request and response
                        let request_sdp = extract_sdp_from_request(request);
                        let response_sdp = extract_sdp_from_response(response);
                        
                        // Initialize SDP context based on initiator role
                        if is_initiator {
                            // We sent the offer, received the answer
                            if let Some(local_sdp) = request_sdp {
                                if let Some(remote_sdp) = response_sdp {
                                    sdp_context.local_sdp = Some(local_sdp);
                                    sdp_context.remote_sdp = Some(remote_sdp);
                                    sdp_context.state = crate::sdp::NegotiationState::Complete;
                                }
                            }
                        } else {
                            // We received the offer, sent the answer
                            if let Some(remote_sdp) = request_sdp {
                                if let Some(local_sdp) = response_sdp {
                                    sdp_context.local_sdp = Some(local_sdp);
                                    sdp_context.remote_sdp = Some(remote_sdp);
                                    sdp_context.state = crate::sdp::NegotiationState::Complete;
                                }
                            }
                        }
                        
                        return Some(Self {
                            id: DialogId::new(),
                            state: DialogState::Confirmed,
                            call_id,
                            local_uri,
                            remote_uri,
                            local_tag,
                            remote_tag,
                            local_seq: if is_initiator { cseq_number } else { 0 },
                            remote_seq: if is_initiator { 0 } else { cseq_number },
                            remote_target: uri,
                            route_set,
                            is_initiator,
                            sdp_context,
                            last_known_remote_addr: None,
                            last_successful_transaction_time: None,
                            recovery_attempts: 0,
                            recovery_reason: None,
                            recovered_at: None,
                            recovery_start_time: None,
                        });
                    } else {
                        debug!("Dialog creation failed: Empty Contact header");
                        println!("Dialog creation failed: Empty Contact header");
                    }
                },
                _ => {
                    debug!("Dialog creation failed: Invalid Contact header type");
                    println!("Dialog creation failed: Invalid Contact header type");
                }
            }
        } else {
            debug!("Dialog creation failed: Missing Contact header");
            println!("Dialog creation failed: Missing Contact header");
        }
        
        None
    }
    
    /// Create a dialog from an early (1xx) response to an INVITE
    pub fn from_provisional_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        // Only certain provisional responses can create dialogs
        if !matches!(response.status,
            StatusCode::Ringing | 
            StatusCode::SessionProgress | 
            StatusCode::CallIsBeingForwarded | 
            StatusCode::Queued) {
            return None;
        }
        
        if request.method != Method::Invite {
            return None;
        }
        
        // To tag is required for early dialog
        let to_header = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => to,
            _ => return None
        };
        
        if to_header.tag().is_none() {
            return None;  // No tag in To header, can't create early dialog
        }
        
        // Extract Call-ID
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => return None
        };
        
        // Extract CSeq
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => return None
        };
        
        // Extract From
        let from_header = match response.header(&HeaderName::From) {
            Some(TypedHeader::From(from)) => from,
            _ => return None
        };
        
        // Set local and remote tags and URIs based on initiator status
        let (local_tag, remote_tag, local_uri, remote_uri) = if is_initiator {
            // Local UA initiated, so local is From, remote is To
            (from_header.tag().map(|s| s.to_string()), 
             to_header.tag().map(|s| s.to_string()),
             from_header.uri().clone(), 
             to_header.uri().clone())
        } else {
            // Remote UA initiated, so local is To, remote is From
            (to_header.tag().map(|s| s.to_string()), 
             from_header.tag().map(|s| s.to_string()),
             to_header.uri().clone(), 
             from_header.uri().clone())
        };
        
        // Contact is required
        if let Some(contact_header) = response.header(&HeaderName::Contact) {
            match contact_header {
                TypedHeader::Contact(contacts) => {
                    if let Some(contact) = contacts.0.first() {
                        // Extract the URI from contact
                        let uri = extract_uri_from_contact(contact).expect("Failed to extract URI from contact");
                        
                        // Extract Route set (similar to confirmed dialog)
                        let route_set = if is_initiator {
                            response.headers.iter()
                                .filter_map(|h| if h.name() == HeaderName::RecordRoute {
                                    match h {
                                        TypedHeader::RecordRoute(routes) => {
                                            // Access the routes directly as a field
                                            Some(routes.0.iter()
                                                .map(|route| route.uri().clone())
                                                .collect::<Vec<Uri>>())
                                        },
                                        _ => None
                                    }
                                } else { None })
                                .flatten()
                                .rev()
                                .collect()
                        } else {
                            response.headers.iter()
                                .filter_map(|h| if h.name() == HeaderName::RecordRoute {
                                    match h {
                                        TypedHeader::RecordRoute(routes) => {
                                            // Access the routes directly as a field
                                            Some(routes.0.iter()
                                                .map(|route| route.uri().clone())
                                                .collect::<Vec<Uri>>())
                                        },
                                        _ => None
                                    }
                                } else { None })
                                .flatten()
                                .collect()
                        };
                        
                        // Create SDP context
                        let mut sdp_context = SdpContext::new();
                        
                        // Extract SDP from request and response
                        let request_sdp = extract_sdp_from_request(request);
                        let response_sdp = extract_sdp_from_response(response);
                        
                        // Initialize SDP context based on initiator role
                        if is_initiator {
                            // We sent the offer
                            if let Some(local_sdp) = request_sdp {
                                sdp_context.local_sdp = Some(local_sdp);
                                sdp_context.state = crate::sdp::NegotiationState::OfferSent;
                                
                                // If provisional has SDP, it's an early media answer
                                if let Some(remote_sdp) = response_sdp {
                                    sdp_context.remote_sdp = Some(remote_sdp);
                                    sdp_context.state = crate::sdp::NegotiationState::Complete;
                                }
                            }
                        } else {
                            // We received the offer
                            if let Some(remote_sdp) = request_sdp {
                                sdp_context.remote_sdp = Some(remote_sdp);
                                sdp_context.state = crate::sdp::NegotiationState::OfferReceived;
                                
                                // If we sent SDP in provisional, it's an early media answer
                                if let Some(local_sdp) = response_sdp {
                                    sdp_context.local_sdp = Some(local_sdp);
                                    sdp_context.state = crate::sdp::NegotiationState::Complete;
                                }
                            }
                        }
                        
                        return Some(Self {
                            id: DialogId::new(),
                            state: DialogState::Early,
                            call_id,
                            local_uri,
                            remote_uri,
                            local_tag,
                            remote_tag,
                            local_seq: if is_initiator { cseq_number } else { 0 },
                            remote_seq: if is_initiator { 0 } else { cseq_number },
                            remote_target: uri,
                            route_set,
                            is_initiator,
                            sdp_context,
                            last_known_remote_addr: None,
                            last_successful_transaction_time: None,
                            recovery_attempts: 0,
                            recovery_reason: None,
                            recovered_at: None,
                            recovery_start_time: None,
                        });
                    }
                },
                _ => {}
            }
        }
        
        None
    }
    
    /// Terminate a dialog
    pub fn terminate(&mut self) {
        self.state = DialogState::Terminated;
    }
    
    /// Check if dialog is terminated
    pub fn is_terminated(&self) -> bool {
        self.state == DialogState::Terminated
    }
    
    /// Create a new request within this dialog
    pub fn create_request(&mut self, method: Method) -> Request {
        // Increment local sequence number for new request
        if method != Method::Ack {
            self.local_seq += 1;
        }
        
        // Create base request
        let mut request = Request::new(method.clone(), self.remote_target.clone());
        
        // Add headers
        request.headers.push(TypedHeader::CallId(
            rvoip_sip_core::types::call_id::CallId(self.call_id.clone())
        ));
        
        // Add From header with our tag
        if let Some(local_tag) = &self.local_tag {
            let mut from_addr = Address::new(self.local_uri.clone());
            from_addr.set_tag(local_tag);
            let from = FromHeader(from_addr);
            request.headers.push(TypedHeader::From(from));
        } else {
            // This shouldn't happen in an established dialog
            let from_addr = Address::new(self.local_uri.clone());
            request.headers.push(TypedHeader::From(
                FromHeader(from_addr)
            ));
        }
        
        // Add To header with remote tag
        if let Some(remote_tag) = &self.remote_tag {
            let mut to_addr = Address::new(self.remote_uri.clone());
            to_addr.set_tag(remote_tag);
            let to = ToHeader(to_addr);
            request.headers.push(TypedHeader::To(to));
        } else {
            // This shouldn't happen in an established dialog
            let to_addr = Address::new(self.remote_uri.clone());
            request.headers.push(TypedHeader::To(
                ToHeader(to_addr)
            ));
        }
        
        // Add CSeq header
        request.headers.push(TypedHeader::CSeq(
            rvoip_sip_core::types::cseq::CSeq::new(self.local_seq, method.clone())
        ));
        
        // Add Route headers if we have a route set
        if !self.route_set.is_empty() {
            for route_uri in &self.route_set {
                // Create a new Route header with this URI
                // First create an Address from the URI
                let route_addr = Address::new(route_uri.clone());
                // Then create a RouteEntry using the tuple struct pattern
                let route_entry = RouteEntry(route_addr);
                // Finally create a Route with the entry
                let route = Route::new(vec![route_entry]);
                request.headers.push(TypedHeader::Route(route));
            }
        }
        
        request
    }
    
    /// Update dialog from a successful response
    pub fn update_from_response(&mut self, response: &Response) -> bool {
        if self.state == DialogState::Early && 
           (response.status == StatusCode::Ok || response.status == StatusCode::Accepted) {
            // Transition from early to confirmed
            self.state = DialogState::Confirmed;
            
            // Update remote target if Contact is present
            if let Some(TypedHeader::Contact(contacts)) = response.header(&HeaderName::Contact) {
                if let Some(contact) = contacts.0.first() {
                    if let Some(uri) = extract_uri_from_contact(contact) {
                        self.remote_target = uri;
                    }
                }
            }
            
            // Update remote tag if it changed
            if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
                if let Some(tag) = to.tag() {
                    self.remote_tag = Some(tag.to_string());
                }
            }
            
            // Update SDP if present
            if let Some(remote_sdp) = extract_sdp_from_response(response) {
                match self.sdp_context.state {
                    crate::sdp::NegotiationState::OfferSent => {
                        self.sdp_context.update_with_remote_answer(remote_sdp);
                    },
                    _ => {
                        // This is an unexpected case - we got a final response with SDP but we hadn't sent an offer
                        debug!("Received final response with SDP but we were not in OfferSent state");
                    }
                }
            }
            
            return true;
        }
        
        false
    }
    
    /// Update dialog with a received request
    pub fn update_from_request(&mut self, request: &Request) -> bool {
        // Update remote sequence number if higher than current
        if let Some(TypedHeader::CSeq(cseq)) = request.header(&HeaderName::CSeq) {
            let seq = cseq.sequence();
            if seq > self.remote_seq {
                self.remote_seq = seq;
            }
        }
        
        // Update remote target if request contains Contact
        if let Some(TypedHeader::Contact(contacts)) = request.header(&HeaderName::Contact) {
            if let Some(contact) = contacts.0.first() {
                self.remote_target = extract_uri_from_contact(contact).expect("Failed to extract URI from contact");
            }
        }
        
        // Handle SDP in the request
        if request.method == Method::Invite {
            if let Some(remote_sdp) = extract_sdp_from_request(request) {
                self.sdp_context.update_with_remote_offer(remote_sdp);
            }
        }
        
        true
    }
    
    /// Update dialog SDP state with a local SDP offer
    pub fn update_with_local_sdp_offer(&mut self, offer: SessionDescription) {
        self.sdp_context.update_with_local_offer(offer);
    }
    
    /// Update dialog SDP state with a local SDP answer
    pub fn update_with_local_sdp_answer(&mut self, answer: SessionDescription) {
        self.sdp_context.update_with_local_answer(answer);
    }
    
    /// Update dialog SDP state for re-negotiation
    pub fn prepare_sdp_renegotiation(&mut self) {
        self.sdp_context.reset_for_renegotiation();
    }
    
    /// Get the current local SDP
    pub fn local_sdp(&self) -> Option<&SessionDescription> {
        self.sdp_context.local_sdp.as_ref()
    }
    
    /// Get the current remote SDP
    pub fn remote_sdp(&self) -> Option<&SessionDescription> {
        self.sdp_context.remote_sdp.as_ref()
    }
    
    /// Generate a dialog ID tuple (call-id, local-tag, remote-tag)
    pub fn dialog_id_tuple(&self) -> Option<(String, String, String)> {
        if let (Some(local_tag), Some(remote_tag)) = (&self.local_tag, &self.remote_tag) {
            Some((self.call_id.clone(), local_tag.clone(), remote_tag.clone()))
        } else {
            None
        }
    }
    
    /// Update dialog from a 2xx response
    pub fn update_from_2xx(&mut self, response: &Response) -> bool {
        if self.state == DialogState::Early && 
           (response.status == StatusCode::Ok || response.status == StatusCode::Accepted) {
            // Transition from early to confirmed
            self.state = DialogState::Confirmed;
            
            // Update remote target if Contact is present
            if let Some(TypedHeader::Contact(contacts)) = response.header(&HeaderName::Contact) {
                if let Some(contact) = contacts.0.first() {
                    if let Some(uri) = extract_uri_from_contact(contact) {
                        self.remote_target = uri;
                    }
                }
            }
            
            // Update remote tag if it changed
            if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
                if let Some(tag) = to.tag() {
                    self.remote_tag = Some(tag.to_string());
                }
            }
            
            // Update SDP if present
            if let Some(remote_sdp) = extract_sdp_from_response(response) {
                match self.sdp_context.state {
                    crate::sdp::NegotiationState::OfferSent => {
                        self.sdp_context.update_with_remote_answer(remote_sdp);
                    },
                    _ => {
                        // This is an unexpected case - we got a final response with SDP but we hadn't sent an offer
                        debug!("Received final response with SDP but we were not in OfferSent state");
                    }
                }
            }
            
            return true;
        }
        
        false
    }
    
    /// Mark this dialog as recovering from a network failure
    pub fn enter_recovery_mode(&mut self, reason: &str) {
        super::recovery::begin_recovery(self, reason);
    }
    
    /// Increment recovery attempts
    pub fn increment_recovery_attempts(&mut self) -> u32 {
        self.recovery_attempts += 1;
        self.recovery_attempts
    }
    
    /// Complete recovery and return to normal state
    pub fn complete_recovery(&mut self) -> bool {
        super::recovery::complete_recovery(self)
    }
    
    /// Abandon recovery and terminate the dialog
    pub fn abandon_recovery(&mut self) {
        super::recovery::abandon_recovery(self);
    }
    
    /// Update the last known remote address
    pub fn update_remote_address(&mut self, remote_addr: std::net::SocketAddr) {
        self.last_known_remote_addr = Some(remote_addr);
        self.last_successful_transaction_time = Some(std::time::SystemTime::now());
    }
    
    /// Check if this dialog is in recovery mode
    pub fn is_recovering(&self) -> bool {
        self.state == DialogState::Recovering
    }
    
    /// Get time since last successful transaction
    pub fn time_since_last_transaction(&self) -> Option<std::time::Duration> {
        self.last_successful_transaction_time.map(|time| {
            std::time::SystemTime::now()
                .duration_since(time)
                .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        })
    }
}

/// Extract SDP from a SIP request if present
fn extract_sdp_from_request(request: &Request) -> Option<SessionDescription> {
    // Check for SDP content
    if request.body.is_empty() {
        return None;
    }
    
    // Check Content-Type
    if let Some(TypedHeader::ContentType(content_type)) = request.header(&HeaderName::ContentType) {
        if content_type.to_string() == "application/sdp" {
            // Parse SDP
            if let Ok(sdp_str) = std::str::from_utf8(&request.body) {
                if let Ok(sdp) = SessionDescription::from_str(sdp_str) {
                    return Some(sdp);
                }
            }
        }
    }
    
    None
}

/// Extract SDP from a SIP response if present
fn extract_sdp_from_response(response: &Response) -> Option<SessionDescription> {
    // Check for SDP content
    if response.body.is_empty() {
        return None;
    }
    
    // Check Content-Type
    if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
        if content_type.to_string() == "application/sdp" {
            // Parse SDP
            if let Ok(sdp_str) = std::str::from_utf8(&response.body) {
                if let Ok(sdp) = SessionDescription::from_str(sdp_str) {
                    return Some(sdp);
                }
            }
        }
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::address::Address;
    use rvoip_sip_core::types::param::Param;
    use rvoip_sip_core::types::call_id::CallId;
    use rvoip_sip_core::types::from::From as FromHeader;
    use rvoip_sip_core::types::to::To as ToHeader;
    use rvoip_sip_core::types::cseq::CSeq;
    use rvoip_sip_core::types::contact::ContactParamInfo;
    use rvoip_sip_core::types::contact::Contact;
    
    fn create_mock_invite_request() -> Request {
        let mut request = Request::new(Method::Invite, Uri::sip("bob@example.com"));
        
        // Add Call-ID
        let call_id = CallId("test-call-id".to_string());
        request.headers.push(TypedHeader::CallId(call_id));
        
        // Add From with tag using proper API
        let from_uri = Uri::sip("alice@example.com");
        let from_addr = Address::new(from_uri).with_tag("alice-tag");
        let from = FromHeader(from_addr);
        request.headers.push(TypedHeader::From(from));
        
        // Add To
        let to_uri = Uri::sip("bob@example.com");
        let to = ToHeader::new(Address::new(to_uri));
        request.headers.push(TypedHeader::To(to));
        
        // Add CSeq
        let cseq = CSeq::new(1, Method::Invite);
        request.headers.push(TypedHeader::CSeq(cseq));
        
        request
    }
    
    fn create_mock_response(status: StatusCode, with_to_tag: bool) -> Response {
        let mut response = Response::new(status);
        
        // Add Call-ID
        let call_id = CallId("test-call-id".to_string());
        response.headers.push(TypedHeader::CallId(call_id));
        
        // Add From with tag using proper API
        let from_uri = Uri::sip("alice@example.com");
        let from_addr = Address::new(from_uri).with_tag("alice-tag");
        let from = FromHeader(from_addr);
        response.headers.push(TypedHeader::From(from));
        
        // Add To, optionally with tag using proper API
        let to_uri = Uri::sip("bob@example.com");
        let to_addr = if with_to_tag {
            Address::new(to_uri).with_tag("bob-tag")
        } else {
            Address::new(to_uri)
        };
        let to = ToHeader(to_addr);
        response.headers.push(TypedHeader::To(to));
        
        // Add Contact
        let contact_uri = Uri::sip("bob@192.168.1.2");
        let contact_addr = Address::new(contact_uri);
        
        // Create contact header using the correct API
        let contact_param = ContactParamInfo { address: contact_addr };
        let contact = Contact::new_params(vec![contact_param]);
        response.headers.push(TypedHeader::Contact(contact));
        
        response
    }
    
    #[test]
    fn test_dialog_creation_from_2xx() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog as UAC (initiator)
        let dialog = Dialog::from_2xx_response(&request, &response, true);
        assert!(dialog.is_some(), "Dialog creation failed");
        
        let dialog = dialog.unwrap();
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert_eq!(dialog.call_id, "test-call-id");
        assert_eq!(dialog.local_tag, Some("alice-tag".to_string()));
        assert_eq!(dialog.remote_tag, Some("bob-tag".to_string()));
        assert_eq!(dialog.local_seq, 1);
        assert_eq!(dialog.remote_seq, 0);
        assert_eq!(dialog.is_initiator, true);
    }
    
    #[test]
    fn test_dialog_creation_from_provisional() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 180 Ringing response with to-tag
        let response = create_mock_response(StatusCode::Ringing, true);
        
        // Create dialog as UAC (initiator)
        let dialog = Dialog::from_provisional_response(&request, &response, true);
        assert!(dialog.is_some(), "Dialog creation failed");
        
        let dialog = dialog.unwrap();
        assert_eq!(dialog.state, DialogState::Early);
        assert_eq!(dialog.call_id, "test-call-id");
        assert_eq!(dialog.local_tag, Some("alice-tag".to_string()));
        assert_eq!(dialog.remote_tag, Some("bob-tag".to_string()));
        assert_eq!(dialog.local_seq, 1);
        assert_eq!(dialog.remote_seq, 0);
        assert_eq!(dialog.is_initiator, true);
    }
    
    #[test]
    fn test_dialog_update_from_2xx() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 180 Ringing response with to-tag
        let provisional = create_mock_response(StatusCode::Ringing, true);
        
        // Create early dialog
        let mut dialog = Dialog::from_provisional_response(&request, &provisional, true).unwrap();
        assert_eq!(dialog.state, DialogState::Early);
        
        // Create a mock 200 OK response with to-tag
        let final_response = create_mock_response(StatusCode::Ok, true);
        
        // Update the dialog
        let updated = dialog.update_from_2xx(&final_response);
        assert!(updated, "Dialog update failed");
        assert_eq!(dialog.state, DialogState::Confirmed);
    }
    
    #[test]
    fn test_dialog_terminate() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog
        let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
        assert_eq!(dialog.state, DialogState::Confirmed);
        
        // Terminate the dialog
        dialog.terminate();
        assert_eq!(dialog.state, DialogState::Terminated);
        assert!(dialog.is_terminated());
    }
    
    #[test]
    fn test_dialog_create_request() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog
        let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
        
        // Create a BYE request
        let bye_request = dialog.create_request(Method::Bye);
        
        // Verify the request
        assert_eq!(bye_request.method, Method::Bye);
        assert_eq!(dialog.local_seq, 2); // Should be incremented
        
        // Check headers
        assert!(bye_request.header(&HeaderName::CallId).is_some());
        assert!(bye_request.header(&HeaderName::From).is_some());
        assert!(bye_request.header(&HeaderName::To).is_some());
        assert!(bye_request.header(&HeaderName::CSeq).is_some());
        
        // Verify From contains local tag
        if let Some(TypedHeader::From(from)) = bye_request.header(&HeaderName::From) {
            assert_eq!(from.tag(), dialog.local_tag.as_deref());
        }
        
        // Verify To contains remote tag
        if let Some(TypedHeader::To(to)) = bye_request.header(&HeaderName::To) {
            assert_eq!(to.tag(), dialog.remote_tag.as_deref());
        }
    }
    
    #[test]
    fn test_dialog_recovery_mode() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog
        let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
        assert_eq!(dialog.state, DialogState::Confirmed);
        
        // Enter recovery mode
        let reason = "Network failure";
        dialog.enter_recovery_mode(reason);
        
        // Verify state and reason
        assert_eq!(dialog.state, DialogState::Recovering);
        assert_eq!(dialog.recovery_reason, Some(reason.to_string()));
        assert_eq!(dialog.recovery_attempts, 0);
        assert!(dialog.is_recovering());
        
        // Increment recovery attempts
        let attempts = dialog.increment_recovery_attempts();
        assert_eq!(attempts, 1);
        assert_eq!(dialog.recovery_attempts, 1);
        
        // Complete recovery
        let recovery_completed = dialog.complete_recovery();
        assert!(recovery_completed);
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert_eq!(dialog.recovery_reason, None);
        assert!(dialog.last_successful_transaction_time.is_some());
        assert!(!dialog.is_recovering());
    }
    
    #[test]
    fn test_dialog_recovery_abandonment() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog
        let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
        
        // Enter recovery mode
        dialog.enter_recovery_mode("Network failure");
        assert_eq!(dialog.state, DialogState::Recovering);
        
        // Abandon recovery
        dialog.abandon_recovery();
        assert_eq!(dialog.state, DialogState::Terminated);
        assert!(dialog.recovery_reason.is_some());
        assert!(dialog.recovery_reason.unwrap().contains("failed"));
    }
    
    #[test]
    fn test_dialog_remote_address_tracking() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog
        let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
        
        // Update remote address
        let remote_addr = "192.168.1.100:5060".parse().unwrap();
        dialog.update_remote_address(remote_addr);
        
        // Verify address and timestamp
        assert_eq!(dialog.last_known_remote_addr, Some(remote_addr));
        assert!(dialog.last_successful_transaction_time.is_some());
        
        // Check time since last transaction
        let duration = dialog.time_since_last_transaction();
        assert!(duration.is_some());
        assert!(duration.unwrap().as_secs() < 1); // Should be very recent
    }
    
    #[test]
    fn test_dialog_recovery_only_for_active_dialogs() {
        // Create a mock INVITE request
        let request = create_mock_invite_request();
        
        // Create a mock 200 OK response with to-tag
        let response = create_mock_response(StatusCode::Ok, true);
        
        // Create dialog and terminate it
        let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
        dialog.terminate();
        assert_eq!(dialog.state, DialogState::Terminated);
        
        // Try to enter recovery mode on terminated dialog
        dialog.enter_recovery_mode("Network failure");
        
        // Verify state didn't change
        assert_eq!(dialog.state, DialogState::Terminated);
        assert_eq!(dialog.recovery_reason, None);
        assert!(!dialog.is_recovering());
    }
} 