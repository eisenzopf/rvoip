# Session-Core Examples

This directory contains practical, end-to-end examples demonstrating how to use the `rvoip-session-core` library to build SIP applications. Examples are organized into subdirectories by use case.

## Quick Start

To run any example:

```bash
# From the session-core directory
cargo run --example <example_name>

# For examples requiring multiple processes or scripts
cd examples/<subdirectory>
./run_test.sh  # or specific script
```

## Example Categories

### 1. **peer-to-peer/** - Direct P2P Communication

#### simple_peer_to_peer.rs
Direct peer-to-peer SIP session between two endpoints without a server. This example demonstrates:
- Creating SIP endpoints that communicate directly
- Making and receiving calls without a central server
- Basic media session setup
- Clean session termination

```bash
# Terminal 1 (callee)
cargo run --example simple_peer_to_peer -- --role callee --port 5061

# Terminal 2 (caller)  
cargo run --example simple_peer_to_peer -- --role caller --target 127.0.0.1:5061
```

### 2. **client-server/** - Client-Server Architecture

#### uac_client.rs & uas_server.rs
Classic SIP User Agent Client (UAC) and User Agent Server (UAS) implementation:
- **uas_server.rs**: SIP server that accepts incoming calls
- **uac_client.rs**: SIP client that makes outgoing calls
- Demonstrates proper client-server SIP communication
- Shows session management through a server

```bash
# Start the server first
cargo run --example uas_server

# In another terminal, run the client
cargo run --example uac_client

# Or use the provided test script
cd examples/client-server
./run_test.sh
```

### 3. **api_best_practices/** - Clean API Usage

#### uac_client_clean.rs & uas_server_clean.rs
Demonstrates best practices for using the session-core API:
- Clean separation of concerns
- Proper error handling
- Efficient resource management
- Production-ready patterns

```bash
cd examples/api_best_practices
./run_clean_examples.sh
```

### 4. **sipp_tests/** - SIPp Integration Testing

Contains SIPp XML scenarios and scripts for automated testing:
- Various SIP call flow scenarios
- Performance testing configurations
- Compatibility testing with industry-standard tools

```bash
cd examples/sipp_tests
# Follow the README in that directory for SIPp testing
```

## Common Usage Patterns

### Basic SessionCoordinator Setup

```rust
use rvoip_session_core::prelude::*;

let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(Arc::new(MyCallHandler))
    .build()
    .await?;

SessionControl::start(&coordinator).await?;
```

### Making Outgoing Calls

```rust
let session = coordinator.create_outgoing_call(
    "sip:alice@example.com",
    "sip:bob@example.com",
    Some(sdp_offer)
).await?;
```

### Handling Incoming Calls

```rust
#[derive(Debug)]
struct MyHandler;

#[async_trait::async_trait]
impl CallHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("Incoming call from {}", call.from);
        CallDecision::Accept
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call {} ended: {}", call.id(), reason);
    }
}
```

### Bridge Management (2-party Conference)

```rust
// Bridge two active sessions
let bridge_id = coordinator.bridge_sessions(&session1_id, &session2_id).await?;

// Monitor bridge events
let mut events = coordinator.subscribe_to_bridge_events().await;
while let Some(event) = events.recv().await {
    match event {
        BridgeEvent::ParticipantAdded { bridge_id, session_id } => {
            println!("Session {} joined bridge {}", session_id, bridge_id);
        }
        BridgeEvent::ParticipantRemoved { bridge_id, session_id } => {
            println!("Session {} left bridge {}", session_id, bridge_id);
        }
        BridgeEvent::BridgeDestroyed { bridge_id } => {
            println!("Bridge {} destroyed", bridge_id);
        }
    }
}
```

## Architecture Overview

Session-core uses a **SessionCoordinator** as the central hub that:
- Manages SIP sessions and their lifecycle
- Integrates with dialog-core for dialog management
- Coordinates with media-core for RTP streams
- Provides bridge management for 2-party conferences
- Offers a unified API for both client and server use cases

## Dependencies and Setup

All examples require:
- Rust 1.70+
- Network access for SIP communication
- Optional: Audio device access for media examples

### External Tools (Optional)

For testing and validation:
- **SIPp**: For automated testing scenarios (see sipp_tests/)
- **Wireshark**: For packet analysis
- **netcat/socat**: For low-level debugging

## Testing Your Examples

Each example includes logging support. To see detailed debug information:

```bash
RUST_LOG=debug cargo run --example <example_name>
```

For network debugging, use Wireshark to capture SIP traffic on the relevant ports (usually 5060-5065).

## Directory Structure

```
examples/
├── README.md                    # This file
├── peer-to-peer/               # Direct P2P examples
│   └── simple_peer_to_peer.rs
├── client-server/              # Classic UAC/UAS examples
│   ├── uac_client.rs
│   ├── uas_server.rs
│   ├── run_test.sh
│   └── README.md
├── api_best_practices/         # Clean API usage examples
│   ├── uac_client_clean.rs
│   ├── uas_server_clean.rs
│   ├── run_clean_examples.sh
│   └── README.md
└── sipp_tests/                 # SIPp integration tests
    └── [various .xml scenarios]
```

## Contributing

When adding new examples:
1. Place in appropriate subdirectory or create a new category
2. Include comprehensive comments explaining the concepts
3. Add proper error handling
4. Update this README
5. Test with real SIP clients when possible
6. Consider adding a run script if multiple processes are needed

## Troubleshooting

### Common Issues

- **Port already in use**: Change the SIP port with `--port` argument
- **Permission denied**: Some examples may need elevated privileges
- **No audio**: Check audio device permissions and codec compatibility
- **Connection timeout**: Verify firewall settings

### Getting Help

For more detailed information:
- Library documentation: `cargo doc --open`
- Integration tests in `tests/`
- Main project README at the repository root 