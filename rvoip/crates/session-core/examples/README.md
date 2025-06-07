# Session-Core Examples

This directory contains practical, end-to-end examples demonstrating how to use the `rvoip-session-core` library to build SIP applications. Each example is self-contained and can be run independently.

## Quick Start

To run any example:

```bash
# From the session-core directory
cargo run --example <example_name>

# For examples requiring multiple processes
cargo run --example <example_name> -- --help
```

## Basic Examples

### 1. **simple_peer_to_peer** - Basic P2P SIP Call
**File:** `simple_peer_to_peer.rs`

Two SIP clients that establish a direct peer-to-peer session and exchange audio. This example demonstrates:
- Creating SIP clients without a server
- Making outgoing calls
- Accepting incoming calls 
- Basic media session setup
- Call termination

```bash
# Terminal 1 (callee)
cargo run --example simple_peer_to_peer -- --role callee --port 5061

# Terminal 2 (caller)  
cargo run --example simple_peer_to_peer -- --role caller --target 127.0.0.1:5061
```

### 2. **sip_server_basic** - Simple SIP Server
**File:** `sip_server_basic.rs`

A basic SIP server that allows multiple clients to connect and make calls through it. Features:
- Auto-accepting incoming calls
- Session management
- Basic call routing
- Multiple concurrent sessions

```bash
# Start the server
cargo run --example sip_server_basic

# Use SIP clients to connect to localhost:5060
```

### 3. **two_clients_via_server** - Two Clients Through Server
**File:** `two_clients_via_server.rs`

Complete example showing two SIP clients connecting through a SIP server and establishing a call. Demonstrates:
- Server-mediated call setup
- Audio exchange through server
- Call lifecycle management
- End-to-end session coordination

```bash
cargo run --example two_clients_via_server
```

## Advanced Examples

### 4. **conference_bridge** - Multi-Party Conference
**File:** `conference_bridge.rs`

A conference bridge allowing multiple participants to join the same call. Features:
- Conference room creation
- Dynamic participant addition/removal
- Audio mixing simulation
- Conference controls (mute, kick, etc.)

```bash
cargo run --example conference_bridge -- --room-id 1234
```

### 5. **call_hold_transfer** - Hold and Transfer Operations
**File:** `call_hold_transfer.rs`

Demonstrates advanced call control features:
- Putting calls on hold
- Resuming held calls
- Transferring calls to other parties
- Managing multiple concurrent sessions

```bash
cargo run --example call_hold_transfer
```

### 6. **media_negotiation** - Advanced Media Features
**File:** `media_negotiation.rs`

Shows advanced media capabilities:
- Multiple codec negotiation
- Media parameter modification
- DTMF handling
- SDP offer/answer processing

```bash
cargo run --example media_negotiation
```

## Testing and Integration Examples

### 7. **sipp_integration** - SIPp Testing Integration
**File:** `sipp_integration.rs`

A SIP server designed to work with SIPp test scenarios. Demonstrates:
- SIPp-compatible SIP message handling
- Automated testing integration
- Performance testing support
- Various SIP protocol scenarios

```bash
# Start the test server
cargo run --example sipp_integration

# In another terminal, run SIPp tests
cd sipp_scenarios
./run_tests.sh
```

### 8. **stress_test_server** - High-Load Testing
**File:** `stress_test_server.rs`

A robust SIP server for stress testing:
- High concurrent call capacity
- Resource monitoring
- Performance metrics
- Load balancing capabilities

```bash
cargo run --example stress_test_server -- --max-calls 1000 --metrics-port 8080
```

## Dependencies and Setup

All examples use the session-core library and require:

- Rust 1.70+
- Network access for SIP communication
- Audio device access for media examples (optional)

### External Tools (Optional)

For testing and validation:

- **SIPp**: For automated testing scenarios
- **Wireshark**: For packet analysis
- **netcat/socat**: For low-level debugging

## Common Usage Patterns

### Basic Session Manager Setup

```rust
use rvoip_session_core::prelude::*;

let session_manager = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(Arc::new(AutoAnswerHandler))
    .build()
    .await?;
```

### Making Outgoing Calls

```rust
let call = make_call_with_manager(
    &session_manager,
    "sip:alice@example.com",
    "sip:bob@example.com"
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

## Testing Your Examples

Each example includes basic error handling and logging. To see detailed debug information:

```bash
RUST_LOG=debug cargo run --example <example_name>
```

For network debugging, use Wireshark to capture SIP traffic on the relevant ports (usually 5060-5065).

## Contributing

When adding new examples:

1. Include comprehensive comments explaining the concepts
2. Add command-line argument parsing for flexibility
3. Include proper error handling
4. Update this README with the new example
5. Test with real SIP clients when possible

## Troubleshooting

### Common Issues

- **Port already in use**: Change the SIP port with `--port` argument
- **Permission denied**: Some examples may need elevated privileges for network binding
- **No audio**: Check audio device permissions and codec compatibility
- **Connection timeout**: Verify firewall settings and network connectivity

### Getting Help

For more detailed information about the session-core API, see:
- Library documentation: `cargo doc --open`
- API examples in `src/api/examples.rs`
- Integration tests in `tests/` 