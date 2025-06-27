# SIP REGISTER Demo

This demo showcases the SIP REGISTER functionality with session-core's new `SipClient` trait.

## Overview

The demo consists of:
- **server.rs**: CallCenterEngine server that receives and processes REGISTER requests
- **client.rs**: Client using session-core's SipClient API to send REGISTER requests
- **run_demo.sh**: Shell script that runs both server and client

## Architecture

```
Client (session-core SipClient)
    ↓
    └─→ register() method
            ↓
        SessionCoordinator
            ↓
        [TODO: dialog-core transport]
            ↓
        Network (UDP:5061 → UDP:5060)
            ↓
Server (CallCenterEngine)
    ↓
    └─→ dialog-core → session-core → CallCenterEngine
            ↓
        SipRegistrar processes registration
            ↓
        Response sent back through stack
```

## Running the Demo

```bash
cd rvoip/crates/call-engine
./examples/register_demo/run_demo.sh
```

## What It Demonstrates

1. **SipClient API**: Clean, high-level API for non-session SIP operations
2. **REGISTER Flow**: Complete registration lifecycle (register, refresh, unregister)
3. **Event Handling**: How REGISTER events flow through the stack
4. **Response Generation**: Proper SIP responses with status codes and headers

## Current Status

✅ **Completed**:
- SipClient trait defined in session-core
- Client demo uses session-core API exclusively
- Server properly handles REGISTER without auto-response
- Registration state tracked by SipRegistrar
- Proper SIP responses sent

⚠️ **Pending**:
- dialog-core needs to expose non-dialog request sending
- Complete implementation of register() method
- Authentication support (401/407 challenges)
- Other SipClient methods (OPTIONS, MESSAGE, SUBSCRIBE)

## Implementation Details

### SipClient Trait

```rust
#[async_trait]
pub trait SipClient: Send + Sync {
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle>;
    
    async fn send_options(&self, target_uri: &str) -> Result<SipResponse>;
    async fn send_message(&self, to_uri: &str, message: &str, content_type: Option<&str>) -> Result<SipResponse>;
    async fn subscribe(&self, target_uri: &str, event_type: &str, expires: u32) -> Result<SubscriptionHandle>;
    async fn send_raw_request(&self, request: Request, timeout: Duration) -> Result<SipResponse>;
}
```

### Usage Example

```rust
let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5061)
    .enable_sip_client()  // Enable SIP client features
    .build()
    .await?;

// Register with server
let registration = coordinator.register(
    "sip:registrar.example.com",
    "sip:alice@example.com",
    "sip:alice@192.168.1.100:5060",
    3600  // 1 hour
).await?;

// Later, unregister
coordinator.register(
    "sip:registrar.example.com",
    "sip:alice@example.com",  
    "sip:alice@192.168.1.100:5060",
    0  // expires=0 means unregister
).await?;
```

## Next Steps

1. **Implement dialog-core transport access**:
   ```rust
   // In dialog-core UnifiedDialogApi
   pub async fn send_non_dialog_request(
       &self,
       request: Request,
       destination: SocketAddr,
       timeout: Duration,
   ) -> ApiResult<Response>
   ```

2. **Complete register() implementation**:
   - Use dialog-core to send the actual REGISTER request
   - Wait for and parse the response
   - Handle error cases properly

3. **Add authentication support**:
   - Handle 401/407 challenges
   - Compute digest authentication
   - Retry with credentials

4. **Implement remaining methods**:
   - OPTIONS for keepalive/capability discovery
   - MESSAGE for instant messaging
   - SUBSCRIBE/NOTIFY for presence
   - Raw request sending for advanced use

## Design Philosophy

The SipClient trait extends session-core's capabilities beyond session management to support the full range of SIP operations. This provides:

- **Clean separation**: Session operations vs. non-session operations
- **Consistent API**: Follows session-core patterns
- **Type safety**: Rust's type system prevents misuse
- **Extensibility**: Easy to add new SIP methods

See [SIP_CLIENT_DESIGN.md](../../../SIP_CLIENT_DESIGN.md) for the complete design documentation. 