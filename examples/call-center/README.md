# Call Center Demo

This example demonstrates a complete call center system using the RVOIP libraries. It showcases how customers can call a support line, get routed to available agents, and establish bidirectional RTP media sessions.

## Overview

The demo consists of three main components:

- **Call Center Server** (`server.rs`) - Uses `call-engine` to handle incoming calls and route them to agents
- **Agents** (`agent.rs`) - Use `client-core` to register with the server and handle customer calls
- **Customer** (`customer.rs`) - Uses `client-core` to call the support line

## Architecture

```
┌─────────────────┐    REGISTER         ┌──────────────────────┐
│  Agent Alice    │ sip:alice@127.0.0.1 │                      │
│  (Port 5071)    ├────────────────────►│   Call Center Server │
└─────────────────┘                     │    (0.0.0.0:5060)   │
                                        │                      │
┌─────────────────┐    REGISTER         │  • Routes calls      │
│   Agent Bob     │ sip:bob@127.0.0.1   │  • Manages queues    │
│  (Port 5072)    ├────────────────────►│  • Bridges audio    │
└─────────────────┘                     └──────────┬───────────┘
                                                   │
┌─────────────────┐      INVITE                   │
│    Customer     │  sip:support@127.0.0.1        │
│  (Port 5080)    ├───────────────────────────────┘
└─────────────────┘    
```

## Demo Flow

1. **Server** starts and listens on port 5060
2. **Agent Alice** registers as `sip:alice@127.0.0.1`
3. **Agent Bob** registers as `sip:bob@127.0.0.1`
4. **Customer** calls `sip:support@127.0.0.1`
5. **Server** routes the call to an available agent
6. **Agent** answers and establishes media session
7. **Customer** and **Agent** exchange RTP audio for ~12 seconds
8. **Agent** hangs up automatically
9. **Customer** completes and shows statistics

## Quick Start

### Prerequisites

- Rust 1.70+
- `cargo` build tool
- Local network access (uses localhost)

### Running the Demo

```bash
# Navigate to the call-center directory
cd examples/call-center

# Make the script executable
chmod +x run_demo.sh

# Run the complete demo
./run_demo.sh
```

The script will automatically:
- Build all components
- Start the call center server
- Start two agents (Alice and Bob)
- Execute a customer call
- Monitor progress and generate reports
- Clean up all processes

### Expected Output

```
🏢 RVOIP Call Center Demo
===============================

🔨 Building call center components...
✅ Build successful

🏢 Starting Call Center Server...
✅ Call center server is ready

👩‍💼 Starting Agent Alice...
✅ Agent Alice is ready

👨‍💼 Starting Agent Bob...  
✅ Agent Bob is ready

👤 Starting Customer Call...

📋 Demo Flow:
   1. Customer calls sip:support@127.0.0.1
   2. Call center server receives the call
   3. Server routes call to available agent (Alice or Bob)
   4. Agent accepts and handles the call
   5. Customer and agent exchange RTP media
   6. Agent hangs up after 12 seconds
   7. Customer completes after 15 seconds

⏳ Waiting for demo to complete (about 20 seconds)...

📊 Demo Results:
==================================
✅ Customer completed successfully

📞 Call Routing:
✅ Customer successfully connected to an agent
✅ Alice handled 1 call(s)

🎵 Media Exchange:
✅ RTP media exchange successful

🎉 CALL CENTER DEMO SUCCESSFUL!
   ✅ Customer connected to agent
   ✅ Call routed successfully
   ✅ Media exchanged successfully
   ✅ Call completed cleanly
```

## Components

### Call Center Server

**File**: `src/server.rs`  
**Port**: 5060  
**Features**:
- Accepts SIP REGISTER from agents
- Receives calls to `sip:support@127.0.0.1`
- Routes calls to available agents
- Manages call queues
- Bridges customer and agent audio

**Key Configuration**:
```rust
let mut config = CallCenterConfig::default();
config.general.local_signaling_addr = "0.0.0.0:5060".parse()?;
config.general.domain = "127.0.0.1".to_string();
config.agents.default_max_concurrent_calls = 1;
```

### Agent

**File**: `src/agent.rs`  
**Default Ports**: Alice (5071), Bob (5072)  
**Features**:
- Registers with call center server
- Auto-accepts incoming calls
- Handles calls for configurable duration
- Provides detailed logging

**Usage**:
```bash
# Start Alice agent
cargo run --bin agent -- --name alice --port 5071 --call-duration 10

# Start Bob agent  
cargo run --bin agent -- --name bob --port 5072 --call-duration 10
```

### Customer

**File**: `src/customer.rs`  
**Default Port**: 5080  
**Features**:
- Calls the support line (`sip:support@127.0.0.1`)
- Establishes media session
- Reports RTP statistics
- Configurable call duration

**Usage**:
```bash
# Make a customer call
cargo run --bin customer -- --name customer --call-duration 15
```

## Manual Testing

You can run components individually for testing:

### Terminal 1: Start Server
```bash
cargo run --bin server
```

### Terminal 2: Start Agent Alice
```bash
cargo run --bin agent -- --name alice --port 5071
```

### Terminal 3: Start Agent Bob
```bash
cargo run --bin agent -- --name bob --port 5072
```

### Terminal 4: Make Customer Call
```bash
cargo run --bin customer -- --call-duration 20
```

## Configuration Options

### Server Configuration

The server uses minimal configuration for the demo:
- **Domain**: `127.0.0.1` (localhost)
- **SIP Port**: `5060`
- **Database**: In-memory SQLite
- **Queue Timeout**: 60 seconds
- **Ring Timeout**: 10 seconds

### Agent Configuration

Agents can be customized with command-line options:

```bash
cargo run --bin agent -- \
    --name alice \
    --server 127.0.0.1:5060 \
    --port 5071 \
    --call-duration 15
```

### Customer Configuration

Customers support several options:

```bash
cargo run --bin customer -- \
    --name customer \
    --server 127.0.0.1:5060 \
    --port 5080 \
    --call-duration 20 \
    --wait-time 5
```

## Generated Logs

The demo creates comprehensive logs in the `logs/` directory:

### Primary Logs

- **`server_stdout.log`** - Call center server activity
- **`alice_stdout.log`** - Alice agent detailed events
- **`bob_stdout.log`** - Bob agent detailed events  
- **`customer_stdout.log`** - Customer call activity
- **`call_flow.log`** - Combined timeline of all events

### Log Content Examples

**Agent Registration**:
```
[alice] ✅ Registration active: sip:alice@127.0.0.1
[alice] 👂 Agent alice ready to receive calls!
```

**Customer Call**:
```
[customer] 📞 Calling call center support line...
[customer] 🔔 Call is ringing... waiting for agent to answer
[customer] ✅ Connected to agent! Starting media session...
```

**Call Routing**:
```
[alice] 📞 Incoming call from sip:customer@127.0.0.1:5080
[alice] ✅ Accepting call call-123
[alice] 🎉 Call call-123 connected! Starting media...
```

## Network Configuration

The demo uses the following port allocation:

| Component | SIP Port | Media Port Range |
|-----------|----------|------------------|
| Server    | 5060     | N/A              |
| Alice     | 5071     | 6071-6171        |
| Bob       | 5072     | 6072-6172        |
| Customer  | 5080     | 7080-7180        |

All components bind to `0.0.0.0` and communicate via `127.0.0.1`.

## Technical Details

### SIP Configuration

- **Codecs**: PCMU (G.711 μ-law) and PCMA (G.711 A-law)
- **RTP Payload**: 160 bytes per packet (20ms @ 8kHz)
- **Packet Rate**: ~50 packets/second per direction
- **Registration Expiry**: 300 seconds (agents)

### Media Configuration

Both agents and customers use identical media settings:
```rust
MediaConfig {
    preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
    dtmf_enabled: true,
    echo_cancellation: false,
    noise_suppression: false,
    auto_gain_control: false,
    ..Default::default()
}
```

## Troubleshooting

### Common Issues

**Port conflicts:**
```
Error: Address already in use (os error 48)
```
- Solution: Kill any processes using the ports or change port configuration

**Agent registration fails:**
```
❌ Registration failed: timeout
```
- Check if server is running and accessible
- Verify network connectivity to 127.0.0.1:5060
- Ensure no firewall blocking connections

**No call routing:**
```
❌ Customer failed to connect to an agent
```
- Verify agents are registered and showing as available
- Check server logs for routing decisions
- Ensure agents are not busy with other calls

**Media exchange fails:**
```
❌ RTP media exchange failed
```
- Check media port availability
- Verify RTP port ranges don't conflict
- Review media session establishment in logs

### Debug Mode

For more detailed logging:

```bash
# Enable debug logging for all components
RUST_LOG=debug ./run_demo.sh

# Or for specific libraries
RUST_LOG=rvoip_call_engine=debug,rvoip_client_core=debug ./run_demo.sh
```

### Manual Debugging

Check process status:
```bash
# See what's listening on SIP ports
lsof -i :5060
lsof -i :5071  
lsof -i :5072

# Check for running demo processes
ps aux | grep call-center-demo
```

## Extending the Demo

### Adding More Agents

Start additional agents with unique names and ports:
```bash
cargo run --bin agent -- --name charlie --port 5073
cargo run --bin agent -- --name diana --port 5074
```

### Multiple Customers

Run multiple customer calls simultaneously:
```bash
# Terminal 1
cargo run --bin customer -- --name customer1 --port 5081

# Terminal 2  
cargo run --bin customer -- --name customer2 --port 5082
```

### Custom Call Scenarios

Modify the customer to:
- Add longer call durations
- Implement DTMF tones
- Add call transfer scenarios
- Test hold/resume functionality

### Performance Testing

Scale the demo for performance testing:
- Increase number of concurrent agents
- Generate multiple simultaneous calls
- Monitor resource usage and call quality
- Test queue overflow scenarios

## Integration with Other Examples

This call center demo can be combined with:
- **peer-to-peer** example for direct agent-to-agent calls
- **session-core** examples for advanced SDP negotiation
- **media-core** examples for audio processing features

## Next Steps

1. **Review the logs** - Understand the call flow and SIP signaling
2. **Modify configurations** - Experiment with different timeouts and codecs
3. **Add features** - Implement call transfer, conference calling, or IVR
4. **Scale testing** - Try multiple agents and concurrent calls
5. **Custom scenarios** - Create specific call center use cases

The minimal codebase makes it easy to understand and extend for real-world call center applications. 