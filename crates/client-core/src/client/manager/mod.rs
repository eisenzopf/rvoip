mod registration_ops;

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, Mutex};
use dashmap::DashMap;
use uuid::Uuid;

// Import session-core APIs - UPDATED to use new API structure
use rvoip_session_core::api::{
    SessionCoordinator,
    SessionManagerBuilder,
    SessionManagerConfig,
    SessionControl,
    MediaControl,
    SipClient,
    MediaConfig as SessionMediaConfig,
    types::SessionId,
    handlers::CallHandler,
};

// Import client-core types
use crate::{
    ClientConfig, ClientResult, ClientError,
    call::{CallId, CallInfo},
    registration::{RegistrationConfig, RegistrationInfo},
    events::{ClientEventHandler, ClientEvent},
};

// Import types from our types module
use super::types::*;
use super::events::ClientCallHandler;
use super::recovery::{retry_with_backoff, RetryConfig, ErrorContext};
use super::config::MediaConfig;

/// High-level SIP client manager that coordinates all client operations
/// 
/// The `ClientManager` is the primary entry point for VoIP client functionality.
/// It provides a high-level, async API for SIP registration, call management,
/// and media control while delegating to session-core for the underlying
/// SIP protocol implementation.
/// 
/// # Architecture
/// 
/// ```text
/// ┌─────────────────────────┐
/// │   Application Layer     │
/// └───────────┬─────────────┘
///             │
/// ┌───────────▼─────────────┐
/// │   ClientManager         │ ◄── This Layer
/// │ ┌─────────────────────┐ │
/// │ │ Registration Mgmt   │ │  • SIP REGISTER handling
/// │ │ Call Management     │ │  • Event coordination
/// │ │ Media Integration   │ │  • State management
/// │ │ Event Broadcasting  │ │  • Error handling
/// │ └─────────────────────┘ │
/// └───────────┬─────────────┘
///             │
/// ┌───────────▼─────────────┐
/// │    session-core         │
/// │  SessionCoordinator     │
/// └─────────────────────────┘
/// ```
/// 
/// # Core Features
/// 
/// ## Registration Management
/// - **SIP Registration**: Register with SIP servers using REGISTER requests
/// - **Authentication**: Handle digest authentication challenges
/// - **Refresh**: Automatic and manual registration refresh
/// - **Multiple Registrations**: Support multiple simultaneous registrations
/// 
/// ## Call Management
/// - **Outbound Calls**: Initiate calls to SIP URIs or phone numbers
/// - **Inbound Calls**: Accept incoming calls with proper SDP negotiation
/// - **Call Control**: Hold, resume, transfer, and hangup operations
/// - **DTMF**: Send dual-tone multi-frequency signals during calls
/// 
/// ## Media Integration
/// - **Codec Support**: Multiple audio codecs (G.711, G.729, Opus)
/// - **Quality Control**: Real-time media quality monitoring
/// - **Echo Cancellation**: Built-in acoustic echo cancellation
/// - **Noise Suppression**: Advanced noise reduction algorithms
/// 
/// ## Event System
/// - **Real-time Events**: Registration, call, and media events
/// - **Broadcast Channel**: Multi-consumer event distribution
/// - **Typed Events**: Strongly-typed event structures
/// - **Priority Levels**: Event prioritization for handling
/// 
/// # Usage Examples
/// 
/// ## Basic Client Setup
/// 
/// ```rust
/// use rvoip_client_core::{ClientManager, ClientConfig};
/// use std::net::SocketAddr;
/// 
/// async fn basic_setup() -> Result<(), Box<dyn std::error::Error>> {
///     // Create client configuration
///     let config = ClientConfig::new()
///         .with_sip_addr("127.0.0.1:5060".parse()?);
///     
///     // Create and start client manager
///     let client = ClientManager::new(config).await?;
///     client.start().await?;
///     
///     println!("✅ SIP client started successfully");
///     
///     // Clean shutdown
///     client.stop().await?;
///     Ok(())
/// }
/// ```
/// 
/// ## Registration and Call Flow
/// 
/// ```rust
/// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
/// use std::time::Duration;
/// 
/// async fn registration_flow() -> Result<(), Box<dyn std::error::Error>> {
///     let config = ClientConfig::new()
///         .with_sip_addr("127.0.0.1:5061".parse()?);
///     
///     let client = ClientManager::new(config).await?;
///     client.start().await?;
///     
///     // Register with SIP server
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:192.168.1.100:5060".to_string(),
    ///         from_uri: "sip:alice@example.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5061".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
///     
///     let registration_id = client.register(reg_config).await?;
///     println!("✅ Registered with ID: {}", registration_id);
///     
///     // Make a call (would be implemented in calls.rs)
///     // let call_id = client.make_call("sip:bob@example.com").await?;
///     
///     // Clean up
///     client.unregister(registration_id).await?;
///     client.stop().await?;
///     Ok(())
/// }
/// ```
/// 
/// ## Event Monitoring
/// 
/// ```rust
/// use rvoip_client_core::{ClientManager, ClientConfig, ClientEvent};
/// use tokio::time::{timeout, Duration};
/// 
/// async fn event_monitoring() -> Result<(), Box<dyn std::error::Error>> {
///     let config = ClientConfig::new()
///         .with_sip_addr("127.0.0.1:5062".parse()?);
///     
///     let client = ClientManager::new(config).await?;
///     client.start().await?;
///     
///     // Subscribe to events
///     let mut event_rx = client.subscribe_events();
///     
///     // Monitor events for a short time
///     let event_task = tokio::spawn(async move {
///         let mut event_count = 0;
///         while event_count < 3 {
    ///             if let Ok(event) = timeout(Duration::from_millis(100), event_rx.recv()).await {
    ///                 match event {
    ///                     Ok(ClientEvent::RegistrationStatusChanged { info, .. }) => {
    ///                         println!("📋 Registration event: {} -> {:?}", 
    ///                             info.user_uri, info.status);
    ///                     }
    ///                     Ok(ClientEvent::CallStateChanged { info, .. }) => {
    ///                         println!("📞 Call event: {} -> {:?}", 
    ///                             info.call_id, info.new_state);
    ///                     }
    ///                     Ok(ClientEvent::MediaEvent { info, .. }) => {
    ///                         println!("🎵 Media event: Call {} event occurred", 
    ///                             info.call_id);
    ///                     }
    ///                     Ok(ClientEvent::IncomingCall { .. }) | 
    ///                     Ok(ClientEvent::ClientError { .. }) | 
    ///                     Ok(ClientEvent::NetworkEvent { .. }) => {
    ///                         // Handle other events as needed
    ///                     }
    ///                     Err(_) => break,
    ///                 }
///                 event_count += 1;
///             } else {
///                 break; // Timeout
///             }
///         }
///     });
///     
///     // Wait for event monitoring to complete
///     let _ = event_task.await;
///     
///     client.stop().await?;
///     Ok(())
/// }
/// ```
pub struct ClientManager {
    /// Session coordinator from session-core
    pub(crate) coordinator: Arc<SessionCoordinator>,
    
    /// Local SIP address (bound)
    pub(crate) local_sip_addr: std::net::SocketAddr,
    
    /// Media configuration
    pub(crate) media_config: MediaConfig,
    
    /// Whether the client is running
    pub(crate) is_running: Arc<RwLock<bool>>,
    
    /// Statistics
    pub(crate) stats: Arc<Mutex<ClientStats>>,
    
    /// Active registrations
    pub(crate) registrations: Arc<RwLock<HashMap<Uuid, RegistrationInfo>>>,
    
    /// Call/Session mapping (CallId -> SessionId)
    pub(crate) session_mapping: Arc<DashMap<CallId, SessionId>>,
    
    /// Call info storage
    pub(crate) call_info: Arc<DashMap<CallId, CallInfo>>,
    
    /// Call handler
    pub(crate) call_handler: Arc<ClientCallHandler>,
    
    /// Event broadcast channel
    pub(crate) event_tx: tokio::sync::broadcast::Sender<ClientEvent>,
    
    /// Tracks which calls have audio frame subscription set up
    pub(crate) audio_setup_calls: Arc<DashMap<CallId, bool>>,
    
    /// Handle to the audio setup task
    audio_setup_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,

    /// Maximum number of concurrent calls allowed
    pub(crate) max_concurrent_calls: usize,
}

impl ClientManager {
    /// Create a new client manager with the given configuration
    /// 
    /// This method initializes a new `ClientManager` instance with the provided
    /// configuration. It sets up the underlying session coordinator, event system,
    /// call mapping structures, and media configuration.
    /// 
    /// # Arguments
    /// 
    /// * `config` - The client configuration specifying SIP addresses, media settings,
    ///              codec preferences, and other operational parameters
    /// 
    /// # Returns
    /// 
    /// Returns an `Arc<ClientManager>` wrapped in a `ClientResult`. The Arc allows
    /// the manager to be shared across multiple async tasks safely.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InternalError` - If the session coordinator cannot be created
    ///   due to invalid configuration or system resource constraints
    /// 
    /// # Examples
    /// 
    /// ## Basic Client Creation
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn create_basic_client() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5060".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     println!("✅ Client created successfully");
    ///     
    ///     // Client is ready but not started yet
    ///     assert!(!client.is_running().await);
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Client with Custom Media Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, MediaConfig, MediaPreset};
    /// 
    /// async fn create_custom_media_client() -> Result<(), Box<dyn std::error::Error>> {
    ///     use rvoip_client_core::client::config::MediaPreset;
    ///     let mut media_config = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
    ///     media_config.echo_cancellation = true;
    ///     media_config.noise_suppression = true;
    ///     media_config.rtp_port_start = 10000;
    ///     media_config.rtp_port_end = 20000;
    ///     
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5061".parse()?)
    ///         .with_media(media_config);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Verify media configuration was applied
    ///     let applied_config = client.get_media_config();
    ///     assert!(applied_config.echo_cancellation);
    ///     assert!(applied_config.noise_suppression);
    ///     assert_eq!(applied_config.rtp_port_start, 10000);
    ///     assert_eq!(applied_config.rtp_port_end, 20000);
    ///     
    ///     println!("✅ Custom media client created");
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Enterprise Client Setup
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, MediaConfig};
    /// 
    /// async fn create_enterprise_client() -> Result<(), Box<dyn std::error::Error>> {
    ///     use rvoip_client_core::client::config::MediaPreset;
    ///     let mut media_config = MediaConfig::from_preset(MediaPreset::Secure);
    ///     media_config.max_bandwidth_kbps = Some(128);  // 128 kbps max
    ///     media_config.preferred_ptime = Some(20);      // 20ms packet time
    ///     
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("0.0.0.0:5060".parse()?)  // Bind to all interfaces
    ///         .with_media_addr("0.0.0.0:0".parse()?)   // Dynamic media port
    ///         .with_media(media_config);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     println!("✅ Enterprise client ready for production");
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Error Handling
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, ClientError};
    /// 
    /// async fn handle_creation_errors() -> Result<(), Box<dyn std::error::Error>> {
    ///     // Try to create client with potentially problematic config
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5062".parse()?);
    ///     
    ///     match ClientManager::new(config).await {
    ///         Ok(client) => {
    ///             println!("✅ Client created successfully");
    ///             // Use client...
    ///         }
    ///         Err(ClientError::InternalError { message }) => {
    ///             println!("❌ Failed to create client: {}", message);
    ///             // Handle error (retry, use different config, etc.)
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Unexpected error: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Multi-Client Architecture
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// use std::sync::Arc;
    /// 
    /// async fn multi_client_setup() -> Result<(), Box<dyn std::error::Error>> {
    ///     // Create multiple clients for different purposes
    ///     let client1_config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5060".parse()?);
    ///     let client1 = ClientManager::new(client1_config).await?;
    ///     
    ///     let client2_config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5061".parse()?);
    ///     let client2 = ClientManager::new(client2_config).await?;
    ///     
    ///     // Clients can be shared across tasks
    ///     let client1_clone = Arc::clone(&client1);
    ///     let task1 = tokio::spawn(async move {
    ///         // Use client1_clone in this task
    ///         println!("Task 1 using client on port 5060");
    ///     });
    ///     
    ///     let client2_clone = Arc::clone(&client2);
    ///     let task2 = tokio::spawn(async move {
    ///         // Use client2_clone in this task
    ///         println!("Task 2 using client on port 5061");
    ///     });
    ///     
    ///     // Wait for tasks to complete
    ///     let _ = tokio::try_join!(task1, task2)?;
    ///     
    ///     println!("✅ Multi-client setup complete");
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Implementation Notes
    /// 
    /// The constructor performs several key initialization steps:
    /// 
    /// 1. **Session Coordinator Setup**: Creates the underlying session-core coordinator
    ///    with media preferences and SIP configuration
    /// 2. **Event System**: Initializes broadcast channels for real-time events
    /// 3. **Call Mapping**: Sets up concurrent data structures for call tracking
    /// 4. **Media Configuration**: Applies codec preferences and quality settings
    /// 5. **Handler Registration**: Registers the call handler for SIP events
    /// 
    /// The returned `Arc<ClientManager>` enables safe sharing across async tasks
    /// and ensures proper cleanup through RAII patterns.
    pub async fn new(config: ClientConfig) -> ClientResult<Arc<Self>> {
        // Create call/session mapping
        let call_mapping = Arc::new(DashMap::new());
        let session_mapping = Arc::new(DashMap::new());
        let call_info = Arc::new(DashMap::new());
        let incoming_calls = Arc::new(DashMap::new());
        
        // Create event broadcast channel
        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        
        // Create channel for call establishment notifications
        let (call_established_tx, call_established_rx) = tokio::sync::mpsc::unbounded_channel();
        

        
        // Build session coordinator with media preferences
        // The media preferences will be used by session-core's SDP negotiator
        // to generate offers/answers based on the configured codecs
        let session_media_config = SessionMediaConfig {
            preferred_codecs: config.media.preferred_codecs.clone(),
            port_range: Some((config.media.rtp_port_start, config.media.rtp_port_end)),
            quality_monitoring: true,
            dtmf_support: config.media.dtmf_enabled,
            echo_cancellation: config.media.echo_cancellation,
            noise_suppression: config.media.noise_suppression,
            auto_gain_control: config.media.auto_gain_control,
            max_bandwidth_kbps: config.media.max_bandwidth_kbps,
            preferred_ptime: config.media.preferred_ptime,
            custom_sdp_attributes: config.media.custom_sdp_attributes.clone(),
            music_on_hold_path: config.media.music_on_hold_path.clone(),
            ..Default::default()
        };
        
        // Note: If media port is 0, it signals automatic allocation
        // The actual port will be allocated by session-core when creating media sessions
        // This is the proper layered approach that respects the architecture

        // Create the call handler
        let call_handler = Arc::new(ClientCallHandler::new(
            call_mapping.clone(),
            session_mapping.clone(),
            call_info.clone(),
            incoming_calls.clone(),
        ).with_event_tx(event_tx.clone())
        .with_call_established_tx(call_established_tx.clone()));
        
        // Create session manager using session-core builder with media preferences
        let mut session_builder = SessionManagerBuilder::new()
            .with_local_address(&format!("sip:client@{}", config.local_sip_addr.ip()))
            .with_sip_port(config.local_sip_addr.port())
            .with_local_bind_addr(config.local_sip_addr)  // Add this line to propagate bind address
            .with_media_ports(config.media.rtp_port_start, config.media.rtp_port_end)
            .with_media_config(session_media_config)  // Pass media preferences to session-core
            .with_handler(call_handler.clone() as Arc<dyn CallHandler>)
            .enable_sip_client();  // Enable SIP client features for REGISTER support

        // Apply WebSocket transport if configured
        if let Some(secure) = config.websocket {
            if secure {
                session_builder = session_builder.with_secure_websocket();
            } else {
                session_builder = session_builder.with_websocket();
            }
        }

        let coordinator = session_builder
            .build()
            .await
            .map_err(|e| ClientError::InternalError {
                message: format!("Failed to create session coordinator: {}", e)
            })?;
        
        // Now set the session event channel on the call handler
        // This allows client-core to send cleanup confirmations back to session-core
        let session_event_tx = coordinator.event_tx().await
            .map_err(|e| ClientError::InternalError {
                message: format!("Failed to get session event sender: {}", e)
            })?;
        call_handler.set_session_event_tx(session_event_tx).await;
        
        // Subscribe to session events to handle transfer events
        let session_event_subscriber = coordinator.event_processor.subscribe().await
            .map_err(|e| ClientError::InternalError {
                message: format!("Failed to subscribe to session events: {}", e)
            })?;

            
        let mut stats = ClientStats {
            is_running: false,
            local_sip_addr: config.local_sip_addr,
            local_media_addr: config.local_media_addr,
            total_calls: 0,
            connected_calls: 0,
            total_registrations: 0,
            active_registrations: 0,
                };
        

        
        let audio_setup_calls = Arc::new(DashMap::new());
        
        // Clone for the client event task (it handles session events and converts them to client events)
        let client_event_tx_for_session = event_tx.clone();
        let session_mapping_for_session = session_mapping.clone();
        
        // Create the client manager
        let client = Arc::new(Self {
            coordinator,
            local_sip_addr: config.local_sip_addr,
            media_config: config.media.clone(),
            is_running: Arc::new(RwLock::new(false)),
            stats: Arc::new(Mutex::new(stats)),
            registrations: Arc::new(RwLock::new(HashMap::new())),
            session_mapping,
            call_info,
            call_handler,
            event_tx,
            audio_setup_calls,
            audio_setup_task: Arc::new(Mutex::new(None)),
            max_concurrent_calls: config.max_concurrent_calls,
        });
        
        // Spawn task to handle call establishment notifications
        let client_clone = client.clone();
        let mut call_established_rx = call_established_rx;
        let audio_setup_task = tokio::spawn(async move {
            while let Some(call_id) = call_established_rx.recv().await {
                // Set up audio for the established call
                if let Err(e) = client_clone.setup_call_audio(&call_id).await {
                    tracing::warn!("Failed to set up audio for established call {}: {}", call_id, e);
                }
            }
        });
        
        // Store the task handle
        *client.audio_setup_task.lock().await = Some(audio_setup_task);
        
        // Spawn task to process session events and convert them to client events
        let mut session_event_sub = session_event_subscriber;
        tokio::spawn(async move {
            use rvoip_session_core::manager::events::{SessionEvent, SessionTransferStatus};
            use crate::events::{ClientEvent, TransferStatus, EventPriority};
            
            loop {
                match session_event_sub.receive().await {
                    Ok(SessionEvent::IncomingTransferRequest { session_id, target_uri, referred_by, replaces }) => {
                        // Find the call ID for this session
                        let call_id = session_mapping_for_session.iter()
                            .find(|entry| entry.value() == &session_id)
                            .map(|entry| *entry.key());
                        
                        if let Some(call_id) = call_id {
                            let event = ClientEvent::IncomingTransferRequest {
                                call_id,
                                target_uri,
                                referred_by,
                                is_attended: replaces.is_some(),
                                priority: EventPriority::High,
                            };
                            
                            if let Err(e) = client_event_tx_for_session.send(event) {
                                tracing::warn!("Failed to send IncomingTransferRequest event: {}", e);
                            }
                        }
                    }
                    Ok(SessionEvent::TransferProgress { session_id, status }) => {
                        // Find the call ID for this session
                        let call_id = session_mapping_for_session.iter()
                            .find(|entry| entry.value() == &session_id)
                            .map(|entry| *entry.key());
                        
                        if let Some(call_id) = call_id {
                            let transfer_status = match status {
                                SessionTransferStatus::Trying => TransferStatus::Accepted,
                                SessionTransferStatus::Ringing => TransferStatus::Ringing,
                                SessionTransferStatus::Success => TransferStatus::Completed,
                                SessionTransferStatus::Failed(reason) => TransferStatus::Failed(reason),
                            };
                            
                            let event = ClientEvent::TransferProgress {
                                call_id,
                                status: transfer_status,
                                priority: EventPriority::Normal,
                            };
                            
                            if let Err(e) = client_event_tx_for_session.send(event) {
                                tracing::warn!("Failed to send TransferProgress event: {}", e);
                            }
                        }
                    }
                    Ok(_) => {
                        // Ignore other session events
                    }
                    Err(e) => {
                        tracing::debug!("Session event receiver ended: {}", e);
                        break;
                    }
                }
            }
        });
        
        Ok(client)
    }
    
    /// Set the event handler for client events
    /// 
    /// This method registers an application-provided event handler that will receive
    /// notifications for all client events including registration changes, call status
    /// updates, and media quality notifications. The handler is called asynchronously
    /// and should not block for extended periods.
    /// 
    /// # Arguments
    /// 
    /// * `handler` - An implementation of the `ClientEventHandler` trait wrapped in an `Arc`
    ///               for thread-safe sharing across the event system
    /// 
    /// # Examples
    /// 
    /// ## Basic Event Handler
    /// 
    /// ```rust
    /// use rvoip_client_core::{
    ///     ClientManager, ClientConfig, ClientEventHandler,
    ///     events::{CallStatusInfo, RegistrationStatusInfo, MediaEventInfo, IncomingCallInfo, CallAction}
    /// };
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct MyEventHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for MyEventHandler {
    ///     async fn on_incoming_call(&self, _info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     
    ///     async fn on_call_state_changed(&self, info: CallStatusInfo) {
    ///         println!("📞 Call {} changed to {:?}", info.call_id, info.new_state);
    ///     }
    ///     
    ///     async fn on_registration_status_changed(&self, info: RegistrationStatusInfo) {
    ///         println!("📋 Registration {} changed to {:?}", info.user_uri, info.status);
    ///     }
    ///     
    ///     async fn on_media_event(&self, info: MediaEventInfo) {
    ///         println!("🎵 Media event for call {}: {:?}", info.call_id, info.event_type);
    ///     }
    /// }
    /// 
    /// async fn setup_event_handler() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5063".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Register our event handler
    ///     let handler = Arc::new(MyEventHandler);
    ///     client.set_event_handler(handler).await;
    ///     
    ///     client.start().await?;
    ///     println!("✅ Event handler registered and client started");
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Stateful Event Handler
    /// 
    /// ```rust
    /// use rvoip_client_core::{
    ///     ClientManager, ClientConfig, ClientEventHandler,
    ///     events::{CallStatusInfo, RegistrationStatusInfo, MediaEventInfo, IncomingCallInfo, CallAction}
    /// };
    /// use async_trait::async_trait;
    /// use std::sync::{Arc, Mutex};
    /// use std::collections::HashMap;
    /// 
    /// struct StatefulEventHandler {
    ///     call_states: Mutex<HashMap<String, String>>,
    ///     event_count: Mutex<u64>,
    /// }
    /// 
    /// impl StatefulEventHandler {
    ///     fn new() -> Self {
    ///         Self {
    ///             call_states: Mutex::new(HashMap::new()),
    ///             event_count: Mutex::new(0),
    ///         }
    ///     }
    /// }
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for StatefulEventHandler {
    ///     async fn on_incoming_call(&self, _info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     
    ///     async fn on_call_state_changed(&self, info: CallStatusInfo) {
    ///         // Update state tracking
    ///         let mut states = self.call_states.lock().unwrap();
    ///         states.insert(info.call_id.to_string(), format!("{:?}", info.new_state));
    ///         
    ///         let mut count = self.event_count.lock().unwrap();
    ///         *count += 1;
    ///         
    ///         println!("📞 Call event #{}: {} -> {:?}", *count, info.call_id, info.new_state);
    ///     }
    ///     
    ///     async fn on_registration_status_changed(&self, info: RegistrationStatusInfo) {
    ///         println!("📋 Registration: {} -> {:?}", info.user_uri, info.status);
    ///     }
    ///     
    ///     async fn on_media_event(&self, info: MediaEventInfo) {
    ///         println!("🎵 Media: Call {} -> {:?}", info.call_id, info.event_type);
    ///     }
    /// }
    /// 
    /// async fn stateful_handler() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5064".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Create stateful handler
    ///     let handler = Arc::new(StatefulEventHandler::new());
    ///     client.set_event_handler(handler.clone()).await;
    ///     
    ///     client.start().await?;
    ///     
    ///     // Handler is now tracking events
    ///     println!("✅ Stateful event handler active");
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Logging Event Handler
    /// 
    /// ```rust
    /// use rvoip_client_core::{
    ///     ClientManager, ClientConfig, ClientEventHandler,
    ///     events::{CallStatusInfo, RegistrationStatusInfo, MediaEventInfo, IncomingCallInfo, CallAction}
    /// };
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// use chrono::Utc;
    /// 
    /// struct LoggingEventHandler {
    ///     component_name: String,
    /// }
    /// 
    /// impl LoggingEventHandler {
    ///     fn new(name: &str) -> Self {
    ///         Self {
    ///             component_name: name.to_string(),
    ///         }
    ///     }
    /// }
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for LoggingEventHandler {
    ///     async fn on_incoming_call(&self, _info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     
    ///     async fn on_call_state_changed(&self, info: CallStatusInfo) {
    ///         tracing::info!(
    ///             component = %self.component_name,
    ///             call_id = %info.call_id,
    ///             previous_state = ?info.previous_state,
    ///             new_state = ?info.new_state,
    ///             timestamp = %info.timestamp,
    ///             "Call status changed"
    ///         );
    ///     }
    ///     
    ///     async fn on_registration_status_changed(&self, info: RegistrationStatusInfo) {
    ///         tracing::info!(
    ///             component = %self.component_name,
    ///             user_uri = %info.user_uri,
    ///             status = ?info.status,
    ///             server = %info.server_uri,
    ///             "Registration status changed"
    ///         );
    ///     }
    ///     
    ///     async fn on_media_event(&self, info: MediaEventInfo) {
    ///         tracing::debug!(
    ///             component = %self.component_name,
    ///             call_id = %info.call_id,
    ///             event_type = ?info.event_type,
    ///             "Media event occurred"
    ///         );
    ///     }
    /// }
    /// 
    /// async fn logging_handler() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5065".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Create logging handler
    ///     let handler = Arc::new(LoggingEventHandler::new("MyVoIPApp"));
    ///     client.set_event_handler(handler).await;
    ///     
    ///     client.start().await?;
    ///     println!("✅ Logging event handler registered");
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Event Handler Replacement
    /// 
    /// ```rust
    /// use rvoip_client_core::{
    ///     ClientManager, ClientConfig, ClientEventHandler,
    ///     events::{CallStatusInfo, RegistrationStatusInfo, MediaEventInfo, IncomingCallInfo, CallAction}
    /// };
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct Handler1;
    /// struct Handler2;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for Handler1 {
    ///     async fn on_incoming_call(&self, _info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, info: CallStatusInfo) {
    ///         println!("Handler1: Call {} -> {:?}", info.call_id, info.new_state);
    ///     }
    ///     async fn on_registration_status_changed(&self, _info: RegistrationStatusInfo) {}
    ///     async fn on_media_event(&self, _info: MediaEventInfo) {}
    /// }
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for Handler2 {
    ///     async fn on_incoming_call(&self, _info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, info: CallStatusInfo) {
    ///         println!("Handler2: Call {} -> {:?}", info.call_id, info.new_state);
    ///     }
    ///     async fn on_registration_status_changed(&self, _info: RegistrationStatusInfo) {}
    ///     async fn on_media_event(&self, _info: MediaEventInfo) {}
    /// }
    /// 
    /// async fn handler_replacement() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5066".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Set initial handler
    ///     let handler1 = Arc::new(Handler1);
    ///     client.set_event_handler(handler1).await;
    ///     println!("✅ Handler1 registered");
    ///     
    ///     // Replace with different handler
    ///     let handler2 = Arc::new(Handler2);
    ///     client.set_event_handler(handler2).await;
    ///     println!("✅ Handler2 replaced Handler1");
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Implementation Notes
    /// 
    /// - **Thread Safety**: The handler is stored in an Arc<RwLock> for safe concurrent access
    /// - **Async Execution**: All handler methods are called asynchronously
    /// - **No Blocking**: Handlers should avoid blocking operations to prevent event queue backup
    /// - **Error Handling**: Handler errors are logged but don't affect client operation
    /// - **Replacement**: Setting a new handler replaces the previous one
    /// 
    /// # Best Practices
    /// 
    /// 1. **Keep handlers lightweight** - Avoid heavy computation in event callbacks
    /// 2. **Use async patterns** - Leverage tokio for concurrent event processing
    /// 3. **Handle errors gracefully** - Don't panic in event handlers
    /// 4. **Consider batching** - For high-frequency events, consider batching updates
    /// 5. **State management** - Use appropriate synchronization for handler state
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        self.call_handler.set_event_handler(handler).await;
    }
    
    /// Start the client manager
    /// 
    /// This method starts the client manager, initializing the underlying SIP transport,
    /// binding to network addresses, and beginning event processing. The client must be
    /// started before it can handle registrations, calls, or other SIP operations.
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the client started successfully.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InternalError` - If the session coordinator fails to start
    ///   (e.g., port already in use, network unavailable)
    /// 
    /// # Examples
    /// 
    /// ## Basic Start/Stop Cycle
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn start_stop_cycle() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5067".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Initially not running
    ///     assert!(!client.is_running().await);
    ///     
    ///     // Start the client
    ///     client.start().await?;
    ///     assert!(client.is_running().await);
    ///     println!("✅ Client started successfully");
    ///     
    ///     // Stop the client
    ///     client.stop().await?;
    ///     assert!(!client.is_running().await);
    ///     println!("✅ Client stopped successfully");
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Error Handling on Start
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, ClientError};
    /// 
    /// async fn handle_start_errors() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5068".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     match client.start().await {
    ///         Ok(()) => {
    ///             println!("✅ Client started successfully");
    ///             client.stop().await?;
    ///         }
    ///         Err(ClientError::InternalError { message }) => {
    ///             println!("❌ Failed to start client: {}", message);
    ///             // Handle the error (retry with different port, etc.)
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Unexpected error: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Multiple Start Attempts
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn multiple_start_safe() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5069".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Start the client
    ///     client.start().await?;
    ///     println!("✅ First start successful");
    ///     
    ///     // Multiple starts should be safe (idempotent)
    ///     client.start().await?;
    ///     println!("✅ Second start (should be no-op)");
    ///     
    ///     assert!(client.is_running().await);
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn start(&self) -> ClientResult<()> {
        // Start the session coordinator using SessionControl trait
        SessionControl::start(&self.coordinator)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to start session coordinator: {}", e) 
            })?;
            
        *self.is_running.write().await = true;
        
        // Update stats with actual bound addresses
        let actual_addr = SessionControl::get_bound_address(&self.coordinator);
        let mut stats = self.stats.lock().await;
        stats.is_running = true;
        stats.local_sip_addr = actual_addr;
        
        tracing::info!("ClientManager started on {}", actual_addr);
        Ok(())
    }
    
    /// Stop the client manager
    /// 
    /// This method gracefully shuts down the client manager, terminating all active
    /// calls, cleaning up network resources, and stopping event processing. Any active
    /// registrations will be automatically unregistered.
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the client stopped successfully.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InternalError` - If the session coordinator fails to stop cleanly
    /// 
    /// # Examples
    /// 
    /// ## Graceful Shutdown
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn graceful_shutdown() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5070".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Do some work...
    ///     println!("Client running...");
    ///     
    ///     // Graceful shutdown
    ///     client.stop().await?;
    ///     assert!(!client.is_running().await);
    ///     println!("✅ Client stopped gracefully");
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Error Handling on Stop
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, ClientError};
    /// 
    /// async fn handle_stop_errors() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5071".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     match client.stop().await {
    ///         Ok(()) => {
    ///             println!("✅ Client stopped successfully");
    ///         }
    ///         Err(ClientError::InternalError { message }) => {
    ///             println!("⚠️  Stop had issues: {}", message);
    ///             // Resources may still be partially cleaned up
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Unexpected error during stop: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Multiple Stop Attempts
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn multiple_stop_safe() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5072".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Stop the client
    ///     client.stop().await?;
    ///     println!("✅ First stop successful");
    ///     
    ///     // Multiple stops should be safe (idempotent)
    ///     client.stop().await?;
    ///     println!("✅ Second stop (should be no-op)");
    ///     
    ///     assert!(!client.is_running().await);
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub async fn stop(&self) -> ClientResult<()> {
        // Cancel the audio setup task if it's running
        if let Some(task) = self.audio_setup_task.lock().await.take() {
            task.abort();
        }
        
        SessionControl::stop(&self.coordinator)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to stop session coordinator: {}", e) 
            })?;
            
        *self.is_running.write().await = false;
        
        let mut stats = self.stats.lock().await;
        stats.is_running = false;
        
        tracing::info!("ClientManager stopped");
        Ok(())
    }
    

    
    /// Subscribe to client events
    /// 
    /// Creates a new receiver for the client event broadcast channel. Multiple
    /// subscribers can listen to the same events simultaneously.
    /// 
    /// # Returns
    /// 
    /// Returns a `broadcast::Receiver<ClientEvent>` for receiving real-time events.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, ClientEvent};
    /// use tokio::time::{timeout, Duration};
    /// 
    /// async fn event_subscription() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5076".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     let mut events = client.subscribe_events();
    ///     
    ///     client.start().await?;
    ///     
    ///     // Listen for events (with timeout for doc test)
    ///     if let Ok(Ok(event)) = timeout(Duration::from_millis(10), events.recv()).await {
    ///         match event {
    ///             ClientEvent::CallStateChanged { info, .. } => {
    ///                 println!("Call event: {:?}", info.new_state);
    ///             }
    ///             ClientEvent::RegistrationStatusChanged { info, .. } => {
    ///                 println!("Registration event: {:?}", info.status);
    ///             }
    ///             ClientEvent::MediaEvent { info, .. } => {
    ///                 println!("Media event for call: {}", info.call_id);
    ///             }
    ///             ClientEvent::IncomingCall { .. } | 
    ///             ClientEvent::ClientError { .. } | 
    ///             ClientEvent::NetworkEvent { .. } => {
    ///                 // Handle other events as needed
    ///             }
    ///         }
    ///     }
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<ClientEvent> {
        self.event_tx.subscribe()
    }
    
    /// Check if the client is running
    /// 
    /// Returns the current running state of the client manager. A client must be
    /// started before it can handle SIP operations.
    /// 
    /// # Returns
    /// 
    /// Returns `true` if the client is currently running, `false` otherwise.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn check_running_state() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5077".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Initially not running
    ///     assert!(!client.is_running().await);
    ///     
    ///     // Start and check
    ///     client.start().await?;
    ///     assert!(client.is_running().await);
    ///     
    ///     // Stop and check
    ///     client.stop().await?;
    ///     assert!(!client.is_running().await);
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
    
    /// Get the media configuration
    /// 
    /// Returns a reference to the current media configuration being used by the client.
    /// This includes codec preferences, quality settings, and network parameters.
    /// 
    /// # Returns
    /// 
    /// Returns a reference to the `MediaConfig` used during client initialization.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, MediaConfig, MediaPreset};
    /// 
    /// async fn check_media_config() -> Result<(), Box<dyn std::error::Error>> {
    ///     use rvoip_client_core::client::config::MediaPreset;
    ///     let mut media_config = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
    ///     media_config.echo_cancellation = true;
    ///     media_config.max_bandwidth_kbps = Some(128);
    ///     
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5078".parse()?)
    ///         .with_media(media_config);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     
    ///     // Check applied configuration
    ///     let applied_config = client.get_media_config();
    ///     assert!(applied_config.echo_cancellation);
    ///     assert_eq!(applied_config.max_bandwidth_kbps, Some(128));
    ///     
    ///     println!("Echo cancellation: {}", applied_config.echo_cancellation);
    ///     println!("Noise suppression: {}", applied_config.noise_suppression);
    ///     println!("RTP port range: {}-{}", 
    ///         applied_config.rtp_port_start, applied_config.rtp_port_end);
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub fn get_media_config(&self) -> &MediaConfig {
        &self.media_config
    }
    
    // ===== PRIORITY 3.2: CALL CONTROL OPERATIONS =====
    // Call control operations have been moved to controls.rs
    
    // ===== PRIORITY 4.1: ENHANCED MEDIA INTEGRATION =====
    // Media operations have been moved to media.rs
    
    /// Set up audio frame subscription for a call
    /// 
    /// This internal method is called when a call becomes established to automatically
    /// set up audio frame subscription, enabling audio to flow for the call.
    pub(crate) async fn setup_call_audio(&self, call_id: &CallId) -> ClientResult<()> {
        // Get the session ID for this call
        if let Some(session_id_entry) = self.session_mapping.get(call_id) {
            let session_id = session_id_entry.clone();
            
            // Subscribe to audio frames from this session
            match MediaControl::subscribe_to_audio_frames(&self.coordinator, &session_id).await {
                Ok(audio_subscriber) => {
                    // Mark that audio is set up for this call
                    self.audio_setup_calls.insert(*call_id, true);
                    
                    // Audio subscriber is now available for the application to use
                    // The application (e.g., sip-client) can integrate with audio-core
                    // to connect this subscriber to speakers and set up microphone capture
                    tracing::info!("Audio frame subscription ready for call {}", call_id);
                    tracing::info!("To enable audio, integrate with audio-core in your application");
                    
                    // For now, just drop the subscriber as we can't use audio-core directly
                    // due to circular dependency issues
                    drop(audio_subscriber);
                    
                    // TODO: In the future, this is where we would connect to audio-core
                    // to route audio frames to the appropriate audio device.
                    // For now, the audio frames are available via the subscriber.
                    
                    tracing::info!("Set up audio frame subscription for call {}", call_id);
                    Ok(())
                }
                Err(e) => {
                    // Log the error but don't fail the call - audio might still work
                    // through other means or this might be a non-audio call
                    tracing::warn!("Failed to set up audio frame subscription for call {}: {}", call_id, e);
                    Err(ClientError::MediaError { 
                        details: format!("Failed to subscribe to audio frames: {}", e) 
                    })
                }
            }
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
}

impl Drop for ClientManager {
    fn drop(&mut self) {
        // Check if still running using try_read to avoid blocking
        if let Ok(is_running) = self.is_running.try_read() {
            if *is_running {
                tracing::warn!("ClientManager dropped while still running! Call stop() before dropping to ensure proper cleanup.");
            }
        }
    }
}


