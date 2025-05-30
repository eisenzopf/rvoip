/*!
# RVOIP SIP Client

A simple, clean SIP client library and CLI tool built on the robust RVOIP infrastructure.

## Features

- **Simple API**: Clean, intuitive interface for SIP operations
- **Robust Foundation**: Built on proven `client-core` infrastructure
- **CLI Tool**: Ready-to-use command-line interface
- **Call-Engine Integration**: Seamless integration with RVOIP call center
- **Real-World Focus**: Designed for actual SIP communication scenarios

## Quick Start

### Library Usage

```rust
use rvoip_sip_client::{SipClient, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create and configure client
    let config = Config::new()
        .with_credentials("alice", "password", "sip.example.com")
        .with_local_port(5060);
        
    let mut client = SipClient::new(config).await?;
    
    // Register with SIP server
    client.register().await?;
    println!("Registered successfully!");
    
    // Make a call
    let call = client.call("sip:bob@example.com").await?;
    call.wait_for_answer().await?;
    println!("Call connected!");
    
    // Keep call active for 30 seconds
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    call.hangup().await?;
    
    Ok(())
}
```

### Receiving Calls

```rust
use rvoip_sip_client::{SipClient, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = SipClient::new(Config::default()).await?;
    client.register().await?;
    
    println!("Waiting for incoming calls...");
    while let Some(incoming) = client.next_incoming_call().await {
        println!("📞 Incoming call from {}", incoming.caller());
        
        // Auto-answer (or you could prompt user)
        incoming.answer().await?;
        println!("✅ Call answered!");
        
        // Handle the call...
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        incoming.hangup().await?;
    }
    
    Ok(())
}
```

### CLI Tool

```bash
# Register with SIP server
rvoip-sip-client register alice password sip.example.com

# Make a call
rvoip-sip-client call sip:bob@example.com

# Wait for incoming calls
rvoip-sip-client receive

# Check status
rvoip-sip-client status
```

## Call-Engine Integration

```rust
use rvoip_sip_client::{SipClient, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Configure as call center agent
    let config = Config::new()
        .with_credentials("agent1", "password", "callcenter.com")
        .with_call_engine("127.0.0.1:8080");
        
    let mut agent = SipClient::new(config).await?;
    agent.register().await?;
    
    // Register as available agent
    agent.register_as_agent("support_queue").await?;
    
    // Handle assigned calls from call-engine
    while let Some(assigned_call) = agent.next_assigned_call().await {
        println!("📞 Call assigned from call center");
        assigned_call.answer().await?;
        // Handle customer interaction...
    }
    
    Ok(())
}
```
*/

// Re-export key types for convenience
pub use rvoip_client_core::{
    CallId, CallState,
    call::CallDirection,
    client::ClientStats as CoreStats,
    RegistrationConfig, RegistrationStatus
};

// Public API modules
mod client;
mod call;
mod config;
mod error;
mod events;

// Re-export main types
pub use client::SipClient;
pub use call::{Call, IncomingCall};
pub use config::Config;
pub use error::{Error, Result};
pub use events::{SipEvent, CallEvent};

// CLI module (internal)
pub mod cli;

/// Version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default SIP port
pub const DEFAULT_SIP_PORT: u16 = 5060;

/// Default SIPS (secure) port
pub const DEFAULT_SIPS_PORT: u16 = 5061; 