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

use super::dialog_state::DialogState;
use super::dialog_id::DialogId;
use super::dialog_utils::{extract_tag, extract_uri};

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
        let (to_tag, to_value) = match response.header(&HeaderName::To) {
            Some(TypedHeader::To(to)) => {
                let to_tag = to.tag();
                let to_str = to.to_string();
                if to_tag.is_none() {
                    debug!("Dialog creation failed: Missing tag in To header");
                    println!("Dialog creation failed: Missing tag in To header");
                }
                (to_tag, to_str)
            },
            _ => {
                debug!("Dialog creation failed: Missing or invalid To header");
                println!("Dialog creation failed: Missing or invalid To header");
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
                    println!("Dialog creation failed: Missing tag in From header");
                }
                (from_tag, from_str)
            },
            _ => {
                debug!("Dialog creation failed: Missing or invalid From header");
                println!("Dialog creation failed: Missing or invalid From header");
                return None;
            }
        };
        
        debug!("Tags - From: {:?}, To: {:?}", from_tag, to_tag);
        println!("Tags - From: {:?}, To: {:?}", from_tag, to_tag);
        
        // From RFC 3261: The to-tag and from-tag are required for dialog
        if to_tag.is_none() {
            debug!("Dialog creation failed: Missing required To tag");
            println!("Dialog creation failed: Missing required To tag");
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
            println!("Dialog creation failed: Could not extract local URI from {}", 
                if is_initiator { "From" } else { "To" });
            return None;
        }
        let local_uri = local_uri_result.unwrap();
        
        let remote_uri_result = extract_uri(if is_initiator { &to_value } else { &from_value });
        if remote_uri_result.is_none() {
            debug!("Dialog creation failed: Could not extract remote URI from {}", 
                if is_initiator { "To" } else { "From" });
            println!("Dialog creation failed: Could not extract remote URI from {}", 
                if is_initiator { "To" } else { "From" });
            return None;
        }
        let remote_uri = remote_uri_result.unwrap();
        
        // Extract Contact using TypedHeader pattern
        println!("Extracting Contact header...");
        if let Some(contact_header) = response.header(&HeaderName::Contact) {
            println!("Found contact header: {:?}", contact_header);
            match contact_header {
                TypedHeader::Contact(contact) => {
                    println!("Contact value: {:?}", contact);
                    // Use the address() method to get the first address
                    if let Some(address) = contact.address() {
                        let uri = address.uri.clone();
                        debug!("Using contact URI: {}", uri);
                        println!("Using contact URI: {}", uri);
                        
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
                        println!("Dialog created successfully");
                        
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
                // Use the address() method to get the first address
                if let Some(address) = contact.address() {
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
        let call_id = TypedHeader::CallId(rvoip_sip_core::types::call_id::CallId(self.call_id.clone()));
        
        // Add From header with local tag using proper API
        let mut local_addr = Address::new(self.local_uri.clone());
        if let Some(tag) = self.local_tag.as_ref() {
            local_addr.set_tag(tag);
        }
        let from = TypedHeader::From(FromHeader(local_addr));
        
        // Add To header with remote tag using proper API
        let mut remote_addr = Address::new(self.remote_uri.clone());
        if let Some(tag) = self.remote_tag.as_ref() {
            remote_addr.set_tag(tag);
        }
        let to = TypedHeader::To(ToHeader(remote_addr));
        
        // Add CSeq
        let cseq_value = rvoip_sip_core::types::cseq::CSeq::new(self.local_seq, request.method.clone());
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
} 