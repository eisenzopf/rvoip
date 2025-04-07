use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
use serde::{Serialize, Deserialize};

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, 
    Uri, Header, HeaderName
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
        if !matches!(response.status, StatusCode::Ok | StatusCode::Accepted) {
            return None;
        }
        
        if request.method != Method::Invite {
            return None;
        }
        
        let call_id = response.header(&HeaderName::CallId)?.value.as_text()?.to_string();
        
        let cseq_header = request.header(&HeaderName::CSeq)?;
        let cseq_parts: Vec<&str> = cseq_header.value.as_text()?.splitn(2, ' ').collect();
        let cseq_number = cseq_parts[0].parse::<u32>().ok()?;
        
        let to_header = response.header(&HeaderName::To)?;
        let to_value = to_header.value.as_text()?;
        let to_tag = extract_tag(to_value);
        
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
        // Increment local sequence number for new requests
        if method != Method::Ack {
            self.local_seq += 1;
        }
        
        let mut request = Request::new(method, self.remote_target.clone());
        
        // Add dialog identifiers
        request.headers.push(Header::text(HeaderName::CallId, &self.call_id));
        
        // Add From header with local tag
        let mut from_value = format!("<{}>", self.local_uri);
        if let Some(tag) = &self.local_tag {
            from_value.push_str(&format!(";tag={}", tag));
        }
        request.headers.push(Header::text(HeaderName::From, from_value));
        
        // Add To header with remote tag
        let mut to_value = format!("<{}>", self.remote_uri);
        if let Some(tag) = &self.remote_tag {
            to_value.push_str(&format!(";tag={}", tag));
        }
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        // Add CSeq
        let cseq_value = format!("{} {}", self.local_seq, request.method);
        request.headers.push(Header::text(HeaderName::CSeq, cseq_value));
        
        // Add route set if present
        if !self.route_set.is_empty() {
            for uri in &self.route_set {
                let route_value = format!("<{}>", uri);
                request.headers.push(Header::text(HeaderName::Route, route_value));
            }
        }
        
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

/// Helper function to extract tag parameter from a header value
pub fn extract_tag(header_value: &str) -> Option<String> {
    // Improved tag extraction with more robust parsing
    if let Some(tag_pos) = header_value.find(";tag=") {
        let tag_start = tag_pos + 5; // length of ";tag="
        
        // Find the end of the tag (either at the next semicolon or the end of the string)
        let tag_end = header_value[tag_start..]
            .find(|c: char| c == ';' || c == '>' || c.is_whitespace())
            .map(|pos| tag_start + pos)
            .unwrap_or(header_value.len());
        
        // Extract and trim any whitespace
        let tag = header_value[tag_start..tag_end].trim();
        
        // Only return if tag is not empty
        if !tag.is_empty() {
            return Some(tag.to_string());
        }
    }
    
    // If this is a Contact or From header that should always have a tag
    // but doesn't, we can generate one
    if header_value.contains("From:") || header_value.contains("<sip:") {
        if !header_value.contains(";tag=") {
            // No tag present, generate one for interoperability
            use uuid::Uuid;
            return Some(format!("autogen-{}", Uuid::new_v4().to_string().split('-').next().unwrap_or("tag")));
        }
    }
    
    None
}

/// Helper function to extract a URI from a header value
pub fn extract_uri(header_value: &str) -> Option<Uri> {
    // Special handling for Contact headers which often look like: <sip:user@domain:port;transport=udp>
    if header_value.contains("Contact:") || header_value.contains("contact:") {
        // Contact headers often have special parameters that need to be preserved
        if let Some(start) = header_value.find('<') {
            let start_idx = start + 1;
            if let Some(end) = header_value[start_idx..].find('>') {
                let uri_str = header_value[start_idx..(start_idx + end)].trim();
                if let Ok(uri) = Uri::from_str(uri_str) {
                    return Some(uri);
                }
            }
        }
    }
    
    // Improved URI extraction with more robust parsing
    
    // If the header has angle brackets <sip:user@domain>, extract the part between them
    if let Some(start) = header_value.find('<') {
        let start_idx = start + 1;
        if let Some(end) = header_value[start_idx..].find('>') {
            let uri_str = header_value[start_idx..(start_idx + end)].trim();
            return Uri::from_str(uri_str).ok();
        }
    }
    
    // If no angle brackets, try parsing the whole string (minus any parameters)
    // This handles formats like "sip:user@domain;tag=1234"
    let uri_str = match header_value.find(';') {
        Some(idx) => header_value[0..idx].trim(),
        None => header_value.trim()
    };
    
    // Try parsing as URI
    if let Ok(uri) = Uri::from_str(uri_str) {
        return Some(uri);
    }
    
    // Check if it's just a domain without the scheme
    if !uri_str.contains(':') && !uri_str.contains('@') {
        // Try adding "sip:" prefix
        if let Ok(uri) = Uri::from_str(&format!("sip:{}", uri_str)) {
            return Some(uri);
        }
    }
    
    // Last resort - try to extract just the host part
    let host_part = uri_str
        .trim_start_matches("sip:")
        .trim_start_matches("sips:")
        .trim_start_matches("tel:")
        .split('@')
        .last()
        .unwrap_or(uri_str);
    
    Uri::from_str(&format!("sip:{}", host_part)).ok()
} 