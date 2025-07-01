# Call Engine - Proof of Concept Call Center Library

A **working proof-of-concept call center library** that builds on [session-core](../session-core) to provide essential call center functionality. While it has limited features compared to commercial solutions, it's **fully functional** and demonstrates all the core components needed to build a complete call center system.

## 🎯 Current Status: **Fully Working Basic Call Center**

**✅ What's Working Now:**
- **Agent Registration**: SIP-based agent registration and management
- **Incoming Call Routing**: Customer calls automatically routed to available agents
- **Queue Management**: Database-backed call queuing with overflow handling
- **Round-Robin Load Balancing**: Fair distribution of calls across agents
- **B2BUA Call Bridging**: Proper two-way audio between customers and agents
- **Agent Status Management**: Available/Busy/Offline state tracking
- **Call Termination**: Clean call cleanup and resource management
- **Database Persistence**: SQLite-compatible storage with atomic operations
- **End-to-End Testing**: Complete test suite with SIPp scenarios

**🔧 Recently Fixed:**
- Database integration with proper schema
- BYE message routing and timeouts
- Configuration management (no hardcoded IPs)
- Race condition fixes in queue management
- Event-driven architecture throughout

## Architecture Overview

```
┌─────────────────┐  ┌──────────────────┐  ┌─────────────────┐
│  Customer Calls │  │   Agent Apps     │  │  Admin Tools    │
│  (SIP Phones)   │  │  (Softphones)    │  │  (Monitoring)   │
└────────┬────────┘  └────────┬─────────┘  └────────┬────────┘
         │                    │                     │
    ┌────▼────────┐    ┌──────▼──────┐       ┌──────▼──────┐
    │   SIP       │    │    Agent    │       │    Queue    │
    │  Transport  │    │ Registration│       │ Management  │
    └────────┬────┘    └──────┬──────┘       └──────┬──────┘
             └──────────────────┼─────────────────────┘
                               │
                    ┌──────────▼──────────┐
                    │  CallCenterEngine   │
                    │  - Call routing     │
                    │  - Agent management │
                    │  - Queue processing │
                    └──────────┬──────────┘
                               │
                    ┌──────────▼──────────┐
                    │    session-core     │
                    │  - SIP handling     │
                    │  - RTP/Media        │
                    │  - Call bridging    │
                    └─────────────────────┘
```

## Quick Start

### Prerequisites

```toml
[dependencies]
rvoip-call-engine = { path = "../call-engine" }
rvoip-session-core = { path = "../session-core" }
tokio = { version = "1.0", features = ["full"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

### Basic Call Center Server

```rust
use rvoip_call_engine::prelude::*;
use tracing_subscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Create call center with default configuration
    let engine = CallCenterEngine::new(CallCenterConfig::default()).await?;
    
    println!("🏢 Call Center Server starting on port 5060...");
    
    // Start the server (will run indefinitely)
    engine.run().await?;
    
    Ok(())
}
```

### Agent Application

```rust
use rvoip_client_core::prelude::*;

#[tokio::main] 
async fn main() -> anyhow::Result<()> {
    // Create SIP client for agent
    let config = ClientConfig {
        sip_uri: "sip:alice@127.0.0.1".to_string(),
        server_uri: "sip:127.0.0.1:5060".to_string(),
        local_port: 5071,
        ..Default::default()
    };
    
    let client = ClientManager::new(config).await?;
    
    // Register with call center
    client.register().await?;
    println!("👤 Agent Alice registered and ready for calls");
    
    // Handle incoming calls automatically
    client.run().await?;
    
    Ok(())
}
```

### Testing with SIPp

The E2E test suite demonstrates a **complete working call center** with real SIP calls and audio bridging:

**What it demonstrates:**
- ✅ SIP-based agent registration (Alice & Bob register as agents)
- ✅ Customer calls routed through the call center server
- ✅ Fair load balancing (calls distributed between agents)
- ✅ B2BUA call bridging with full audio flow
- ✅ Database-backed queue management and agent status
- ✅ Clean call termination and resource cleanup

**What it simulates:**
- **Customers**: SIPp generates 5 test calls to `sip:support@127.0.0.1`
- **Call Center**: Server receives calls, manages queues, routes to agents
- **Agents**: Two agent applications (Alice/Bob) handle incoming calls
- **Network**: All SIP/RTP traffic captured via tcpdump for analysis

**Limitations tested:**
- ❌ No IVR menu navigation (calls go directly to agents)
- ❌ No call recording verification
- ❌ No supervisor features or monitoring
- ❌ Basic round-robin routing only

```bash
cd examples/e2e_test
./run_e2e_test.sh
```

**Expected Result**: 5 customer calls distributed between Alice and Bob (typically 3/2 or 2/3 split), with successful call completion and clean BYE message handling.

For detailed test setup, troubleshooting, and analysis instructions, see the [E2E Test README](./examples/e2e_test/README.md).

## Current Feature Set

### ✅ Core Features (Working)

- **Agent Management**
  - SIP REGISTER-based agent registration
  - Real-time status tracking (Available/Busy/Offline)
  - Automatic status transitions during calls
  - Fair round-robin call distribution

- **Call Processing**
  - Incoming call reception and routing
  - Queue-based call distribution
  - B2BUA call bridging with proper media flow
  - Clean call termination and resource cleanup

- **Queue Management** 
  - Database-backed call queuing
  - Configurable queue timeouts and capacities
  - Atomic assignment operations
  - Overflow handling and re-queuing

- **Database Integration**
  - SQLite storage
  - Agent status persistence
  - Call history tracking
  - Atomic operations for consistency

### 🚧 Limitations (Not Yet Implemented)

- **No IVR System**: Calls go directly to agents (no menu navigation)
- **No Call Recording**: Audio is not recorded or stored
- **No Supervisor Features**: No monitoring, whisper, or barge-in
- **No REST API**: Management only via code (no web interface)
- **Basic Routing**: Only round-robin (no skills-based routing)
- **No Reporting**: Limited metrics and analytics
- **Single-Tenant**: No multi-tenant support

## Configuration

```rust
use rvoip_call_engine::config::*;

let config = CallCenterConfig {
    general: GeneralConfig {
        domain: "call-center.local".to_string(),
        local_ip: "127.0.0.1".to_string(),
        port: 5060,
        bye_timeout_seconds: 15,
        ..Default::default()
    },
    database: DatabaseConfig {
        url: "sqlite:call_center.db".to_string(),
        max_connections: 5,
        ..Default::default()
    },
    ..Default::default()
};

let engine = CallCenterEngine::new(config).await?;
```

## Examples

The [examples](./examples) directory contains:

- **`call_center_server.rs`**: Complete call center server implementation
- **`agent_client.rs`**: Agent application for handling calls
- **`e2e_test/`**: End-to-end testing with SIPp scenarios

## What Can You Build?

Despite its limitations, call-engine provides a solid foundation for:

### ✅ **Small Call Centers (5-50 agents)**
- Basic inbound call handling
- Agent queue management
- Simple call distribution
- Call center server deployment

### ✅ **Proof-of-Concept Systems**
- Demonstrate SIP call center concepts
- Test call routing algorithms
- Prototype custom call flows
- Educational and learning projects

### ✅ **Development Platform**
- Build IVR systems on top
- Add custom routing logic
- Integrate with external systems
- Extend with REST APIs

## Future Roadmap

See [TODO.md](./TODO.md) for the comprehensive development plan, including:

- **Phase 1**: IVR system with DTMF handling
- **Phase 2**: Skills-based routing and advanced queuing
- **Phase 3**: Call recording and supervisor features  
- **Phase 4**: REST API and web interfaces
- **Phase 5**: Production scaling and monitoring
- **Phase 6**: Enterprise features and integrations

**Estimated Timeline**: 5-6 months for full production readiness

## Contributing

This is a proof-of-concept library under active development. Key areas where contributions are welcome:

1. **IVR System Implementation** - DTMF handling and menu navigation
2. **REST API Development** - Management and monitoring interfaces
3. **Advanced Routing** - Skills-based and intelligent routing
4. **Testing and Documentation** - More examples and test scenarios
5. **Performance Optimization** - Scaling and resource management

## Dependencies

Built on top of the RVOIP ecosystem:
- **[session-core](../session-core)**: SIP session management and call bridging
- **[dialog-core](../dialog-core)**: SIP dialog state management  
- **[sip-core](../sip-core)**: SIP message parsing and generation
- **[rtp-core](../rtp-core)**: RTP media handling

## License

See the main RVOIP project license.

---

**TL;DR**: This is a **working call center** that can route customer calls to agents with proper queuing and load balancing. It's perfect for small deployments, learning, and as a foundation for building more advanced call center systems. 