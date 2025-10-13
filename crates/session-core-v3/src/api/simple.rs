//! Truly Simple API - Event-driven SIP peer
//!
//! This is the simplest possible API - event-driven with direct method calls.
//! No callbacks, no complex state management, just simple async event handling.

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

// Re-export types that users of SimplePeer will need
pub use crate::api::unified::Config;
pub use crate::api::events::{Event, CallHandle, CallId};

/// A simple SIP peer that can make and receive calls
pub struct SimplePeer {
    /// The coordinator that does all the work
    coordinator: Arc<UnifiedCoordinator>,
    
    /// Event receiver for all SimplePeer events
    event_rx: mpsc::Receiver<Event>,
    
    /// Local SIP URI
    local_uri: String,
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
        })
    }
    
    // ===== Event Handling =====
    
    /// Get the next event (unified event handling)
    /// 
    /// This is the core method for event-driven programming with SimplePeer.
    /// All events (incoming calls, transfers, audio, etc.) come through this method.
    /// 
    /// # Returns
    /// * `Some(event)` - An event occurred
    /// * `None` - Event channel closed (peer shutting down)
    /// 
    /// # Example
    /// ```rust,no_run
    /// let mut alice = SimplePeer::new("alice").await?;
    /// 
    /// while let Some(event) = alice.next_event().await {
    ///     match event {
    ///         Event::IncomingCall { call_id, from, .. } => {
    ///             if from.contains("spam") {
    ///                 alice.reject(&call_id).await?;
    ///             } else {
    ///                 let call_handle = alice.accept(&call_id).await?;
    ///                 // Use call_handle for audio
    ///             }
    ///         }
    ///         Event::ReferReceived { refer_to, .. } => {
    ///             let new_call = alice.call(&refer_to).await?;
    ///             // Transfer completed
    ///         }
    ///         Event::CallEnded { .. } => break,
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub async fn next_event(&mut self) -> Option<Event> {
        self.event_rx.recv().await
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call and get a call handle
    /// 
    /// # Returns
    /// CallHandle with audio channels for this specific call
    /// 
    /// # Example
    /// ```rust,no_run
    /// let mut alice = SimplePeer::new("alice").await?;
    /// let call_handle = alice.call("sip:bob@example.com").await?;
    /// 
    /// // Send audio via the call handle
    /// let samples = vec![100, 200, 300]; // Simple audio data
    /// call_handle.send_audio(samples).await?;
    /// ```
    pub async fn call(&mut self, to: &str) -> Result<CallHandle> {
        let call_id = self.coordinator.make_call(&self.local_uri, to).await?;
        let (call_handle, _audio_rx_for_coordinator, _audio_tx_for_coordinator) = CallHandle::new(call_id.clone());
        // TODO: Wire up audio channels to media adapter
        Ok(call_handle)
    }
    
    /// Accept an incoming call and get a call handle
    /// 
    /// # Returns
    /// CallHandle with audio channels for this specific call
    /// 
    /// # Example
    /// ```rust,no_run
    /// while let Some(event) = alice.next_event().await {
    ///     match event {
    ///         Event::IncomingCall { call_id, from, .. } => {
    ///             let call_handle = alice.accept(&call_id).await?;
    ///             // Use call_handle for audio
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub async fn accept(&mut self, call_id: &CallId) -> Result<CallHandle> {
        self.coordinator.accept_call(call_id).await?;
        let (call_handle, _audio_rx_for_coordinator, _audio_tx_for_coordinator) = CallHandle::new(call_id.clone());
        // TODO: Wire up audio channels to media adapter
        Ok(call_handle)
    }
    
    /// Reject an incoming call
    /// 
    /// # Example
    /// ```rust,no_run
    /// while let Some(event) = alice.next_event().await {
    ///     match event {
    ///         Event::IncomingCall { call_id, from, .. } => {
    ///             if from.contains("spam") {
    ///                 alice.reject(&call_id).await?;
    ///             }
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub async fn reject(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.reject_call(call_id, "Busy").await
    }
    
    /// Hangup a call
    /// 
    /// # Example
    /// ```rust,no_run
    /// alice.hangup(&call_handle.call_id()).await?;
    /// ```
    pub async fn hangup(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    // ===== Call Control =====
    
    /// Put call on hold
    pub async fn hold(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.hold(call_id).await
    }
    
    /// Resume from hold
    pub async fn resume(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.resume(call_id).await
    }
    
    /// Mute microphone for call
    pub async fn mute(&mut self, _call_id: &CallId) -> Result<()> {
        // TODO: Implement mute via coordinator method
        warn!("Mute not implemented yet");
        Ok(())
    }
    
    /// Unmute microphone for call
    pub async fn unmute(&mut self, _call_id: &CallId) -> Result<()> {
        // TODO: Implement unmute via coordinator method  
        warn!("Unmute not implemented yet");
        Ok(())
    }
    
    // ===== Transfer Operations =====
    
    /// Send REFER message to initiate transfer
    /// 
    /// # Example
    /// ```rust,no_run
    /// // Bob sends REFER to Alice
    /// bob.send_refer(&call_id, "sip:charlie@example.com").await?;
    /// // Alice will receive Event::ReferReceived
    /// ```
    pub async fn send_refer(&mut self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, refer_to).await
    }
    
    // ===== DTMF Operations =====
    
    /// Send DTMF digit
    /// 
    /// # Example
    /// ```rust,no_run
    /// alice.send_dtmf(&call_id, '1').await?;
    /// alice.send_dtmf(&call_id, '#').await?;
    /// ```
    pub async fn send_dtmf(&mut self, call_id: &CallId, digit: char) -> Result<()> {
        self.coordinator.send_dtmf(call_id, digit).await
    }
}