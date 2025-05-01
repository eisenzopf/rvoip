//! SIP Dialog Example
//!
//! This example demonstrates a complete SIP dialog between two endpoints,
//! including dialog state tracking and transaction handling.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use std::collections::HashMap;
use tracing::{debug, info};
use uuid::Uuid;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Dialog Example");
    
    // Create a simulated SIP dialog between Alice and Bob
    let mut dialog_sim = DialogSimulation::new();
    
    // Run the complete dialog simulation
    dialog_sim.run_simulation();
    
    info!("Dialog simulation completed successfully!");
}

/// Represents a SIP dialog participant (User Agent)
struct UserAgent {
    /// Name of the user agent
    name: String,
    /// SIP URI for this user agent
    uri: String,
    /// SIP address for this user agent
    address: Address,
    /// Contact URI for this user agent (where to send direct messages)
    contact_uri: Uri,
    /// Active dialogs this UA is participating in
    dialogs: HashMap<String, Dialog>,
    /// Local CSeq sequence number
    local_cseq: u32,
}

impl UserAgent {
    /// Create a new User Agent
    fn new(name: &str, uri: &str, ip: &str, port: u16) -> Self {
        let parsed_uri = uri.parse::<Uri>().expect("Invalid URI");
        let address = Address::new_with_display_name(name, parsed_uri.clone());
        let contact_uri = format!("sip:{}@{}:{}", name.to_lowercase(), ip, port)
            .parse::<Uri>()
            .expect("Invalid contact URI");
            
        Self {
            name: name.to_string(),
            uri: uri.to_string(),
            address,
            contact_uri,
            dialogs: HashMap::new(),
            local_cseq: 1,
        }
    }
    
    /// Get the next CSeq value
    fn next_cseq(&mut self) -> u32 {
        let cseq = self.local_cseq;
        self.local_cseq += 1;
        cseq
    }
    
    /// Create a branch parameter for Via header
    fn create_branch(&self) -> String {
        format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap())
    }
    
    /// Create a new Call-ID
    fn create_call_id(&self) -> String {
        format!("{}@{}", Uuid::new_v4(), self.contact_uri.host())
    }
    
    /// Create an INVITE request to start a dialog
    fn create_invite(&mut self, to_uri: &str) -> Request {
        let to_uri = to_uri.parse::<Uri>().expect("Invalid URI");
        let to_address = Address::new(to_uri);
        let call_id = self.create_call_id();
        let branch = self.create_branch();
        let cseq = self.next_cseq();
        
        // Basic SDP body (simplified for example)
        let sdp_body = 
            "v=0\r\n\
             o=- 1234567890 1234567890 IN IP4 192.168.0.1\r\n\
             s=SIP Call\r\n\
             c=IN IP4 192.168.0.1\r\n\
             t=0 0\r\n\
             m=audio 49170 RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n";
        
        // Create the INVITE request
        let invite = sip! {
            method: Method::Invite,
            uri: to_uri.to_string(),
            headers: {
                Via: format!("SIP/2.0/UDP {};branch={}", self.contact_uri.host_port(), branch),
                MaxForwards: 70,
                To: to_address.to_string(),
                From: format!("{};tag={}", self.address.to_string(), Uuid::new_v4().to_string().split('-').next().unwrap()),
                CallId: call_id,
                CSeq: format!("{} INVITE", cseq),
                Contact: format!("<{}>", self.contact_uri),
                ContentType: "application/sdp",
                ContentLength: sdp_body.len()
            },
            body: sdp_body
        };
        
        info!("{} is sending INVITE to {}", self.name, to_uri);
        debug!("INVITE:\n{}", std::str::from_utf8(&invite.to_bytes()).unwrap());
        
        invite
    }
    
    /// Process an incoming INVITE request
    fn process_invite(&mut self, invite: &Request) -> Response {
        info!("{} received INVITE", self.name);
        
        // Extract dialog-forming headers
        let from = invite.typed_header::<From>().expect("Missing From header");
        let to = invite.typed_header::<To>().expect("Missing To header");
        let call_id = invite.typed_header::<CallId>().expect("Missing Call-ID header");
        let cseq = invite.typed_header::<CSeq>().expect("Missing CSeq header");
        let contact = invite.typed_header::<Contact>().expect("Missing Contact header");
        
        // First, create a 180 Ringing response
        let ringing = create_response(invite, StatusCode::Ringing, Some(&self.contact_uri));
        
        info!("{} is sending 180 Ringing", self.name);
        debug!("180 Ringing:\n{}", std::str::from_utf8(&ringing.to_bytes()).unwrap());
        
        // Then, create a 200 OK response
        let tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
        
        // Create basic SDP body for the response (simplified for example)
        let sdp_body = 
            "v=0\r\n\
             o=- 9876543210 9876543210 IN IP4 192.168.0.2\r\n\
             s=SIP Call\r\n\
             c=IN IP4 192.168.0.2\r\n\
             t=0 0\r\n\
             m=audio 49180 RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n";
        
        // Create the 200 OK response
        let ok_response = ResponseBuilder::new(StatusCode::OK)
            .unwrap()
            .headers(invite.headers().clone())
            .header(TypedHeader::To(to.clone().with_tag(&tag)))
            .header(TypedHeader::Contact(Contact::new(Address::new(self.contact_uri.clone()))))
            .header(TypedHeader::ContentType(ContentType::new("application/sdp")))
            .header(TypedHeader::ContentLength(ContentLength::new(sdp_body.len())))
            .body(Bytes::from(sdp_body))
            .build();
        
        info!("{} is sending 200 OK", self.name);
        debug!("200 OK:\n{}", std::str::from_utf8(&ok_response.to_bytes()).unwrap());
        
        // Create dialog state
        let remote_target = contact.address().uri().clone();
        let local_seq = 0; // We haven't sent a request yet
        let remote_seq = cseq.sequence();
        let local_uri = to.address().uri().clone();
        let remote_uri = from.address().uri().clone();
        
        // Create dialog object and store in our dialogs map
        let dialog = Dialog::new(
            call_id.value(),
            &tag,
            from.tag().expect("From tag missing"),
            local_uri,
            remote_uri,
            remote_target,
            local_seq,
            remote_seq,
        );
        
        self.dialogs.insert(call_id.value().to_string(), dialog);
        
        ok_response
    }
    
    /// Create an ACK for a 200 OK response to an INVITE
    fn create_ack(&mut self, original_invite: &Request, ok_response: &Response) -> Request {
        // Extract necessary headers
        let call_id = original_invite.typed_header::<CallId>().expect("Missing Call-ID");
        let from = original_invite.typed_header::<From>().expect("Missing From");
        let to = ok_response.typed_header::<To>().expect("Missing To");
        let contact = ok_response.typed_header::<Contact>().expect("Missing Contact");
        
        // Get dialog based on Call-ID
        let dialog = self.dialogs.get_mut(call_id.value())
            .expect("Dialog not found");
            
        // Update dialog with remote target from Contact in 200 OK
        dialog.remote_target = contact.address().uri().clone();
        
        // Update dialog with remote tag from To in 200 OK
        dialog.remote_tag = to.tag().expect("To tag missing").to_string();
        
        // Create the ACK request
        let ack = sip! {
            method: Method::Ack,
            uri: dialog.remote_target.to_string(),
            headers: {
                Via: format!("SIP/2.0/UDP {};branch={}", self.contact_uri.host_port(), self.create_branch()),
                MaxForwards: 70,
                To: to.to_string(),
                From: from.to_string(),
                CallId: call_id.value(),
                CSeq: format!("{} ACK", original_invite.typed_header::<CSeq>().unwrap().sequence()),
                ContentLength: 0
            }
        };
        
        info!("{} is sending ACK", self.name);
        debug!("ACK:\n{}", std::str::from_utf8(&ack.to_bytes()).unwrap());
        
        ack
    }
    
    /// Process an incoming 200 OK response
    fn process_ok_response(&mut self, original_invite: &Request, ok_response: &Response) -> Request {
        info!("{} received 200 OK", self.name);
        
        // Extract dialog-forming headers
        let from = ok_response.typed_header::<From>().expect("Missing From header");
        let to = ok_response.typed_header::<To>().expect("Missing To header");
        let call_id = ok_response.typed_header::<CallId>().expect("Missing Call-ID header");
        let cseq = ok_response.typed_header::<CSeq>().expect("Missing CSeq header");
        let contact = ok_response.typed_header::<Contact>().expect("Missing Contact header");
        
        // Extract To tag (must be present in 2xx response)
        let to_tag = to.tag().expect("To tag missing in 200 OK");
        
        // Create dialog state
        let remote_target = contact.address().uri().clone();
        let local_seq = cseq.sequence();
        let remote_seq = 0; // We haven't received a request yet
        let local_uri = from.address().uri().clone();
        let remote_uri = to.address().uri().clone();
        
        // Create dialog object and store in our dialogs map
        let dialog = Dialog::new(
            call_id.value(),
            from.tag().expect("From tag missing"),
            to_tag,
            local_uri,
            remote_uri,
            remote_target,
            local_seq,
            remote_seq,
        );
        
        self.dialogs.insert(call_id.value().to_string(), dialog);
        
        // Create ACK
        self.create_ack(original_invite, ok_response)
    }
    
    /// Create a BYE request to end a dialog
    fn create_bye(&mut self, call_id: &str) -> Option<Request> {
        // Get dialog
        let dialog = match self.dialogs.get_mut(call_id) {
            Some(dialog) => dialog,
            None => {
                info!("No dialog found with Call-ID: {}", call_id);
                return None;
            }
        };
        
        // Increment local sequence number for the dialog
        dialog.local_seq += 1;
        
        // Create the BYE request
        let bye = sip! {
            method: Method::Bye,
            uri: dialog.remote_target.to_string(),
            headers: {
                Via: format!("SIP/2.0/UDP {};branch={}", self.contact_uri.host_port(), self.create_branch()),
                MaxForwards: 70,
                To: format!("<{}>;tag={}", dialog.remote_uri, dialog.remote_tag),
                From: format!("<{}>;tag={}", dialog.local_uri, dialog.local_tag),
                CallId: call_id,
                CSeq: format!("{} BYE", dialog.local_seq),
                ContentLength: 0
            }
        };
        
        info!("{} is sending BYE", self.name);
        debug!("BYE:\n{}", std::str::from_utf8(&bye.to_bytes()).unwrap());
        
        Some(bye)
    }
    
    /// Process an incoming BYE request
    fn process_bye(&mut self, bye: &Request) -> Response {
        info!("{} received BYE", self.name);
        
        // Extract Call-ID
        let call_id = bye.typed_header::<CallId>().expect("Missing Call-ID header");
        
        // Remove dialog from our dialogs map
        self.dialogs.remove(call_id.value());
        
        // Create 200 OK response
        let ok_response = create_response(bye, StatusCode::OK, None);
        
        info!("{} is sending 200 OK for BYE", self.name);
        debug!("200 OK:\n{}", std::str::from_utf8(&ok_response.to_bytes()).unwrap());
        
        ok_response
    }
    
    /// Process an incoming 200 OK response to a BYE
    fn process_ok_for_bye(&mut self, bye: &Request, _ok_response: &Response) {
        info!("{} received 200 OK for BYE", self.name);
        
        // Extract Call-ID
        let call_id = bye.typed_header::<CallId>().expect("Missing Call-ID header");
        
        // Remove dialog from our dialogs map
        self.dialogs.remove(call_id.value());
        
        info!("Dialog with Call-ID {} terminated", call_id.value());
    }
}

/// Helper function to create a SIP response from a request
fn create_response(request: &Request, status_code: StatusCode, contact_uri: Option<&Uri>) -> Response {
    let mut builder = ResponseBuilder::new(status_code)
        .unwrap()
        .headers(request.headers().clone());
    
    // Add Contact header if provided
    if let Some(uri) = contact_uri {
        builder = builder.header(TypedHeader::Contact(Contact::new(Address::new(uri.clone()))));
    }
    
    builder.build()
}

/// Represents a SIP dialog between two user agents
struct Dialog {
    call_id: String,
    local_tag: String,
    remote_tag: String,
    local_uri: Uri,
    remote_uri: Uri,
    remote_target: Uri,
    local_seq: u32,
    remote_seq: u32,
}

impl Dialog {
    fn new(
        call_id: &str,
        local_tag: &str,
        remote_tag: &str,
        local_uri: Uri,
        remote_uri: Uri,
        remote_target: Uri,
        local_seq: u32,
        remote_seq: u32,
    ) -> Self {
        Self {
            call_id: call_id.to_string(),
            local_tag: local_tag.to_string(),
            remote_tag: remote_tag.to_string(),
            local_uri,
            remote_uri,
            remote_target,
            local_seq,
            remote_seq,
        }
    }
}

/// Simulates a complete SIP dialog between two User Agents
struct DialogSimulation {
    alice: UserAgent,
    bob: UserAgent,
}

impl DialogSimulation {
    fn new() -> Self {
        let alice = UserAgent::new("Alice", "sip:alice@atlanta.com", "192.168.0.1", 5060);
        let bob = UserAgent::new("Bob", "sip:bob@example.com", "192.168.0.2", 5060);
        
        Self { alice, bob }
    }
    
    fn run_simulation(&mut self) {
        info!("Starting SIP dialog simulation");
        
        // Step 1: Alice sends INVITE to Bob
        let invite = self.alice.create_invite(&self.bob.uri);
        
        // Step 2: Bob processes INVITE and sends 200 OK
        let ok_response = self.bob.process_invite(&invite);
        
        // Step 3: Alice processes 200 OK and sends ACK
        let ack = self.alice.process_ok_response(&invite, &ok_response);
        
        // At this point, the dialog is established
        info!("Dialog established successfully!");
        
        // Let's imagine the call is active for a while...
        info!("Call is in progress...");
        
        // Step 4: Alice sends BYE to terminate the dialog
        let call_id = invite.typed_header::<CallId>().unwrap().value();
        let bye = self.alice.create_bye(call_id).expect("Failed to create BYE");
        
        // Step 5: Bob processes BYE and sends 200 OK
        let ok_for_bye = self.bob.process_bye(&bye);
        
        // Step 6: Alice processes 200 OK for BYE
        self.alice.process_ok_for_bye(&bye, &ok_for_bye);
        
        // At this point, the dialog is terminated
        info!("Dialog terminated successfully!");
        
        // Verify that both Alice and Bob have no active dialogs
        assert!(self.alice.dialogs.is_empty(), "Alice should have no active dialogs");
        assert!(self.bob.dialogs.is_empty(), "Bob should have no active dialogs");
    }
} 