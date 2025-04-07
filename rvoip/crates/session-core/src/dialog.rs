use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
use serde::{Serialize, Deserialize};

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, 
    Uri, Header, HeaderName, Message
};

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

/// The state of a SIP dialog
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogState {
    /// Dialog is being established but is not yet confirmed
    Early,
    
    /// Dialog is established and confirmed
    Confirmed,
    
    /// Dialog is being terminated
    Terminating,
    
    /// Dialog has been terminated
    Terminated,
}

impl fmt::Display for DialogState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogState::Early => write!(f, "Early"),
            DialogState::Confirmed => write!(f, "Confirmed"),
            DialogState::Terminating => write!(f, "Terminating"),
            DialogState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// A SIP dialog as defined in RFC 3261
#[derive(Debug, Clone)]
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
        
        let call_id = match response.header(&HeaderName::CallId) {
            Some(h) => match h.value.as_text() {
                Some(t) => t.to_string(),
                None => {
                    debug!("Dialog creation failed: Call-ID header value is not text");
                    return None;
                }
            },
            None => {
                debug!("Dialog creation failed: Missing Call-ID header");
                return None;
            }
        };
        
        let cseq_header = match request.header(&HeaderName::CSeq) {
            Some(h) => h,
            None => {
                debug!("Dialog creation failed: Missing CSeq header in request");
                return None;
            }
        };
        
        let cseq_text = match cseq_header.value.as_text() {
            Some(t) => t,
            None => {
                debug!("Dialog creation failed: CSeq header value is not text");
                return None;
            }
        };
        
        let cseq_parts: Vec<&str> = cseq_text.splitn(2, ' ').collect();
        if cseq_parts.len() < 2 {
            debug!("Dialog creation failed: Invalid CSeq format: {}", cseq_text);
            return None;
        }
        
        let cseq_number = match cseq_parts[0].parse::<u32>() {
            Ok(n) => n,
            Err(e) => {
                debug!("Dialog creation failed: Could not parse CSeq number: {} ({})", cseq_parts[0], e);
                return None;
            }
        };
        
        let to_header = match response.header(&HeaderName::To) {
            Some(h) => h,
            None => {
                debug!("Dialog creation failed: Missing To header");
                return None;
            }
        };
        
        let to_value = match to_header.value.as_text() {
            Some(t) => t,
            None => {
                debug!("Dialog creation failed: To header value is not text");
                return None;
            }
        };
        
        let to_tag = extract_tag(to_value);
        if to_tag.is_none() {
            debug!("Dialog creation failed: Missing tag in To header: {}", to_value);
        }
        
        let from_header = match response.header(&HeaderName::From) {
            Some(h) => h,
            None => {
                debug!("Dialog creation failed: Missing From header");
                return None;
            }
        };
        
        let from_value = match from_header.value.as_text() {
            Some(t) => t,
            None => {
                debug!("Dialog creation failed: From header value is not text");
                return None;
            }
        };
        
        let from_tag = extract_tag(from_value);
        if from_tag.is_none() {
            debug!("Dialog creation failed: Missing tag in From header: {}", from_value);
        }
        
        debug!("Tags - From: {:?}, To: {:?}", from_tag, to_tag);
        
        // From RFC 3261: The to-tag and from-tag are required for dialog
        if to_tag.is_none() {
            debug!("Dialog creation failed: Missing required To tag");
            return None;
        }
        
        let remote_tag = if is_initiator { to_tag.clone() } else { from_tag.clone() };
        let local_tag = if is_initiator { from_tag.clone() } else { to_tag.clone() };
        
        let local_uri_result = extract_uri(if is_initiator { from_value } else { to_value });
        if local_uri_result.is_none() {
            debug!("Dialog creation failed: Could not extract local URI from {}", 
                if is_initiator { "From" } else { "To" });
            return None;
        }
        let local_uri = local_uri_result.unwrap();
        
        let remote_uri_result = extract_uri(if is_initiator { to_value } else { from_value });
        if remote_uri_result.is_none() {
            debug!("Dialog creation failed: Could not extract remote URI from {}", 
                if is_initiator { "To" } else { "From" });
            return None;
        }
        let remote_uri = remote_uri_result.unwrap();
        
        let contact_header = match response.header(&HeaderName::Contact) {
            Some(h) => h,
            None => {
                debug!("Dialog creation failed: Missing Contact header");
                return None;
            }
        };
        
        let contact = match contact_header.value.as_text() {
            Some(t) => t,
            None => {
                debug!("Dialog creation failed: Contact header value is not text");
                return None;
            }
        };
        
        let contact_uri_result = extract_uri(contact);
        if contact_uri_result.is_none() {
            debug!("Dialog creation failed: Could not extract URI from Contact: {}", contact);
            return None;
        }
        let contact_uri = contact_uri_result.unwrap();
        
        // Extract route set from Record-Route headers
        let mut route_set = Vec::new();
        // Record-Route headers may not exist, so this part is optional
        if let Some(record_route) = response.header(&HeaderName::RecordRoute) {
            if let Some(rr_list) = record_route.value.as_text_list() {
                for rr in rr_list.iter().rev() {
                    if let Some(uri) = extract_uri(rr) {
                        route_set.push(uri);
                    }
                }
            } else if let Some(rr_text) = record_route.value.as_text() {
                if let Some(uri) = extract_uri(rr_text) {
                    route_set.push(uri);
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
        
        // Need a to-tag to create a dialog
        let to_header = response.header(&HeaderName::To)?;
        let to_value = to_header.value.as_text()?;
        let to_tag = extract_tag(to_value);
        if to_tag.is_none() {
            return None;
        }
        
        let call_id = response.header(&HeaderName::CallId)?.value.as_text()?.to_string();
        
        let cseq_header = request.header(&HeaderName::CSeq)?;
        let cseq_parts: Vec<&str> = cseq_header.value.as_text()?.splitn(2, ' ').collect();
        let cseq_number = cseq_parts[0].parse::<u32>().ok()?;
        
        let from_header = response.header(&HeaderName::From)?;
        let from_value = from_header.value.as_text()?;
        let from_tag = extract_tag(from_value);
        
        let remote_tag = if is_initiator { to_tag.clone() } else { from_tag.clone() };
        let local_tag = if is_initiator { from_tag.clone() } else { to_tag.clone() };
        
        let local_uri = extract_uri(if is_initiator { from_value } else { to_value })?;
        let remote_uri = extract_uri(if is_initiator { to_value } else { from_value })?;
        
        // Get contact from response
        let contact = response.header(&HeaderName::Contact)?.value.as_text()?;
        let contact_uri = extract_uri(contact)?;
        
        // Extract route set from Record-Route headers
        let mut route_set = Vec::new();
        if let Some(record_route) = response.header(&HeaderName::RecordRoute) {
            if let Some(rr_list) = record_route.value.as_text_list() {
                for rr in rr_list.iter().rev() {
                    if let Some(uri) = extract_uri(rr) {
                        route_set.push(uri);
                    }
                }
            } else if let Some(rr_text) = record_route.value.as_text() {
                if let Some(uri) = extract_uri(rr_text) {
                    route_set.push(uri);
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
            if let Some(contact_uri) = contact.value.as_text().and_then(extract_uri) {
                self.remote_target = contact_uri;
            }
        }
        
        // Update remote tag if needed
        if let Some(to) = response.header(&HeaderName::To) {
            if let Some(to_value) = to.value.as_text() {
                if self.is_initiator {
                    self.remote_tag = extract_tag(to_value);
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
        
        // Add dialog identifiers
        request.headers.push(Header::text(HeaderName::CallId, &self.call_id));
        
        // Add From header with local tag
        let local_tag_value = self.local_tag.as_ref().unwrap_or(&"".to_string()).clone();
        let from_value = format!("<{}>;tag={}", self.local_uri, local_tag_value);
        request.headers.push(Header::text(HeaderName::From, from_value));
        
        // Add To header with remote tag
        let mut to_value = format!("<{}>", self.remote_uri);
        if let Some(tag) = &self.remote_tag {
            to_value.push_str(&format!(";tag={}", tag));
        } else {
            debug!("Warning: Remote tag is missing in dialog");
        }
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        // Add CSeq
        let cseq_value = format!("{} {}", self.local_seq, request.method);
        request.headers.push(Header::text(HeaderName::CSeq, cseq_value));
        
        // Add route set if present
        if !self.route_set.is_empty() {
            debug!("Adding {} route headers", self.route_set.len());
            for uri in &self.route_set {
                let route_value = format!("<{}>", uri);
                request.headers.push(Header::text(HeaderName::Route, route_value));
            }
        }
        
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