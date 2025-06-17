# Session-Core API Improvements for UAS Support

## Current Issues

1. **Direct internal access**: UAS examples access `coordinator.media_manager` directly
2. **Inconsistent imports**: Mixed use of direct imports vs API module imports
3. **Missing abstractions**: Some UAS operations aren't cleanly exposed through the API

## Recommended API Improvements

### 1. Add missing methods to MediaControl trait

```rust
pub trait MediaControl {
    // Existing methods...
    
    /// Create a media session without generating SDP
    /// Useful when you need to prepare media before SDP negotiation
    async fn create_media_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Update media session with remote SDP (without starting transmission)
    /// Separate from establish_media_flow for cases where you parse SDP first
    async fn update_remote_sdp(&self, session_id: &SessionId, remote_sdp: &str) -> Result<()>;
    
    /// Generate SDP answer based on received offer
    /// For UAS scenarios where you need to respond to an offer
    async fn generate_sdp_answer(&self, session_id: &SessionId, offer: &str) -> Result<String>;
}
```

### 2. Hide internal implementation details

Make `SessionCoordinator` fields private to prevent direct access:
```rust
pub struct SessionCoordinator {
    // These should be private, not pub(crate)
    media_manager: Arc<MediaSessionManager>,
    dialog_manager: Arc<DialogManager>,
    // ...
}
```

### 3. Update examples to use API consistently

All examples should import exclusively from the API:
```rust
// Good - single API import
use rvoip_session_core::api::{
    SessionCoordinator, SessionManagerBuilder, MediaControl,
    CallHandler, CallSession, CallState, IncomingCall, CallDecision,
};

// Bad - mixed imports
use rvoip_session_core::{SessionCoordinator, SessionManagerBuilder};
use rvoip_session_core::api::{CallHandler, CallSession};
```

### 4. Consider a dedicated UAS builder pattern

For complex UAS configurations:
```rust
pub struct UasServerBuilder {
    // UAS-specific configuration options
}

impl UasServerBuilder {
    pub fn with_auto_answer(mut self, enabled: bool) -> Self { ... }
    pub fn with_max_concurrent_calls(mut self, max: usize) -> Self { ... }
    pub fn with_codec_preferences(mut self, codecs: Vec<String>) -> Self { ... }
}
```

## Benefits

1. **Consistent API usage**: Both UAC and UAS can use the same clean API
2. **Better encapsulation**: Internal details are hidden
3. **Clearer examples**: Show best practices for library usage
4. **Future-proof**: Can change internals without breaking user code

## Migration Path

1. Add new MediaControl methods
2. Update examples to use API methods
3. Mark direct field access as deprecated
4. Eventually make fields private in a major version update 