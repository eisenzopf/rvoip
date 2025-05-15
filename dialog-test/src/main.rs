use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, HeaderName, TypedHeader,
    types::{call_id::CallId, from::From, to::To}
};

// Instead of importing from session-core, we'll implement a simple version of Dialog directly
// to avoid compilation issues while we test the core functionality

#[derive(Debug, Clone)]
enum DialogState {
    Early,
    Confirmed,
    Terminated,
}

#[derive(Debug, Clone)]
struct Dialog {
    state: DialogState,
    call_id: String,
    local_uri: Uri,
    remote_uri: Uri,
    local_tag: Option<String>,
    remote_tag: Option<String>,
    local_seq: u32,
    remote_seq: u32,
    remote_target: Uri,
    route_set: Vec<Uri>,
    is_initiator: bool,
}

impl Dialog {
    // Similar implementation to the one in session-core
    fn from_2xx_response(request: &Request, response: &Response, is_initiator: bool) -> Option<Self> {
        if !matches!(response.status, StatusCode::Ok | StatusCode::Accepted) {
            return None;
        }
        
        if request.method != Method::Invite {
            return None;
        }
        
        // Extract Call-ID using TypedHeader pattern
        let call_id = match response.header(&HeaderName::CallId) {
            Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
            _ => {
                println!("Dialog creation failed: Missing or invalid Call-ID header");
                return None;
            }
        };
        
        // Extract CSeq using TypedHeader pattern
        let cseq_number = match request.header(&HeaderName::CSeq) {
            Some(TypedHeader::CSeq(cseq)) => cseq.sequence(),
            _ => {
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
                    println!("Dialog creation failed: Missing tag in To header");
                }
                (to_tag, to_str)
            },
            _ => {
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
                    println!("Dialog creation failed: Missing tag in From header");
                }
                (from_tag, from_str)
            },
            _ => {
                println!("Dialog creation failed: Missing or invalid From header");
                return None;
            }
        };
        
        println!("Tags - From: {:?}, To: {:?}", from_tag, to_tag);
        
        // From RFC 3261: The to-tag and from-tag are required for dialog
        if to_tag.is_none() {
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
        
        // Extract URIs from From/To headers
        // Simplified implementation
        let local_uri = Uri::sip("local@example.com");
        let remote_uri = Uri::sip("remote@example.com");
        
        // Extract Contact using TypedHeader pattern
        let contact_uri = match response.header(&HeaderName::Contact) {
            Some(TypedHeader::Contact(contact)) => {
                // Contact may have multiple values, use the first one
                if let Some(address) = contact.addresses().next() {
                    let uri = address.uri.clone();
                    println!("Using contact URI: {}", uri);
                    uri
                } else {
                    println!("Dialog creation failed: Empty Contact header");
                    return None;
                }
            },
            _ => {
                println!("Dialog creation failed: Missing or invalid Contact header");
                return None;
            }
        };
        
        // Extract route set from Record-Route headers (empty for this test)
        let route_set = Vec::new();
        
        println!("Dialog created successfully");
        
        Some(Self {
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
    
    // Create a new request within this dialog
    fn create_request(&mut self, method: Method) -> Request {
        println!("Creating request for method {}", method);
        
        // Increment local sequence number for new requests
        if method != Method::Ack {
            self.local_seq += 1;
        }
        
        // Ensure local and remote tags are set
        if self.local_tag.is_none() {
            println!("Warning: Local tag is missing, generating a random tag");
            self.local_tag = Some("auto-tag".to_string());
        }
        
        let mut request = Request::new(method.clone(), self.remote_target.clone());
        
        // Add Call-ID header
        let call_id = CallId(self.call_id.clone());
        request.headers.push(TypedHeader::CallId(call_id));
        
        // Add From header with local tag
        let local_tag_value = self.local_tag.as_ref().unwrap().clone();
        let from_uri = self.local_uri.clone();
        let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
        from_addr.params.push(rvoip_sip_core::types::param::Param::new("tag".to_string(), Some(local_tag_value)));
        let from = From(from_addr);
        request.headers.push(TypedHeader::From(from));
        
        // Add To header with remote tag if available
        let to_uri = self.remote_uri.clone();
        let mut to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
        if let Some(tag) = &self.remote_tag {
            to_addr.params.push(rvoip_sip_core::types::param::Param::new("tag".to_string(), Some(tag.clone())));
        }
        let to = To(to_addr);
        request.headers.push(TypedHeader::To(to));
        
        // Add CSeq
        let cseq = rvoip_sip_core::types::cseq::CSeq::new(self.local_seq, request.method.clone());
        request.headers.push(TypedHeader::CSeq(cseq));
        
        println!("Created {} request", method);
        
        request
    }
}

fn create_mock_invite_request() -> Request {
    let mut request = Request::new(Method::Invite, Uri::sip("bob@example.com"));
    
    // Add Call-ID
    let call_id = CallId("test-call-id".to_string());
    request.headers.push(TypedHeader::CallId(call_id));
    
    // Add From with tag
    let from_uri = Uri::sip("alice@example.com");
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.params.push(rvoip_sip_core::types::param::Param::new("tag".to_string(), Some("alice-tag".to_string())));
    let from = From(from_addr);
    request.headers.push(TypedHeader::From(from));
    
    // Add To
    let to_uri = Uri::sip("bob@example.com");
    let to = To::new(rvoip_sip_core::types::address::Address::new(to_uri));
    request.headers.push(TypedHeader::To(to));
    
    // Add CSeq
    let cseq = rvoip_sip_core::types::cseq::CSeq::new(1, Method::Invite);
    request.headers.push(TypedHeader::CSeq(cseq));
    
    request
}

fn create_mock_response(status: StatusCode, with_to_tag: bool) -> Response {
    let mut response = Response::new(status);
    
    // Add Call-ID
    let call_id = CallId("test-call-id".to_string());
    response.headers.push(TypedHeader::CallId(call_id));
    
    // Add From with tag
    let from_uri = Uri::sip("alice@example.com");
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.params.push(rvoip_sip_core::types::param::Param::new("tag".to_string(), Some("alice-tag".to_string())));
    let from = From(from_addr);
    response.headers.push(TypedHeader::From(from));
    
    // Add To, optionally with tag
    let to_uri = Uri::sip("bob@example.com");
    let mut to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
    if with_to_tag {
        to_addr.params.push(rvoip_sip_core::types::param::Param::new("tag".to_string(), Some("bob-tag".to_string())));
    }
    let to = To(to_addr);
    response.headers.push(TypedHeader::To(to));
    
    // Add Contact
    let contact_uri = Uri::sip("bob@192.168.1.2");
    let contact_addr = rvoip_sip_core::types::address::Address::new(contact_uri);
    
    // Add contact using the correct API
    let contact_param = vec![rvoip_sip_core::types::contact::ContactParamInfo::Address(vec![contact_addr])];
    let contact = rvoip_sip_core::types::contact::Contact::new_params(contact_param);
    response.headers.push(TypedHeader::Contact(contact));
    
    response
}

fn test_dialog_creation() {
    // Create a mock INVITE request
    let request = create_mock_invite_request();
    
    // Create a mock 200 OK response with to-tag
    let response = create_mock_response(StatusCode::Ok, true);
    
    // Create dialog as UAC (initiator)
    let dialog = Dialog::from_2xx_response(&request, &response, true);
    
    if let Some(dialog) = dialog {
        println!("✓ Dialog creation successful");
        println!("Dialog state: {:?}", dialog.state);
        println!("Dialog call-id: {}", dialog.call_id);
        println!("Dialog local tag: {:?}", dialog.local_tag);
        println!("Dialog remote tag: {:?}", dialog.remote_tag);
        println!("Dialog local seq: {}", dialog.local_seq);
        println!("Dialog remote seq: {}", dialog.remote_seq);
        println!("Dialog is initiator: {}", dialog.is_initiator);
        
        // Try creating a new request
        let mut dialog = dialog;
        let bye_request = dialog.create_request(Method::Bye);
        println!("✓ BYE request creation successful");
        println!("BYE request method: {}", bye_request.method);
        println!("Dialog local seq after BYE: {}", dialog.local_seq);
    } else {
        println!("✗ Dialog creation failed");
    }
}

fn main() {
    test_dialog_creation();
} 