# Session Manager Developer Scenarios

This directory contains practical examples showing how to build real-world SIP applications using the session-core simple API.

## 🚀 Quick Start

Each scenario demonstrates the ultra-simple pattern:

```rust
// For SIP Servers: Implement CallHandler trait
session_manager.set_call_handler(Arc::new(MyHandler)).await?;

// For SIP Clients: Use make_call() method  
let call = session_manager.make_call(from, to, None).await?;

// For Peer-to-Peer: Direct client-to-client calling
client_a.call_peer("sip:clientB@192.168.1.100:5060").await?;

// For Coordination: Use simple methods
session_manager.bridge_calls(call1.id(), call2.id()).await?;
```

## 📞 SIP Server Scenarios
*Handle incoming calls by implementing the `CallHandler` trait*

| # | Scenario | Description | Use Case | Example File |
|---|----------|-------------|----------|--------------|
| **01** | **Auto-Answer Server** | Automatically answers every incoming call | Testing, simple endpoints | `01_auto_answer_server.rs` |
| **02** | **Voicemail Server** | Records messages, hangs up after 30 seconds | Basic voicemail system | `02_voicemail_server.rs` |
| **03** | **Call Screening** | Only accepts calls from allowed numbers | Security, private lines | `03_call_screening.rs` |
| **04** | **Business Hours** | Accepts calls 9-5, rejects after hours | Office phone systems | `04_business_hours.rs` |
| **05** | **Conference Bridge** | Connects all callers together | Group meetings, conferences | `05_conference_bridge.rs` |
| **06** | **Call Queue/ACD** | Queues calls for available agents | Customer service centers | `06_call_queue_acd.rs` |
| **07** | **Multi-Tenant Server** | Handles calls for multiple companies | SaaS phone providers | `07_multi_tenant_server.rs` |

## 📱 SIP Client Scenarios  
*Make outgoing calls using the `make_call()` method*

| # | Scenario | Description | Use Case | Example File |
|---|----------|-------------|----------|--------------|
| **08** | **Simple Call Client** | Makes basic outgoing calls | Basic softphone functionality | `08_simple_call_client.rs` |
| **09** | **Auto-Dialer** | Automatically dials a list of numbers | Telemarketing, notifications | `09_auto_dialer.rs` |
| **10** | **Softphone Client** | Full softphone with call management | Desktop/mobile SIP client | `10_softphone_client.rs` |
| **11** | **Load Testing Client** | Generates high call volume | Performance testing | `11_load_testing_client.rs` |
| **12** | **Call Quality Monitor** | Monitors and measures call quality | Network diagnostics | `12_call_quality_monitor.rs` |
| **13** | **Emergency Dialer** | High-priority emergency calling | Safety systems | `13_emergency_dialer.rs` |
| **14** | **Callback Service** | Schedules and manages callbacks | Customer service | `14_callback_service.rs` |

## 🤝 Peer-to-Peer SIP Client Scenarios
*Direct client-to-client calling without servers*

| # | Scenario | Description | Use Case | Example File |
|---|----------|-------------|----------|--------------|
| **15** | **Peer-to-Peer Direct Call** | Simple peer-to-peer calling between two clients | Direct communication, gaming | `15_peer_to_peer_direct.rs` |
| **16** | **Mesh Network Communication** | Clients in a mesh that can discover each other | Distributed teams, IoT devices | `16_mesh_network.rs` |
| **17** | **Distributed Softphone Network** | Multiple softphones with contact management | Decentralized phone systems | `17_distributed_softphone.rs` |

## 🌉 Coordination Features

All scenarios can use these simple coordination methods:

- **`bridge_calls()`** - Connect two calls together
- **`set_call_priority()`** - Set call priority (Emergency, High, Normal, Low)
- **`create_group()`** - Group related calls together
- **`get_resource_usage()`** - Monitor system resources
- **`active_calls()`** - List all active calls

## 🚀 Usage

Each example can be run individually to demonstrate specific functionality:

```bash
# Server examples (handle incoming calls)
cargo run --example 01_auto_answer_server
cargo run --example 02_voicemail_server  
cargo run --example 05_conference_bridge

# Client examples (make outgoing calls)
cargo run --example 08_simple_call_client sip:user@localhost
cargo run --example 09_auto_dialer sip:dialer@localhost numbers.csv
cargo run --example 10_softphone_client alice localhost password

# P2P examples (direct communication)
cargo run --example 15_peer_to_peer_direct 192.168.1.100:5060 "Alice"
```

## 🧩 Simple API Design

All examples use the same simple, developer-friendly interface:

```rust
// Create session manager
let session_manager = SessionManager::new(config).await?;

// For servers: set call handler
session_manager.set_call_handler(Arc::new(MyHandler)).await?;
session_manager.start_server("0.0.0.0:5060").await?;

// For clients: make calls
let call = session_manager.make_call(from, to, options).await?;
call.on_answered(|call| async { /* handle */ }).await;
```

## 📂 Files

### Individual Examples (01-17)
- **`01_auto_answer_server.rs`** through **`17_distributed_softphone.rs`** - Individual scenario implementations
- **`bridge_two_calls.rs`** - Detailed example of call bridging with coordination  
- **`developer_scenarios.rs`** - Combined implementations of all 17 scenarios
- **`README.md`** - This overview (you are here)

## 🎯 Implementation Status

- ✅ **Fully Implemented**: Examples 01-10, 15 (core server/client/P2P functionality)
- 🚧 **Stubs Created**: Examples 11-14, 16-17 (advanced features - coming soon!)

The implemented examples demonstrate the complete session-core API and can be used as starting points for real applications.

## 🎯 Choosing Your Scenario

**Building a SIP Server?** → Pick from the **Server Scenarios** and implement `CallHandler`

**Building a SIP Client?** → Pick from the **Client Scenarios** and use `make_call()`

**Building peer-to-peer communication?** → Pick from the **Peer-to-Peer Scenarios** for direct client calling

**Need call coordination?** → Use the **Coordination Features** with any scenario

## 💡 Example Usage

```rust
use rvoip_session_core::api::simple::*;

// Auto-answer server (simplest possible)
struct MyServer;
impl CallHandler for MyServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Answering call from {}", call.from());
        CallAction::Answer
    }
}

// Set up and run
let config = SessionConfig::default();
let session_manager = SessionManager::new(config).await?;
session_manager.set_call_handler(Arc::new(MyServer)).await?;
session_manager.start_server("0.0.0.0:5060").await?;
```

**That's it!** 🎉 Your SIP application is running in just a few lines of code.

**Each scenario shows the pattern for:**
- 📞 **Servers**: Implement CallHandler trait → handle incoming calls
- 📱 **Clients**: Use make_call() → handle outgoing calls  
- 🤝 **Peer-to-Peer**: Direct client calling → no server needed
- 🌉 **Coordination**: Use simple methods → bridge, group, prioritize

---

*For complete implementations and detailed examples, see `developer_scenarios.rs`* 