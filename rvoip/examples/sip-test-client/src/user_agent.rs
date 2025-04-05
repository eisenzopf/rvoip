use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, Context};
use tokio::signal::ctrl_c;
use tracing::{info, debug, error, warn};
use uuid::Uuid;
use bytes::Bytes;

// Rust converts "-" to "_" when importing crates
use rvoip_sip_core as _;
use rvoip_sip_transport as _;
use rvoip_transaction_core as _;
use rvoip_session_core as _;
// Add RTP, media core, and session core imports
use rvoip_rtp_core::{RtpSession, RtpSessionConfig, RtpTimestamp, RtpPacket};
use rvoip_rtp_core::session::RtpSessionEvent;
use rvoip_media_core::{AudioBuffer, AudioFormat, SampleRate};
use rvoip_media_core::codec::{G711Codec, G711Variant, Codec};
use rvoip_session_core::sdp::{SessionDescription, extract_rtp_port_from_sdp};

// Now import the specific types
use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Header, HeaderName
};
use rvoip_sip_transport::UdpTransport;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};

// Struct to track active calls
#[derive(Default)]
struct ActiveCalls {
    calls: tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl ActiveCalls {
    async fn add(&self, call_id: String, handle: tokio::task::JoinHandle<()>) {
        let mut calls = self.calls.lock().await;
        calls.insert(call_id, handle);
    }
    
    async fn remove(&self, call_id: &str) {
        let mut calls = self.calls.lock().await;
        if let Some(handle) = calls.remove(call_id) {
            handle.abort();
        }
    }
    
    async fn terminate_all(&self) {
        let mut calls = self.calls.lock().await;
        for (_, handle) in calls.drain() {
            handle.abort();
        }
    }
}

/// Simple user agent that can respond to SIP requests
pub struct UserAgent {
    /// Local address
    local_addr: SocketAddr,
    
    /// Username
    username: String,
    
    /// Domain
    domain: String,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Event receiver
    events_rx: tokio::sync::mpsc::Receiver<TransactionEvent>,
    
    /// Active calls
    active_calls: Arc<ActiveCalls>,
}

impl UserAgent {
    /// Create a new user agent
    pub async fn new(
        local_addr: SocketAddr,
        username: String,
        domain: String,
    ) -> Result<Self> {
        // Create UDP transport
        let (udp_transport, transport_rx) = UdpTransport::bind(local_addr, None).await
            .context("Failed to bind UDP transport")?;
        
        info!("User agent UDP transport bound to {}", local_addr);
        
        // Wrap transport in Arc
        let arc_transport = Arc::new(udp_transport);
        
        // Create transaction manager
        let (transaction_manager, events_rx) = TransactionManager::new(
            arc_transport,
            transport_rx,
            None, // Use default event capacity
        ).await.context("Failed to create transaction manager")?;
        
        info!("User agent {} initialized", username);
        
        Ok(Self {
            local_addr,
            username,
            domain,
            transaction_manager: Arc::new(transaction_manager),
            events_rx,
            active_calls: Arc::new(ActiveCalls::default()),
        })
    }
    
    /// Generate a new branch parameter
    fn new_branch(&self) -> String {
        format!("z9hG4bK-{}", Uuid::new_v4())
    }
    
    /// Process incoming requests and respond appropriately
    pub async fn process_requests(&mut self) -> Result<()> {
        info!("User agent {} started, waiting for requests on {}...", self.username, self.local_addr);
        
        // Process events from our channel
        while let Some(event) = self.events_rx.recv().await {
            match event {
                TransactionEvent::UnmatchedMessage { message, source } => {
                    match message {
                        Message::Request(request) => {
                            info!("Received {} request from {}", request.method, source);
                            debug!("Request details: {:?}", request);
                            
                            // Print out important headers for debugging
                            if let Some(h) = request.header(&HeaderName::From) {
                                info!("From: {}", h.value.as_text().unwrap_or("(invalid)"));
                            }
                            if let Some(h) = request.header(&HeaderName::To) {
                                info!("To: {}", h.value.as_text().unwrap_or("(invalid)"));
                            }
                            if let Some(h) = request.header(&HeaderName::CallId) {
                                info!("Call-ID: {}", h.value.as_text().unwrap_or("(invalid)"));
                            }
                            
                            let response = match request.method {
                                Method::Register => {
                                    info!("Received REGISTER request (ignoring, this is a user agent)");
                                    None
                                },
                                Method::Invite => {
                                    info!("Received INVITE request, sending 200 OK");
                                    self.handle_invite(request, source).await?
                                },
                                Method::Ack => {
                                    info!("Received ACK request");
                                    None // No response for ACK
                                },
                                Method::Bye => {
                                    info!("Received BYE request, sending 200 OK");
                                    self.handle_bye(request, source).await?
                                },
                                Method::Cancel => {
                                    info!("Received CANCEL request, sending 200 OK");
                                    let mut response = Response::new(StatusCode::Ok);
                                    self.add_common_headers(&mut response, &request);
                                    Some(response)
                                },
                                Method::Options => {
                                    info!("Received OPTIONS request, sending 200 OK");
                                    let mut response = Response::new(StatusCode::Ok);
                                    self.add_common_headers(&mut response, &request);
                                    Some(response)
                                },
                                _ => {
                                    warn!("Received unsupported request: {}", request.method);
                                    let mut response = Response::new(StatusCode::NotImplemented);
                                    self.add_common_headers(&mut response, &request);
                                    Some(response)
                                }
                            };
                            
                            // Send response if one was created
                            if let Some(response) = response {
                                info!("Sending {} response", response.status);
                                let message = Message::Response(response);
                                if let Err(e) = self.transaction_manager.transport().send_message(message, source).await {
                                    error!("Failed to send response: {}", e);
                                } else {
                                    info!("Response sent successfully");
                                }
                            }
                        },
                        Message::Response(response) => {
                            info!("Received unmatched response: {:?} from {}", response.status, source);
                        }
                    }
                },
                TransactionEvent::TransactionCreated { transaction_id } => {
                    debug!("Transaction created: {}", transaction_id);
                },
                TransactionEvent::TransactionCompleted { transaction_id, response } => {
                    if let Some(resp) = response {
                        info!("Transaction completed: {}, response: {:?}", transaction_id, resp.status);
                    } else {
                        info!("Transaction completed: {}, no response", transaction_id);
                    }
                },
                TransactionEvent::TransactionTerminated { transaction_id } => {
                    debug!("Transaction terminated: {}", transaction_id);
                },
                TransactionEvent::Error { error, transaction_id } => {
                    error!("Transaction error: {}, id: {:?}", error, transaction_id);
                },
            }
        }
        
        Ok(())
    }
    
    /// Handle an INVITE request
    async fn handle_invite(&self, request: Request, source: SocketAddr) -> Result<Option<Response>> {
        // Extract Call-ID for tracking
        let call_id = request.call_id().unwrap_or("unknown");
        
        // Generate a tag for To header
        let tag = format!("tag-{}", Uuid::new_v4());
        
        info!("Processing INVITE for call {}", call_id);
        
        // Extract remote RTP port from SDP
        let remote_rtp_port = if !request.body.is_empty() {
            extract_rtp_port_from_sdp(&request.body)
        } else {
            None
        };
        
        if remote_rtp_port.is_none() {
            warn!("Could not extract RTP port from INVITE SDP");
        } else {
            info!("Remote endpoint RTP port is {}", remote_rtp_port.unwrap());
        }
        
        // Define our RTP port - Bob uses port 10002 for RTP
        let local_rtp_port = 10002;
        
        // Create 200 OK response
        let mut response = Response::new(StatusCode::Ok);
        
        // Add common headers
        self.add_common_headers(&mut response, &request);
        
        // Add To header with tag
        if let Some(to_header) = request.header(&HeaderName::To) {
            if let Some(to_text) = to_header.value.as_text() {
                info!("Adding To header with tag: {}", tag);
                response.headers.push(Header::text(
                    HeaderName::To,
                    format!("{};tag={}", to_text, tag)
                ));
            }
        }
        
        // Add Contact header
        let contact = format!("<sip:{}@{}>", self.username, self.local_addr);
        response.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Create SDP answer using the abstraction
        let sdp = SessionDescription::new_audio_call(
            &self.username,
            self.local_addr.ip(),
            local_rtp_port
        );
        
        let sdp_str = sdp.to_string();
        
        // Add Content-Type
        response.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
        
        // Add Content-Length
        response.headers.push(Header::text(
            HeaderName::ContentLength, 
            sdp_str.len().to_string()
        ));
        
        // Add SDP body
        response.body = sdp_str.into();
        
        info!("Created OK response for INVITE with {} headers", response.headers.len());
        
        // If we have a remote RTP port, set up the RTP session
        if let Some(remote_port) = remote_rtp_port {
            let remote_ip = source.ip();
            let remote_rtp_addr = SocketAddr::new(remote_ip, remote_port);
            info!("Setting up RTP session with remote endpoint at {}", remote_rtp_addr);
            
            // Setup RTP session
            let rtp_config = RtpSessionConfig {
                local_addr: format!("{}:{}", self.local_addr.ip(), local_rtp_port).parse()?,
                remote_addr: Some(remote_rtp_addr),
                payload_type: 0, // PCMU
                clock_rate: 8000,
                ..Default::default()
            };
            
            // Create the RTP session
            match RtpSession::new(rtp_config).await {
                Ok(mut rtp_session) => {
                    info!("RTP session established on port {}", local_rtp_port);
                    
                    // Create channel for packet communication
                    let (packet_tx, mut packet_rx) = tokio::sync::mpsc::channel::<RtpPacket>(100);
                    
                    // Clone call id for the task
                    let call_id_clone = call_id.to_string();
                    
                    // Clone active calls reference
                    let active_calls = self.active_calls.clone();
                    
                    // Start RTP handling task
                    let handle = tokio::spawn(async move {
                        // Create G.711 codec for sending
                        let send_codec = G711Codec::new(G711Variant::PCMU);
                        let sample_interval = Duration::from_millis(20); // 20ms packets
                        let samples_per_packet = 160; // 20ms of 8kHz audio
                        
                        // Generate a different tone (880 Hz) to distinguish from caller
                        let test_tone = generate_sine_wave(880.0, samples_per_packet);
                        let audio_format = AudioFormat::mono_16bit(SampleRate::Rate8000);
                        
                        // Start a separate task for receiving
                        let recv_codec = G711Codec::new(G711Variant::PCMU);
                        let _recv_task = tokio::spawn(async move {
                            while let Some(packet) = packet_rx.recv().await {
                                info!("Received RTP packet: seq={}, ts={}, len={}",
                                    packet.header.sequence_number,
                                    packet.header.timestamp,
                                    packet.payload.len());
                                
                                // Decode audio
                                match recv_codec.decode(&packet.payload) {
                                    Ok(audio_buffer) => {
                                        // In a real application, you would play this audio
                                        debug!("Decoded audio: {} samples", audio_buffer.samples());
                                        
                                        // Here you would typically send to an audio output device
                                        // play_audio(&audio_buffer);
                                    },
                                    Err(e) => error!("Failed to decode audio: {}", e),
                                }
                            }
                        });
                        
                        // Main task handles sending and forwards received packets
                        let mut timestamp: RtpTimestamp = 0;
                        loop {
                            // Create audio buffer with the test tone
                            let audio_buffer = AudioBuffer::new(Bytes::from(test_tone.clone()), audio_format);
                            
                            // Encode and send
                            match send_codec.encode(&audio_buffer) {
                                Ok(encoded) => {
                                    if let Err(e) = rtp_session.send_packet(timestamp, encoded, false).await {
                                        error!("Failed to send RTP packet: {}", e);
                                    } else {
                                        debug!("Sent RTP packet");
                                    }
                                },
                                Err(e) => error!("Failed to encode audio: {}", e),
                            }
                            
                            // Check for received packets
                            while let Ok(packet) = rtp_session.receive_packet().await {
                                // Forward to receiver task
                                if packet_tx.send(packet).await.is_err() {
                                    error!("Failed to forward RTP packet to receiver task");
                                    break;
                                }
                            }
                            
                            timestamp = timestamp.wrapping_add(samples_per_packet as u32);
                            tokio::time::sleep(sample_interval).await;
                        }
                        
                        // This code won't be reached but Rust needs it
                        #[allow(unreachable_code)]
                        {
                            let _ = rtp_session.close().await;
                        }
                    });
                    
                    // Store the handle so we can terminate it when the call ends
                    active_calls.add(call_id_clone, handle).await;
                },
                Err(e) => {
                    error!("Failed to create RTP session: {}", e);
                }
            }
        }
        
        Ok(Some(response))
    }
    
    /// Handle a BYE request
    async fn handle_bye(&self, request: Request, _source: SocketAddr) -> Result<Option<Response>> {
        // Extract Call-ID for tracking
        let call_id = request.call_id().unwrap_or("unknown");
        
        info!("Processing BYE for call {}", call_id);
        
        // Terminate the RTP session for this call
        self.active_calls.remove(call_id).await;
        
        // Create 200 OK response
        let mut response = Response::new(StatusCode::Ok);
        
        // Add common headers
        self.add_common_headers(&mut response, &request);
        
        // Add Content-Length
        response.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        Ok(Some(response))
    }
    
    /// Add common headers to a response based on the request
    fn add_common_headers(&self, response: &mut Response, request: &Request) {
        info!("Adding common headers to response");
        
        // Copy Via headers from request
        for header in &request.headers {
            if header.name == HeaderName::Via {
                response.headers.push(header.clone());
                debug!("Added Via header: {:?}", header.value);
            }
        }
        
        // Copy From header
        if let Some(from_header) = request.header(&HeaderName::From) {
            response.headers.push(from_header.clone());
            debug!("Added From header: {:?}", from_header.value);
        }
        
        // Copy To header if not already added
        if response.headers.iter().find(|h| h.name == HeaderName::To).is_none() {
            if let Some(to_header) = request.header(&HeaderName::To) {
                response.headers.push(to_header.clone());
                debug!("Added To header: {:?}", to_header.value);
            }
        }
        
        // Copy Call-ID
        if let Some(call_id_header) = request.header(&HeaderName::CallId) {
            response.headers.push(call_id_header.clone());
            debug!("Added Call-ID header: {:?}", call_id_header.value);
        }
        
        // Copy CSeq
        if let Some(cseq_header) = request.header(&HeaderName::CSeq) {
            response.headers.push(cseq_header.clone());
            debug!("Added CSeq header: {:?}", cseq_header.value);
        }
        
        // Add User-Agent
        response.headers.push(Header::text(
            HeaderName::UserAgent,
            "RVOIP-Test-UA/0.1.0"
        ));
        
        info!("Added {} common headers to response", response.headers.len());
    }
}

/// Run the user agent and wait for requests
pub async fn run_user_agent(addr: &str, username: &str, domain: &str) -> Result<()> {
    // Parse socket address
    let local_addr: SocketAddr = addr.parse()
        .context("Invalid address format")?;
    
    // Create user agent
    let mut user_agent = UserAgent::new(
        local_addr,
        username.to_string(),
        domain.to_string(),
    ).await?;
    
    // Process requests in background
    let active_calls = user_agent.active_calls.clone();
    let handle = tokio::spawn(async move {
        if let Err(e) = user_agent.process_requests().await {
            error!("Error processing requests: {}", e);
        }
    });
    
    // Wait for Ctrl+C to shutdown
    ctrl_c().await.context("Failed to listen for ctrl+c signal")?;
    info!("Shutting down user agent...");
    
    // Terminate all active calls
    active_calls.terminate_all().await;
    
    // Cancel the handle
    handle.abort();
    
    Ok(())
}

/// Generate a simple sine wave for testing
fn generate_sine_wave(frequency: f32, num_samples: usize) -> Vec<u8> {
    let mut pcm_data = Vec::with_capacity(num_samples * 2); // 16-bit samples = 2 bytes each
    let sample_rate = 8000.0; // 8kHz
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate;
        let amplitude = 0.5; // 50% volume
        let value = (amplitude * (2.0 * std::f32::consts::PI * frequency * t).sin() * 32767.0) as i16;
        
        // Convert i16 to bytes (little endian)
        pcm_data.push((value & 0xFF) as u8);
        pcm_data.push(((value >> 8) & 0xFF) as u8);
    }
    
    pcm_data
} 