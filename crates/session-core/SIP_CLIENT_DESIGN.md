# SipClient Trait Design for Session-Core

## Overview

This document describes the design and implementation of the `SipClient` trait, which extends session-core to support non-session SIP operations like REGISTER, OPTIONS, MESSAGE, and SUBSCRIBE.

## Motivation

Session-core currently focuses exclusively on SIP session management (calls). However, many SIP applications need to perform non-session operations:
- **REGISTER**: Endpoint registration with a SIP registrar
- **OPTIONS**: Capability discovery and keepalive
- **MESSAGE**: Instant messaging
- **SUBSCRIBE/NOTIFY**: Event subscriptions
- **PUBLISH**: Presence updates

These operations don't create dialogs and don't fit into session-core's current abstraction model.

## Design Principles

1. **Clean Separation**: Non-session operations are clearly separated from session operations
2. **Reuse Infrastructure**: Leverage existing transport and transaction layers from dialog-core
3. **Consistent API**: Follow session-core's existing patterns and conventions
4. **Type Safety**: Use Rust's type system to prevent misuse
5. **Async-First**: All operations are async following Tokio patterns

## API Design

### Core Trait

```rust
use async_trait::async_trait;
use std::time::Duration;
use crate::errors::Result;

/// Handle for tracking registration state
#[derive(Debug, Clone)]
pub struct RegistrationHandle {
    /// Transaction ID for the REGISTER
    pub transaction_id: String,
    /// Registration expiration in seconds
    pub expires: u32,
    /// Contact URI that was registered
    pub contact_uri: String,
    /// Registrar URI
    pub registrar_uri: String,
}

/// Response from a SIP request
#[derive(Debug, Clone)]
pub struct SipResponse {
    /// Status code (e.g., 200, 404, 401)
    pub status_code: u16,
    /// Reason phrase
    pub reason_phrase: String,
    /// Response headers
    pub headers: std::collections::HashMap<String, String>,
    /// Response body
    pub body: Option<String>,
}

/// Trait for non-session SIP operations
#[async_trait]
pub trait SipClient: Send + Sync {
    /// Send a REGISTER request
    /// 
    /// # Arguments
    /// * `registrar_uri` - The registrar server URI (e.g., "sip:registrar.example.com")
    /// * `from_uri` - The AOR being registered (e.g., "sip:alice@example.com")
    /// * `contact_uri` - Where to reach this endpoint (e.g., "sip:alice@192.168.1.100:5060")
    /// * `expires` - Registration duration in seconds (0 to unregister)
    /// 
    /// # Returns
    /// A handle tracking the registration or an error
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle>;
    
    /// Send an OPTIONS request (keepalive/capability query)
    /// 
    /// # Arguments
    /// * `target_uri` - The target to query
    /// 
    /// # Returns
    /// The OPTIONS response
    async fn send_options(&self, target_uri: &str) -> Result<SipResponse>;
    
    /// Send a MESSAGE request (instant message)
    /// 
    /// # Arguments
    /// * `to_uri` - Message recipient
    /// * `message` - Message content
    /// * `content_type` - MIME type (defaults to "text/plain")
    /// 
    /// # Returns
    /// The MESSAGE response
    async fn send_message(
        &self,
        to_uri: &str,
        message: &str,
        content_type: Option<&str>,
    ) -> Result<SipResponse>;
    
    /// Send a SUBSCRIBE request
    /// 
    /// # Arguments
    /// * `target_uri` - What to subscribe to
    /// * `event_type` - Event package (e.g., "presence", "dialog")
    /// * `expires` - Subscription duration in seconds
    /// 
    /// # Returns
    /// Subscription handle for managing the subscription
    async fn subscribe(
        &self,
        target_uri: &str,
        event_type: &str,
        expires: u32,
    ) -> Result<SubscriptionHandle>;
    
    /// Send a raw SIP request (advanced use)
    /// 
    /// # Arguments
    /// * `request` - Complete SIP request to send
    /// * `timeout` - Response timeout
    /// 
    /// # Returns
    /// The SIP response
    async fn send_raw_request(
        &self,
        request: rvoip_sip_core::Request,
        timeout: Duration,
    ) -> Result<SipResponse>;
}

/// Handle for managing subscriptions
#[derive(Debug, Clone)]
pub struct SubscriptionHandle {
    /// Subscription dialog ID
    pub dialog_id: String,
    /// Event type
    pub event_type: String,
    /// Expiration time
    pub expires_at: std::time::Instant,
}
```

### Implementation Strategy

The implementation will be added to `SessionCoordinator`:

```rust
// In coordinator/mod.rs
impl SipClient for Arc<SessionCoordinator> {
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle> {
        // Implementation details below
    }
    // ... other methods
}
```

## Implementation Details

### 1. Transport Access

We need to expose non-dialog request sending in dialog-core:

```rust
// In dialog-core UnifiedDialogApi
pub async fn send_non_dialog_request(
    &self,
    request: Request,
    destination: SocketAddr,
    timeout: Duration,
) -> ApiResult<Response> {
    // Use transaction manager directly
    let tx_id = self.transaction_manager
        .create_non_dialog_transaction(request, destination)
        .await?;
    
    // Wait for response with timeout
    self.transaction_manager
        .wait_for_response(tx_id, timeout)
        .await
}
```

### 2. REGISTER Implementation

```rust
async fn register(
    &self,
    registrar_uri: &str,
    from_uri: &str,
    contact_uri: &str,
    expires: u32,
) -> Result<RegistrationHandle> {
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::{TypedHeader, expires::Expires};
    
    // Parse registrar URI to get destination
    let uri: Uri = registrar_uri.parse()
        .map_err(|e| SessionError::InvalidUri(format!("Invalid registrar URI: {}", e)))?;
    
    let destination = resolve_sip_uri(&uri).await?;
    
    // Build REGISTER request
    let request = SimpleRequestBuilder::register(registrar_uri)?
        .from("", from_uri, Some(&generate_tag()))
        .to("", from_uri, None)
        .call_id(&generate_call_id())
        .cseq(1)
        .via(&self.get_local_address(), "UDP", Some(&generate_branch()))
        .contact(contact_uri, None)
        .header(TypedHeader::Expires(Expires::new(expires)))
        .max_forwards(70)
        .build();
    
    // Send via dialog-core
    let response = self.dialog_api
        .send_non_dialog_request(request, destination, Duration::from_secs(32))
        .await
        .map_err(|e| SessionError::internal(&format!("REGISTER failed: {}", e)))?;
    
    // Check response
    if response.status_code() != 200 {
        return Err(SessionError::ProtocolError {
            message: format!("REGISTER failed: {} {}", 
                response.status_code(), 
                response.reason_phrase().unwrap_or("Unknown"))
        });
    }
    
    Ok(RegistrationHandle {
        transaction_id: response.call_id()?.to_string(),
        expires,
        contact_uri: contact_uri.to_string(),
        registrar_uri: registrar_uri.to_string(),
    })
}
```

### 3. Event Handling

For SUBSCRIBE/NOTIFY support, we'll need to handle incoming NOTIFY messages:

```rust
// Extend SessionEvent enum
pub enum SessionEvent {
    // ... existing variants
    
    /// NOTIFY received for a subscription
    NotifyReceived {
        subscription_id: String,
        event_type: String,
        body: Option<String>,
    },
    
    /// MESSAGE received
    MessageReceived {
        from: String,
        to: String,
        body: String,
        content_type: String,
    },
}
```

## Usage Examples

### Basic Registration

```rust
use rvoip_session_core::api::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create session coordinator with SipClient support
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:agent@192.168.1.100")
        .enable_sip_client()  // New option
        .build()
        .await?;
    
    // Register with server
    let registration = coordinator.register(
        "sip:registrar.example.com",
        "sip:agent001@example.com",
        "sip:agent001@192.168.1.100:5060",
        3600  // 1 hour
    ).await?;
    
    println!("Registered with transaction ID: {}", registration.transaction_id);
    
    // Later, unregister
    let _ = coordinator.register(
        "sip:registrar.example.com",
        "sip:agent001@example.com",
        "sip:agent001@192.168.1.100:5060",
        0  // Unregister
    ).await?;
    
    Ok(())
}
```

### Sending Instant Messages

```rust
// Send an instant message
let response = coordinator.send_message(
    "sip:bob@example.com",
    "Hello from session-core!",
    Some("text/plain")
).await?;

if response.status_code == 200 {
    println!("Message delivered!");
}
```

### Subscribing to Events

```rust
// Subscribe to presence
let subscription = coordinator.subscribe(
    "sip:alice@example.com",
    "presence",
    3600
).await?;

// Handle incoming NOTIFY messages via event stream
let mut events = coordinator.subscribe_to_events().await;
while let Some(event) = events.recv().await {
    if let SessionEvent::NotifyReceived { event_type, body, .. } = event {
        if event_type == "presence" {
            println!("Presence update: {:?}", body);
        }
    }
}
```

## Integration Points

### 1. Dialog-Core Changes

We need to add to `UnifiedDialogApi`:
- `send_non_dialog_request()` method
- Access to transaction manager for non-dialog operations
- Event forwarding for MESSAGE and NOTIFY

### 2. Session-Core Changes

- Add `SipClient` trait to `api/mod.rs`
- Implement trait on `SessionCoordinator`
- Add configuration option to enable SIP client features
- Extend `SessionEvent` for non-session events

### 3. Builder Updates

Add to `SessionManagerBuilder`:
```rust
pub fn enable_sip_client(mut self) -> Self {
    self.config.enable_sip_client = true;
    self
}
```

## Testing Strategy

1. **Unit Tests**: Test request building and response parsing
2. **Integration Tests**: Test against mock SIP servers
3. **Example Programs**: Demonstrate each operation type
4. **Interop Testing**: Test with real SIP servers (Asterisk, Kamailio)

## Migration Path

1. **Phase 1**: Implement core trait and REGISTER support
2. **Phase 2**: Add OPTIONS and MESSAGE support
3. **Phase 3**: Add SUBSCRIBE/NOTIFY support
4. **Phase 4**: Advanced features (authentication, TLS, etc.)

## Future Enhancements

1. **Authentication**: Digest authentication support
2. **TLS Support**: Secure registration and messaging
3. **Batch Operations**: Register multiple contacts
4. **Event Packages**: Support various SUBSCRIBE event types
5. **Forking**: Handle multiple responses to MESSAGE/SUBSCRIBE 