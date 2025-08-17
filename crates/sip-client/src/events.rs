//! Event system for the SIP client

use crate::types::{AudioQualityMetrics, Call, CallId, CallState};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

/// Events emitted by the SIP client
#[derive(Debug, Clone)]
pub enum SipClientEvent {
    // Call events
    /// Incoming call received
    IncomingCall {
        /// The call object
        call: std::sync::Arc<Call>,
        /// From URI
        from: String,
        /// Display name if available
        display_name: Option<String>,
    },
    
    /// Outgoing call initiated
    OutgoingCall {
        /// The call object
        call: std::sync::Arc<Call>,
    },
    
    /// Call state changed
    CallStateChanged {
        /// The call object
        call: std::sync::Arc<Call>,
        /// Previous state
        previous_state: CallState,
        /// New state
        new_state: CallState,
        /// Reason for the change
        reason: Option<String>,
    },
    
    /// Call connected
    CallConnected {
        /// Call ID
        call_id: CallId,
        /// Selected codec
        codec: String,
        /// Remote SDP
        remote_sdp: String,
    },
    
    /// Call terminated
    CallTerminated {
        /// Call ID
        call_id: CallId,
        /// Termination reason
        reason: String,
        /// Call duration in seconds
        duration_secs: u64,
    },
    
    /// Call ended
    CallEnded {
        /// The call
        call: std::sync::Arc<Call>,
    },
    
    // Audio events
    /// Audio device changed
    AudioDeviceChanged {
        /// Device direction (input/output)
        direction: rvoip_audio_core::AudioDirection,
        /// Old device name
        old_device: Option<String>,
        /// New device name
        new_device: Option<String>,
    },
    
    /// Audio level changed
    AudioLevelChanged {
        /// Call ID (if during a call)
        call_id: Option<CallId>,
        /// Audio direction
        direction: rvoip_audio_core::AudioDirection,
        /// Level (0.0 to 1.0)
        level: f32,
        /// Peak level (0.0 to 1.0)
        peak: f32,
    },
    
    /// Audio device error
    AudioDeviceError {
        /// Error message
        message: String,
        /// Device that failed
        device: Option<String>,
    },
    
    /// Call was transferred (outgoing transfer)
    CallTransferred {
        /// The call that was transferred
        call: std::sync::Arc<Call>,
        /// Target URI for the transfer
        target: String,
    },
    
    /// Incoming transfer request received
    IncomingTransferRequest {
        /// The call being transferred
        call: std::sync::Arc<Call>,
        /// Target URI to transfer to
        target_uri: String,
        /// Who initiated the transfer (optional)
        referred_by: Option<String>,
        /// Whether this is attended transfer (has Replaces)
        is_attended: bool,
    },
    
    /// Transfer progress notification
    TransferProgress {
        /// Call ID of the original call
        call_id: CallId,
        /// Transfer status
        status: TransferStatus,
        /// Optional message
        message: Option<String>,
    },
    
    /// Call was put on hold
    CallOnHold {
        /// The call that was put on hold
        call: std::sync::Arc<Call>,
    },
    
    /// Call was resumed from hold
    CallResumed {
        /// The call that was resumed
        call: std::sync::Arc<Call>,
    },
    
    // Quality events
    /// Call quality report
    CallQualityReport {
        /// Call ID
        call_id: CallId,
        /// Quality metrics
        metrics: AudioQualityMetrics,
    },
    
    /// Network quality changed
    NetworkQualityChanged {
        /// Call ID (if during a call)
        call_id: Option<CallId>,
        /// Packet loss percentage
        packet_loss: f64,
        /// Jitter in milliseconds
        jitter_ms: f64,
        /// Round-trip time in milliseconds
        rtt_ms: f64,
    },
    
    // Codec events
    /// Codec changed during call
    CodecChanged {
        /// Call ID
        call_id: CallId,
        /// Old codec
        old_codec: String,
        /// New codec
        new_codec: String,
        /// Reason for change
        reason: String,
    },
    
    /// Codec negotiation failed
    CodecNegotiationFailed {
        /// Call ID
        call_id: CallId,
        /// Available local codecs
        local_codecs: Vec<String>,
        /// Available remote codecs
        remote_codecs: Vec<String>,
    },
    
    // Registration events
    /// Registration status changed
    RegistrationStatusChanged {
        /// User URI
        uri: String,
        /// Status string
        status: String,
        /// Optional reason
        reason: Option<String>,
    },
    
    /// Registration successful
    RegistrationSuccessful {
        /// Server URI
        server: String,
        /// Expiry time in seconds
        expires: u32,
    },
    
    /// Registration failed
    RegistrationFailed {
        /// Server URI
        server: String,
        /// Error reason
        reason: String,
        /// SIP response code
        code: Option<u16>,
    },
    
    /// Registration expired
    RegistrationExpired {
        /// Server URI
        server: String,
    },
    
    // Error events
    /// General error occurred
    Error {
        /// Associated call
        call: Option<std::sync::Arc<Call>>,
        /// Error message
        message: String,
        /// Error category
        category: ErrorCategory,
    },
    
    // Client lifecycle events
    /// Client started
    Started,
    
    /// Client stopped
    Stopped,
    
    // Media events
    /// Media started
    MediaStarted {
        /// The call
        call: std::sync::Arc<Call>,
        /// Media type (audio/video)
        media_type: String,
    },
    
    /// Media stopped
    MediaStopped {
        /// The call
        call: std::sync::Arc<Call>,
        /// Media type (audio/video)
        media_type: String,
    },
    
    /// DTMF sent
    DtmfSent {
        /// The call
        call: std::sync::Arc<Call>,
        /// DTMF digits
        digits: String,
    },
    
    /// Quality report
    QualityReport {
        /// The call
        call: std::sync::Arc<Call>,
        /// MOS score (1.0 to 5.0)
        mos_score: f32,
        /// Packet loss percentage
        packet_loss: f32,
        /// Jitter in milliseconds
        jitter_ms: f32,
    },
    
    // Network events
    /// Network connected
    NetworkConnected {
        /// Optional reason
        reason: Option<String>,
    },
    
    /// Network disconnected
    NetworkDisconnected {
        /// Reason for disconnection
        reason: String,
    },
    
    // Recovery events
    /// Recovery succeeded for a component
    RecoverySucceeded {
        /// Component that recovered
        component: String,
        /// Number of attempts it took
        attempts: u32,
    },
    
    /// Recovery failed for a component
    RecoveryFailed {
        /// Component that failed to recover
        component: String,
        /// Error message
        error: String,
        /// Number of attempts made
        attempts: u32,
    },
    
    /// Connection lost
    ConnectionLost {
        /// Reason for connection loss
        reason: String,
    },
    
    /// Connection restored
    ConnectionRestored,
    
    /// Degradation applied
    DegradationApplied {
        /// Degradation level (0 = normal, higher = more degraded)
        level: u8,
        /// Actions taken
        actions: crate::recovery::DegradationActions,
    },
    
    // Reconnection events
    /// Reconnection failed
    ReconnectionFailed {
        /// Connection type that failed
        connection_type: String,
        /// Error message
        error: String,
    },
    
    /// Registration restored after reconnection
    RegistrationRestored,
    
    /// Call recovered after disconnection
    CallRecovered {
        /// Recovered call ID
        call_id: CallId,
    },
    
    /// Call lost and cannot be recovered
    CallLost {
        /// Lost call ID
        call_id: CallId,
        /// Reason for loss
        reason: String,
    },
    
    /// Audio devices restored after reconnection
    AudioDevicesRestored,
}

/// Transfer status for tracking transfer progress
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Transfer accepted, attempting to call target
    Accepted,
    /// Target is ringing
    Ringing,
    /// Transfer completed successfully
    Completed,
    /// Transfer failed
    Failed(String),
}

/// Error categories for event classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// Network-related error
    Network,
    /// Audio device error
    Audio,
    /// Codec-related error
    Codec,
    /// Protocol error
    Protocol,
    /// Configuration error
    Configuration,
    /// Internal error
    Internal,
}

/// Event stream type
pub type EventStream = BroadcastStream<SipClientEvent>;

/// Simple event iterator that doesn't require StreamExt
pub struct EventIterator {
    stream: EventStream,
}

impl EventIterator {
    /// Create a new event iterator from a stream
    pub fn new(stream: EventStream) -> Self {
        Self { stream }
    }
    
    /// Get the next event (async)
    pub async fn next(&mut self) -> Option<SipClientEvent> {
        use tokio_stream::StreamExt;
        match self.stream.next().await {
            Some(Ok(event)) => Some(event),
            _ => None,
        }
    }
}

/// Event emitter for the SIP client
#[derive(Clone)]
pub struct EventEmitter {
    sender: broadcast::Sender<SipClientEvent>,
}

impl EventEmitter {
    /// Create a new event emitter with the specified capacity
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
    
    /// Emit an event
    pub fn emit(&self, event: SipClientEvent) {
        // Ignore send errors (no receivers)
        let _ = self.sender.send(event);
    }
    
    /// Subscribe to events
    pub fn subscribe(&self) -> EventStream {
        BroadcastStream::new(self.sender.subscribe())
    }
    
    /// Subscribe to events with a simple iterator
    pub fn subscribe_simple(&self) -> EventIterator {
        EventIterator::new(self.subscribe())
    }
    
    /// Get the number of active receivers
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Event aggregator that combines events from multiple sources
pub struct EventAggregator {
    emitter: EventEmitter,
    client_events: Option<tokio::sync::mpsc::UnboundedReceiver<rvoip_client_core::ClientEvent>>,
}

impl EventAggregator {
    /// Create a new event aggregator
    pub fn new(emitter: EventEmitter) -> Self {
        Self {
            emitter,
            client_events: None,
        }
    }
    
    /// Set the client event receiver
    pub fn set_client_events(&mut self, receiver: tokio::sync::mpsc::UnboundedReceiver<rvoip_client_core::ClientEvent>) {
        self.client_events = Some(receiver);
    }
    
    /// Start aggregating events
    pub async fn start(mut self) {
        loop {
            tokio::select! {
                // Handle client-core events
                Some(event) = async {
                    self.client_events.as_mut()?.recv().await
                } => {
                    if let Some(sip_event) = self.convert_client_event(event) {
                        self.emitter.emit(sip_event);
                    }
                }
                
                // Add more event sources here as needed
                
                else => {
                    // All channels closed, exit
                    break;
                }
            }
        }
    }
    
    /// Convert client-core event to SIP client event
    fn convert_client_event(&self, event: rvoip_client_core::ClientEvent) -> Option<SipClientEvent> {
        // This would need to be reimplemented to work with Arc<Call>
        // For now, return None
        None
    }
}