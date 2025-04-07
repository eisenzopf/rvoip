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
mod config;
pub use config::{ClientConfig, CallConfig};

// Client implementation
mod client;
pub use client::{SipClient, SipClientEvent};

// User agent implementation
mod user_agent;
pub use user_agent::UserAgent;

// Call management
mod call;
pub use call::{Call, CallState, CallEvent, CallDirection, WeakCall};

/// Call history and registry for tracking and querying SIP calls
/// 
/// The `CallRegistry` provides storage for active calls and call history,
/// as well as methods to query and manage call records. It can optionally
/// persist call history between restarts.
/// 
/// # Basic Usage
/// 
/// ```
/// use rvoip_sip_client::{UserAgent, ClientConfig, CallRegistry, CallState, CallFilter};
/// use std::time::Duration;
/// 
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     // Create SIP client
///     let config = ClientConfig::new()
///         .with_username("user")
///         .with_domain("example.com")
///         .with_max_call_history(Some(100));
///     
///     let mut user_agent = UserAgent::new(config).await?;
///     user_agent.start().await?;
///     
///     // Get the call registry
///     let registry = user_agent.registry();
///     
///     // Get all established calls
///     let established_calls = registry.get_calls_by_state(CallState::Established).await;
///     println!("Currently {} established calls", established_calls.len());
///     
///     // Find a specific call by ID
///     if let Some(result) = registry.find_call_by_id("call-123").await {
///         println!("Found call: {} to {}", result.record.id, result.record.remote_uri);
///         
///         // If it's active, we can interact with it
///         if let Some(call) = result.active_call {
///             println!("Call is active in state: {}", call.state().await);
///             // Interact with the call directly
///         } else {
///             println!("Call is in history with state: {}", result.record.state);
///         }
///         
///         // We can always use the weak reference for memory-safe operations
///         if let Some(weak_call) = result.weak_call {
///             // Memory-safe interaction with even non-active calls
///             if let Err(e) = weak_call.hangup().await {
///                 println!("Unable to hang up: {}", e);
///             }
///         }
///     }
///     
///     // Get recent calls from the last hour
///     let recent_calls = registry.get_recent_calls(Duration::from_secs(3600)).await;
///     println!("Found {} calls in the last hour", recent_calls.len());
///     
///     // Get calls within a specific time range
///     use std::time::{SystemTime, Duration};
///     let yesterday = SystemTime::now().checked_sub(Duration::from_secs(24 * 3600)).unwrap();
///     let now = SystemTime::now();
///     let calls_today = registry.get_calls_in_time_range(yesterday, now).await;
///     println!("Found {} calls in the last 24 hours", calls_today.len());
///     
///     Ok(())
/// }
/// ```
///
/// # Statistics and Reporting
///
/// ```
/// use rvoip_sip_client::{UserAgent, ClientConfig, CallRegistry};
/// use std::time::{SystemTime, Duration};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let mut user_agent = UserAgent::new(ClientConfig::default()).await?;
///     let registry = user_agent.registry();
///     
///     // Generate statistics for all calls
///     let stats = registry.calculate_statistics().await;
///     println!("Total calls: {}", stats.total_calls);
///     println!("Incoming calls: {}, Outgoing calls: {}", 
///         stats.incoming_calls, stats.outgoing_calls);
///     println!("Missed calls: {}", stats.missed_calls);
///     println!("Average call duration: {:?}", stats.average_duration);
///     
///     // Generate statistics for calls in the last hour
///     let hourly_stats = registry.calculate_recent_statistics(Duration::from_secs(3600)).await;
///     println!("Calls in the last hour: {}", hourly_stats.total_calls);
///     println!("Failed calls in the last hour: {}", hourly_stats.failed_calls);
///     
///     // Generate statistics for custom date range
///     let last_week = SystemTime::now().checked_sub(Duration::from_secs(7 * 24 * 3600)).unwrap();
///     let now = SystemTime::now();
///     let weekly_stats = registry.calculate_statistics_in_time_range(last_week, now).await;
///     println!("Weekly call statistics:");
///     println!("  Total calls: {}", weekly_stats.total_calls);
///     println!("  Total talk time: {:?}", weekly_stats.total_duration);
///     println!("  Longest call: {:?}", weekly_stats.max_duration);
///     
///     Ok(())
/// }
/// ```
// Call history and registry
mod call_registry;
pub use call_registry::{CallRegistry, CallRecord, CallStateRecord, CallFilter, CallStatistics, CallLookupResult, SerializableCallLookupResult};

// Media handling
pub mod media;
pub use media::{MediaSession, MediaType};

/// Version of the crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION"); 