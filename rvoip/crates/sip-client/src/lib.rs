/*!
# RVOIP SIP Client Library

This crate provides a high-level client library for the RVOIP SIP stack. It simplifies the process
of creating SIP user agents, making and receiving calls, and managing media sessions.

## Features

- SIP client creation and management
- Registration with SIP registrars
- Making outgoing calls
- Receiving incoming calls
- Media session management with RTP/RTCP
- Audio streaming with G.711 and other codecs

## Usage Examples

### Basic SIP Client Setup

```rust
use rvoip_sip_client::{SipClient, ClientConfig};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a client configuration
    let config = ClientConfig::new()
        .with_username("alice")
        .with_domain("example.com")
        .with_local_addr("127.0.0.1:5060".parse()?);
    
    // Create a SIP client
    let mut client = SipClient::new(config).await?;
    
    // Register with a SIP server
    client.register("sip.example.com:5060".parse()?).await?;
    
    // Wait for incoming calls or make outgoing calls
    client.run().await
}
```

### Making an Outgoing Call

```rust
use rvoip_sip_client::{SipClient, ClientConfig, CallConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up client
    let config = ClientConfig::new()
        .with_username("alice")
        .with_domain("example.com")
        .with_local_addr("127.0.0.1:5060".parse()?);
    
    let mut client = SipClient::new(config).await?;
    
    // Make a call
    let call_config = CallConfig::default()
        .with_audio(true)
        .with_dtmf(true);
    
    let call = client.call("sip:bob@example.com", call_config).await?;
    
    // Handle the call
    call.wait_until_established().await?;
    println!("Call established!");
    
    // End the call after 30 seconds
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    call.hangup().await?;
    
    Ok(())
}
```

### Receiving Calls

```rust
use rvoip_sip_client::{SipClient, ClientConfig, CallEvent};
use futures::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up client
    let config = ClientConfig::new()
        .with_username("bob")
        .with_domain("example.com")
        .with_local_addr("127.0.0.1:5060".parse()?);
    
    let mut client = SipClient::new(config).await?;
    
    // Get a stream of call events
    let mut events = client.event_stream();
    
    // Process incoming calls
    while let Some(event) = events.next().await {
        match event {
            CallEvent::IncomingCall(call) => {
                println!("Incoming call from {}", call.caller_id());
                call.answer().await?;
                
                // Do something with the call
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                call.hangup().await?;
            }
            // Handle other events
            _ => {}
        }
    }
    
    Ok(())
}
```
*/

// Re-export error types
pub mod error;
pub use error::{Error, Result};

// Configuration
pub mod config;
pub use config::{ClientConfig, CallConfig};

// Client implementation
mod client;
pub use client::{SipClient, SipClientEvent};

// User agent implementation
mod user_agent;
pub use user_agent::UserAgent;

// Call management
mod call;
pub use call::{Call, CallState, CallEvent, CallDirection};

// Media handling
pub mod media;
pub use media::MediaSession;

/// Version of the crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION"); 