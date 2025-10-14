//! Truly Simple API - Sequential blocking-style SIP peer
//!
//! This is the simplest possible API for developers who don't know SIP.
//! Just call methods sequentially like you would in any normal program.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use rvoip_media_core::types::AudioFrame;

// Re-export types that users need
pub use crate::api::unified::Config;
pub use crate::api::events::{Event, CallId};

/// Information about an incoming REFER request
#[derive(Debug, Clone)]
pub struct ReferRequest {
    pub call_id: CallId,
    pub refer_to: String,
    pub transaction_id: String,
    pub transfer_type: String, // "blind" or "attended"
}

/// A simple SIP peer with sequential API
pub struct SimplePeer {
    coordinator: Arc<UnifiedCoordinator>,
    event_rx: mpsc::Receiver<Event>,
    local_uri: String,
    // Keep these for forceful shutdown
    is_shutdown: Arc<std::sync::atomic::AtomicBool>,
    // Track pending transfer context
    pending_refer: Arc<tokio::sync::Mutex<Option<ReferRequest>>>,
}

impl SimplePeer {
    /// Create a new peer
    pub async fn new(name: &str) -> Result<Self> {
        let mut config = Config::default();
        config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        Self::with_config(name, config).await
    }
    
    /// Create a new peer with custom configuration
    pub async fn with_config(name: &str, mut config: Config) -> Result<Self> {
        if config.local_uri.starts_with("sip:user@") {
            config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        }
        let local_uri = config.local_uri.clone();
        
        let (event_tx, event_rx) = mpsc::channel(1000);
        let coordinator = UnifiedCoordinator::with_simple_peer_events(config, event_tx).await?;

        Ok(Self {
            coordinator,
            event_rx,
            local_uri,
            is_shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending_refer: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call (returns immediately with call ID)
    pub async fn call(&mut self, target: &str) -> Result<CallId> {
        self.coordinator.make_call(&self.local_uri, target).await
    }
    
    /// Wait for a call to be answered
    pub async fn wait_for_answered(&mut self, expected_call_id: &CallId) -> Result<CallId> {
        while let Some(event) = self.event_rx.recv().await {
            if let Event::CallAnswered { call_id, .. } = event {
                if &call_id == expected_call_id {
                    return Ok(call_id);
                }
            }
        }
        Err(crate::errors::SessionError::Other("Event channel closed".to_string()))
    }
    
    /// Wait for incoming call
    pub async fn wait_for_incoming_call(&mut self) -> Result<(CallId, String)> {
        while let Some(event) = self.event_rx.recv().await {
            if let Event::IncomingCall { call_id, from, .. } = event {
                return Ok((call_id, from));
            }
        }
        Err(crate::errors::SessionError::Other("Event channel closed".to_string()))
    }
    
    /// Wait for REFER request with full context
    pub async fn wait_for_refer(&mut self) -> Result<Option<ReferRequest>> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::ReferReceived { call_id, refer_to, transaction_id, transfer_type, .. } => {
                    let refer = ReferRequest {
                        call_id,
                        refer_to,
                        transaction_id,
                        transfer_type,
                    };
                    
                    // Store for later use
                    *self.pending_refer.lock().await = Some(refer.clone());
                    
                    return Ok(Some(refer));
                }
                Event::CallEnded { .. } => return Ok(None),
                _ => {} // Ignore other events
            }
        }
        Ok(None)
    }
    
    /// Accept an incoming call
    pub async fn accept(&mut self, call_id: &CallId) -> Result<()> {
        self.coordinator.accept_call(call_id).await
    }
    
    /// Hangup a call (fire-and-forget to avoid blocking)
    pub async fn hangup(&mut self, call_id: &CallId) -> Result<()> {
        tracing::info!("[SimplePeer] hangup() called - initiating BYE (fire-and-forget)");
        
        // Fire-and-forget: Spawn hangup in background to avoid blocking
        // The background event loop will handle the BYE response and CallEnded event
        // This prevents deadlocks from nested locks in state machine
        let coordinator = self.coordinator.clone();
        let call_id_clone = call_id.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.hangup(&call_id_clone).await {
                tracing::warn!("Background hangup failed for {}: {}", call_id_clone, e);
            } else {
                tracing::info!("[SimplePeer] Background hangup completed for {}", call_id_clone);
            }
        });
        
        tracing::info!("[SimplePeer] hangup() returning immediately (BYE sent in background)");
        Ok(())
    }
    
    /// Exchange audio for a duration (send and receive simultaneously)
    pub async fn exchange_audio(
        &mut self,
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
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        
        // Collect remaining audio
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        while let Ok(samples) = rx.try_recv() {
            received_samples.extend(samples);
        }
        
        Ok((sent_samples, received_samples))
    }
    
    /// Send REFER for call transfer
    pub async fn send_refer(&mut self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, refer_to).await
    }
    
    /// Complete a blind transfer by terminating current call and calling target
    pub async fn complete_blind_transfer(
        &mut self, 
        refer: &ReferRequest,
    ) -> Result<CallId> {
        tracing::info!("[SimplePeer] Completing blind transfer from call {} to {}", 
                       refer.call_id, refer.refer_to);
        
        // 1. Hangup current call (this triggers BYE)
        self.hangup(&refer.call_id).await?;
        
        // 2. Wait for call to actually end
        let ended_call = self.wait_for_call_ended().await?;
        tracing::info!("[SimplePeer] Original call {} ended", ended_call);
        
        // 3. Make new call to transfer target
        let new_call_id = self.call(&refer.refer_to).await?;
        tracing::info!("[SimplePeer] Transfer call initiated: {}", new_call_id);
        
        // 4. When call is established, the state machine will send NOTIFY (success)
        // This happens automatically via the state table transitions
        
        Ok(new_call_id)
    }
    
    /// Wait for call to end
    pub async fn wait_for_call_ended(&mut self) -> Result<CallId> {
        while let Some(event) = self.event_rx.recv().await {
            if let Event::CallEnded { call_id, .. } = event {
                return Ok(call_id);
            }
        }
        Err(crate::errors::SessionError::Other("Event channel closed".to_string()))
    }
    
    /// Forceful shutdown - stops EVERYTHING and exits
    pub async fn shutdown(self, _timeout: Duration) -> Result<()> {
        tracing::info!("[SimplePeer] shutdown() called - exiting immediately");
        
        // FORCEFUL: Kill the entire process immediately
        // This stops all tokio tasks, all threads, everything
        // Background event loop tasks would keep the process alive otherwise
        std::process::exit(0);
    }
}
