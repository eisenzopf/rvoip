//! Core SessionCoordinator structure and initialization

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Mutex};
use crate::api::{
    types::{SessionId, SessionStats, MediaInfo},
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
    
    // SDP Negotiator
    pub sdp_negotiator: Arc<SdpNegotiator>,
    
    // User handler
    pub handler: Option<Arc<dyn CallHandler>>,
    
    // Configuration
    pub config: SessionManagerConfig,
    
    // Event channels
    pub event_tx: mpsc::Sender<SessionEvent>,
    
    // Bridge event subscribers
    pub bridge_event_subscribers: Arc<RwLock<Vec<mpsc::UnboundedSender<BridgeEvent>>>>,
    
    // Negotiated media configs
    pub negotiated_configs: Arc<RwLock<HashMap<SessionId, NegotiatedMediaConfig>>>,
    
    // Two-phase termination tracking
    pub pending_cleanups: Arc<Mutex<HashMap<SessionId, CleanupTracker>>>,
}

impl SessionCoordinator {
    /// Create and initialize the entire system
    pub async fn new(
        config: SessionManagerConfig,
        handler: Option<Arc<dyn CallHandler>>,
    ) -> Result<Arc<Self>> {
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

        // Create event channels
        let (event_tx, event_rx) = mpsc::channel(1000);
        let (dialog_coord_tx, dialog_coord_rx) = mpsc::channel(1000);
        
        // Create subsystem coordinators
        let session_to_dialog = Arc::new(DashMap::new());
        let dialog_coordinator = Arc::new(SessionDialogCoordinator::new(
            dialog_api,
            registry.clone(),
            handler.clone(),
            event_tx.clone(),
            dialog_to_session,
            session_to_dialog,
            Arc::new(DashMap::new()),
            event_processor.clone(),
        ));

        let media_coordinator = Arc::new(SessionMediaCoordinator::new(
            media_manager.clone()
        ));

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
            sdp_negotiator,
            handler,
            config,
            event_tx: event_tx.clone(),
            bridge_event_subscribers: Arc::new(RwLock::new(Vec::new())),
            negotiated_configs: Arc::new(RwLock::new(HashMap::new())),
            pending_cleanups: Arc::new(Mutex::new(HashMap::new())),
        });

        // Initialize subsystems
        coordinator.initialize(event_rx, dialog_coord_tx, dialog_coord_rx).await?;

        Ok(coordinator)
    }

    /// Initialize all subsystems and start event loops
    async fn initialize(
        self: &Arc<Self>,
        event_rx: mpsc::Receiver<SessionEvent>,
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
        tokio::spawn(async move {
            if let Err(e) = dialog_coordinator.start_event_loop(dialog_coord_rx).await {
                tracing::error!("Dialog event loop error: {}", e);
            }
        });

        // Start main event loop
        let coordinator = self.clone();
        tokio::spawn(async move {
            coordinator.run_event_loop(event_rx).await;
        });

        tracing::info!("SessionCoordinator initialized on port {}", self.config.sip_port);
        Ok(())
    }

    /// Start all subsystems
    pub async fn start(&self) -> Result<()> {
        self.dialog_manager.start().await
            .map_err(|e| SessionError::internal(&format!("Failed to start dialog manager: {}", e)))?;
        
        self.cleanup_manager.start().await?;
        
        tracing::info!("SessionCoordinator started");
        Ok(())
    }

    /// Stop all subsystems
    pub async fn stop(&self) -> Result<()> {
        self.cleanup_manager.stop().await?;
        self.event_processor.stop().await?;
        
        self.dialog_manager.stop().await
            .map_err(|e| SessionError::internal(&format!("Failed to stop dialog manager: {}", e)))?;
            
        tracing::info!("SessionCoordinator stopped");
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
    
    /// Get a reference to the configuration
    pub fn config(&self) -> &SessionManagerConfig {
        &self.config
    }

    /// Start media session
    pub(crate) async fn start_media_session(&self, session_id: &SessionId) -> Result<()> {
        println!("🚀 start_media_session called for {}", session_id);
        
        // Check if media session already exists for THIS specific session
        if let Ok(Some(_)) = self.media_manager.get_media_info(session_id).await {
            println!("⏭️ Media session already exists for {}, skipping duplicate creation", session_id);
            return Ok(());
        }
        
        // Also check if session mapping exists directly for THIS specific session
        if self.media_manager.has_session_mapping(session_id).await {
            println!("⏭️ Session mapping exists for {}, skipping media creation", session_id);
            return Ok(());
        }
        
        println!("🎬 Creating new media session for {}", session_id);
        match self.media_coordinator.on_session_created(session_id).await {
            Ok(()) => {
                println!("✅ Successfully started media session for {}", session_id);
                Ok(())
            }
            Err(e) => {
                println!("❌ FAILED to create media session for {}: {}", session_id, e);
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
        let _ = self.event_tx.send(SessionEvent::CleanupConfirmation {
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
        
        // Emit event
        let _ = self.event_tx.send(SessionEvent::MediaNegotiated {
            session_id: session_id.clone(),
            local_addr: negotiated.local_addr,
            remote_addr: negotiated.remote_addr,
            codec: negotiated.codec.clone(),
        }).await;
        
        Ok(negotiated)
    }
    
    /// Negotiate SDP as UAS (we received offer, generate answer)
    pub async fn negotiate_sdp_as_uas(
        &self,
        session_id: &SessionId,
        their_offer: &str,
    ) -> Result<(String, NegotiatedMediaConfig)> {
        let (answer, negotiated) = self.sdp_negotiator.negotiate_as_uas(
            session_id,
            their_offer,
        ).await?;
        
        // Store negotiated config
        self.negotiated_configs.write().await.insert(session_id.clone(), negotiated.clone());
        
        // Emit event
        let _ = self.event_tx.send(SessionEvent::MediaNegotiated {
            session_id: session_id.clone(),
            local_addr: negotiated.local_addr,
            remote_addr: negotiated.remote_addr,
            codec: negotiated.codec.clone(),
        }).await;
        
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
}

impl std::fmt::Debug for SessionCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCoordinator")
            .field("config", &self.config)
            .field("has_handler", &self.handler.is_some())
            .finish()
    }
}