//! WebRTC Integration Example
//!
//! This example demonstrates how to integrate SIP signaling with WebRTC media,
//! including SDP handling, ICE candidates, and building a SIP-to-WebRTC bridge.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Main function - demonstrates WebRTC integration with SIP
#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    info!("SIP Core WebRTC Integration Example");
    
    // Example 1: WebRTC SDP handling
    webrtc_sdp_handling().await;
    
    // Example 2: SIP signaling for WebRTC
    sip_signaling_for_webrtc().await;
    
    // Example 3: WebRTC gateway implementation
    webrtc_gateway_demo().await;
    
    info!("WebRTC integration example completed!");
}

/// Example 1: WebRTC SDP handling
async fn webrtc_sdp_handling() {
    info!("Example 1: WebRTC SDP Handling");
    
    // Create a WebRTC SDP offer
    let webrtc_offer = create_webrtc_offer();
    info!("Created WebRTC SDP offer:\n{}", webrtc_offer);
    
    // Extract and process ICE candidates from the SDP
    let ice_candidates = extract_ice_candidates(&webrtc_offer);
    info!("Extracted {} ICE candidates from the offer", ice_candidates.len());
    
    // Create a WebRTC SDP answer
    let webrtc_answer = create_webrtc_answer(&webrtc_offer);
    info!("Created WebRTC SDP answer:\n{}", webrtc_answer);
    
    // Compare standard SIP SDP vs WebRTC SDP
    compare_sdp_formats();
}

/// Example 2: SIP signaling for WebRTC
async fn sip_signaling_for_webrtc() {
    info!("Example 2: SIP Signaling for WebRTC");
    
    // Create a WebRTC gateway
    let mut gateway = WebRtcGateway::new();
    
    // Simulate a WebRTC client connecting to the gateway
    let webrtc_client = WebRtcClient::new("web-client", "browser.example.com");
    
    // Simulate a SIP client that will be called from the WebRTC client
    let sip_client = SipClient::new("sip-user", "sip.example.com");
    
    // Register the clients with the gateway
    gateway.register_webrtc_client(webrtc_client.clone());
    gateway.register_sip_client(sip_client.clone());
    
    // Simulate a call from WebRTC client to SIP client
    info!("Initiating call from WebRTC client to SIP client");
    let call_result = gateway.initiate_webrtc_to_sip_call(&webrtc_client.id, &sip_client.id).await;
    
    if call_result {
        info!("Call successfully established between WebRTC and SIP clients!");
        
        // Simulate call duration
        sleep(Duration::from_secs(2)).await;
        
        // End the call
        info!("Ending the call");
        gateway.end_call(&webrtc_client.id, &sip_client.id).await;
    } else {
        info!("Failed to establish call!");
    }
}

/// Example 3: WebRTC gateway implementation
async fn webrtc_gateway_demo() {
    info!("Example 3: WebRTC Gateway Implementation");
    
    // Create a simple communication system
    let mut comm_system = CommunicationSystem::new();
    
    // Start the system
    comm_system.start().await;
    
    // Register some users
    let alice_web = "alice@browser.example.com";
    let bob_sip = "bob@sip.example.com";
    let charlie_web = "charlie@browser.example.com";
    
    comm_system.register_user(alice_web, ClientType::WebRtc).await;
    comm_system.register_user(bob_sip, ClientType::Sip).await;
    comm_system.register_user(charlie_web, ClientType::WebRtc).await;
    
    // Make some calls to demonstrate the gateway
    info!("Making a call from WebRTC client to SIP client");
    comm_system.make_call(alice_web, bob_sip).await;
    
    sleep(Duration::from_secs(3)).await;
    
    info!("Making a call between two WebRTC clients (through SIP infrastructure)");
    comm_system.make_call(alice_web, charlie_web).await;
    
    sleep(Duration::from_secs(3)).await;
    
    // Stop the system
    comm_system.stop().await;
}

/// Create a WebRTC SDP offer
fn create_webrtc_offer() -> String {
    // In a real app, this would use the WebRTC API or a library like libwebrtc
    // Here we'll create a sample SDP offer with WebRTC-specific attributes
    format!(
        "v=0\r\n\
         o=- {} 2 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         t=0 0\r\n\
         a=group:BUNDLE audio video\r\n\
         a=msid-semantic: WMS stream_id\r\n\
         a=ice-ufrag:F7gI\r\n\
         a=ice-pwd:x9cml/NzpTmwkjkdPLl1YQdB\r\n\
         a=ice-options:trickle\r\n\
         a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:6D:B1:99:5C:3C:40:2A:4F:1D:EA:2F:4A:A7:8E:7E:7C:16:51:18:63:2B:FB:AA:10:0F:2C\r\n\
         a=setup:actpass\r\n\
         m=audio 9 UDP/TLS/RTP/SAVPF 111 103 104 9 0 8 106 105 13 110 112 113 126\r\n\
         c=IN IP4 0.0.0.0\r\n\
         a=rtcp:9 IN IP4 0.0.0.0\r\n\
         a=candidate:1 1 UDP 2130706431 192.168.1.100 49203 typ host\r\n\
         a=candidate:2 1 UDP 1694498815 203.0.113.100 49203 typ srflx raddr 192.168.1.100 rport 49203\r\n\
         a=ice-ufrag:F7gI\r\n\
         a=ice-pwd:x9cml/NzpTmwkjkdPLl1YQdB\r\n\
         a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:6D:B1:99:5C:3C:40:2A:4F:1D:EA:2F:4A:A7:8E:7E:7C:16:51:18:63:2B:FB:AA:10:0F:2C\r\n\
         a=setup:actpass\r\n\
         a=mid:audio\r\n\
         a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\n\
         a=sendrecv\r\n\
         a=rtcp-mux\r\n\
         a=rtpmap:111 opus/48000/2\r\n\
         a=rtcp-fb:111 transport-cc\r\n\
         a=fmtp:111 minptime=10;useinbandfec=1\r\n\
         a=rtpmap:103 ISAC/16000\r\n\
         a=rtpmap:104 ISAC/32000\r\n\
         a=rtpmap:9 G722/8000\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=ssrc:1001 cname:webrtc-audio-cname\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 96 97 98 99 100 101 102\r\n\
         c=IN IP4 0.0.0.0\r\n\
         a=rtcp:9 IN IP4 0.0.0.0\r\n\
         a=candidate:1 1 UDP 2130706431 192.168.1.100 49205 typ host\r\n\
         a=candidate:2 1 UDP 1694498815 203.0.113.100 49205 typ srflx raddr 192.168.1.100 rport 49205\r\n\
         a=ice-ufrag:F7gI\r\n\
         a=ice-pwd:x9cml/NzpTmwkjkdPLl1YQdB\r\n\
         a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:6D:B1:99:5C:3C:40:2A:4F:1D:EA:2F:4A:A7:8E:7E:7C:16:51:18:63:2B:FB:AA:10:0F:2C\r\n\
         a=setup:actpass\r\n\
         a=mid:video\r\n\
         a=extmap:2 urn:ietf:params:rtp-hdrext:toffset\r\n\
         a=extmap:3 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r\n\
         a=extmap:4 urn:3gpp:video-orientation\r\n\
         a=sendrecv\r\n\
         a=rtcp-mux\r\n\
         a=rtcp-rsize\r\n\
         a=rtpmap:96 VP8/90000\r\n\
         a=rtcp-fb:96 goog-remb\r\n\
         a=rtcp-fb:96 transport-cc\r\n\
         a=rtcp-fb:96 ccm fir\r\n\
         a=rtcp-fb:96 nack\r\n\
         a=rtcp-fb:96 nack pli\r\n\
         a=rtpmap:97 rtx/90000\r\n\
         a=fmtp:97 apt=96\r\n\
         a=rtpmap:98 H264/90000\r\n\
         a=rtcp-fb:98 goog-remb\r\n\
         a=rtcp-fb:98 transport-cc\r\n\
         a=rtcp-fb:98 ccm fir\r\n\
         a=rtcp-fb:98 nack\r\n\
         a=rtcp-fb:98 nack pli\r\n\
         a=fmtp:98 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
         a=ssrc:2002 cname:webrtc-video-cname\r\n",
        Uuid::new_v4().as_u128()
    )
}

/// Extract ICE candidates from WebRTC SDP
fn extract_ice_candidates(sdp: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    
    for line in sdp.lines() {
        if line.starts_with("a=candidate:") {
            candidates.push(line.to_string());
        }
    }
    
    candidates
}

/// Create a WebRTC SDP answer
fn create_webrtc_answer(offer: &str) -> String {
    // This is a simplified implementation
    // In a real system, this would analyze the offer and create a compatible answer
    
    // Replace some attributes to simulate an answer
    let answer = offer
        .replace("a=setup:actpass", "a=setup:passive")
        .replace("o=- ", "o=answer ")
        .replace("a=ice-ufrag:F7gI", "a=ice-ufrag:H8jQ")
        .replace("a=ice-pwd:x9cml/NzpTmwkjkdPLl1YQdB", "a=ice-pwd:y0rcl/MWbUWxihePOm2IXSvF");
    
    // Add some additional ICE candidates for the answer
    let answer = format!(
        "{}\r\n\
         a=candidate:1 1 UDP 2130706431 192.168.1.200 50987 typ host\r\n\
         a=candidate:2 1 UDP 1694498815 203.0.113.200 50987 typ srflx raddr 192.168.1.200 rport 50987\r\n",
        answer
    );
    
    answer
}

/// Compare standard SIP SDP with WebRTC SDP
fn compare_sdp_formats() {
    info!("Comparing standard SIP SDP with WebRTC SDP");
    
    // Create a standard SIP SDP
    let standard_sdp = concat!(
        "v=0\r\n",
        "o=alice 2890844526 2890844526 IN IP4 192.168.1.100\r\n",
        "s=SIP Call\r\n",
        "c=IN IP4 192.168.1.100\r\n",
        "t=0 0\r\n",
        "m=audio 49172 RTP/AVP 0 8\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n"
    );
    
    // Create a WebRTC SDP (simplified)
    let webrtc_sdp = concat!(
        "v=0\r\n",
        "o=- 1234567890 2 IN IP4 127.0.0.1\r\n",
        "s=-\r\n",
        "t=0 0\r\n",
        "a=group:BUNDLE audio\r\n",
        "a=ice-ufrag:F7gI\r\n",
        "a=ice-pwd:x9cml/NzpTmwkjkdPLl1YQdB\r\n",
        "a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:6D:B1:99:5C:3C:40:2A:4F:1D:EA\r\n",
        "a=setup:actpass\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 0 8\r\n",
        "c=IN IP4 0.0.0.0\r\n",
        "a=rtcp:9 IN IP4 0.0.0.0\r\n",
        "a=candidate:1 1 UDP 2130706431 192.168.1.100 49203 typ host\r\n",
        "a=rtcp-mux\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n"
    );
    
    info!("Standard SIP SDP:\n{}", standard_sdp);
    info!("WebRTC SDP:\n{}", webrtc_sdp);
    
    info!("Key differences in WebRTC SDP:");
    info!("1. ICE candidate information (a=candidate, a=ice-ufrag, a=ice-pwd)");
    info!("2. DTLS-SRTP setup (a=fingerprint, a=setup)");
    info!("3. Secure media transport (UDP/TLS/RTP/SAVPF instead of RTP/AVP)");
    info!("4. BUNDLE and RTCP multiplexing support (a=group:BUNDLE, a=rtcp-mux)");
    info!("5. ICE-lite support indicators may be present");
    info!("6. Additional WebRTC-specific extensions and attributes");
}

/// Represents a WebRTC client
#[derive(Clone)]
struct WebRtcClient {
    id: String,
    domain: String,
    connected: bool,
}

impl WebRtcClient {
    fn new(id: &str, domain: &str) -> Self {
        Self {
            id: id.to_string(),
            domain: domain.to_string(),
            connected: true,
        }
    }
    
    fn create_offer(&self) -> String {
        create_webrtc_offer()
    }
    
    fn process_answer(&self, answer: &str) -> bool {
        // In a real implementation, this would process the SDP answer
        // and establish the WebRTC connection
        info!("WebRTC client processing SDP answer");
        true
    }
    
    fn get_address(&self) -> String {
        format!("{}@{}", self.id, self.domain)
    }
}

/// Represents a SIP client
#[derive(Clone)]
struct SipClient {
    id: String,
    domain: String,
    registered: bool,
}

impl SipClient {
    fn new(id: &str, domain: &str) -> Self {
        Self {
            id: id.to_string(),
            domain: domain.to_string(),
            registered: true,
        }
    }
    
    fn create_invite(&self, to: &str, webrtc_sdp: &str) -> Request {
        // Create a SIP INVITE with the WebRTC SDP as the body
        let call_id = format!("{}@{}", Uuid::new_v4().to_string().split('-').next().unwrap(), self.domain);
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        let from_tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
        
        sip! {
            method: Method::Invite,
            uri: format!("sip:{}", to),
            headers: {
                Via: format!("SIP/2.0/WSS {};branch={}", self.domain, branch),
                MaxForwards: 70,
                To: format!("<sip:{}>", to),
                From: format!("<sip:{}@{}>;tag={}", self.id, self.domain, from_tag),
                CallId: call_id,
                CSeq: "1 INVITE",
                Contact: format!("<sip:{}@{};transport=ws>", self.id, self.domain),
                ContentType: "application/sdp",
                ContentLength: webrtc_sdp.len()
            },
            body: webrtc_sdp.as_bytes().to_vec()
        }
    }
    
    fn process_200_ok(&self, response: &Response) -> Option<String> {
        // Extract the SDP answer from a 200 OK response
        if response.status_code() == StatusCode::OK {
            if let Some(body) = response.body() {
                return Some(String::from_utf8_lossy(body).to_string());
            }
        }
        None
    }
    
    fn get_address(&self) -> String {
        format!("{}@{}", self.id, self.domain)
    }
}

/// WebRTC to SIP Gateway
struct WebRtcGateway {
    webrtc_clients: HashMap<String, WebRtcClient>,
    sip_clients: HashMap<String, SipClient>,
    active_calls: HashMap<String, ActiveCall>,
}

impl WebRtcGateway {
    fn new() -> Self {
        Self {
            webrtc_clients: HashMap::new(),
            sip_clients: HashMap::new(),
            active_calls: HashMap::new(),
        }
    }
    
    fn register_webrtc_client(&mut self, client: WebRtcClient) {
        info!("Registering WebRTC client: {}", client.get_address());
        self.webrtc_clients.insert(client.id.clone(), client);
    }
    
    fn register_sip_client(&mut self, client: SipClient) {
        info!("Registering SIP client: {}", client.get_address());
        self.sip_clients.insert(client.id.clone(), client);
    }
    
    async fn initiate_webrtc_to_sip_call(&mut self, webrtc_id: &str, sip_id: &str) -> bool {
        // Get the clients
        let webrtc_client = if let Some(client) = self.webrtc_clients.get(webrtc_id) {
            client.clone()
        } else {
            warn!("WebRTC client not found: {}", webrtc_id);
            return false;
        };
        
        let sip_client = if let Some(client) = self.sip_clients.get(sip_id) {
            client.clone()
        } else {
            warn!("SIP client not found: {}", sip_id);
            return false;
        };
        
        info!("Initiating call from {} to {}", webrtc_client.get_address(), sip_client.get_address());
        
        // Create WebRTC offer
        let webrtc_offer = webrtc_client.create_offer();
        info!("WebRTC offer created");
        
        // Create SIP INVITE with the WebRTC SDP
        let invite = sip_client.create_invite(&sip_client.get_address(), &webrtc_offer);
        info!("SIP INVITE created:\n{}", std::str::from_utf8(&invite.to_bytes()).unwrap());
        
        // Simulate sending INVITE and receiving 200 OK
        info!("Sending INVITE to SIP client");
        
        // Simulate some network delay
        sleep(Duration::from_millis(500)).await;
        
        // Create a 200 OK response with SDP answer
        let webrtc_answer = create_webrtc_answer(&webrtc_offer);
        let ok_response = create_200_ok_response(&invite, &webrtc_answer);
        info!("Received 200 OK response:\n{}", std::str::from_utf8(&ok_response.to_bytes()).unwrap());
        
        // Process the 200 OK response to extract the SDP answer
        if let Some(sdp_answer) = sip_client.process_200_ok(&ok_response) {
            info!("Extracted SDP answer from 200 OK");
            
            // Pass the SDP answer to the WebRTC client
            if webrtc_client.process_answer(&sdp_answer) {
                info!("WebRTC client processed SDP answer successfully");
                
                // Create a call record
                let call_id = format!("call-{}", Uuid::new_v4().to_string().split('-').next().unwrap());
                let active_call = ActiveCall {
                    call_id: call_id.clone(),
                    webrtc_client_id: webrtc_id.to_string(),
                    sip_client_id: sip_id.to_string(),
                    sip_call_id: invite.typed_header::<CallId>().unwrap().value().to_string(),
                };
                
                // Store the call
                self.active_calls.insert(call_id, active_call);
                
                // Call established
                return true;
            } else {
                warn!("WebRTC client failed to process SDP answer");
            }
        } else {
            warn!("Failed to extract SDP answer from 200 OK");
        }
        
        false
    }
    
    async fn end_call(&mut self, webrtc_id: &str, sip_id: &str) -> bool {
        // Find the call
        let call_opt = self.active_calls.iter()
            .find(|(_, call)| call.webrtc_client_id == webrtc_id && call.sip_client_id == sip_id)
            .map(|(id, _)| id.clone());
        
        if let Some(call_id) = call_opt {
            info!("Ending call: {}", call_id);
            
            // Remove the call
            if let Some(call) = self.active_calls.remove(&call_id) {
                // In a real implementation, we would send a BYE request
                info!("Call terminated: WebRTC {} -> SIP {}", call.webrtc_client_id, call.sip_client_id);
                return true;
            }
        } else {
            warn!("No active call found between {} and {}", webrtc_id, sip_id);
        }
        
        false
    }
}

/// Represents an active call between WebRTC and SIP clients
struct ActiveCall {
    call_id: String,
    webrtc_client_id: String,
    sip_client_id: String,
    sip_call_id: String,
}

/// Creates a 200 OK response for an INVITE request
fn create_200_ok_response(invite: &Request, sdp_answer: &str) -> Response {
    let to = invite.typed_header::<To>().unwrap();
    let from = invite.typed_header::<From>().unwrap();
    let call_id = invite.typed_header::<CallId>().unwrap();
    let cseq = invite.typed_header::<CSeq>().unwrap();
    
    // Add a to tag for the response
    let to_tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
    let to_with_tag = to.clone().with_tag(&to_tag);
    
    ResponseBuilder::new(StatusCode::OK)
        .unwrap()
        .header(TypedHeader::Via(invite.typed_header::<Via>().unwrap().clone()))
        .header(TypedHeader::From(from.clone()))
        .header(TypedHeader::To(to_with_tag))
        .header(TypedHeader::CallId(call_id.clone()))
        .header(TypedHeader::CSeq(cseq.clone()))
        .header(TypedHeader::Contact(Contact::new(
            Address::new("sip:bob@10.0.0.1:5060".parse::<Uri>().unwrap())
        )))
        .header(TypedHeader::ContentType(ContentType::new("application/sdp")))
        .header(TypedHeader::ContentLength(ContentLength::new(sdp_answer.len() as u32)))
        .body(Bytes::from(sdp_answer.to_string()))
        .build()
}

/// Client types in the communication system
#[derive(Clone, Debug)]
enum ClientType {
    WebRtc,
    Sip,
}

/// Events in the communication system
enum CommEvent {
    Stop,
    RegisterUser {
        address: String,
        client_type: ClientType,
        response_tx: Sender<bool>,
    },
    MakeCall {
        from: String,
        to: String,
        response_tx: Sender<bool>,
    },
    EndCall {
        call_id: String,
        response_tx: Sender<bool>,
    },
}

/// A complete communication system with integrated WebRTC and SIP
struct CommunicationSystem {
    event_tx: Option<Sender<CommEvent>>,
    gateway: Arc<Mutex<WebRtcGateway>>,
    registered_users: HashMap<String, ClientType>,
    active_calls: HashMap<String, (String, String)>, // call_id -> (from, to)
}

impl CommunicationSystem {
    fn new() -> Self {
        Self {
            event_tx: None,
            gateway: Arc::new(Mutex::new(WebRtcGateway::new())),
            registered_users: HashMap::new(),
            active_calls: HashMap::new(),
        }
    }
    
    async fn start(&mut self) {
        info!("Starting communication system");
        
        // Create channels for events
        let (tx, rx) = mpsc::channel::<CommEvent>(100);
        self.event_tx = Some(tx);
        
        // Clone shared state for the event loop
        let gateway = Arc::clone(&self.gateway);
        
        // Start the event loop
        tokio::spawn(async move {
            Self::event_loop(rx, gateway).await;
        });
        
        info!("Communication system started");
    }
    
    async fn stop(&self) {
        info!("Stopping communication system");
        
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(CommEvent::Stop).await;
        }
        
        // Give it a moment to shut down
        sleep(Duration::from_millis(100)).await;
        
        info!("Communication system stopped");
    }
    
    async fn register_user(&mut self, address: &str, client_type: ClientType) -> bool {
        info!("Registering user: {} as {:?}", address, client_type);
        
        if let Some(tx) = &self.event_tx {
            let (response_tx, response_rx) = mpsc::channel::<bool>(1);
            
            // Send the registration event
            let _ = tx.send(CommEvent::RegisterUser {
                address: address.to_string(),
                client_type: client_type.clone(),
                response_tx,
            }).await;
            
            // Wait for the response
            if let Some(result) = response_rx.recv().await {
                if result {
                    // Store locally
                    self.registered_users.insert(address.to_string(), client_type);
                    info!("User {} registered successfully", address);
                    return true;
                }
            }
        }
        
        warn!("Failed to register user: {}", address);
        false
    }
    
    async fn make_call(&mut self, from: &str, to: &str) -> bool {
        info!("Making call from {} to {}", from, to);
        
        // Check that both users are registered
        if !self.registered_users.contains_key(from) || !self.registered_users.contains_key(to) {
            warn!("Call failed: one or both users are not registered");
            return false;
        }
        
        if let Some(tx) = &self.event_tx {
            let (response_tx, response_rx) = mpsc::channel::<bool>(1);
            
            // Send the make call event
            let _ = tx.send(CommEvent::MakeCall {
                from: from.to_string(),
                to: to.to_string(),
                response_tx,
            }).await;
            
            // Wait for the response
            if let Some(result) = response_rx.recv().await {
                if result {
                    info!("Call from {} to {} established", from, to);
                    return true;
                }
            }
        }
        
        warn!("Failed to establish call from {} to {}", from, to);
        false
    }
    
    async fn event_loop(mut rx: Receiver<CommEvent>, gateway: Arc<Mutex<WebRtcGateway>>) {
        info!("Communication system event loop started");
        
        while let Some(event) = rx.recv().await {
            match event {
                CommEvent::Stop => {
                    info!("Received stop event");
                    break;
                }
                CommEvent::RegisterUser { address, client_type, response_tx } => {
                    // Extract user and domain
                    let parts: Vec<&str> = address.split('@').collect();
                    if parts.len() != 2 {
                        let _ = response_tx.send(false).await;
                        continue;
                    }
                    
                    let user_id = parts[0];
                    let domain = parts[1];
                    
                    // Register with the gateway
                    let mut success = false;
                    let mut gateway = gateway.lock().unwrap();
                    
                    match client_type {
                        ClientType::WebRtc => {
                            let client = WebRtcClient::new(user_id, domain);
                            gateway.register_webrtc_client(client);
                            success = true;
                        }
                        ClientType::Sip => {
                            let client = SipClient::new(user_id, domain);
                            gateway.register_sip_client(client);
                            success = true;
                        }
                    }
                    
                    // Send response
                    let _ = response_tx.send(success).await;
                }
                CommEvent::MakeCall { from, to, response_tx } => {
                    // Parse addresses
                    let from_parts: Vec<&str> = from.split('@').collect();
                    let to_parts: Vec<&str> = to.split('@').collect();
                    
                    if from_parts.len() != 2 || to_parts.len() != 2 {
                        let _ = response_tx.send(false).await;
                        continue;
                    }
                    
                    let from_user = from_parts[0];
                    let to_user = to_parts[0];
                    
                    // Make the call
                    let success = {
                        let mut gateway = gateway.lock().unwrap();
                        
                        // For this example, we'll only support WebRTC to SIP calls
                        // In a real system, we'd handle all combinations
                        gateway.initiate_webrtc_to_sip_call(from_user, to_user).await
                    };
                    
                    // Send response
                    let _ = response_tx.send(success).await;
                }
                CommEvent::EndCall { call_id, response_tx } => {
                    // End the call
                    let _ = response_tx.send(false).await;
                }
            }
        }
        
        info!("Communication system event loop terminated");
    }
} 