//! Truly Simple API - Callback-based SIP peer
//!
//! This is the simplest possible API - callback-based with automatic event handling.
//! Register callbacks for events, and the library handles everything in the background.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::pin::Pin;
use std::future::Future;
use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::task::JoinHandle;
use tracing::{warn, debug};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use rvoip_media_core::types::AudioFrame;

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
    local_uri: String,
}

impl SimplePeerController {
    fn new(coordinator: Arc<UnifiedCoordinator>, local_uri: String) -> Self {
        Self { coordinator, local_uri }
    }
    
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<CallId> {
        self.coordinator.make_call(&self.local_uri, target).await
    }
    
    /// Accept an incoming call
    pub async fn accept(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.accept_call(call_id).await
    }
    
    /// Hang up a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    /// Send a REFER request for call transfer
    pub async fn send_refer(&self, call_id: &CallId, target: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, target).await
    }
    
    /// Exchange audio for a duration (send and receive simultaneously)
    pub async fn exchange_audio(
        &self,
        call_id: &CallId,
        duration: std::time::Duration,
        generator: impl Fn(usize) -> Vec<i16>,
    ) -> Result<(Vec<i16>, Vec<i16>)> {
        let mut sent_samples = Vec::new();
        let mut received_samples = Vec::new();
        
        // Subscribe to receive audio
        let mut audio_rx = self.coordinator.subscribe_to_audio(call_id).await?;
        
        // Spawn receiving task
        let (tx, mut rx) = mpsc::channel(1000);
        tokio::spawn(async move {
            while let Some(frame) = audio_rx.recv().await {
                if tx.send(frame.samples).await.is_err() {
                    break;
                }
            }
        });
        
        // Send and receive for duration
        let start = std::time::Instant::now();
        let mut timestamp = 0u32;
        let mut frame_count = 0;
        
        while start.elapsed() < duration {
            // Generate and send REAL audio to media system → RTP
            let samples = generator(frame_count);
            let frame = AudioFrame::new(samples.clone(), 8000, 1, timestamp);
            self.coordinator.send_audio(call_id, frame).await?;
            sent_samples.extend(samples);
            
            // Receive REAL audio from RTP → media system (non-blocking)
            while let Ok(samples) = rx.try_recv() {
                received_samples.extend(samples);
            }
            
            timestamp += 160;
            frame_count += 1;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        
        // Collect any remaining received audio
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        while let Ok(samples) = rx.try_recv() {
            received_samples.extend(samples);
        }
        
        Ok((sent_samples, received_samples))
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
        let coordinator = UnifiedCoordinator::with_simple_peer_events(config, event_tx).await?;

        let controller = Arc::new(SimplePeerController::new(coordinator.clone(), local_uri.clone()));
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
        debug!("SimplePeer background task starting");
        
        let handlers = self.handlers.clone();
        let controller = self.controller.clone();
        let is_running = self.is_running.clone();
        
        is_running.store(true, Ordering::Relaxed);
        
        let task = tokio::spawn(async move {
            while is_running.load(Ordering::Relaxed) {
                match event_rx.recv().await {
                    Some(event) => {
                        Self::dispatch_event(event, &handlers, controller.clone()).await;
                    }
                    None => break,
                }
            }
        });
        
        {
            let mut bg_task = self.background_task.lock().await;
            *bg_task = Some(task);
        }
        
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
            _ => None,
        };
        
        drop(handlers_guard);
        
        if let Some(handler) = handler {
            let event_clone = event.clone();
            let controller_clone = controller.clone();
            
            tokio::spawn(async move {
                handler(event_clone, controller_clone).await;
            });
        }
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<CallId> {
        self.controller.call(target).await
    }
    
    /// Accept an incoming call
    pub async fn accept(&self, call_id: &CallId) -> Result<()> {
        self.controller.accept(call_id).await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.controller.hangup(call_id).await
    }
    
    /// Send REFER for call transfer
    pub async fn send_refer(&self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.controller.send_refer(call_id, refer_to).await
    }
}

/// Cleanup when SimplePeer is dropped
impl Drop for SimplePeer {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);
        
        if let Ok(mut bg_task) = self.background_task.try_lock() {
            if let Some(task) = bg_task.take() {
                task.abort();
            }
        }
    }
}
