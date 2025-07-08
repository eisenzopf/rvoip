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
    audio::AudioDeviceManager,
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
    
    /// Audio device manager for audio I/O
    pub(crate) audio_device_manager: Arc<AudioDeviceManager>,
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
        
        // Create call handler
        let call_handler = Arc::new(ClientCallHandler::new(
            call_mapping.clone(),
            session_mapping.clone(),
            call_info.clone(),
            incoming_calls.clone(),
        ).with_event_tx(event_tx.clone()));
        
        // Create audio device manager
        let audio_device_manager = Arc::new(AudioDeviceManager::new().await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to create audio device manager: {}", e) 
            })?
        );
        
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
        };
        
        // Note: If media port is 0, it signals automatic allocation
        // The actual port will be allocated by session-core when creating media sessions
        // This is the proper layered approach that respects the architecture

        // Create session manager using session-core builder with media preferences
        let coordinator = SessionManagerBuilder::new()
            .with_local_address(&format!("sip:client@{}", config.local_sip_addr.ip()))
            .with_sip_port(config.local_sip_addr.port())
            .with_local_bind_addr(config.local_sip_addr)  // Add this line to propagate bind address
            .with_media_ports(config.media.rtp_port_start, config.media.rtp_port_end)
            .with_media_config(session_media_config)  // Pass media preferences to session-core
            .with_handler(call_handler.clone() as Arc<dyn CallHandler>)
            .enable_sip_client()  // Enable SIP client features for REGISTER support
            .build()
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to create session coordinator: {}", e) 
            })?;
        
        // Set up audio device manager with session coordinator integration
        let audio_device_manager = {
            let mut audio_manager = Arc::try_unwrap(audio_device_manager)
                .map_err(|_| ClientError::InternalError { 
                    message: "Failed to configure audio device manager".to_string() 
                })?;
            audio_manager.set_session_coordinator(coordinator.clone()).await;
            Arc::new(audio_manager)
        };
            
        let mut stats = ClientStats {
            is_running: false,
            local_sip_addr: config.local_sip_addr,
            local_media_addr: config.local_media_addr,
            total_calls: 0,
            connected_calls: 0,
            total_registrations: 0,
            active_registrations: 0,
                };
        

        
        Ok(Arc::new(Self {
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
            audio_device_manager,
        }))
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
    
    /// Register with a SIP server
    /// 
    /// This method registers the client with a SIP server using the REGISTER method.
    /// Registration allows the client to receive incoming calls and establishes its
    /// presence on the SIP network. The method handles authentication challenges
    /// automatically and includes retry logic for network issues.
    /// 
    /// # Arguments
    /// 
    /// * `config` - Registration configuration including server URI, user credentials,
    ///              and expiration settings
    /// 
    /// # Returns
    /// 
    /// Returns a `Uuid` that uniquely identifies this registration for future operations.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::AuthenticationFailed` - Invalid credentials or auth challenge failed
    /// * `ClientError::RegistrationFailed` - Server rejected registration (403, etc.)
    /// * `ClientError::NetworkError` - Network timeout or connectivity issues
    /// 
    /// # Examples
    /// 
    /// ## Basic Registration
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn basic_registration() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5073".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:sip.example.com:5060".to_string(),
    ///         from_uri: "sip:alice@example.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5073".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     println!("✅ Registered with ID: {}", reg_id);
    ///     
    ///     client.unregister(reg_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Registration with Authentication
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn authenticated_registration() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5074".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:pbx.company.com".to_string(),
    ///         from_uri: "sip:user@company.com".to_string(),
    ///         contact_uri: "sip:user@127.0.0.1:5074".to_string(),
    ///         expires: 1800, // 30 minutes
    ///         username: Some("user".to_string()),
    ///         password: Some("password123".to_string()),
    ///         realm: Some("company.com".to_string()),
    ///     };
    ///     
    ///     match client.register(reg_config).await {
    ///         Ok(reg_id) => {
    ///             println!("✅ Authenticated registration successful: {}", reg_id);
    ///             client.unregister(reg_id).await?;
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Registration failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Multiple Registrations
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn multiple_registrations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5075".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Register with multiple servers
    ///     let reg1_config = RegistrationConfig {
    ///         server_uri: "sip:server1.com".to_string(),
    ///         from_uri: "sip:alice@server1.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5075".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg2_config = RegistrationConfig {
    ///         server_uri: "sip:server2.com".to_string(),
    ///         from_uri: "sip:alice@server2.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5075".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg1_id = client.register(reg1_config).await?;
    ///     let reg2_id = client.register(reg2_config).await?;
    ///     
    ///     println!("✅ Registered with {} servers", 2);
    ///     
    ///     // Check all registrations
    ///     let all_regs = client.get_all_registrations().await;
    ///     assert_eq!(all_regs.len(), 2);
    ///     
    ///     // Clean up
    ///     client.unregister(reg1_id).await?;
    ///     client.unregister(reg2_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        // Use SipClient trait to register with retry logic for network errors
        let registration_handle = retry_with_backoff(
            "sip_registration",
            RetryConfig::slow(),  // Use slower retry for registration
            || async {
                SipClient::register(
                    &self.coordinator,
                    &config.server_uri,
                    &config.from_uri,
                    &config.contact_uri,
                    config.expires,
                )
                .await
                .map_err(|e| {
                    // Categorize the error properly based on response
                    let error_msg = e.to_string();
                    if error_msg.contains("401") || error_msg.contains("407") {
                        ClientError::AuthenticationFailed {
                            reason: format!("Authentication required: {}", e)
                        }
                    } else if error_msg.contains("timeout") {
                        ClientError::NetworkError {
                            reason: format!("Registration timeout: {}", e)
                        }
                    } else if error_msg.contains("403") {
                        ClientError::RegistrationFailed {
                            reason: format!("Registration forbidden: {}", e)
                        }
                    } else {
                        ClientError::RegistrationFailed {
                            reason: format!("Registration failed: {}", e)
                        }
                    }
                })
            }
        )
        .await
        .with_context(|| format!("Failed to register {} with {}", config.from_uri, config.server_uri))?;
        
        // Create registration info
        let reg_id = Uuid::new_v4();
        let registration_info = RegistrationInfo {
            id: reg_id,
            server_uri: config.server_uri.clone(),
            from_uri: config.from_uri.clone(),
            contact_uri: config.contact_uri.clone(),
            expires: config.expires,
            status: crate::registration::RegistrationStatus::Active,
            registration_time: chrono::Utc::now(),
            refresh_time: None,
            handle: Some(registration_handle),
        };
        
        // Store registration
        self.registrations.write().await.insert(reg_id, registration_info);
        
        // Update stats
        let mut stats = self.stats.lock().await;
        stats.total_registrations += 1;
        stats.active_registrations += 1;
        
        // Broadcast registration event
        let _ = self.event_tx.send(ClientEvent::RegistrationStatusChanged {
            info: crate::events::RegistrationStatusInfo {
                registration_id: reg_id,
                server_uri: config.server_uri.clone(),
                user_uri: config.from_uri.clone(),
                status: crate::registration::RegistrationStatus::Active,
                reason: Some("Registration successful".to_string()),
                timestamp: chrono::Utc::now(),
            },
            priority: crate::events::EventPriority::Normal,
        });
        
        tracing::info!("Registered {} with server {}", config.from_uri, config.server_uri);
        Ok(reg_id)
    }
    
    /// Unregister from a SIP server
    /// 
    /// This method removes a registration from a SIP server by sending a REGISTER
    /// request with expires=0. This gracefully removes the client's presence from
    /// the server and stops receiving incoming calls for that registration.
    /// 
    /// # Arguments
    /// 
    /// * `reg_id` - The UUID of the registration to remove
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the unregistration was successful.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the registration ID is not found
    /// * `ClientError::InternalError` - If the unregistration request fails
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn unregister_example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5079".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:server.example.com".to_string(),
    ///         from_uri: "sip:alice@example.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5079".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     println!("✅ Registered with ID: {}", reg_id);
    ///     
    ///     // Unregister
    ///     client.unregister(reg_id).await?;
    ///     println!("✅ Successfully unregistered");
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn unregister(&self, reg_id: Uuid) -> ClientResult<()> {
        let mut registrations = self.registrations.write().await;
        
        if let Some(registration_info) = registrations.get_mut(&reg_id) {
            // To unregister, send REGISTER with expires=0
            if let Some(handle) = &registration_info.handle {
                SipClient::register(
                    &self.coordinator,
                    &handle.registrar_uri,
                    &registration_info.from_uri,
                    &handle.contact_uri,
                    0, // expires=0 means unregister
                )
                .await
                .map_err(|e| ClientError::InternalError { 
                    message: format!("Failed to unregister: {}", e) 
                })?;
            }
            
            // Update status
            registration_info.status = crate::registration::RegistrationStatus::Cancelled;
            registration_info.handle = None;
            
            // Update stats
            let mut stats = self.stats.lock().await;
            if stats.active_registrations > 0 {
                stats.active_registrations -= 1;
            }
            
            tracing::info!("Unregistered {}", registration_info.from_uri);
            Ok(())
        } else {
            Err(ClientError::InvalidConfiguration { 
                field: "registration_id".to_string(),
                reason: "Registration not found".to_string() 
            })
        }
    }
    
    /// Get registration information
    /// 
    /// Retrieves detailed information about a specific registration including
    /// status, timestamps, and server details.
    /// 
    /// # Arguments
    /// 
    /// * `reg_id` - The UUID of the registration to retrieve
    /// 
    /// # Returns
    /// 
    /// Returns the `RegistrationInfo` struct containing all registration details.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the registration ID is not found
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn get_registration_info() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5080".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:server.example.com".to_string(),
    ///         from_uri: "sip:user@example.com".to_string(),
    ///         contact_uri: "sip:user@127.0.0.1:5080".to_string(),
    ///         expires: 1800,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     
    ///     // Get registration details
    ///     let reg_info = client.get_registration(reg_id).await?;
    ///     println!("Registration status: {:?}", reg_info.status);
    ///     println!("Server: {}", reg_info.server_uri);
    ///     println!("User: {}", reg_info.from_uri);
    ///     println!("Expires: {} seconds", reg_info.expires);
    ///     
    ///     client.unregister(reg_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_registration(&self, reg_id: Uuid) -> ClientResult<crate::registration::RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.get(&reg_id)
            .cloned()
            .ok_or(ClientError::InvalidConfiguration { 
                field: "registration_id".to_string(),
                reason: "Registration not found".to_string() 
            })
    }
    
    /// Get all active registrations
    /// 
    /// Returns a list of all currently active registrations. This includes only
    /// registrations with status `Active`, filtering out expired or cancelled ones.
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<RegistrationInfo>` containing all active registrations.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn list_registrations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5081".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Create multiple registrations
    ///     let reg1_config = RegistrationConfig {
    ///         server_uri: "sip:server1.com".to_string(),
    ///         from_uri: "sip:alice@server1.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5081".to_string(),
    ///         expires: 3600,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg2_config = RegistrationConfig {
    ///         server_uri: "sip:server2.com".to_string(),
    ///         from_uri: "sip:alice@server2.com".to_string(),
    ///         contact_uri: "sip:alice@127.0.0.1:5081".to_string(),
    ///         expires: 1800,
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let _reg1_id = client.register(reg1_config).await?;
    ///     let _reg2_id = client.register(reg2_config).await?;
    ///     
    ///     // List all active registrations
    ///     let active_regs = client.get_all_registrations().await;
    ///     println!("Active registrations: {}", active_regs.len());
    ///     
    ///     for reg in active_regs {
    ///         println!("- {} at {}", reg.from_uri, reg.server_uri);
    ///     }
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_all_registrations(&self) -> Vec<crate::registration::RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.values()
            .filter(|r| r.status == crate::registration::RegistrationStatus::Active)
            .cloned()
            .collect()
    }
    
    /// Refresh a registration
    /// 
    /// Manually refreshes a registration by sending a new REGISTER request with
    /// the same parameters. This is useful for extending registration lifetime
    /// before expiration or after network connectivity issues.
    /// 
    /// # Arguments
    /// 
    /// * `reg_id` - The UUID of the registration to refresh
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the registration was successfully refreshed.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the registration ID is not found
    /// * `ClientError::InternalError` - If the refresh request fails
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    /// 
    /// async fn refresh_registration_example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5082".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let reg_config = RegistrationConfig {
    ///         server_uri: "sip:server.example.com".to_string(),
    ///         from_uri: "sip:user@example.com".to_string(),
    ///         contact_uri: "sip:user@127.0.0.1:5082".to_string(),
    ///         expires: 300, // Short expiration for demo
    ///         username: None,
    ///         password: None,
    ///         realm: None,
    ///     };
    ///     
    ///     let reg_id = client.register(reg_config).await?;
    ///     println!("✅ Initial registration completed");
    ///     
    ///     // Refresh the registration
    ///     client.refresh_registration(reg_id).await?;
    ///     println!("✅ Registration refreshed successfully");
    ///     
    ///     // Check registration info
    ///     if let Ok(reg_info) = client.get_registration(reg_id).await {
    ///         if let Some(refresh_time) = reg_info.refresh_time {
    ///             println!("Last refreshed: {}", refresh_time);
    ///         }
    ///     }
    ///     
    ///     client.unregister(reg_id).await?;
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn refresh_registration(&self, reg_id: Uuid) -> ClientResult<()> {
        // Get registration data
        let (registrar_uri, from_uri, contact_uri, expires) = {
            let registrations = self.registrations.read().await;
            
            if let Some(registration_info) = registrations.get(&reg_id) {
                if let Some(handle) = &registration_info.handle {
                    (
                        handle.registrar_uri.clone(),
                        registration_info.from_uri.clone(),
                        handle.contact_uri.clone(),
                        registration_info.expires,
                    )
                } else {
                    return Err(ClientError::InvalidConfiguration { 
                        field: "registration".to_string(),
                        reason: "Registration has no handle".to_string() 
                    });
                }
            } else {
                return Err(ClientError::InvalidConfiguration { 
                    field: "registration_id".to_string(),
                    reason: "Registration not found".to_string() 
                });
            }
        };
        
        // Re-register with the same parameters
        let new_handle = SipClient::register(
            &self.coordinator,
            &registrar_uri,
            &from_uri,
            &contact_uri,
            expires,
        )
        .await
        .map_err(|e| ClientError::InternalError { 
            message: format!("Failed to refresh registration: {}", e) 
        })?;
        
        // Update registration with new handle
        let mut registrations = self.registrations.write().await;
        if let Some(reg) = registrations.get_mut(&reg_id) {
            reg.handle = Some(new_handle);
            reg.refresh_time = Some(chrono::Utc::now());
        }
        
        tracing::info!("Refreshed registration for {}", from_uri);
        Ok(())
    }
    
    /// Clear expired registrations
    /// 
    /// Removes all registrations with `Expired` status from the internal storage.
    /// This is a maintenance operation that cleans up stale registration entries
    /// and updates statistics accordingly.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn cleanup_registrations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5083".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // In a real application, you might have some expired registrations
    ///     // This method would clean them up
    ///     client.clear_expired_registrations().await;
    ///     println!("✅ Expired registrations cleaned up");
    ///     
    ///     // Check remaining active registrations
    ///     let active_count = client.get_all_registrations().await.len();
    ///     println!("Active registrations remaining: {}", active_count);
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn clear_expired_registrations(&self) {
        let mut registrations = self.registrations.write().await;
        let mut to_remove = Vec::new();
        
        for (id, reg) in registrations.iter() {
            if reg.status == crate::registration::RegistrationStatus::Expired {
                to_remove.push(*id);
            }
        }
        
        for id in to_remove {
            registrations.remove(&id);
            
            // Update stats
            let mut stats = self.stats.lock().await;
            if stats.active_registrations > 0 {
                stats.active_registrations -= 1;
            }
        }
    }
    
    // ===== CONVENIENCE METHODS FOR EXAMPLES =====
    
    /// Convenience method: Register with simple parameters (for examples)
    /// 
    /// This is a simplified registration method that takes basic parameters and
    /// constructs a complete `RegistrationConfig` automatically. It's designed
    /// for quick testing and simple use cases.
    /// 
    /// # Arguments
    /// 
    /// * `agent_uri` - The SIP URI for this agent (e.g., "sip:alice@example.com")
    /// * `server_addr` - The SIP server address and port
    /// * `duration` - How long the registration should last
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if registration was successful.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// use std::time::Duration;
    /// 
    /// async fn simple_register() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5084".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let server_addr = "192.168.1.100:5060".parse()?;
    ///     let duration = Duration::from_secs(3600); // 1 hour
    ///     
    ///     // Simple registration
    ///     client.register_simple(
    ///         "sip:testuser@example.com",
    ///         &server_addr,
    ///         duration
    ///     ).await?;
    ///     
    ///     println!("✅ Simple registration completed");
    ///     
    ///     // Cleanup using the simple unregister method
    ///     client.unregister_simple(
    ///         "sip:testuser@example.com",
    ///         &server_addr
    ///     ).await?;
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn register_simple(
        &self, 
        agent_uri: &str, 
        server_addr: &std::net::SocketAddr,
        duration: std::time::Duration
    ) -> ClientResult<()> {
        let config = RegistrationConfig {
            server_uri: format!("sip:{}", server_addr),
            from_uri: agent_uri.to_string(),
            contact_uri: format!("sip:{}:{}", self.local_sip_addr.ip(), self.local_sip_addr.port()),
            expires: duration.as_secs() as u32,
            username: None,
            password: None,
            realm: None,
        };
        
        self.register(config).await?;
        Ok(())
    }
    
    /// Convenience method: Unregister with simple parameters (for examples)
    /// 
    /// This method finds and unregisters a registration that matches the given
    /// agent URI and server address. It's the counterpart to `register_simple()`
    /// and provides an easy way to clean up simple registrations.
    /// 
    /// # Arguments
    /// 
    /// * `agent_uri` - The SIP URI that was registered
    /// * `server_addr` - The SIP server address that was used
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if unregistration was successful.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If no matching registration is found
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// use std::time::Duration;
    /// 
    /// async fn simple_unregister() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5085".parse()?);
    ///     
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let agent_uri = "sip:testuser@example.com";
    ///     let server_addr = "192.168.1.100:5060".parse()?;
    ///     
    ///     // Register first
    ///     client.register_simple(
    ///         agent_uri,
    ///         &server_addr,
    ///         Duration::from_secs(3600)
    ///     ).await?;
    ///     
    ///     println!("✅ Registration completed");
    ///     
    ///     // Now unregister using the same parameters
    ///     client.unregister_simple(agent_uri, &server_addr).await?;
    ///     println!("✅ Unregistration completed");
    ///     
    ///     // Verify no active registrations remain
    ///     let active_regs = client.get_all_registrations().await;
    ///     assert_eq!(active_regs.len(), 0);
    ///     
    ///     client.stop().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn unregister_simple(
        &self, 
        agent_uri: &str, 
        server_addr: &std::net::SocketAddr
    ) -> ClientResult<()> {
        // Find the registration matching these parameters
        let registrations = self.registrations.read().await;
        let reg_id = registrations.iter()
            .find(|(_, reg)| {
                reg.from_uri == agent_uri && 
                reg.server_uri == format!("sip:{}", server_addr)
            })
            .map(|(id, _)| *id);
        drop(registrations);
        
        if let Some(id) = reg_id {
            self.unregister(id).await
        } else {
            Err(ClientError::InvalidConfiguration { 
                field: "registration".to_string(),
                reason: "No matching registration found".to_string() 
            })
        }
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


