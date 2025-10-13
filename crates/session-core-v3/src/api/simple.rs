//! Truly Simple API - Callback-based SIP peer
//!
//! This is the simplest possible API - callback-based with automatic event handling.
//! Register callbacks for events, and the library handles everything in the background.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::task::JoinHandle;
use tracing::{warn, debug, error};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

// Re-export types that users of SimplePeer will need
pub use crate::api::unified::Config;
pub use crate::api::events::{Event, CallHandle, CallId};

/// Async event handler type
type AsyncEventHandler = Arc<dyn Fn(Event, Arc<SimplePeerController>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Event handlers for different event types
struct EventHandlers {
    on_incoming_call: Option<AsyncEventHandler>,
    on_call_answered: Option<AsyncEventHandler>,
    on_refer_received: Option<AsyncEventHandler>,
    on_call_ended: Option<AsyncEventHandler>,
}

impl Default for EventHandlers {
    fn default() -> Self {
        Self {
            on_incoming_call: None,
            on_call_answered: None,
            on_refer_received: None,
            on_call_ended: None,
        }
    }
}

/// Controller interface for handlers to perform call operations
pub struct SimplePeerController {
    coordinator: Arc<UnifiedCoordinator>,
}

impl SimplePeerController {
    fn new(coordinator: Arc<UnifiedCoordinator>) -> Self {
        Self { coordinator }
    }
    
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<CallHandle> {
        // Use a dummy local URI - in practice this should come from config
        let local_uri = "sip:user@127.0.0.1:5060"; // TODO: Get from SimplePeer
        let call_id = self.coordinator.make_call(local_uri, target).await?;
        let (call_handle, _audio_rx, _audio_tx) = CallHandle::new(call_id);
        Ok(call_handle)
    }
    
    /// Accept an incoming call
    pub async fn accept(&self, call_id: &CallId) -> Result<CallHandle> {
        self.coordinator.accept_call(call_id).await?;
        let (call_handle, _audio_rx, _audio_tx) = CallHandle::new(call_id.clone());
        Ok(call_handle)
    }
    
    /// Hang up a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    /// Send a REFER request for call transfer
    pub async fn send_refer(&self, call_id: &CallId, target: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, target).await
    }
    
    /// Hold a call
    pub async fn hold(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hold(call_id).await
    }
    
    /// Resume a held call
    pub async fn resume(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.resume(call_id).await
    }
    
    /// Mute a call
    pub async fn mute(&self, _call_id: &CallId) -> Result<()> {
        warn!("Mute functionality not yet implemented");
        Ok(())
    }
    
    /// Unmute a call
    pub async fn unmute(&self, _call_id: &CallId) -> Result<()> {
        warn!("Unmute functionality not yet implemented");
        Ok(())
    }
    
    /// Send DTMF tones
    pub async fn send_dtmf(&self, call_id: &CallId, tones: &str) -> Result<()> {
        // UnifiedCoordinator expects a single char, so send first char
        if let Some(digit) = tones.chars().next() {
            self.coordinator.send_dtmf(call_id, digit).await
        } else {
            Ok(())
        }
    }
}

/// A simple SIP peer that can make and receive calls using callbacks
pub struct SimplePeer {
    /// The coordinator that does all the work
    coordinator: Arc<UnifiedCoordinator>,
    
    /// Event handlers for different event types
    handlers: Arc<RwLock<EventHandlers>>,
    
    /// Background task handle
    background_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    
    /// Flag to control background processing
    is_running: Arc<AtomicBool>,
    
    /// Local SIP URI
    local_uri: String,
    
    /// Controller for handlers
    controller: Arc<SimplePeerController>,
}

impl SimplePeer {
    /// Create a new peer with default configuration
    pub async fn new(name: &str) -> Result<Self> {
        let mut config = Config::default();
        config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        Self::with_config(name, config).await
    }
    
    /// Create a new peer with custom configuration
    pub async fn with_config(name: &str, mut config: Config) -> Result<Self> {
        // Update local_uri if not explicitly set
        if config.local_uri.starts_with("sip:user@") {
            config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        }
        let local_uri = config.local_uri.clone();
        
        // Create event channel for SimplePeer events
        let (event_tx, event_rx) = mpsc::channel(100);
        
        // Create coordinator with event channel
        debug!("🔍 [DEBUG] SimplePeer creating coordinator with event channel...");
        let coordinator = UnifiedCoordinator::with_simple_peer_events(config, event_tx).await?;
        debug!("🔍 [DEBUG] SimplePeer coordinator created successfully");

        let controller = Arc::new(SimplePeerController::new(coordinator.clone()));
        let handlers = Arc::new(RwLock::new(EventHandlers::default()));
        let background_task = Arc::new(Mutex::new(None));
        let is_running = Arc::new(AtomicBool::new(false));

        let mut peer = Self {
            coordinator: coordinator.clone(),
            handlers,
            background_task,
            is_running,
            local_uri,
            controller,
        };

        // Auto-start background processing
        peer.start_background_processing(event_rx).await?;

        Ok(peer)
    }
    
    // ===== Event Handler Registration =====
    
    /// Register a handler for incoming call events
    /// 
    /// # Example
    /// ```rust,no_run
    /// peer.on_incoming_call(|event, controller| async move {
    ///     if let Event::IncomingCall { call_id, from, .. } = event {
    ///         println!("Call from: {}", from);
    ///         controller.accept(&call_id).await.ok();
    ///     }
    /// });
    /// ```
    pub async fn on_incoming_call<F, Fut>(&mut self, handler: F) -> &mut Self
    where 
        F: Fn(Event, Arc<SimplePeerController>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler = Arc::new(move |event: Event, controller: Arc<SimplePeerController>| {
            Box::pin(handler(event, controller)) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        
        {
            let mut handlers = self.handlers.write().await;
            handlers.on_incoming_call = Some(handler);
        }
        self
    }
    
    /// Register a handler for call answered events
    pub async fn on_call_answered<F, Fut>(&mut self, handler: F) -> &mut Self
    where 
        F: Fn(Event, Arc<SimplePeerController>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler = Arc::new(move |event: Event, controller: Arc<SimplePeerController>| {
            Box::pin(handler(event, controller)) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        
        {
            let mut handlers = self.handlers.write().await;
            handlers.on_call_answered = Some(handler);
        }
        self
    }
    
    /// Register a handler for REFER received events
    pub async fn on_refer_received<F, Fut>(&mut self, handler: F) -> &mut Self
    where 
        F: Fn(Event, Arc<SimplePeerController>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler = Arc::new(move |event: Event, controller: Arc<SimplePeerController>| {
            Box::pin(handler(event, controller)) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        
        {
            let mut handlers = self.handlers.write().await;
            handlers.on_refer_received = Some(handler);
        }
        self
    }
    
    /// Register a handler for call ended events
    pub async fn on_call_ended<F, Fut>(&mut self, handler: F) -> &mut Self
    where 
        F: Fn(Event, Arc<SimplePeerController>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler = Arc::new(move |event: Event, controller: Arc<SimplePeerController>| {
            Box::pin(handler(event, controller)) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        
        {
            let mut handlers = self.handlers.write().await;
            handlers.on_call_ended = Some(handler);
        }
        self
    }
    
    // ===== Background Processing =====
    
    /// Start background event processing (called automatically in constructor)
    async fn start_background_processing(&mut self, mut event_rx: mpsc::Receiver<Event>) -> Result<()> {
        debug!("🔍 [DEBUG] Starting SimplePeer background event processing...");
        
        let handlers = self.handlers.clone();
        let controller = self.controller.clone();
        let is_running = self.is_running.clone();
        
        is_running.store(true, Ordering::Relaxed);
        
        let task = tokio::spawn(async move {
            debug!("🔍 [DEBUG] SimplePeer background task started");
            
            while is_running.load(Ordering::Relaxed) {
                match event_rx.recv().await {
                    Some(event) => {
                        debug!("🔍 [DEBUG] SimplePeer background task received event: {:?}", event);
                        Self::dispatch_event(event, &handlers, controller.clone()).await;
                    }
                    None => {
                        debug!("🔍 [DEBUG] SimplePeer event channel closed, stopping background task");
                        break;
                    }
                }
            }
            
            debug!("🔍 [DEBUG] SimplePeer background task ended");
        });
        
        {
            let mut bg_task = self.background_task.lock().await;
            *bg_task = Some(task);
        }
        
        debug!("🔍 [DEBUG] SimplePeer background processing started successfully");
        Ok(())
    }
    
    /// Dispatch an event to the appropriate handler
    async fn dispatch_event(
        event: Event, 
        handlers: &Arc<RwLock<EventHandlers>>, 
        controller: Arc<SimplePeerController>
    ) {
        let handlers_guard = handlers.read().await;
        
        let handler = match &event {
            Event::IncomingCall { .. } => handlers_guard.on_incoming_call.clone(),
            Event::CallAnswered { .. } => handlers_guard.on_call_answered.clone(),
            Event::ReferReceived { .. } => handlers_guard.on_refer_received.clone(),
            Event::CallEnded { .. } => handlers_guard.on_call_ended.clone(),
            _ => None, // Ignore other events for now
        };
        
        drop(handlers_guard); // Release lock before spawning handler
        
        if let Some(handler) = handler {
            let event_clone = event.clone();
            let controller_clone = controller.clone();
            
            tokio::spawn(async move {
                debug!("🔍 [DEBUG] Executing handler for event: {:?}", event_clone);
                handler(event_clone, controller_clone).await;
                debug!("🔍 [DEBUG] Handler completed");
            });
        } else {
            debug!("🔍 [DEBUG] No handler registered for event: {:?}", event);
        }
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call and get a call handle
    /// 
    /// # Returns
    /// CallHandle with audio channels for this specific call
    /// 
    /// # Example
    /// ```rust,no_run
    /// let alice = SimplePeer::new("alice").await?;
    /// let call_handle = alice.call("sip:bob@example.com").await?;
    /// 
    /// // Send audio via the call handle
    /// let samples = vec![100, 200, 300]; // Simple audio data
    /// call_handle.send_audio(samples).await?;
    /// ```
    pub async fn call(&self, target: &str) -> Result<CallHandle> {
        self.controller.call(target).await
    }
    
    /// Accept an incoming call and get a call handle
    /// 
    /// # Returns
    /// CallHandle with audio channels for this specific call
    /// 
    /// # Example
    /// ```rust,no_run
    /// alice.on_incoming_call(|event, controller| async move {
    ///     if let Event::IncomingCall { call_id, from, .. } = event {
    ///         let call_handle = controller.accept(&call_id).await?;
    ///         // Use call_handle for audio
    ///     }
    /// });
    /// ```
    pub async fn accept(&self, call_id: &CallId) -> Result<CallHandle> {
        self.controller.accept(call_id).await
    }
    
    /// Hangup a call
    /// 
    /// # Example
    /// ```rust,no_run
    /// alice.hangup(&call_handle.call_id()).await?;
    /// ```
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.controller.hangup(call_id).await
    }
    
    // ===== Call Control =====
    
    /// Put call on hold
    pub async fn hold(&self, call_id: &CallId) -> Result<()> {
        self.controller.hold(call_id).await
    }
    
    /// Resume from hold
    pub async fn resume(&self, call_id: &CallId) -> Result<()> {
        self.controller.resume(call_id).await
    }
    
    /// Mute microphone for call
    pub async fn mute(&self, call_id: &CallId) -> Result<()> {
        self.controller.mute(call_id).await
    }
    
    /// Unmute microphone for call
    pub async fn unmute(&self, call_id: &CallId) -> Result<()> {
        self.controller.unmute(call_id).await
    }
    
    // ===== Transfer Operations =====
    
    /// Send REFER message to initiate transfer
    /// 
    /// # Example
    /// ```rust,no_run
    /// // Bob sends REFER to Alice
    /// bob.send_refer(&call_id, "sip:charlie@example.com").await?;
    /// // Alice will receive Event::ReferReceived via callback
    /// ```
    pub async fn send_refer(&self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.controller.send_refer(call_id, refer_to).await
    }
    
    // ===== DTMF Operations =====
    
    /// Send DTMF tones
    /// 
    /// # Example
    /// ```rust,no_run
    /// alice.send_dtmf(&call_id, "123#").await?;
    /// ```
    pub async fn send_dtmf(&self, call_id: &CallId, tones: &str) -> Result<()> {
        self.controller.send_dtmf(call_id, tones).await
    }
}

/// Cleanup when SimplePeer is dropped
impl Drop for SimplePeer {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);
        
        // Note: Can't use async in Drop, so we'll abort the task directly
        // The background task will detect is_running = false and exit gracefully
        if let Ok(mut bg_task) = self.background_task.try_lock() {
            if let Some(task) = bg_task.take() {
                task.abort();
            }
        }
    }
}