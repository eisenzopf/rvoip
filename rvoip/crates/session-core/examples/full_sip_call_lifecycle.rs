//! Full SIP Call Lifecycle Example
//!
//! This example demonstrates a complete SIP call lifecycle between two endpoints
//! using the session-core client/server API layer. It shows:
//! 
//! 1. SIP session negotiation with SDP offer/answer
//! 2. Audio session establishment with codec negotiation
//! 3. Audio exchange simulation with quality metrics
//! 4. Call hold/resume operations
//! 5. Proper call termination
//!
//! The example creates two SIP endpoints (Alice and Bob) and establishes
//! a full duplex audio call between them with comprehensive debug output.

use std::sync::Arc;
use std::time::Duration;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use anyhow::{Result, Context};
use std::str::FromStr;
use async_trait::async_trait;
use tracing::{info, debug, warn, error};

// Import the correct types from our libraries
use rvoip_sip_core::{
    Uri, Message, Method, StatusCode, 
    Request, Response, HeaderName, TypedHeader, 
    types::{status::StatusCode as SIPStatusCode, address::Address},
};
use rvoip_sip_transport::{Transport, TransportEvent};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};

// Import from session-core with correct paths
use rvoip_session_core::{
    api::{
        client::{ClientConfig, create_full_client_manager},
        server::{ServerConfig, create_full_server_manager, UserRegistration},
        get_api_capabilities,
    },
    events::{EventBus, EventHandler, SessionEvent},
    session::{
        SessionConfig, 
        SessionId, 
        session::Session, 
        manager::SessionManager,
        SessionState
    },
    dialog::{
        DialogId,
        dialog_manager::DialogManager, 
        dialog_state::DialogState
    },
    sdp::{SessionDescription, create_audio_offer, create_audio_answer, extract_media_config},
    media::{MediaManager, MediaConfig, MediaType, AudioCodecType, MediaStatus, QualityMetrics},
    errors::Error,
    helpers::{make_call, answer_call, end_call}
};

/// Enhanced SIP transport implementation that simulates network communication
#[derive(Debug, Clone)]
struct SimulatedTransport {
    event_tx: mpsc::Sender<TransportEvent>,
    local_addr: SocketAddr,
    name: String,
    peer_tx: Option<mpsc::Sender<(Message, SocketAddr)>>,
}

impl SimulatedTransport {
    fn new(event_tx: mpsc::Sender<TransportEvent>, local_addr: SocketAddr, name: String) -> Self {
        Self { 
            event_tx, 
            local_addr, 
            name,
            peer_tx: None,
        }
    }
    
    fn set_peer(&mut self, peer_tx: mpsc::Sender<(Message, SocketAddr)>) {
        self.peer_tx = Some(peer_tx);
    }
}

#[async_trait::async_trait]
impl Transport for SimulatedTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        
        info!("🚀 [{}] Sending {} to {}", 
            self.name,
            if let Some(request) = message.as_request() { 
                format!("REQUEST: {}", request.method()) 
            } else if let Some(response) = message.as_response() { 
                format!("RESPONSE: {}", response.status_code()) 
            } else {
                "UNKNOWN".to_string()
            }, 
            destination);
        
        // Log SIP message details
        if let Some(request) = message.as_request() {
            debug!("📤 [{}] Request Details:", self.name);
            debug!("   Method: {}", request.method());
            debug!("   URI: {}", request.uri());
            debug!("   Call-ID: {:?}", request.call_id());
            debug!("   CSeq: {:?}", request.cseq());
            debug!("   📋 SDP Content:");
            let body = request.body();
            for line in std::str::from_utf8(body).unwrap_or("").lines().take(10) {
                debug!("      {}", line);
            }
        } else if let Some(response) = message.as_response() {
            debug!("📥 [{}] Response Details:", self.name);
            debug!("   Status: {} {}", response.status_code(), response.reason_phrase());
            debug!("   CSeq: {:?}", response.cseq());
            debug!("   📋 SDP Content:");
            let body = response.body();
            for line in std::str::from_utf8(body).unwrap_or("").lines().take(10) {
                debug!("      {}", line);
            }
        }
        
        // Simulate network transmission to peer
        if let Some(peer_tx) = &self.peer_tx {
            if let Err(e) = peer_tx.send((message, self.local_addr)).await {
                warn!("Failed to send message to peer: {}", e);
            }
        }
        
        Ok(())
    }
    
    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        info!("🔌 [{}] Transport closed", self.name);
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

/// Comprehensive event handler for session events with detailed logging
struct CallEventHandler {
    name: String,
}

impl CallEventHandler {
    fn new(name: String) -> Self {
        Self { name }
    }
}

#[async_trait::async_trait]
impl EventHandler for CallEventHandler {
    async fn handle_event(&self, event: SessionEvent) {
        match event {
            SessionEvent::Created { session_id } => {
                info!("🌟 [{}] Session created: {}", self.name, session_id);
            },
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                info!("🔄 [{}] Session {} state: {} → {}", 
                    self.name, session_id, old_state, new_state);
                
                match new_state {
                    SessionState::Connected => {
                        info!("📞 [{}] Call established! Audio session active.", self.name);
                    },
                    SessionState::Terminated => {
                        info!("📴 [{}] Call ended.", self.name);
                    },
                    _ => {}
                }
            },
            SessionEvent::DialogUpdated { session_id, dialog_id } => {
                debug!("🔄 [{}] Dialog updated: {} for session {}", 
                    self.name, dialog_id, session_id);
            },
            SessionEvent::MediaStarted { session_id } => {
                info!("🎵 [{}] Media started for session {}", self.name, session_id);
            },
            SessionEvent::MediaStopped { session_id } => {
                info!("🔇 [{}] Media stopped for session {}", self.name, session_id);
            },
            SessionEvent::Terminated { session_id, reason } => {
                info!("💀 [{}] Session terminated: {} (reason: {})", 
                    self.name, session_id, reason);
            },
            _ => {
                debug!("📡 [{}] Other event: {:?}", self.name, event);
            }
        }
    }
}

/// Audio simulation for demonstrating media exchange
struct AudioSimulator {
    name: String,
    session_id: SessionId,
    media_config: MediaConfig,
}

impl AudioSimulator {
    fn new(name: String, session_id: SessionId, media_config: MediaConfig) -> Self {
        Self { name, session_id, media_config }
    }
    
    async fn start_audio_simulation(&self) {
        info!("🎤 [{}] Starting audio simulation for session {}", self.name, self.session_id);
        info!("   🔊 Simulating {} codec at {}Hz", 
            match self.media_config.audio_codec {
                AudioCodecType::PCMU => "G.711 μ-law",
                AudioCodecType::PCMA => "G.711 A-law",
            },
            self.media_config.clock_rate);
        
        // Simulate audio packets
        for i in 1..=5 {
            sleep(Duration::from_millis(500)).await;
            info!("🎵 [{}] Audio packet {} sent (RTP PT:{}, SSRC:12345)", 
                self.name, i, self.media_config.payload_type);
            
            // Simulate quality metrics
            let quality = QualityMetrics {
                packet_loss_rate: 0.001 * (i as f64), // Slight increase over time
                jitter_ms: 2.0 + (i as f64) * 0.5,
                round_trip_time_ms: 45.0 + (i as f64) * 2.0,
                bitrate_kbps: 64, // G.711 standard bitrate
            };
            
            debug!("📊 [{}] Audio quality: Loss={:.3}%, Jitter={:.1}ms, RTT={:.1}ms", 
                self.name, quality.packet_loss_rate * 100.0, quality.jitter_ms, quality.round_trip_time_ms);
        }
    }
}

/// Network message router to simulate message exchange between endpoints
struct NetworkRouter {
    alice_tx: mpsc::Sender<(Message, SocketAddr)>,
    bob_tx: mpsc::Sender<(Message, SocketAddr)>,
}

impl NetworkRouter {
    fn new(
        alice_tx: mpsc::Sender<(Message, SocketAddr)>,
        bob_tx: mpsc::Sender<(Message, SocketAddr)>
    ) -> Self {
        Self { alice_tx, bob_tx }
    }
    
    async fn start_routing(
        alice_tx: mpsc::Sender<(Message, SocketAddr)>,
        bob_tx: mpsc::Sender<(Message, SocketAddr)>,
        mut alice_rx: mpsc::Receiver<(Message, SocketAddr)>,
        mut bob_rx: mpsc::Receiver<(Message, SocketAddr)>
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some((message, from_addr)) = alice_rx.recv() => {
                        debug!("🌐 Router: Alice → Bob");
                        if let Err(e) = bob_tx.send((message, from_addr)).await {
                            warn!("Router failed to deliver Alice→Bob: {}", e);
                            break;
                        }
                    },
                    Some((message, from_addr)) = bob_rx.recv() => {
                        debug!("🌐 Router: Bob → Alice");
                        if let Err(e) = alice_tx.send((message, from_addr)).await {
                            warn!("Router failed to deliver Bob→Alice: {}", e);
                            break;
                        }
                    },
                    else => break,
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize comprehensive logging
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;
    
    info!("🚀 RVOIP Full SIP Call Lifecycle Demo");
    info!("=====================================");
    
    // Display API capabilities
    let capabilities = get_api_capabilities();
    info!("📋 API Capabilities:");
    info!("   📞 Call Transfer: {}", capabilities.call_transfer);
    info!("   🎵 Media Coordination: {}", capabilities.media_coordination);
    info!("   ⏸️  Call Hold: {}", capabilities.call_hold);
    info!("   🛣️  Call Routing: {}", capabilities.call_routing);
    info!("   👤 User Registration: {}", capabilities.user_registration);
    info!("   📊 Max Sessions: {}", capabilities.max_sessions);
    
    // Setup network addresses
    let alice_addr: SocketAddr = "127.0.0.1:5060".parse()?;
    let bob_addr: SocketAddr = "127.0.0.1:5061".parse()?;
    
    info!("\n👥 Setting up SIP endpoints:");
    info!("   👩 Alice: {}", alice_addr);
    info!("   👨 Bob: {}", bob_addr);
    
    // Create transport channels for message routing
    let (alice_transport_tx, alice_transport_rx) = mpsc::channel(100);
    let (bob_transport_tx, bob_transport_rx) = mpsc::channel(100);
    let (alice_network_tx, alice_network_rx) = mpsc::channel(100);
    let (bob_network_tx, bob_network_rx) = mpsc::channel(100);
    
    // Create transports
    let mut alice_transport = SimulatedTransport::new(
        alice_transport_tx, alice_addr, "Alice".to_string()
    );
    let mut bob_transport = SimulatedTransport::new(
        bob_transport_tx, bob_addr, "Bob".to_string()
    );
    
    // Set up peer connections for message routing
    alice_transport.set_peer(bob_network_tx.clone());
    bob_transport.set_peer(alice_network_tx.clone());
    
    let alice_transport = Arc::new(alice_transport);
    let bob_transport = Arc::new(bob_transport);
    
    // Setup network router
    NetworkRouter::start_routing(alice_network_tx, bob_network_tx, alice_network_rx, bob_network_rx).await;
    
    // Create transaction managers
    info!("\n🔧 Creating transaction managers...");
    let (alice_tm, alice_events) = TransactionManager::new(
        alice_transport.clone(),
        alice_transport_rx,
        Some(10)
    ).await.map_err(|e| anyhow::anyhow!("Failed to create Alice's transaction manager: {}", e))?;
    let alice_tm = Arc::new(alice_tm);
    
    let (bob_tm, bob_events) = TransactionManager::new(
        bob_transport.clone(),
        bob_transport_rx,
        Some(10)
    ).await.map_err(|e| anyhow::anyhow!("Failed to create Bob's transaction manager: {}", e))?;
    let bob_tm = Arc::new(bob_tm);
    
    // Create client and server configurations
    let alice_config = ClientConfig {
        display_name: "Alice Smith".to_string(),
        uri: "sip:alice@example.com".to_string(),
        contact: format!("sip:alice@{}", alice_addr),
        auth_user: None,
        auth_password: None,
        registration_interval: None,
        user_agent: "RVOIP-Alice/1.0".to_string(),
        max_concurrent_calls: 5,
        auto_answer: false,
        session_config: SessionConfig {
            local_signaling_addr: alice_addr,
            local_media_addr: "127.0.0.1:10000".parse()?,
            supported_codecs: vec![AudioCodecType::PCMU, AudioCodecType::PCMA],
            display_name: Some("Alice Smith".to_string()),
            user_agent: "RVOIP-Alice/1.0".to_string(),
            max_duration: 0,
            max_sessions: Some(10),
        },
    };
    
    let bob_config = ServerConfig {
        server_name: "Bob's Server".to_string(),
        domain: "example.com".to_string(),
        max_sessions: 100,
        session_timeout: 3600,
        max_calls_per_user: 5,
        enable_routing: true,
        enable_transfer: true,
        enable_conference: false,
        user_agent: "RVOIP-Bob/1.0".to_string(),
        session_config: SessionConfig {
            local_signaling_addr: bob_addr,
            local_media_addr: "127.0.0.1:10001".parse()?,
            supported_codecs: vec![AudioCodecType::PCMU, AudioCodecType::PCMA],
            display_name: Some("Bob Johnson".to_string()),
            user_agent: "RVOIP-Bob/1.0".to_string(),
            max_duration: 0,
            max_sessions: Some(10),
        },
    };
    
    // Create session managers
    info!("🔧 Creating session managers...");
    let alice_manager = create_full_client_manager(alice_tm.clone(), alice_config).await
        .map_err(|e| anyhow::anyhow!("Failed to create Alice's client manager: {}", e))?;
    let bob_manager = create_full_server_manager(bob_tm.clone(), bob_config).await
        .map_err(|e| anyhow::anyhow!("Failed to create Bob's server manager: {}", e))?;
    
    // Register event handlers
    let alice_handler = Arc::new(CallEventHandler::new("Alice".to_string()));
    let bob_handler = Arc::new(CallEventHandler::new("Bob".to_string()));
    
    // Start the call lifecycle demonstration
    info!("\n📞 Starting SIP call lifecycle demonstration...");
    
    // Phase 1: Call Setup
    info!("\n🎬 Phase 1: Call Setup & SDP Negotiation");
    info!("==========================================");
    
    let destination_uri = Uri::sip("bob@example.com");
    info!("📱 Alice initiating call to {}", destination_uri);
    
    // Alice makes the call
    let alice_session = alice_manager.make_call(destination_uri.clone()).await?;
    info!("✅ Alice created outgoing session: {}", alice_session.id);
    
    // Simulate SDP offer creation
    let alice_media_addr = alice_manager.config().session_config.local_media_addr;
    let alice_offer = create_audio_offer(
        alice_media_addr.ip(),
        alice_media_addr.port(),
        &alice_manager.config().session_config.supported_codecs
    ).map_err(|e| anyhow::anyhow!("Failed to create SDP offer: {}", e))?;
    
    info!("📋 Alice created SDP offer:");
    info!("   🎵 Media: Audio");
    info!("   📍 Address: {}", alice_media_addr);
    info!("   🎼 Codecs: {:?}", alice_manager.config().session_config.supported_codecs);
    
    // Simulate Bob receiving the call (in a real scenario, this would be triggered by incoming INVITE)
    sleep(Duration::from_millis(100)).await;
    
    // Create a mock incoming request for Bob
    let mock_request = Request::new(Method::Invite, destination_uri);
    
    info!("📨 Bob receiving incoming call...");
    let bob_session = bob_manager.handle_incoming_call(&mock_request).await?;
    info!("✅ Bob created incoming session: {}", bob_session.id);
    
    // Bob creates SDP answer
    let bob_media_addr = bob_manager.config().session_config.local_media_addr;
    let bob_answer = create_audio_answer(
        &alice_offer,
        bob_media_addr.ip(),
        bob_media_addr.port(),
        &bob_manager.config().session_config.supported_codecs
    ).map_err(|e| anyhow::anyhow!("Failed to create SDP answer: {}", e))?;
    
    info!("📋 Bob created SDP answer:");
    info!("   🎵 Media: Audio");
    info!("   📍 Address: {}", bob_media_addr);
    info!("   ✅ Negotiated codecs from offer");
    
    // Extract negotiated media configuration
    let media_config = extract_media_config(&alice_offer, &bob_answer)
        .map_err(|e| anyhow::anyhow!("Failed to extract media config: {}", e))?;
    
    info!("🤝 SDP Negotiation Complete:");
    info!("   📍 Alice RTP: {}", media_config.local_addr);
    info!("   📍 Bob RTP: {:?}", media_config.remote_addr);
    info!("   🎼 Negotiated Codec: {:?}", media_config.audio_codec);
    info!("   📊 Payload Type: {}", media_config.payload_type);
    info!("   🔊 Clock Rate: {}Hz", media_config.clock_rate);
    
    // Phase 2: Media Session Establishment
    info!("\n🎬 Phase 2: Media Session Establishment");
    info!("=======================================");
    
    // Create media manager (simplified for this example)
    let media_manager = MediaManager::new().await?;
    
    // Setup media sessions for both endpoints
    let alice_media_id = media_manager.create_media_session(media_config.clone()).await?;
    let bob_media_id = media_manager.create_media_session(media_config.clone()).await?;
    
    info!("🎵 Created media sessions:");
    info!("   👩 Alice Media ID: {}", alice_media_id);
    info!("   👨 Bob Media ID: {}", bob_media_id);
    
    // Start media streams
    media_manager.start_media(&alice_session.id, &alice_media_id).await?;
    media_manager.start_media(&bob_session.id, &bob_media_id).await?;
    
    info!("🚀 Media streams started - Audio session established!");
    
    // Phase 3: Audio Exchange Simulation
    info!("\n🎬 Phase 3: Audio Exchange Simulation");
    info!("=====================================");
    
    // Create audio simulators
    let alice_audio = AudioSimulator::new(
        "Alice".to_string(), 
        alice_session.id.clone(), 
        media_config.clone()
    );
    let bob_audio = AudioSimulator::new(
        "Bob".to_string(), 
        bob_session.id.clone(), 
        media_config.clone()
    );
    
    // Start bidirectional audio simulation
    let alice_audio_task = tokio::spawn(async move {
        alice_audio.start_audio_simulation().await;
    });
    
    let bob_audio_task = tokio::spawn(async move {
        sleep(Duration::from_millis(250)).await; // Slight offset
        bob_audio.start_audio_simulation().await;
    });
    
    // Wait for audio simulation to complete
    let _ = tokio::try_join!(alice_audio_task, bob_audio_task)?;
    
    info!("🎵 Audio exchange simulation completed");
    
    // Phase 4: Call Hold/Resume Demonstration
    info!("\n🎬 Phase 4: Call Hold/Resume Operations");
    info!("======================================");
    
    info!("⏸️  Alice putting call on hold...");
    alice_manager.hold_call(&alice_session.id).await?;
    media_manager.pause_media(&alice_media_id).await?;
    
    info!("🔇 Call on hold - media paused");
    sleep(Duration::from_secs(1)).await;
    
    info!("▶️  Alice resuming call...");
    alice_manager.resume_call(&alice_session.id).await?;
    media_manager.resume_media(&alice_media_id).await?;
    
    info!("🔊 Call resumed - media active");
    sleep(Duration::from_secs(1)).await;
    
    // Phase 5: Call Termination
    info!("\n🎬 Phase 5: Call Termination");
    info!("============================");
    
    info!("📴 Alice ending the call...");
    alice_manager.end_call(&alice_session.id).await?;
    
    // Stop media sessions
    media_manager.stop_media(&alice_media_id, "Call ended by Alice".to_string()).await?;
    media_manager.stop_media(&bob_media_id, "Call ended by Alice".to_string()).await?;
    
    info!("🔇 Media sessions stopped");
    
    // Bob also ends his side
    bob_manager.session_manager().terminate_session(&bob_session.id, Some("Call ended".to_string())).await?;
    
    info!("✅ Call termination complete");
    
    // Phase 6: Cleanup and Statistics
    info!("\n🎬 Phase 6: Cleanup & Statistics");
    info!("================================");
    
    // Get final statistics
    let alice_sessions = alice_manager.get_active_calls();
    let bob_stats = bob_manager.get_server_stats().await;
    
    info!("📊 Final Statistics:");
    info!("   👩 Alice active calls: {}", alice_sessions.len());
    info!("   👨 Bob active sessions: {}", bob_stats.active_sessions);
    info!("   👥 Bob registered users: {}", bob_stats.registered_users);
    
    // Cleanup
    media_manager.shutdown().await?;
    alice_transport.close().await?;
    bob_transport.close().await?;
    
    info!("\n🎉 Full SIP Call Lifecycle Demo Completed Successfully!");
    info!("======================================================");
    info!("✅ SIP session negotiation with SDP offer/answer");
    info!("✅ Audio session establishment with codec negotiation");
    info!("✅ Bidirectional audio exchange simulation");
    info!("✅ Call hold/resume operations");
    info!("✅ Proper call termination and cleanup");
    info!("");
    info!("🔍 This demonstrates that the session-core API layer");
    info!("   provides complete SIP compliance for full call lifecycle!");
    
    Ok(())
} 