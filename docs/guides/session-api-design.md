# Session-Core Unified API Design

## Executive Summary

This document outlines the comprehensive redesign of session-core's API to provide both UAC (User Agent Client) and UAS (User Agent Server) functionality with progressive disclosure of complexity. The design eliminates the need for client-core while providing simple, standard, and advanced API levels for different use cases.

## Core Principles

1. **Progressive Disclosure**: Simple things should be simple, complex things should be possible
2. **Separation of Concerns**: SIP/RTP in session-core, audio devices in sip-client
3. **Type Safety**: Leverage Rust's type system for compile-time guarantees
4. **Zero-Cost Abstractions**: Higher-level APIs should not impose runtime overhead
5. **Backward Compatibility**: Existing code using low-level APIs continues to work

## Architecture Overview

```
session-core/
├── Low-Level API (existing)
│   ├── SessionCoordinator
│   ├── DialogManager
│   └── MediaManager
│
├── High-Level UAC API (new)
│   ├── Simple Client
│   ├── Standard Client
│   └── Advanced Client
│
└── High-Level UAS API (new)
    ├── Simple Server
    ├── Standard Server
    └── Advanced Server
```

## File Structure

```
session-core/src/
├── api/
│   ├── mod.rs                    // Public API exports
│   ├── control.rs                // Existing low-level control (unchanged)
│   ├── types.rs                  // Shared types
│   ├── events.rs                 // Event definitions
│   │
│   ├── uac/                      // UAC (Client) APIs
│   │   ├── mod.rs                // UAC module exports
│   │   ├── simple.rs             // SimpleUacClient implementation
│   │   ├── standard.rs           // UacClient implementation  
│   │   ├── advanced.rs           // AdvancedUacClient implementation
│   │   ├── builder.rs            // UAC builder patterns
│   │   ├── call.rs               // Call handle and operations
│   │   ├── registration.rs       // Registration management
│   │   ├── media.rs              // Media session coordination
│   │   ├── events.rs             // UAC-specific events
│   │   └── traits.rs             // UAC handler traits
│   │
│   ├── uas/                      // UAS (Server) APIs
│   │   ├── mod.rs                // UAS module exports
│   │   ├── simple.rs             // SimpleUasServer implementation
│   │   ├── standard.rs           // UasServer implementation
│   │   ├── advanced.rs           // AdvancedUasServer implementation
│   │   ├── builder.rs            // UAS builder patterns
│   │   ├── controller.rs         // CallController trait and helpers
│   │   ├── handler.rs            // CallHandler trait and helpers
│   │   ├── events.rs             // UAS-specific events
│   │   └── traits.rs             // UAS handler traits
│   │
│   └── common/                   // Shared functionality
│       ├── mod.rs
│       ├── bridge.rs             // Multi-party bridging
│       ├── dtmf.rs               // DTMF handling
│       ├── quality.rs            // Media quality monitoring
│       ├── recording.rs          // Call recording
│       └── transfer.rs           // Call transfer operations
│
├── coordinator/                  // (existing, unchanged)
├── dialog/                       // (existing, unchanged)
├── manager/                      // (existing, unchanged)
├── media/                        // (existing, unchanged)
└── lib.rs                        // Crate exports
```

## UAC API Design

### Simple UAC Client

```rust
// session-core/src/api/uac/simple.rs

use crate::api::types::*;

/// Simplest possible UAC client with sensible defaults
pub struct SimpleUacClient {
    inner: Arc<UacCore>,
}

impl SimpleUacClient {
    /// Create client with just SIP identity
    pub async fn new(identity: &str) -> Result<Self> {
        Self::builder()
            .with_identity(identity)
            .build()
            .await
    }
    
    /// Make a call - returns when connected or failed
    pub async fn call(&self, uri: &str) -> Result<CallHandle> {
        let call = self.inner.create_call(uri, None).await?;
        call.wait_for_answer().await?;
        Ok(call)
    }
    
    /// Hangup all active calls
    pub async fn hangup_all(&self) -> Result<()> {
        for call in self.inner.active_calls().await {
            call.hangup().await.ok();
        }
        Ok(())
    }
    
    /// Get simplified events
    pub async fn events(&self) -> SimpleEventStream {
        SimpleEventStream::new(self.inner.events())
    }
}

/// Simplified events for basic use cases
pub enum SimpleUacEvent {
    IncomingCall { from: String, call: CallHandle },
    CallConnected { call_id: CallId },
    CallEnded { call_id: CallId, reason: String },
    Error { message: String },
}
```

### Standard UAC Client

```rust
// session-core/src/api/uac/standard.rs

/// Standard UAC client with common features
pub struct UacClient {
    inner: Arc<UacCore>,
    handler: Option<Arc<dyn UacEventHandler>>,
    registration: Option<RegistrationHandle>,
}

/// Event handler for standard client
#[async_trait]
pub trait UacEventHandler: Send + Sync {
    /// Handle incoming calls
    async fn on_incoming_call(&self, call: IncomingCall) -> IncomingCallDecision {
        IncomingCallDecision::Reject(486) // Busy by default
    }
    
    /// Notification when call state changes
    async fn on_call_state_changed(&self, call_id: CallId, old: CallState, new: CallState) {}
    
    /// Media events
    async fn on_media_event(&self, call_id: CallId, event: MediaEvent) {}
    
    /// Registration status changes
    async fn on_registration_changed(&self, status: RegistrationStatus) {}
}

impl UacClient {
    /// Builder pattern for configuration
    pub fn builder() -> UacClientBuilder {
        UacClientBuilder::default()
    }
    
    /// Make an outgoing call
    pub async fn call(&self, uri: &str) -> Result<OutgoingCall> {
        let sdp = self.inner.generate_sdp_offer().await?;
        let call = self.inner.create_call(uri, Some(sdp)).await?;
        Ok(OutgoingCall::new(call))
    }
    
    /// Answer an incoming call
    pub async fn answer(&self, call_id: &CallId) -> Result<()> {
        let call = self.inner.get_call(call_id)?;
        let sdp = self.inner.generate_sdp_answer(&call.remote_sdp).await?;
        call.answer(Some(sdp)).await
    }
    
    /// Reject an incoming call
    pub async fn reject(&self, call_id: &CallId, code: u16) -> Result<()> {
        self.inner.get_call(call_id)?.reject(code).await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.inner.get_call(call_id)?.hangup().await
    }
    
    /// Put call on hold
    pub async fn hold(&self, call_id: &CallId) -> Result<()> {
        let call = self.inner.get_call(call_id)?;
        call.update_media(MediaDirection::SendOnly).await
    }
    
    /// Resume held call
    pub async fn resume(&self, call_id: &CallId) -> Result<()> {
        let call = self.inner.get_call(call_id)?;
        call.update_media(MediaDirection::SendRecv).await
    }
    
    /// Transfer call (blind transfer)
    pub async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()> {
        let call = self.inner.get_call(call_id)?;
        call.transfer(target, TransferType::Blind).await
    }
    
    /// Attended transfer
    pub async fn attended_transfer(
        &self, 
        call_a: &CallId, 
        call_b: &CallId
    ) -> Result<()> {
        self.inner.bridge_and_transfer(call_a, call_b).await
    }
    
    /// Register with SIP server
    pub async fn register(&self, credentials: Credentials) -> Result<()> {
        let handle = self.inner.register(credentials).await?;
        self.registration = Some(handle);
        Ok(())
    }
    
    /// Unregister
    pub async fn unregister(&self) -> Result<()> {
        if let Some(reg) = self.registration.take() {
            reg.unregister().await?;
        }
        Ok(())
    }
    
    /// Send DTMF digits
    pub async fn send_dtmf(&self, call_id: &CallId, digits: &str) -> Result<()> {
        let call = self.inner.get_call(call_id)?;
        for digit in digits.chars() {
            call.send_dtmf_digit(digit).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
    
    /// Get media information for a call
    pub async fn get_media_info(&self, call_id: &CallId) -> Result<MediaInfo> {
        self.inner.get_media_info(call_id).await
    }
    
    /// Subscribe to RTP packets (for external processing)
    pub async fn subscribe_to_rtp(
        &self, 
        call_id: &CallId
    ) -> Result<RtpPacketStream> {
        self.inner.create_rtp_stream(call_id).await
    }
    
    /// Send RTP packet (from external source)
    pub async fn send_rtp(
        &self, 
        call_id: &CallId, 
        packet: RtpPacket
    ) -> Result<()> {
        self.inner.send_rtp_packet(call_id, packet).await
    }
}

/// Outgoing call handle
pub struct OutgoingCall {
    inner: Arc<Call>,
}

impl OutgoingCall {
    /// Wait for the call to be answered
    pub async fn wait_for_answer(&self) -> Result<()> {
        self.inner.wait_for_state(CallState::Connected).await
    }
    
    /// Cancel the call attempt
    pub async fn cancel(&self) -> Result<()> {
        self.inner.cancel().await
    }
    
    /// Get call ID
    pub fn id(&self) -> &CallId {
        &self.inner.id
    }
}
```

### Advanced UAC Client

```rust
// session-core/src/api/uac/advanced.rs

/// Advanced UAC with full control
pub struct AdvancedUacClient {
    inner: Arc<UacCore>,
    controller: Arc<dyn UacController>,
}

/// Full control interface for UAC
#[async_trait]
pub trait UacController: Send + Sync {
    /// Customize SDP offer generation
    async fn generate_sdp_offer(&self, constraints: &MediaConstraints) -> Result<String>;
    
    /// Customize SDP answer generation
    async fn generate_sdp_answer(&self, offer: &str, constraints: &MediaConstraints) -> Result<String>;
    
    /// Control INVITE building
    async fn build_invite(&self, request: &mut Request, call: &CallInfo);
    
    /// Handle provisional responses (1xx)
    async fn on_provisional(&self, call_id: CallId, response: Response) -> ProvisionalAction;
    
    /// Handle successful response (2xx)
    async fn on_success(&self, call_id: CallId, response: Response) -> SuccessAction;
    
    /// Handle failure response (3xx-6xx)
    async fn on_failure(&self, call_id: CallId, response: Response) -> FailureAction;
    
    /// Control ACK generation
    async fn build_ack(&self, request: &mut Request, call: &CallInfo);
    
    /// Handle in-dialog requests
    async fn on_in_dialog_request(&self, call_id: CallId, request: Request) -> Response;
    
    /// Custom registration behavior
    async fn on_registration_challenge(&self, challenge: AuthChallenge) -> AuthResponse;
    
    /// Media session established
    async fn on_media_established(&self, call_id: CallId, session: MediaSession);
    
    /// RTP/RTCP events
    async fn on_rtp_event(&self, call_id: CallId, event: RtpEvent);
    
    /// Custom headers for all requests
    fn additional_headers(&self) -> Vec<Header> {
        vec![]
    }
}

impl AdvancedUacClient {
    /// Create with custom controller
    pub fn new(controller: Arc<dyn UacController>) -> Self {
        Self {
            inner: Arc::new(UacCore::new()),
            controller,
        }
    }
    
    /// Direct access to coordinator for low-level operations
    pub fn coordinator(&self) -> &Arc<SessionCoordinator> {
        &self.inner.coordinator
    }
    
    /// Create call with full control
    pub async fn create_call(&self, params: CallParameters) -> Result<AdvancedCall> {
        // Build INVITE with controller customization
        let mut request = self.inner.build_base_invite(&params)?;
        self.controller.build_invite(&mut request, &params.into()).await;
        
        // Add custom headers
        for header in self.controller.additional_headers() {
            request.headers.push(header);
        }
        
        // Send and track
        let call = self.inner.send_invite(request).await?;
        Ok(AdvancedCall::new(call, self.controller.clone()))
    }
    
    /// Advanced registration with custom handling
    pub async fn register_advanced(&self, params: RegistrationParameters) -> Result<Registration> {
        self.inner.register_with_controller(params, self.controller.clone()).await
    }
    
    /// Subscribe to specific event types
    pub async fn subscribe_to_events(&self, filter: EventFilter) -> EventStream {
        self.inner.create_filtered_stream(filter).await
    }
    
    /// Direct dialog access
    pub async fn get_dialog(&self, call_id: &CallId) -> Result<DialogHandle> {
        self.inner.get_dialog_handle(call_id).await
    }
    
    /// Access to media manager
    pub fn media_manager(&self) -> &Arc<MediaManager> {
        &self.inner.media_manager
    }
}

/// Advanced call handle with full control
pub struct AdvancedCall {
    inner: Arc<Call>,
    controller: Arc<dyn UacController>,
}

impl AdvancedCall {
    /// Send custom in-dialog request
    pub async fn send_request(&self, method: Method, body: Option<String>) -> Result<Response> {
        self.inner.send_in_dialog_request(method, body).await
    }
    
    /// Update session with custom SDP
    pub async fn update_session(&self, sdp: String) -> Result<()> {
        self.inner.send_update(Some(sdp)).await
    }
    
    /// Access to RTP session
    pub async fn rtp_session(&self) -> Result<RtpSession> {
        self.inner.get_rtp_session().await
    }
    
    /// Direct RTCP control
    pub async fn send_rtcp(&self, packet: RtcpPacket) -> Result<()> {
        self.inner.send_rtcp_packet(packet).await
    }
}
```

## UAS API Design

### Simple UAS Server

```rust
// session-core/src/api/uas/simple.rs

/// Simplest possible UAS server
pub struct SimpleUasServer {
    inner: Arc<UasCore>,
    decision: CallDecision,
}

impl SimpleUasServer {
    /// Always accept all calls
    pub async fn always_accept(bind_addr: &str) -> Result<Self> {
        let inner = UasCore::bind(bind_addr).await?;
        Ok(Self {
            inner,
            decision: CallDecision::Accept,
        })
    }
    
    /// Always reject all calls
    pub async fn always_reject(bind_addr: &str, code: u16) -> Result<Self> {
        let inner = UasCore::bind(bind_addr).await?;
        Ok(Self {
            inner,
            decision: CallDecision::Reject(code),
        })
    }
    
    /// Simple filter function
    pub async fn with_filter<F>(bind_addr: &str, filter: F) -> Result<Self>
    where
        F: Fn(&str) -> bool + Send + Sync + 'static
    {
        let inner = UasCore::bind(bind_addr).await?;
        let server = Self {
            inner,
            decision: CallDecision::Filter(Box::new(filter)),
        };
        
        // Start handling
        server.start_handling().await?;
        Ok(server)
    }
    
    /// Start the server
    pub async fn start(&self) -> Result<()> {
        self.inner.start_with_handler(SimpleHandler {
            decision: self.decision.clone(),
        }).await
    }
    
    /// Stop the server
    pub async fn stop(&self) -> Result<()> {
        self.inner.stop().await
    }
}

struct SimpleHandler {
    decision: CallDecision,
}

impl SimpleHandler {
    fn handle_call(&self, call: &IncomingCall) -> CallDecision {
        match &self.decision {
            CallDecision::Accept => CallDecision::Accept,
            CallDecision::Reject(code) => CallDecision::Reject(*code),
            CallDecision::Filter(f) => {
                if f(&call.from) {
                    CallDecision::Accept
                } else {
                    CallDecision::Reject(603)
                }
            }
        }
    }
}
```

### Standard UAS Server

```rust
// session-core/src/api/uas/standard.rs

/// Standard UAS server with typical features
pub struct UasServer {
    inner: Arc<UasCore>,
    handler: Arc<dyn CallHandler>,
    config: UasConfig,
}

/// Configuration for standard UAS
#[derive(Clone, Builder)]
pub struct UasConfig {
    /// SIP domain to accept calls for
    pub domain: Option<String>,
    /// Maximum concurrent calls
    pub max_calls: Option<usize>,
    /// Enable call recording
    pub recording: bool,
    /// Enable metrics collection
    pub metrics: bool,
    /// Custom headers to add to responses
    pub custom_headers: Vec<Header>,
}

/// Standard call handler trait
#[async_trait]
pub trait CallHandler: Send + Sync {
    /// Decide what to do with incoming call
    async fn on_incoming_call(&self, call: IncomingCall) -> IncomingCallDecision;
    
    /// Generate SDP answer (optional override)
    async fn generate_answer_sdp(&self, call: &IncomingCall) -> Option<String> {
        None // Use default
    }
    
    /// Call has been established
    async fn on_call_established(&self, call: EstablishedCall) {}
    
    /// Handle DTMF digit
    async fn on_dtmf(&self, call_id: CallId, digit: char) {}
    
    /// Call state changed
    async fn on_call_state_changed(&self, call_id: CallId, old: CallState, new: CallState) {}
    
    /// Media quality report
    async fn on_media_quality(&self, call_id: CallId, quality: MediaQuality) {}
    
    /// Call ended
    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {}
}

/// Decision for incoming calls
pub enum IncomingCallDecision {
    /// Accept the call
    Accept,
    /// Accept with custom SDP
    AcceptWithSdp(String),
    /// Reject with SIP error code
    Reject(u16),
    /// Redirect to another URI
    Redirect(String),
    /// Queue the call (for call centers)
    Queue { 
        queue_id: String,
        timeout: Duration,
        music_on_hold: Option<String>,
    },
    /// Forward to another handler
    Forward(String),
}

impl UasServer {
    /// Create server with builder pattern
    pub fn builder() -> UasServerBuilder {
        UasServerBuilder::default()
    }
    
    /// Handle a specific call manually
    pub async fn handle_call(&self, call_id: &CallId, action: CallAction) -> Result<()> {
        match action {
            CallAction::Accept => self.accept_call(call_id, None).await,
            CallAction::AcceptWithSdp(sdp) => self.accept_call(call_id, Some(sdp)).await,
            CallAction::Reject(code) => self.reject_call(call_id, code).await,
            CallAction::Transfer(target) => self.transfer_call(call_id, &target).await,
        }
    }
    
    /// Accept an incoming call
    async fn accept_call(&self, call_id: &CallId, sdp: Option<String>) -> Result<()> {
        let call = self.inner.get_incoming_call(call_id)?;
        
        let answer_sdp = if let Some(sdp) = sdp {
            sdp
        } else if let Some(sdp) = self.handler.generate_answer_sdp(&call).await {
            sdp
        } else {
            self.inner.generate_default_answer(&call.offer_sdp).await?
        };
        
        self.inner.send_answer(call_id, answer_sdp).await?;
        
        // Start recording if enabled
        if self.config.recording {
            self.inner.start_recording(call_id).await?;
        }
        
        Ok(())
    }
    
    /// Reject an incoming call
    async fn reject_call(&self, call_id: &CallId, code: u16) -> Result<()> {
        self.inner.send_rejection(call_id, code).await
    }
    
    /// Transfer a call
    async fn transfer_call(&self, call_id: &CallId, target: &str) -> Result<()> {
        self.inner.transfer_call(call_id, target).await
    }
    
    /// Get call statistics
    pub async fn get_call_stats(&self, call_id: &CallId) -> Result<CallStatistics> {
        self.inner.get_statistics(call_id).await
    }
    
    /// List active calls
    pub async fn list_active_calls(&self) -> Result<Vec<CallInfo>> {
        self.inner.list_calls().await
    }
    
    /// Subscribe to server events
    pub async fn events(&self) -> ServerEventStream {
        self.inner.create_event_stream().await
    }
}

/// Builder for UasServer
pub struct UasServerBuilder {
    bind_addr: Option<String>,
    handler: Option<Arc<dyn CallHandler>>,
    config: UasConfig,
}

impl UasServerBuilder {
    pub fn bind(mut self, addr: &str) -> Self {
        self.bind_addr = Some(addr.to_string());
        self
    }
    
    pub fn with_handler(mut self, handler: Arc<dyn CallHandler>) -> Self {
        self.handler = Some(handler);
        self
    }
    
    pub fn with_domain(mut self, domain: &str) -> Self {
        self.config.domain = Some(domain.to_string());
        self
    }
    
    pub fn with_max_calls(mut self, max: usize) -> Self {
        self.config.max_calls = Some(max);
        self
    }
    
    pub fn enable_recording(mut self) -> Self {
        self.config.recording = true;
        self
    }
    
    pub fn enable_metrics(mut self) -> Self {
        self.config.metrics = true;
        self
    }
    
    pub async fn build(self) -> Result<UasServer> {
        let bind_addr = self.bind_addr.ok_or("bind address required")?;
        let handler = self.handler.ok_or("handler required")?;
        
        let inner = UasCore::bind(&bind_addr).await?;
        
        Ok(UasServer {
            inner,
            handler,
            config: self.config,
        })
    }
}
```

### Advanced UAS Server

```rust
// session-core/src/api/uas/advanced.rs

/// Advanced UAS with complete control
pub struct AdvancedUasServer {
    inner: Arc<UasCore>,
    controller: Arc<dyn CallController>,
}

/// Complete control interface for UAS
#[async_trait]
pub trait CallController: Send + Sync {
    /// Pre-INVITE validation (before creating transaction)
    async fn pre_invite_check(
        &self, 
        headers: &Headers, 
        source: SocketAddr
    ) -> PreInviteDecision;
    
    /// Full INVITE processing control
    async fn on_invite(&self, request: InviteRequest) -> InviteResponse;
    
    /// Control provisional responses
    async fn send_provisional(
        &self, 
        call_id: &CallId,
        request: &Request
    ) -> ProvisionalResponse;
    
    /// Control answer generation
    async fn on_answer_call(
        &self, 
        call: &IncomingCall,
        constraints: MediaConstraints
    ) -> AnswerAction;
    
    /// SDP negotiation hook
    async fn on_sdp_negotiation(
        &self,
        offer: &SdpOffer,
        capabilities: &MediaCapabilities
    ) -> SdpNegotiationResult;
    
    /// Media path established
    async fn on_media_ready(&self, call_id: CallId, media: MediaSession);
    
    /// Call fully established with dialog
    async fn on_call_established(&self, call: EstablishedCall, dialog: DialogHandle);
    
    /// Handle in-dialog requests
    async fn on_in_dialog_request(
        &self,
        call_id: CallId,
        request: Request
    ) -> Response;
    
    /// Handle CANCEL request
    async fn on_cancel(&self, call_id: CallId, request: Request) -> Response;
    
    /// Handle BYE request  
    async fn on_bye(&self, call_id: CallId, request: Request) -> Response;
    
    /// Handle OPTIONS request
    async fn on_options(&self, request: Request) -> Response;
    
    /// Handle REGISTER request (if acting as registrar)
    async fn on_register(&self, request: Request) -> Response;
    
    /// Handle SUBSCRIBE request
    async fn on_subscribe(&self, request: Request) -> Response;
    
    /// Handle NOTIFY request
    async fn on_notify(&self, request: Request) -> Response;
    
    /// Handle REFER (transfer) request
    async fn on_refer(&self, call_id: CallId, request: Request) -> ReferResponse;
    
    /// Call state machine transitions
    async fn on_call_state_change(
        &self,
        call_id: CallId,
        old_state: CallState,
        new_state: CallState,
        trigger: StateChangeTrigger
    );
    
    /// Media events
    async fn on_media_event(&self, call_id: CallId, event: MediaEvent);
    
    /// DTMF received
    async fn on_dtmf(&self, call_id: CallId, digit: char, duration: Duration);
    
    /// Call quality metrics
    async fn on_rtcp_report(&self, call_id: CallId, report: RtcpReport);
    
    /// Call ended with full context
    async fn on_call_ended(
        &self,
        call_id: CallId,
        reason: EndReason,
        dialog_info: DialogInfo,
        statistics: CallStatistics
    );
    
    /// Error handling
    async fn on_error(&self, call_id: Option<CallId>, error: ServerError);
}

/// Pre-INVITE decision
pub enum PreInviteDecision {
    /// Continue processing
    Continue,
    /// Reject immediately
    Reject(u16),
    /// Challenge with authentication
    Challenge(AuthChallenge),
    /// Rate limit exceeded
    RateLimit { retry_after: Duration },
}

/// INVITE response decision
pub enum InviteResponse {
    /// Accept the call
    Accept {
        provisional: ProvisionalResponse,
        ring_timeout: Duration,
        tag: Option<String>,
    },
    /// Queue the call
    Queue {
        queue_id: String,
        position: Option<usize>,
        early_media: Option<EarlyMedia>,
        provisional: ProvisionalResponse,
    },
    /// Redirect to another destination
    Redirect {
        contacts: Vec<String>,
        permanent: bool,
    },
    /// Reject the call
    Reject {
        code: u16,
        reason: Option<String>,
        headers: Vec<Header>,
    },
    /// Forward to another server
    Forward {
        destination: String,
        record_route: bool,
    },
    /// Custom response
    Custom(Response),
}

/// Provisional response types
pub enum ProvisionalResponse {
    /// 100 Trying
    Trying,
    /// 180 Ringing
    Ringing,
    /// 183 Session Progress (with early media)
    SessionProgress(String), // SDP for early media
    /// Custom 1xx response
    Custom { code: u16, reason: String },
}

/// REFER response
pub enum ReferResponse {
    /// Accept the transfer
    Accept { notify: bool },
    /// Reject the transfer
    Reject(u16),
    /// Delegate to another handler
    Delegate(String),
}

impl AdvancedUasServer {
    /// Create with custom controller
    pub fn new(controller: Arc<dyn CallController>) -> Self {
        Self {
            inner: Arc::new(UasCore::new()),
            controller,
        }
    }
    
    /// Bind to address and start
    pub async fn bind(self, addr: &str) -> Result<Self> {
        self.inner.bind(addr).await?;
        self.inner.start_with_controller(self.controller.clone()).await?;
        Ok(self)
    }
    
    /// Direct access to coordinator
    pub fn coordinator(&self) -> &Arc<SessionCoordinator> {
        &self.inner.coordinator
    }
    
    /// Direct access to dialog manager
    pub fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.inner.dialog_manager
    }
    
    /// Direct access to media manager
    pub fn media_manager(&self) -> &Arc<MediaManager> {
        &self.inner.media_manager
    }
    
    /// Configure transport layer
    pub async fn configure_transport(&self, config: TransportConfig) -> Result<()> {
        self.inner.configure_transport(config).await
    }
    
    /// Add custom codec
    pub async fn add_codec(&self, codec: Box<dyn Codec>) -> Result<()> {
        self.inner.media_manager.register_codec(codec).await
    }
    
    /// Start call recording for specific call
    pub async fn start_recording(
        &self, 
        call_id: &CallId,
        config: RecordingConfig
    ) -> Result<RecordingHandle> {
        self.inner.start_recording_with_config(call_id, config).await
    }
    
    /// Bridge two calls
    pub async fn bridge_calls(
        &self,
        call_a: &CallId,
        call_b: &CallId
    ) -> Result<BridgeHandle> {
        self.inner.create_bridge(call_a, call_b).await
    }
    
    /// Create conference room
    pub async fn create_conference(
        &self,
        room_id: String,
        config: ConferenceConfig
    ) -> Result<ConferenceHandle> {
        self.inner.create_conference(room_id, config).await
    }
    
    /// Get detailed call information
    pub async fn get_call_details(&self, call_id: &CallId) -> Result<DetailedCallInfo> {
        self.inner.get_detailed_info(call_id).await
    }
    
    /// Inject custom SIP message
    pub async fn inject_message(&self, message: SipMessage) -> Result<()> {
        self.inner.inject_sip_message(message).await
    }
    
    /// Subscribe to raw SIP messages
    pub async fn subscribe_to_sip_messages(&self) -> SipMessageStream {
        self.inner.create_sip_stream().await
    }
    
    /// Get server metrics
    pub async fn metrics(&self) -> ServerMetrics {
        self.inner.collect_metrics().await
    }
}
```

## Common Types and Traits

```rust
// session-core/src/api/types.rs

/// Call identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallId(pub String);

/// Call state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallState {
    /// Initial state
    Idle,
    /// Outgoing call initiated
    Initiating,
    /// Received provisional response
    Proceeding,
    /// Ringing
    Ringing,
    /// Call connected
    Connected,
    /// Call on hold
    OnHold,
    /// Call being transferred
    Transferring,
    /// Call terminating
    Terminating,
    /// Call terminated
    Terminated,
}

/// Media information
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub remote_address: Option<SocketAddr>,
    pub codecs: Vec<CodecInfo>,
    pub direction: MediaDirection,
}

/// Media direction
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

/// End reason for calls
#[derive(Debug, Clone)]
pub enum EndReason {
    /// Normal hangup
    Normal,
    /// Call was cancelled
    Cancelled,
    /// Call was rejected
    Rejected(u16),
    /// Network error
    NetworkError(String),
    /// Media timeout
    MediaTimeout,
    /// Transfer completed
    Transferred,
    /// Server shutdown
    ServerShutdown,
    /// Custom reason
    Custom(String),
}

/// Registration credentials
#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub realm: Option<String>,
}

/// RTP packet for external processing
#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub payload: Vec<u8>,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
}

/// Call statistics
#[derive(Debug, Clone)]
pub struct CallStatistics {
    pub duration: Duration,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packet_loss: f32,
    pub jitter: f32,
    pub round_trip_time: Option<Duration>,
    pub mos_score: Option<f32>,
}

/// Media quality metrics
#[derive(Debug, Clone)]
pub struct MediaQuality {
    pub mos_score: f32,
    pub packet_loss_rate: f32,
    pub jitter_ms: f32,
    pub round_trip_ms: f32,
    pub codec: String,
    pub bitrate: u32,
}
```

## Event System

```rust
// session-core/src/api/events.rs

/// UAC events
#[derive(Debug, Clone)]
pub enum UacEvent {
    /// Incoming call received
    IncomingCall {
        call: IncomingCall,
        auto_answer: bool,
    },
    /// Call state changed
    CallStateChanged {
        call_id: CallId,
        old_state: CallState,
        new_state: CallState,
    },
    /// Media event
    MediaEvent {
        call_id: CallId,
        event: MediaEvent,
    },
    /// Registration status
    RegistrationChanged {
        status: RegistrationStatus,
        expires_in: Option<Duration>,
    },
    /// Error occurred
    Error {
        call_id: Option<CallId>,
        error: ClientError,
    },
}

/// UAS events
#[derive(Debug, Clone)]
pub enum UasEvent {
    /// New incoming call
    IncomingCall {
        call: IncomingCall,
        source: SocketAddr,
    },
    /// Call answered
    CallAnswered {
        call_id: CallId,
        sdp: String,
    },
    /// Call established
    CallEstablished {
        call_id: CallId,
        media_info: MediaInfo,
    },
    /// DTMF digit received
    DtmfReceived {
        call_id: CallId,
        digit: char,
    },
    /// Call ended
    CallEnded {
        call_id: CallId,
        reason: EndReason,
        stats: CallStatistics,
    },
    /// Server error
    ServerError {
        error: ServerError,
    },
}

/// Media events
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// Media started flowing
    MediaStarted,
    /// Media stopped
    MediaStopped,
    /// Codec changed
    CodecChanged { from: String, to: String },
    /// Packet loss detected
    PacketLoss { rate: f32 },
    /// Quality degraded
    QualityDegraded { mos: f32 },
    /// Quality recovered
    QualityRecovered { mos: f32 },
}
```

## Integration Examples

### Example: Simple Auto-Answer Server

```rust
use session_core::api::uas::SimpleUasServer;

#[tokio::main]
async fn main() -> Result<()> {
    // One line to create an auto-answer server
    let server = SimpleUasServer::always_accept("0.0.0.0:5060").await?;
    
    // Server runs until shutdown
    server.start().await?;
    Ok(())
}
```

### Example: Call Center with Advanced UAS

```rust
use session_core::api::uas::{AdvancedUasServer, CallController};

struct CallCenterController {
    agent_pool: Arc<AgentPool>,
    queue_manager: Arc<QueueManager>,
    routing_engine: Arc<RoutingEngine>,
}

#[async_trait]
impl CallController for CallCenterController {
    async fn pre_invite_check(&self, headers: &Headers, source: SocketAddr) -> PreInviteDecision {
        // Rate limiting
        if self.is_rate_limited(source) {
            return PreInviteDecision::RateLimit { 
                retry_after: Duration::from_secs(60) 
            };
        }
        
        // Blacklist check
        if self.is_blacklisted(source) {
            return PreInviteDecision::Reject(403);
        }
        
        PreInviteDecision::Continue
    }
    
    async fn on_invite(&self, request: InviteRequest) -> InviteResponse {
        // Extract routing information
        let skill = request.headers.get("X-Skill-Required");
        let language = request.headers.get("X-Language");
        let priority = request.headers.get("X-Priority").unwrap_or("normal");
        
        // Find best agent
        match self.routing_engine.find_agent(skill, language, priority).await {
            Some(agent) => {
                // Direct to agent
                InviteResponse::Accept {
                    provisional: ProvisionalResponse::Ringing,
                    ring_timeout: Duration::from_secs(20),
                    tag: Some(agent.id()),
                }
            }
            None => {
                // Queue the call
                let queue = self.queue_manager.select_queue(skill, language).await;
                let position = queue.add_call(request.call_id).await;
                
                InviteResponse::Queue {
                    queue_id: queue.id(),
                    position: Some(position),
                    early_media: Some(self.get_queue_music(&queue)),
                    provisional: ProvisionalResponse::SessionProgress,
                }
            }
        }
    }
    
    async fn on_call_established(&self, call: EstablishedCall, dialog: DialogHandle) {
        // Start recording
        dialog.start_recording().await;
        
        // Monitor quality
        dialog.subscribe_to_rtcp(|report| {
            if report.packet_loss > 0.05 {
                self.alert_supervisor(&call.id, "High packet loss");
            }
        });
        
        // Update agent state
        if let Some(agent_id) = call.tag {
            self.agent_pool.mark_busy(agent_id).await;
        }
    }
    
    async fn on_call_ended(&self, call_id: CallId, reason: EndReason, dialog: DialogInfo, stats: CallStatistics) {
        // Free up agent
        if let Some(agent_id) = dialog.tag {
            self.agent_pool.mark_available(agent_id).await;
        }
        
        // Store call record
        self.store_call_record(CallRecord {
            id: call_id,
            duration: stats.duration,
            quality: stats.mos_score,
            ended: reason,
        }).await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let controller = Arc::new(CallCenterController::new());
    
    let server = AdvancedUasServer::new(controller)
        .bind("0.0.0.0:5060")
        .await?;
    
    // Server handles all complexity
    server.start().await
}
```

### Example: SIP Client Using New UAC API

```rust
use session_core::api::uac::UacClient;
use audio_core::AudioDeviceManager;

pub struct SipClient {
    uac: Arc<UacClient>,
    audio: Arc<AudioDeviceManager>,
    pipelines: Arc<RwLock<HashMap<CallId, AudioPipeline>>>,
}

impl SipClient {
    pub async fn new(identity: &str) -> Result<Self> {
        // Use session-core for SIP
        let uac = UacClient::builder()
            .with_identity(identity)
            .build()
            .await?;
        
        // Audio stays in sip-client
        let audio = AudioDeviceManager::new().await?;
        
        Ok(Self {
            uac: Arc::new(uac),
            audio: Arc::new(audio),
            pipelines: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    pub async fn call(&self, uri: &str) -> Result<Call> {
        // Make call using session-core
        let call = self.uac.call(uri).await?;
        
        // Set up audio pipeline
        let pipeline = self.create_audio_pipeline(&call.id()).await?;
        self.pipelines.write().await.insert(call.id().clone(), pipeline);
        
        // Subscribe to RTP from session-core
        let rtp_stream = self.uac.subscribe_to_rtp(&call.id()).await?;
        
        // Connect audio pipeline to RTP
        self.connect_audio_to_rtp(pipeline, rtp_stream).await?;
        
        Ok(call)
    }
    
    // Audio controls remain in sip-client
    pub async fn set_microphone_volume(&self, call_id: &CallId, volume: f32) -> Result<()> {
        if let Some(pipeline) = self.pipelines.read().await.get(call_id) {
            pipeline.set_input_volume(volume).await
        } else {
            Err("Call not found")
        }
    }
    
    pub async fn enable_echo_cancellation(&self, enabled: bool) -> Result<()> {
        self.audio.set_echo_cancellation(enabled).await
    }
}
```

## Required Internal Changes

### Summary of What Already Exists vs What Needs Adding

| Feature | Internal Support | API Exposure | Work Required |
|---------|-----------------|--------------|---------------|
| **Make/Receive Calls** | ✅ Fully exists | ❌ Needs UAC/UAS API | API wrappers only |
| **Answer/Reject/Hangup** | ✅ Fully exists | ❌ Needs UAC/UAS API | API wrappers only |
| **Hold/Resume** | ✅ Via SDP direction | ❌ Needs UAC API | API wrappers only |
| **DTMF** | ✅ `send_dtmf()` exists | ❌ Needs UAC/UAS API | API wrappers only |
| **Transfer (REFER)** | ✅ `transfer_session()` exists | ❌ Needs UAC API | API wrappers only |
| **Call State Events** | ✅ Fully exists | ❌ Needs friendly events | Event adapters only |
| **SDP Negotiation** | ✅ Fully exists | ✅ Already exposed | Nothing needed |
| **Registration** | ⚠️ In dialog-core only | ❌ Needs UAC API | Wire up + API |
| **Audio Frame Access** | ✅ `MediaControl` trait with full implementation | ❌ Needs UAC/UAS API | API wrappers only |
| **Media Info** | ⚠️ Partial (missing remote addr) | ❌ Needs enhancement | Minor addition |

**Key Point:** Most features already exist internally and just need API wrappers!

### session-core Internal Changes

#### 1. Registration Support (coordinator/registration.rs)
```rust
// NEW FILE: session-core/src/coordinator/registration.rs
pub struct RegistrationManager {
    registrations: HashMap<String, RegistrationState>,
}

impl SessionCoordinator {
    // Add to coordinator/mod.rs
    pub async fn register(
        &self,
        uri: &str,
        credentials: Credentials,
        expires: u32
    ) -> Result<RegistrationHandle> {
        // Use dialog_manager to send REGISTER
        // Track registration state
        // Set up refresh timer
    }
    
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()> {
        // Send REGISTER with Expires: 0
    }
}
```
**Files to modify:**
- `src/coordinator/mod.rs` - Add registration manager
- `src/coordinator/registration.rs` - New file for registration logic
**Estimated LOC:** ~200

#### 2. Audio Frame Access - ALREADY EXISTS!
```rust
// ALREADY EXISTS in session-core:
// - src/api/media.rs::MediaControl trait
// - src/api/media.rs::subscribe_to_audio_frames() 
// - src/api/media.rs::send_audio_frame()
// - src/media/manager.rs - Already connected to media-core

// The MediaControl trait already provides:
impl MediaControl for Arc<SessionCoordinator> {
    async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<AudioFrameSubscriber>;
    async fn send_audio_frame(&self, session_id: &SessionId, frame: AudioFrame) -> Result<()>;
}

// client-core already uses these successfully!
// Just need to expose in UAC/UAS API layer
```
**Files to modify:**
- NONE! Already fully implemented
**Estimated LOC:** 0 (just use existing MediaControl trait!)

#### 3. Enhanced Media Info (media/manager.rs)
```rust
// MODIFY: session-core/src/media/manager.rs
pub async fn get_media_info(&self, session_id: &SessionId) -> Result<MediaInfo> {
    // Existing code...
    // ADD: Extract remote RTP address from session
    let remote_addr = session.remote_rtp_endpoint()?;
    // ADD: Include in MediaInfo
}
```
**Files to modify:**
- `src/media/manager.rs` - Enhance get_media_info
**Estimated LOC:** ~20

#### 4. DTMF Support - ALREADY EXISTS
```rust
// ALREADY EXISTS in session-core:
// - src/coordinator/session_ops.rs::send_dtmf()
// - src/manager/core.rs::send_dtmf()
// - src/dialog/manager.rs::send_dtmf()

// Just need to expose in UAC/UAS API layer:
impl UacClient {
    pub async fn send_dtmf(&self, call_id: &CallId, digits: &str) -> Result<()> {
        self.inner.coordinator.send_dtmf(&call_id.into(), digits).await
    }
}
```
**Files to modify:**
- `src/api/uac/standard.rs` - Add send_dtmf wrapper
- `src/api/uas/standard.rs` - Add DTMF event handler
**Estimated LOC:** ~20 (just API wrappers!)

### dialog-core Changes

#### 1. Expose REGISTER Support (api/unified.rs)
```rust
// MODIFY: dialog-core/src/api/unified.rs
impl UnifiedDialogApi {
    pub async fn register(
        &self,
        from_uri: &str,
        registrar: &str,
        credentials: Option<Credentials>,
        expires: u32
    ) -> ApiResult<RegistrationHandle> {
        // Use existing register_handler.rs
        self.manager.send_register(from_uri, registrar, credentials, expires).await
    }
    
    pub async fn unregister(&self, handle: RegistrationHandle) -> ApiResult<()> {
        self.manager.send_register_with_expires_zero(handle).await
    }
}
```
**Files to modify:**
- `src/api/unified.rs` - Add register/unregister methods
**Estimated LOC:** ~50

#### 2. In-Dialog Request Support (api/unified.rs)
```rust
// MODIFY: dialog-core/src/api/unified.rs
impl UnifiedDialogApi {
    pub async fn send_in_dialog_request(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<Bytes>
    ) -> ApiResult<Response> {
        // Already exists internally, just expose it
        self.manager.send_request(dialog_id, method, body).await
    }
}
```
**Files to modify:**
- `src/api/unified.rs` - Expose send_in_dialog_request
**Estimated LOC:** ~20

#### 3. Enhanced Dialog Handle (api/unified.rs)
```rust
// MODIFY: dialog-core/src/api/unified.rs
impl DialogHandle {
    pub fn dialog_id(&self) -> &DialogId {
        &self.dialog_id
    }
    
    pub async fn send_info(&self, body: String) -> ApiResult<()> {
        self.api.send_in_dialog_request(&self.dialog_id, Method::INFO, Some(body.into())).await
    }
}
```
**Files to modify:**
- `src/api/unified.rs` - Enhance DialogHandle
**Estimated LOC:** ~30

## Migration Strategy with Detailed Steps

### Phase 1: Core UAC API - No Internal Changes (Week 1)

**Goal:** Implement basic UAC functionality using only existing internals

#### Steps:
1. **Create UAC module structure**
   - [ ] Create `src/api/uac/mod.rs`
   - [ ] Create `src/api/uac/simple.rs`
   - [ ] Create `src/api/uac/builder.rs`
   - [ ] Create `src/api/uac/types.rs`

2. **Implement SimpleUacClient**
   - [ ] Implement `new()` using existing SessionCoordinator
   - [ ] Implement `call()` using `create_outgoing_call`
   - [ ] Implement `hangup_all()` using `terminate_session`
   - [ ] Add simple event stream wrapper

3. **Implement basic UacClient**
   - [ ] Create UacClientBuilder
   - [ ] Implement `call()`, `answer()`, `reject()`, `hangup()`
   - [ ] Implement `hold()` and `resume()` using SDP direction changes
   - [ ] Add UacEventHandler trait

4. **Testing**
   - [ ] Create unit tests for SimpleUacClient
   - [ ] Create integration test with mock SIP server
   - [ ] Verify compatibility with existing examples

**Deliverable:** Working UAC that can make/receive calls without any internal changes

### Phase 2: Core UAS API - No Internal Changes (Week 1)

**Goal:** Implement basic UAS functionality using only existing internals

#### Steps:
1. **Create UAS module structure**
   - [ ] Create `src/api/uas/mod.rs`
   - [ ] Create `src/api/uas/simple.rs`
   - [ ] Create `src/api/uas/builder.rs`
   - [ ] Create `src/api/uas/traits.rs`

2. **Implement SimpleUasServer**
   - [ ] Implement `always_accept()` using existing handlers
   - [ ] Implement `always_reject()` 
   - [ ] Implement `with_filter()` for basic routing
   - [ ] Wire up to existing SessionCoordinator

3. **Implement standard UasServer**
   - [ ] Create CallHandler trait
   - [ ] Implement UasServerBuilder
   - [ ] Wire up `on_incoming_call`, `on_call_established`, `on_call_ended`
   - [ ] Add server event stream

4. **Testing**
   - [ ] Create unit tests for SimpleUasServer
   - [ ] Test auto-answer functionality
   - [ ] Test with SIPp for compliance
   - [ ] Verify with existing session-core tests

**Deliverable:** Working UAS that can answer calls without any internal changes

### Phase 3: Add Registration Support (Week 2)

**Goal:** Add SIP REGISTER support to UAC

#### Steps:
1. **Add registration to session-core**
   - [ ] Create `src/coordinator/registration.rs`
   - [ ] Add RegistrationManager to SessionCoordinator
   - [ ] Implement registration state tracking
   - [ ] Add refresh timer logic

2. **Expose REGISTER in dialog-core**
   - [ ] Add `register()` to UnifiedDialogApi
   - [ ] Add `unregister()` to UnifiedDialogApi
   - [ ] Create RegistrationHandle type
   - [ ] Add authentication challenge handling

3. **Wire up in UAC API**
   - [ ] Add `register()` to UacClient
   - [ ] Add `unregister()` to UacClient
   - [ ] Add registration events to UacEventHandler
   - [ ] Add auto-refresh configuration

4. **Testing**
   - [ ] Test REGISTER with Kamailio
   - [ ] Test authentication challenges
   - [ ] Test registration refresh
   - [ ] Test unregister

**Deliverable:** UAC with full registration support

### Phase 4: Expose Audio Frame Access (Week 2)

**Goal:** Expose existing MediaControl audio frame methods in UAC/UAS APIs

#### Steps:
1. **Wire up existing MediaControl in UAC API**
   - [ ] Add `subscribe_to_audio_frames()` wrapper - delegates to existing MediaControl implementation
   - [ ] Add `send_audio_frame()` wrapper - delegates to existing MediaControl implementation
   - [ ] Add `get_audio_stream_config()` wrapper - already implemented in MediaControl
   - [ ] Add `set_audio_stream_config()` wrapper - already implemented in MediaControl
   - [ ] Add `start_audio_stream()` wrapper - already implemented in MediaControl
   - [ ] Add `stop_audio_stream()` wrapper - already implemented in MediaControl
   - [ ] Test with existing client-core patterns
   - [ ] **No new internal implementation needed - ALL methods already exist!**

2. **Enhance media info (minor change)**
   - [ ] Modify `get_media_info()` to include remote RTP address
   - [ ] Add codec information to MediaInfo
   - [ ] Add media direction to MediaInfo

3. **Update UAC API to use MediaControl**
   - [ ] Import MediaControl trait
   - [ ] Delegate to existing implementations in `src/api/media.rs`
   - [ ] Add convenience methods

4. **Testing**
   - [ ] Test audio frame streaming
   - [ ] Compare with client-core behavior
   - [ ] Verify no regression
   - [ ] Performance testing

**Deliverable:** UAC/UAS with audio frame access using existing MediaControl trait implementation

### Phase 5: Add Advanced Features (Week 3)

**Goal:** Expose existing DTMF, transfer, and advanced call control in API

#### Steps:
1. **Expose DTMF support (already exists internally)**
   - [ ] Add `send_dtmf()` wrapper to UacClient
   - [ ] Add DTMF received events to UasServer
   - [ ] Test with existing `dialog/manager.rs::send_dtmf()`
   - [ ] No internal implementation needed!

2. **Expose transfer support (already exists internally)**
   - [ ] Add `transfer()` wrapper to UacClient using existing `transfer_session()`
   - [ ] Add `attended_transfer()` using existing bridge functionality
   - [ ] Add transfer events from existing coordinator
   - [ ] Test with existing `coordinator/transfer.rs`

3. **Add AdvancedUacClient**
   - [ ] Create UacController trait
   - [ ] Implement custom SDP control
   - [ ] Add custom header support
   - [ ] Add provisional response handling

4. **Add AdvancedUasServer**
   - [ ] Create CallController trait
   - [ ] Implement pre-INVITE hooks
   - [ ] Add in-dialog request handling
   - [ ] Add RTCP monitoring hooks

5. **Testing**
   - [ ] Test DTMF with Asterisk
   - [ ] Test transfers
   - [ ] Test advanced features
   - [ ] Load testing

**Deliverable:** Full-featured UAC/UAS with all advanced capabilities

### Phase 6: Migrate sip-client (Week 3-4)

**Goal:** Update sip-client to use new session-core UAC API

#### Steps:
1. **Update dependencies**
   - [ ] Remove client-core dependency
   - [ ] Add session-core UAC imports
   - [ ] Update Cargo.toml

2. **Migrate code**
   - [ ] Replace ClientBuilder with UacClient::builder()
   - [ ] Update call handling to use new API
   - [ ] Update event handling
   - [ ] Keep audio pipeline unchanged

3. **Testing**
   - [ ] Run all sip-client tests
   - [ ] Test with real SIP servers
   - [ ] Test audio quality
   - [ ] Performance comparison

**Deliverable:** sip-client using session-core instead of client-core

### Phase 7: Simplify call-engine (Week 4)

**Goal:** Update call-engine to use new session-core UAS API

#### Steps:
1. **Migrate to AdvancedUasServer**
   - [ ] Replace custom SIP handling
   - [ ] Implement CallController
   - [ ] Wire up routing engine
   - [ ] Keep business logic unchanged

2. **Testing**
   - [ ] Test with SIPp scenarios
   - [ ] Load testing
   - [ ] Integration testing

**Deliverable:** Simplified call-engine using session-core UAS

### Phase 8: Deprecate client-core (Week 5)

**Goal:** Complete migration and archive client-core

#### Steps:
1. **Documentation**
   - [ ] Write migration guide
   - [ ] Update all examples
   - [ ] Update README files

2. **Final cleanup**
   - [ ] Move any remaining unique features
   - [ ] Archive client-core repository
   - [ ] Update workspace dependencies

**Deliverable:** client-core deprecated, all functionality in session-core

## Benefits Summary

1. **Developer Experience**
   - Simple: `SimpleUasServer::always_accept()` - one line
   - Standard: Implement one trait for common cases
   - Advanced: Full control when needed

2. **Performance**
   - Zero-cost abstractions
   - Direct access to internals when needed
   - No cross-crate overhead

3. **Maintainability**
   - Single source of truth for SIP/RTP
   - Clear separation of concerns
   - Consistent API patterns

4. **Flexibility**
   - Progressive disclosure of complexity
   - Mix and match API levels
   - Custom controllers for unique requirements

5. **Type Safety**
   - Compile-time guarantees
   - Impossible states unrepresentable
   - Clear error handling

## Conclusion

This design provides a unified, layered API that serves everyone from hobbyists to enterprise call centers. The key innovation is progressive disclosure - simple things are trivial, standard things are easy, and complex things are possible, all within the same crate.