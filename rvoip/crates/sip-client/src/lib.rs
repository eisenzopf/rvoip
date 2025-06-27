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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client with local configuration for testing
    let config = Config::new()
        .with_credentials("alice", "password", "127.0.0.1")
        .with_local_port(0); // Use random port
        
    let client = SipClient::new(config).await?;
    
    // In a real application, you would register with a SIP server:
    // client.register().await?;
    // println!("Registered successfully!");
    
    println!("SIP client created successfully");
    Ok(())
}
```

### Receiving Calls

```rust
use rvoip_sip_client::{SipClient, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client with local configuration
    let config = Config::new()
        .with_credentials("bob", "password", "127.0.0.1")
        .with_local_port(0);
        
    let mut client = SipClient::new(config).await?;
    
    // In a real application, you would:
    // 1. Register with a SIP server
    // 2. Wait for incoming calls in a loop
    // 3. Handle calls appropriately
    
    println!("SIP client ready to receive calls");
    
    // For demonstration, just show the API structure:
    // while let Some(incoming) = client.next_incoming_call().await {
    //     println!("📞 Incoming call from {}", incoming.caller());
    //     let call = incoming.answer().await?;
    //     // Handle the call...
    // }
    
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure as call center agent with local test settings
    let config = Config::new()
        .with_credentials("agent1", "password", "127.0.0.1")
        .with_call_engine("127.0.0.1:8080");
        
    let agent = SipClient::new(config).await?;
    
    // In a real call center environment, you would:
    // 1. Register with the SIP server
    // 2. Register as an available agent
    // 3. Handle assigned calls from the call engine
    
    println!("Call center agent ready");
    
    // Example API structure (commented to avoid hanging in tests):
    // agent.register().await?;
    // agent.register_as_agent("support_queue").await?;
    // while let Some(assigned_call) = agent.next_assigned_call().await {
    //     assigned_call.answer().await?;
    //     // Handle customer interaction...
    // }
    
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