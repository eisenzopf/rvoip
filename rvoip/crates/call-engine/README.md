# Call Engine - Advanced Call Center with Session-Core Integration

This crate provides enterprise-grade call center orchestration functionality with deep integration into session-core for SIP/RTP handling and a clean API layer for different user types.

## Architecture

The call-engine now follows a clean architecture with session-core as its only direct dependency:

```
┌─────────────────┐  ┌──────────────────┐  ┌─────────────────┐
│ Agent Apps      │  │ Supervisor Apps  │  │ Admin Apps      │
└────────┬────────┘  └────────┬─────────┘  └────────┬────────┘
         │                    │                      │
    ┌────▼────────┐    ┌──────▼────────┐    ┌──────▼────────┐
    │CallCenterClient│  │SupervisorApi  │    │AdminApi       │
    └────────┬────────┘ └──────┬────────┘    └──────┬────────┘
             └──────────────────┼────────────────────┘
                               │
                    ┌──────────▼──────────┐
                    │  CallCenterEngine   │
                    │  - CallHandler impl │
                    │  - Event processors │
                    └──────────┬──────────┘
                               │
                    ┌──────────▼──────────┐
                    │    session-core     │
                    │  - SIP handling     │
                    │  - RTP/Media        │
                    │  - Transport        │
                    └─────────────────────┘
```

## Features

### Core Capabilities
- 🎯 **Session-Core Integration**: CallHandler implementation for incoming calls
- 📞 **Real-Time Events**: Call state, media quality, DTMF, and warnings
- 🌉 **Call Bridging**: Automatic agent-customer call bridging
- 🗄️ **Limbo Database**: Modern async SQLite-compatible storage
- 🔌 **Clean API Layer**: Type-safe APIs for agents, supervisors, and admins

### Agent Management
- 👥 **Registration**: SIP-based agent registration with skills
- 📊 **Status Tracking**: Available, busy, break, offline states
- 🎯 **Skill Routing**: Match calls to agents based on skills
- 📈 **Performance**: Track agent performance metrics

### Call Processing
- 📋 **Smart Queuing**: Priority-based queues with overflow
- 🚦 **Routing Engine**: Business rules and skill-based routing
- 📊 **Real-Time Monitoring**: Live call and queue statistics
- 🎙️ **Quality Tracking**: MOS scores and packet loss monitoring

## Quick Start

### Add Dependencies

```toml
[dependencies]
rvoip-call-engine = { path = "../call-engine" }
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
```

### Basic Usage - Agent Application

```rust
use rvoip_call_engine::{
    prelude::*,
    api::{CallCenterClient, CallCenterClientBuilder},
    agent::{Agent, AgentId, AgentStatus},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Build the client
    let client = CallCenterClientBuilder::new()
        .with_config(CallCenterConfig::default())
        .with_database(CallCenterDatabase::new_in_memory().await?)
        .build()
        .await?;
    
    // Register an agent
    let agent = Agent {
        id: AgentId::from("alice-001"),
        sip_uri: "sip:alice@callcenter.local".to_string(),
        display_name: "Alice Smith".to_string(),
        skills: vec!["english".to_string(), "sales".to_string()],
        max_concurrent_calls: 3,
        status: AgentStatus::Available,
        department: Some("sales".to_string()),
        extension: Some("1001".to_string()),
    };
    
    let session_id = client.register_agent(&agent).await?;
    println!("Agent registered with session: {}", session_id);
    
    // Update status
    client.update_agent_status(&agent.id, AgentStatus::Available).await?;
    
    // Check queue stats
    let stats = client.get_queue_stats().await?;
    for (queue, info) in stats {
        println!("Queue {}: {} calls", queue, info.total_calls);
    }
    
    Ok(())
}
```

### Supervisor Monitoring

```rust
use rvoip_call_engine::api::SupervisorApi;

// Create supervisor API
let supervisor = SupervisorApi::new(engine);

// Get real-time statistics
let stats = supervisor.get_stats().await;
println!("Active calls: {}", stats.active_calls);
println!("Available agents: {}", stats.available_agents);

// Monitor specific agent
let agent_calls = supervisor.monitor_agent_calls(&agent_id).await;

// Force assign a queued call
supervisor.force_assign_call(session_id, agent_id).await?;

// Get performance metrics
let metrics = supervisor.get_performance_metrics(start_time, end_time).await;
println!("Service level: {:.1}%", metrics.service_level_percentage);
```

### Administrative Management

```rust
use rvoip_call_engine::api::AdminApi;

// Create admin API
let admin = AdminApi::new(engine);

// Add new agent
admin.add_agent(agent).await?;

// Update agent skills
admin.update_agent_skills(&agent_id, vec![
    AgentSkill { skill_name: "spanish".into(), skill_level: 4 }
]).await?;

// Create queue
admin.create_queue("priority_support").await?;

// Check system health
let health = admin.get_system_health().await;
println!("System status: {:?}", health.status);
```

## Event Handling

The call-engine implements all CallHandler trait methods:

### Core Events
- `on_incoming_call` - Route incoming calls to agents or queues
- `on_call_ended` - Clean up resources and update agent status
- `on_call_established` - Track bridged calls

### New Real-Time Events
- `on_call_state_changed` - Track call lifecycle transitions
- `on_media_quality` - Monitor MOS scores and packet loss
- `on_dtmf` - Handle IVR and feature codes
- `on_media_flow` - Track media stream status
- `on_warning` - System-level alerts and warnings

## Examples

### Running Examples

```bash
cd rvoip/crates/call-engine/examples

# Agent registration with new API
cargo run --example agent_registration_demo

# Basic call flow with all APIs
cargo run --example phase0_basic_call_flow

# Supervisor monitoring dashboard
cargo run --example supervisor_monitoring_demo

# Database integration
cargo run --example call_center_with_database
```

## Database Schema

### Core Tables
- **agents** - Agent profiles, skills, and status
- **call_records** - Complete call history and metrics
- **call_queues** - Queue configuration and policies
- **routing_policies** - Dynamic routing rules
- **agent_skills** - Skill assignments and proficiency

## Performance Optimization

- **Async Architecture**: Non-blocking operations throughout
- **Connection Pooling**: Efficient database connections
- **Event-Driven**: React to changes without polling
- **Minimal Dependencies**: Only depends on session-core
- **Zero-Copy**: Efficient data handling

## Testing

```bash
# Run all tests
cargo test

# Run with logging
RUST_LOG=debug cargo test

# Run specific test
cargo test test_api_layer
```

## Migration from Direct SIP Usage

If you're migrating from direct SIP library usage:

1. **Remove Dependencies**: Remove sip-core, rtp-core, etc.
2. **Use CallCenterClient**: Replace manual SIP handling
3. **Implement CallHandler**: For custom call processing
4. **Use Event Callbacks**: Replace polling with events
5. **Leverage APIs**: Use appropriate API for your user type

## Architecture Benefits

### Clean Separation
- **API Layer**: Type-safe interfaces for different users
- **Session Abstraction**: No direct SIP/RTP handling needed
- **Event-Driven**: Real-time updates without polling
- **Modular Design**: Easy to extend and maintain

### Scalability
- **Async-First**: Handle thousands of concurrent calls
- **Efficient Routing**: O(1) agent lookups
- **Queue Management**: Prevent system overload
- **Database Backed**: Persistent state across restarts

## Future Enhancements

- **WebSocket Events**: Real-time browser dashboards
- **Recording Integration**: Call recording with session-core
- **Advanced Analytics**: ML-based routing optimization
- **Multi-Tenant**: Isolated call center instances
- **High Availability**: Distributed architecture support

---

For more examples and documentation, see the [examples](./examples) directory. 