//! Core SessionCoordinator structure and initialization

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Mutex};
use crate::api::{
    types::{SessionId, SessionStats, MediaInfo, CallSession, CallState},
    handlers::CallHandler,
    builder::{SessionManagerConfig, MediaConfig},
    bridge::BridgeEvent,
};
use crate::errors::{Result, SessionError};
use crate::manager::{
    events::{SessionEventProcessor, SessionEvent},
    cleanup::CleanupManager,
};
use super::registry::InternalSessionRegistry;
use crate::dialog::{DialogManager, SessionDialogCoordinator, DialogBuilder};
use crate::media::{MediaManager, SessionMediaCoordinator};
use crate::conference::{ConferenceManager};
use crate::sdp::{SdpNegotiator, NegotiatedMediaConfig, SdpRole};
use rvoip_dialog_core::events::SessionCoordinationEvent;
use std::collections::HashMap;
use std::time::Instant;
use dashmap::DashMap;

/// Tracks the readiness state of a session for calling on_call_established
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionReadiness {
    pub(crate) dialog_established: bool,
    pub(crate) media_session_ready: bool,
    pub(crate) sdp_negotiated: bool,
    pub(crate) call_session: Option<CallSession>,
    pub(crate) local_sdp: Option<String>,
    pub(crate) remote_sdp: Option<String>,
}

impl SessionReadiness {
    fn is_fully_ready(&self) -> bool {
        self.dialog_established && self.media_session_ready && self.sdp_negotiated
    }
}

/// Tracks cleanup status for each layer during two-phase termination
#[derive(Debug, Clone)]
pub struct CleanupTracker {
    pub media_done: bool,
    pub client_done: bool,
    pub started_at: Instant,
    pub reason: String,
}

/// Identifies which layer is confirming cleanup
#[derive(Debug, Clone)]
pub enum CleanupLayer {
    Media,
    Client,
    Dialog,
}

/// The main coordinator for the entire session system
pub struct SessionCoordinator {
    // Core services
    pub registry: Arc<InternalSessionRegistry>,
    pub event_processor: Arc<SessionEventProcessor>,
    pub cleanup_manager: Arc<CleanupManager>,
    
    // Subsystem managers
    pub dialog_manager: Arc<DialogManager>,
    pub media_manager: Arc<MediaManager>,
    pub conference_manager: Arc<ConferenceManager>,
    
    // Subsystem coordinators
    pub dialog_coordinator: Arc<SessionDialogCoordinator>,
    pub media_coordinator: Arc<SessionMediaCoordinator>,
    pub presence_coordinator: Arc<RwLock<super::presence::PresenceCoordinator>>,
    
    // SDP Negotiator
    pub sdp_negotiator: Arc<SdpNegotiator>,
    
    // User handler
    pub handler: Option<Arc<dyn CallHandler>>,
    
    // Configuration
    pub config: SessionManagerConfig,
    
    // Event processing - now using unified broadcast channel only
    // Internal events are also published through the broadcast channel
    
    // Bridge event subscribers
    pub bridge_event_subscribers: Arc<RwLock<Vec<mpsc::UnboundedSender<BridgeEvent>>>>,
    
    // Negotiated media configs
    pub negotiated_configs: Arc<RwLock<HashMap<SessionId, NegotiatedMediaConfig>>>,
    
    // Two-phase termination tracking
    pub pending_cleanups: Arc<Mutex<HashMap<SessionId, CleanupTracker>>>,
    
    // Session readiness tracking for on_call_established
    pub session_readiness: Arc<RwLock<HashMap<SessionId, SessionReadiness>>>,
    
    // Shutdown handles for event loops
    event_loop_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    dialog_event_loop_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl SessionCoordinator {
    /// Create and initialize the entire system
    pub async fn new(
        config: SessionManagerConfig,
        handler: Option<Arc<dyn CallHandler>>,
    ) -> Result<Arc<Self>> {
        // Increase file descriptor limit to handle many concurrent RTP sessions
        // Each RTP session needs at least one socket (file descriptor)
        // With 500+ concurrent calls, we need thousands of file descriptors
        if let Err(e) = Self::increase_file_descriptor_limit() {
            tracing::warn!("Failed to increase file descriptor limit: {}. System may not handle high concurrent call volumes.", e);
        }
        
        // Create core services
        let registry = Arc::new(InternalSessionRegistry::new());
        let event_processor = Arc::new(SessionEventProcessor::new());
        let cleanup_manager = Arc::new(CleanupManager::new());

        // Create dialog subsystem
        let dialog_builder = DialogBuilder::new(config.clone());
        let dialog_api = dialog_builder.build().await
            .map_err(|e| SessionError::internal(&format!("Failed to create dialog API: {}", e)))?;

        let dialog_to_session = Arc::new(dashmap::DashMap::new());
        let dialog_manager = Arc::new(DialogManager::new(
            dialog_api.clone(),
            registry.clone(),
            dialog_to_session.clone(),
        ));

        // Create media subsystem - use configured local bind address
        let local_bind_addr = config.local_bind_addr;
        
        // Create media config from session config
        let media_config = crate::media::types::MediaConfig {
            preferred_codecs: config.media_config.preferred_codecs.clone(),
            port_range: Some((config.media_port_start, config.media_port_end)),
            quality_monitoring: true,
            dtmf_support: config.media_config.dtmf_support,
            echo_cancellation: config.media_config.echo_cancellation,
            noise_suppression: config.media_config.noise_suppression,
            auto_gain_control: config.media_config.auto_gain_control,
            music_on_hold_path: config.media_config.music_on_hold_path.clone(),
            max_bandwidth_kbps: config.media_config.max_bandwidth_kbps,
            preferred_ptime: config.media_config.preferred_ptime,
            custom_sdp_attributes: config.media_config.custom_sdp_attributes.clone(),
        };
        
        let media_manager = Arc::new(MediaManager::with_port_range_and_config(
            local_bind_addr,
            config.media_port_start,
            config.media_port_end,
            media_config,
        ));
        
        // Create SDP negotiator
        let sdp_negotiator = Arc::new(SdpNegotiator::new(
            config.media_config.clone(),
            media_manager.clone(),
        ));

        // Create dialog coordination channel only
        // Internal events now use the broadcast channel from event_processor
        let (dialog_coord_tx, dialog_coord_rx) = mpsc::channel(1000);
        
        // Create subsystem coordinators
        let session_to_dialog = Arc::new(DashMap::new());
        let dialog_coordinator = Arc::new(SessionDialogCoordinator::new(
            dialog_api,
            registry.clone(),
            handler.clone(),
            event_processor.clone(),
            dialog_to_session,
            session_to_dialog,
            Arc::new(DashMap::new()),
        ));

        let media_coordinator = Arc::new(SessionMediaCoordinator::new(
            media_manager.clone()
        ));
        
        // Create presence coordinator
        let mut presence_coordinator = super::presence::PresenceCoordinator::new();
        presence_coordinator.set_dialog_manager(dialog_manager.clone());
        let presence_coordinator = Arc::new(RwLock::new(presence_coordinator));

        // Create conference manager with configured local IP
        let conference_manager = Arc::new(ConferenceManager::new(config.local_bind_addr.ip()));

        let coordinator = Arc::new(Self {
            registry,
            event_processor,
            cleanup_manager,
            dialog_manager,
            media_manager,
            conference_manager,
            dialog_coordinator,
            media_coordinator,
            presence_coordinator,
            sdp_negotiator,
            handler,
            config,
            bridge_event_subscribers: Arc::new(RwLock::new(Vec::new())),
            negotiated_configs: Arc::new(RwLock::new(HashMap::new())),
            pending_cleanups: Arc::new(Mutex::new(HashMap::new())),
            session_readiness: Arc::new(RwLock::new(HashMap::new())),
            event_loop_handle: Arc::new(Mutex::new(None)),
            dialog_event_loop_handle: Arc::new(Mutex::new(None)),
        });

        // Initialize subsystems
        coordinator.initialize(dialog_coord_tx, dialog_coord_rx).await?;

        Ok(coordinator)
    }
    
    /// Increase the file descriptor limit for high concurrent call volumes
    /// 
    /// This is necessary because each RTP session requires at least one UDP socket,
    /// and each socket uses a file descriptor. For 500+ concurrent calls, we need
    /// thousands of file descriptors available.
    fn increase_file_descriptor_limit() -> std::result::Result<(), Box<dyn std::error::Error>> {
        use rlimit::{Resource, setrlimit, getrlimit};
        
        // First, get the current limits
        let (soft, hard) = getrlimit(Resource::NOFILE)?;
        
        // We want at least 10,000 file descriptors for high concurrency
        const DESIRED_LIMIT: u64 = 10000;
        
        // Only increase if current limit is lower
        if soft < DESIRED_LIMIT {
            // Try to set to our desired limit, but respect the hard limit
            let new_limit = DESIRED_LIMIT.min(hard);
            setrlimit(Resource::NOFILE, new_limit, new_limit)?;
            
            tracing::info!("Increased file descriptor limit from {} to {} (hard limit: {})", 
                         soft, new_limit, hard);
        } else {
            tracing::debug!("File descriptor limit already sufficient: {} (hard: {})", soft, hard);
        }
        
        Ok(())
    }

    /// Initialize all subsystems and start event loops
    async fn initialize(
        self: &Arc<Self>,
        dialog_coord_tx: mpsc::Sender<SessionCoordinationEvent>,
        dialog_coord_rx: mpsc::Receiver<SessionCoordinationEvent>,
    ) -> Result<()> {
        // Start event processor
        self.event_processor.start().await?;

        // Initialize dialog coordination
        self.dialog_coordinator
            .initialize(dialog_coord_tx)
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to initialize dialog coordinator: {}", e)))?;

        // Start dialog event loop
        let dialog_coordinator = self.dialog_coordinator.clone();
        let dialog_handle = tokio::spawn(async move {
            if let Err(e) = dialog_coordinator.start_event_loop(dialog_coord_rx).await {
                tracing::error!("Dialog event loop error: {}", e);
            }
        });
        
        // Store the dialog event loop handle
        let mut dialog_event_loop_handle = self.dialog_event_loop_handle.lock().await;
        *dialog_event_loop_handle = Some(dialog_handle);

        // Start main event loop using broadcast channel
        let coordinator = self.clone();
        let handle = tokio::spawn(async move {
            coordinator.run_event_loop().await;
        });
        
        // Store the handle for clean shutdown
        let mut event_loop_handle = self.event_loop_handle.lock().await;
        *event_loop_handle = Some(handle);

        tracing::info!("SessionCoordinator initialized on port {}", self.config.sip_port);
        Ok(())
    }

    /// Start all subsystems
    pub async fn start(&self) -> Result<()> {
        self.dialog_manager.start().await
            .map_err(|e| SessionError::internal(&format!("Failed to start dialog manager: {}", e)))?;
        
        self.cleanup_manager.start().await?;
        
        // No need for separate broadcast listener - the main event loop now handles everything
        
        tracing::info!("SessionCoordinator started");
        Ok(())
    }

    /// Stop all subsystems in proper order with verification
    /// 
    /// Orchestrates shutdown in proper order with confirmation at each step:
    /// 1. Send ShutdownInitiated event
    /// 2. Wait for components to acknowledge and shutdown in order
    /// 3. Clean up remaining resources
    pub async fn stop(&self) -> Result<()> {
        tracing::info!("🛑 SessionCoordinator stop() called - event-based shutdown");
        tracing::debug!("🛑 SHUTDOWN: SessionCoordinator stop() called - event-based shutdown with acknowledgments");
        
        // Create a temporary event subscriber to track shutdown progress
        let mut shutdown_subscriber = match self.event_processor.subscribe().await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!("Failed to subscribe to events for shutdown: {}", e);
                // Fallback to direct shutdown
                return self.direct_shutdown().await;
            }
        };
        
        // Step 1: Initiate shutdown
        tracing::debug!("📤 SHUTDOWN: Sending ShutdownInitiated event");
        self.publish_event(SessionEvent::ShutdownInitiated {
            reason: Some("Coordinator stop() called".to_string()),
        }).await?;
        
        // Step 2: Wait for acknowledgments with timeout
        let timeout = std::time::Duration::from_secs(5);
        let start = std::time::Instant::now();
        
        // Track which components have completed
        let mut transport_done = false;
        let mut transaction_done = false;
        let mut dialog_done = false;
        
        while !transport_done || !transaction_done || !dialog_done {
            if start.elapsed() > timeout {
                tracing::debug!("⚠️ SHUTDOWN: Timeout waiting for components, forcing shutdown");
                break;
            }
            
            // Wait for shutdown events with a small timeout
            match tokio::time::timeout(
                std::time::Duration::from_millis(100),
                shutdown_subscriber.receive()
            ).await {
                Ok(Ok(event)) => {
                    match event {
                        SessionEvent::ShutdownComplete { component } => {
                            tracing::debug!("✅ SHUTDOWN: {} completed shutdown", component);
                            match component.as_str() {
                                "UdpTransport" => transport_done = true,
                                "TransactionManager" => transaction_done = true,
                                "DialogManager" => dialog_done = true,
                                _ => {}
                            }
                        }
                        SessionEvent::SystemShutdownComplete => {
                            tracing::debug!("✅ SHUTDOWN: System shutdown complete");
                            break;
                        }
                        _ => {} // Ignore other events during shutdown
                    }
                }
                Ok(Err(e)) => {
                    tracing::debug!("Error receiving shutdown event: {}", e);
                }
                Err(_) => {
                    // Timeout on receive, continue checking
                }
            }
        }
        
        // Step 3: Stop event processor (after all shutdown events processed)
        tracing::debug!("🛑 SHUTDOWN: Stopping event processor...");
        self.event_processor.stop().await?;
        tracing::debug!("✅ SHUTDOWN: Event processor stopped");
        
        // Step 4: Cancel event loop tasks
        tracing::debug!("🛑 SHUTDOWN: Cancelling event loops...");
        let mut event_loop_handle = self.event_loop_handle.lock().await;
        if let Some(handle) = event_loop_handle.take() {
            handle.abort();
        }
        
        let mut dialog_event_loop_handle = self.dialog_event_loop_handle.lock().await;
        if let Some(handle) = dialog_event_loop_handle.take() {
            handle.abort();
        }
        tracing::debug!("✅ SHUTDOWN: Event loops cancelled");
        
        // Step 5: Stop cleanup manager
        tracing::debug!("🛑 SHUTDOWN: Stopping cleanup manager...");
        self.cleanup_manager.stop().await?;
        tracing::debug!("✅ SHUTDOWN: Cleanup manager stopped");
        
        // Step 6: Clean up remaining sessions
        let active_session_ids = self.registry.list_active_sessions().await
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to list active sessions: {}", e);
                Vec::new()
            });
        
        if !active_session_ids.is_empty() {
            tracing::debug!("🛑 SHUTDOWN: Cleaning up {} remaining sessions", active_session_ids.len());
            for session_id in active_session_ids {
                let _ = self.stop_media_session(&session_id).await;
                let _ = self.registry.unregister_session(&session_id).await;
            }
        }
        
        tracing::info!("SessionCoordinator stopped - event-based shutdown complete");
        tracing::debug!("✅ SHUTDOWN: SessionCoordinator fully stopped");
        Ok(())
    }
    
    /// Direct shutdown fallback when event system is unavailable
    async fn direct_shutdown(&self) -> Result<()> {
        tracing::debug!("⚠️ SHUTDOWN: Using direct shutdown fallback");
        
        // Direct stop of dialog manager
        if let Err(e) = self.dialog_manager.stop().await {
            tracing::warn!("Dialog manager stop error: {}", e);
        }
        
        // Small drain
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        // Stop event processor
        self.event_processor.stop().await?;
        
        // Cancel loops
        if let Some(handle) = self.event_loop_handle.lock().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.dialog_event_loop_handle.lock().await.take() {
            handle.abort();
        }
        
        // Stop cleanup manager
        self.cleanup_manager.stop().await?;
        
        Ok(())
    }

    /// Get the bound address
    pub fn get_bound_address(&self) -> std::net::SocketAddr {
        self.dialog_manager.get_bound_address()
    }
    
    /// Get a reference to the dialog coordinator
    pub fn dialog_coordinator(&self) -> &Arc<SessionDialogCoordinator> {
        &self.dialog_coordinator
    }
    
    /// Helper method to publish events through the unified broadcast channel
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        self.event_processor.publish_event(event).await
    }
    
    /// Get a reference to the configuration
    pub fn config(&self) -> &SessionManagerConfig {
        &self.config
    }

    /// Start media session
    pub(crate) async fn start_media_session(&self, session_id: &SessionId) -> Result<()> {
        tracing::debug!("🚀 start_media_session called for {}", session_id);
        
        // Check if media session already exists for THIS specific session
        if let Ok(Some(_)) = self.media_manager.get_media_info(session_id).await {
            tracing::debug!("⏭️ Media session already exists for {}, skipping duplicate creation", session_id);
            return Ok(());
        }
        
        // Also check if session mapping exists directly for THIS specific session
        if self.media_manager.has_session_mapping(session_id).await {
            tracing::debug!("⏭️ Session mapping exists for {}, skipping media creation", session_id);
            return Ok(());
        }
        
        tracing::debug!("🎬 Creating new media session for {}", session_id);
        match self.media_coordinator.on_session_created(session_id).await {
            Ok(()) => {
                tracing::debug!("✅ Successfully created media session for {}", session_id);
                
                // Check if SDP negotiation already happened (upfront SDP case)
                // If we have negotiated config, publish MediaSessionReady immediately
                if let Some(_negotiated) = self.negotiated_configs.read().await.get(session_id) {
                    tracing::info!("SDP already negotiated for {}, publishing MediaSessionReady", session_id);
                    let dialog_id = self.dialog_coordinator.get_dialog_id_for_session(session_id).await;
                    let _ = self.publish_event(SessionEvent::MediaSessionReady {
                        session_id: session_id.clone(),
                        dialog_id,
                    }).await;
                } else {
                    // For outbound calls with upfront SDP, negotiation happens later but we should still
                    // mark media as ready since the media session now exists
                    if let Ok(Some(session)) = self.registry.get_session(session_id).await {
                        if session.local_sdp.is_some() {
                            tracing::info!("Outbound call with upfront SDP, publishing MediaSessionReady for {}", session_id);
                            let dialog_id = self.dialog_coordinator.get_dialog_id_for_session(session_id).await;
                            let _ = self.publish_event(SessionEvent::MediaSessionReady {
                                session_id: session_id.clone(),
                                dialog_id,
                            }).await;
                        } else {
                            tracing::debug!("No negotiated config yet for {}, MediaSessionReady will be published after SDP negotiation", session_id);
                        }
                    } else {
                        tracing::debug!("No negotiated config yet for {}, MediaSessionReady will be published after SDP negotiation", session_id);
                    }
                }
                
                Ok(())
            }
            Err(e) => {
                tracing::debug!("❌ FAILED to create media session for {}: {}", session_id, e);
                Err(SessionError::internal(&format!("Failed to start media: {}", e)))
            }
        }
    }

    /// Stop media session
    pub(crate) async fn stop_media_session(&self, session_id: &SessionId) -> Result<()> {
        self.media_coordinator.on_session_terminated(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to stop media: {}", e)))?;
        
        // Send cleanup confirmation for media layer
        // Media cleanup is synchronous, so we can immediately confirm
        let _ = self.publish_event(SessionEvent::CleanupConfirmation {
            session_id: session_id.clone(),
            layer: "Media".to_string(),
        }).await;
        
        Ok(())
    }

    /// Negotiate SDP as UAC (we sent offer, received answer)
    pub async fn negotiate_sdp_as_uac(
        &self,
        session_id: &SessionId,
        our_offer: &str,
        their_answer: &str,
    ) -> Result<NegotiatedMediaConfig> {
        let negotiated = self.sdp_negotiator.negotiate_as_uac(
            session_id,
            our_offer,
            their_answer,
        ).await?;
        
        // Store negotiated config
        self.negotiated_configs.write().await.insert(session_id.clone(), negotiated.clone());
        
        // Emit MediaNegotiated event
        let _ = self.publish_event(SessionEvent::MediaNegotiated {
            session_id: session_id.clone(),
            local_addr: negotiated.local_addr,
            remote_addr: negotiated.remote_addr,
            codec: negotiated.codec.clone(),
        }).await;
        
        // Only emit MediaSessionReady if media session already exists
        // For upfront SDP cases, the media session doesn't exist yet and MediaSessionReady
        // will be published later when start_media_session is called
        if let Ok(Some(media_info)) = self.media_manager.get_media_info(session_id).await {
            tracing::info!("✅ Media session exists for UAC {}, has remote SDP: {}", 
                session_id, media_info.remote_sdp.is_some());
            let dialog_id = self.dialog_coordinator.get_dialog_id_for_session(session_id).await;
            let _ = self.publish_event(SessionEvent::MediaSessionReady {
                session_id: session_id.clone(),
                dialog_id,
            }).await;
            
            // CRITICAL: Also publish MediaFlowEstablished for UAC
            // This ensures both UAC and UAS publish the event when media is ready
            tracing::info!("📢 Publishing MediaFlowEstablished event for UAC session {} after SDP negotiation", session_id);
            let _ = self.publish_event(SessionEvent::MediaFlowEstablished {
                session_id: session_id.clone(),
                local_addr: negotiated.local_addr.to_string(),
                remote_addr: negotiated.remote_addr.to_string(),
                direction: crate::manager::events::MediaFlowDirection::Both,
            }).await;
            tracing::info!("✅ MediaFlowEstablished event published for UAC {}", session_id);
        } else {
            tracing::warn!("⚠️ Media session doesn't exist yet for UAC {} (upfront SDP), deferring MediaSessionReady to start_media_session", session_id);
            // For UAC, we should still publish MediaFlowEstablished since SDP is negotiated
            tracing::info!("📢 Publishing MediaFlowEstablished event for UAC {} even though media session doesn't exist yet", session_id);
            let _ = self.publish_event(SessionEvent::MediaFlowEstablished {
                session_id: session_id.clone(),
                local_addr: negotiated.local_addr.to_string(),
                remote_addr: negotiated.remote_addr.to_string(),
                direction: crate::manager::events::MediaFlowDirection::Both,
            }).await;
            tracing::info!("✅ MediaFlowEstablished event published for UAC {}", session_id);
        }
        
        Ok(negotiated)
    }
    
    /// Negotiate SDP as UAS (we received offer, generate answer)
    pub async fn negotiate_sdp_as_uas(
        &self,
        session_id: &SessionId,
        their_offer: &str,
    ) -> Result<(String, NegotiatedMediaConfig)> {
        tracing::info!("negotiate_sdp_as_uas called for session {}", session_id);
        let (answer, negotiated) = self.sdp_negotiator.negotiate_as_uas(
            session_id,
            their_offer,
        ).await?;
        
        // Store negotiated config
        self.negotiated_configs.write().await.insert(session_id.clone(), negotiated.clone());
        
        // Emit MediaNegotiated event
        let _ = self.publish_event(SessionEvent::MediaNegotiated {
            session_id: session_id.clone(),
            local_addr: negotiated.local_addr,
            remote_addr: negotiated.remote_addr,
            codec: negotiated.codec.clone(),
        }).await;
        
        // Only emit MediaSessionReady if media session already exists
        // For upfront SDP cases, the media session doesn't exist yet and MediaSessionReady
        // will be published later when start_media_session is called
        if let Ok(Some(media_info)) = self.media_manager.get_media_info(session_id).await {
            tracing::info!("✅ Media session exists for UAS {}, has remote SDP: {}", 
                session_id, media_info.remote_sdp.is_some());
            let dialog_id = self.dialog_coordinator.get_dialog_id_for_session(session_id).await;
            let _ = self.publish_event(SessionEvent::MediaSessionReady {
                session_id: session_id.clone(),
                dialog_id,
            }).await;
            
            // CRITICAL: Also publish MediaFlowEstablished for UAS
            // This is needed because when accept_incoming_call directly calls
            // negotiate_sdp_as_uas, it doesn't go through the SdpNegotiationRequested
            // event handler that would normally publish this event
            tracing::info!("📢 Publishing MediaFlowEstablished event for UAS session {} after SDP negotiation", session_id);
            let _ = self.publish_event(SessionEvent::MediaFlowEstablished {
                session_id: session_id.clone(),
                local_addr: negotiated.local_addr.to_string(),
                remote_addr: negotiated.remote_addr.to_string(),
                direction: crate::manager::events::MediaFlowDirection::Both,
            }).await;
            tracing::info!("✅ MediaFlowEstablished event published for UAS {}", session_id);
        } else {
            tracing::warn!("⚠️ Media session doesn't exist yet for UAS {} (upfront SDP), deferring MediaSessionReady to start_media_session", session_id);
            // For UAS, we should still publish MediaFlowEstablished since SDP is negotiated
            // The media session will be created when the call transitions to Active
            tracing::info!("📢 Publishing MediaFlowEstablished event for UAS {} even though media session doesn't exist yet", session_id);
            let _ = self.publish_event(SessionEvent::MediaFlowEstablished {
                session_id: session_id.clone(),
                local_addr: negotiated.local_addr.to_string(),
                remote_addr: negotiated.remote_addr.to_string(),
                direction: crate::manager::events::MediaFlowDirection::Both,
            }).await;
            tracing::info!("✅ MediaFlowEstablished event published for UAS {}", session_id);
        }
        
        Ok((answer, negotiated))
    }
    
    /// Get negotiated media configuration for a session
    pub async fn get_negotiated_config(&self, session_id: &SessionId) -> Option<NegotiatedMediaConfig> {
        self.negotiated_configs.read().await.get(session_id).cloned()
    }
    
    /// Get the event processor for publishing events
    pub fn event_processor(&self) -> Option<Arc<SessionEventProcessor>> {
        Some(self.event_processor.clone())
    }
    
    /// Start music-on-hold for a session
    pub(crate) async fn start_music_on_hold(&self, session_id: &SessionId) -> Result<()> {
        // Check if MoH file is configured
        if let Some(moh_path) = &self.config.media_config.music_on_hold_path {
            tracing::info!("Starting music-on-hold from: {}", moh_path.display());
            
            // Load WAV file using media-core
            match rvoip_media_core::audio::load_music_on_hold(moh_path).await {
                Ok(ulaw_samples) => {
                    // Start transmitting MoH
                    self.media_manager.start_audio_transmission_with_custom_audio(
                        session_id,
                        ulaw_samples,
                        true  // repeat the music
                    ).await
                    .map_err(|e| SessionError::MediaIntegration { 
                        message: format!("Failed to start MoH transmission: {}", e) 
                    })?;
                    
                    tracing::info!("Music-on-hold started for session: {}", session_id);
                    Ok(())
                }
                Err(e) => {
                    // Return error so caller can fallback to mute
                    Err(SessionError::MediaIntegration { 
                        message: format!("Failed to load MoH file: {}", e) 
                    })
                }
            }
        } else {
            // No MoH configured, return error so caller uses mute
            Err(SessionError::ConfigError(
                "No music-on-hold file configured".to_string()
            ))
        }
    }
    
    /// Stop music-on-hold and resume microphone audio
    pub(crate) async fn stop_music_on_hold(&self, session_id: &SessionId) -> Result<()> {
        // Resume normal microphone audio
        self.media_manager.start_audio_transmission(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to resume microphone audio: {}", e) 
            })?;
        
        tracing::info!("Music-on-hold stopped, microphone resumed for session: {}", session_id);
        Ok(())
    }
    
    /// Get the event sender for compatibility with existing code
    /// This provides access to the broadcast sender from the event processor
    pub async fn event_tx(&self) -> Result<tokio::sync::mpsc::Sender<crate::manager::events::SessionEvent>> {
        self.event_processor.create_mpsc_forwarder().await
    }
}

impl std::fmt::Debug for SessionCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCoordinator")
            .field("config", &self.config)
            .field("has_handler", &self.handler.is_some())
            .finish()
    }
}