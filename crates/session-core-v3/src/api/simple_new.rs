//! Truly Simple API - Simplified blocking handler SIP peer
//!
//! This is the simplest possible API - blocking handlers with direct SimplePeer access.
//! Register handlers, call run(), and handle events sequentially with simple linear code.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{warn, debug, info};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use rvoip_media_core::types::AudioFrame;

// Re-export types that users of SimplePeer will need
pub use crate::api::unified::Config;
pub use crate::api::events::{Event, CallId};

/// Simple handler types - blocking closures with direct SimplePeer access
type IncomingCallHandler = Box<dyn FnMut(CallId, String, &mut SimplePeer) -> Result<()> + Send>;
type ReferReceivedHandler = Box<dyn FnMut(CallId, String, &mut SimplePeer) -> Result<()> + Send>;
type CallEndedHandler = Box<dyn FnMut(CallId, String, &mut SimplePeer) -> Result<()> + Send>;

/// A simple SIP peer that can make and receive calls using blocking handlers
pub struct SimplePeer {
    /// The coordinator that does all the work
    coordinator: Arc<UnifiedCoordinator>,
    
    /// Event receiver for all SimplePeer events
    event_rx: mpsc::Receiver<Event>,
    
    /// Local SIP URI
    local_uri: String,
    
    /// Simple handler storage
    on_incoming_call: Option<IncomingCallHandler>,
    on_refer_received: Option<ReferReceivedHandler>,
    on_call_ended: Option<CallEndedHandler>,
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

        Ok(Self {
            coordinator,
            event_rx,
            local_uri,
            on_incoming_call: None,
            on_refer_received: None,
            on_call_ended: None,
        })
    }
    
    // ===== Handler Registration =====
    
    /// Register handler for incoming calls
    pub fn on_incoming_call<F>(&mut self, handler: F) -> &mut Self
    where F: FnMut(CallId, String, &mut SimplePeer) -> Result<()> + Send + 'static
    {
        self.on_incoming_call = Some(Box::new(handler));
        self
    }
    
    /// Register handler for REFER requests  
    pub fn on_refer_received<F>(&mut self, handler: F) -> &mut Self
    where F: FnMut(CallId, String, &mut SimplePeer) -> Result<()> + Send + 'static
    {
        self.on_refer_received = Some(Box::new(handler));
        self
    }
    
    /// Register handler for call ended events
    pub fn on_call_ended<F>(&mut self, handler: F) -> &mut Self
    where F: FnMut(CallId, String, &mut SimplePeer) -> Result<()> + Send + 'static
    {
        self.on_call_ended = Some(Box::new(handler));
        self
    }
    
    // ===== Event Loop =====
    
    /// Run the event loop - processes events and calls handlers sequentially
    pub async fn run(&mut self) -> Result<()> {
        info!("SimplePeer event loop started");
        
        while let Some(event) = self.event_rx.recv().await {
            debug!("Processing event: {:?}", event);
            
            match event {
                Event::IncomingCall { call_id, from, .. } => {
                    if let Some(handler) = &mut self.on_incoming_call {
                        if let Err(e) = handler(call_id, from, self) {
                            warn!("IncomingCall handler error: {}", e);
                        }
                    }
                }
                Event::ReferReceived { call_id, refer_to, .. } => {
                    if let Some(handler) = &mut self.on_refer_received {
                        if let Err(e) = handler(call_id, refer_to, self) {
                            warn!("ReferReceived handler error: {}", e);
                        }
                    }
                }
                Event::CallEnded { call_id, reason, .. } => {
                    if let Some(handler) = &mut self.on_call_ended {
                        if let Err(e) = handler(call_id, reason, self) {
                            warn!("CallEnded handler error: {}", e);
                        }
                    }
                }
                _ => {} // Ignore other events
            }
        }
        
        info!("SimplePeer event loop ended");
        Ok(())
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call
    pub async fn call(&mut self, target: &str) -> Result<CallId> {
        self.coordinator.make_call(&self.local_uri, target).await
    }
    
    /// Accept an incoming call
    pub async fn accept(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.accept_call(call_id).await
    }
    
    /// Hangup a call
    pub async fn hangup(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    /// Send REFER for call transfer
    pub async fn send_refer(&mut self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, refer_to).await
    }
    
    // ===== Audio Operations =====
    
    /// Exchange audio for a duration (send and receive simultaneously)
    pub async fn exchange_audio(
        &mut self,
        call_id: &CallId,
        duration: Duration,
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
            // Generate and send audio
            let samples = generator(frame_count);
            let frame = AudioFrame::new(samples.clone(), 8000, 1, timestamp);
            self.coordinator.send_audio(call_id, frame).await?;
            sent_samples.extend(samples);
            
            // Receive audio (non-blocking)
            while let Ok(samples) = rx.try_recv() {
                received_samples.extend(samples);
            }
            
            timestamp += 160;
            frame_count += 1;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        
        // Collect any remaining received audio
        tokio::time::sleep(Duration::from_millis(100)).await;
        while let Ok(samples) = rx.try_recv() {
            received_samples.extend(samples);
        }
        
        Ok((sent_samples, received_samples))
    }
    
    /// Send audio frame
    pub async fn send_audio(&mut self, call_id: &CallId, frame: AudioFrame) -> Result<()> {
        self.coordinator.send_audio(call_id, frame).await
    }
    
    /// Subscribe to receive audio
    pub async fn subscribe_audio(&mut self, call_id: &CallId) -> Result<crate::types::AudioFrameSubscriber> {
        self.coordinator.subscribe_to_audio(call_id).await
    }
    
    // ===== Call Control =====
    
    pub async fn hold(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.hold(call_id).await
    }
    
    pub async fn resume(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.resume(call_id).await
    }
    
    pub async fn send_dtmf(&mut self, call_id: &CallId, tones: &str) -> Result<()> {
        if let Some(digit) = tones.chars().next() {
            self.coordinator.send_dtmf(call_id, digit).await
        } else {
            Ok(())
        }
    }
}

