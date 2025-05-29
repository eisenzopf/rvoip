# Bridge Tests for Session-Core

This directory contains comprehensive bridge testing infrastructure that validates the multi-session bridging capabilities of session-core.

## Overview

The bridge tests demonstrate and validate:
- **Real SIP call establishment** through transaction-core and dialog-core
- **Automatic bridge creation** when multiple calls are active
- **RTP packet forwarding** between bridged sessions
- **Bridge lifecycle management** (creation, monitoring, destruction)
- **Complete integration** of all session-core components

## Architecture Tested

```
Client A ──INVITE──> Bridge Server ←──INVITE── Client B
         <─180 Ring─                ─180 Ring─>
         <─200 OK───                ─200 OK──>
         ──ACK────>                <────ACK──
         ~~RTP~~~ Bridge Created ~~~RTP~~~
         
Audio Flow: Client A ↔ Bridge Server ↔ Client B
```

## Files

- **`bridge_server.rs`** - Example SIP server with automatic bridge logic
- **`run_bridge_tests.sh`** - Comprehensive test orchestration script
- **`BRIDGE_TESTS.md`** - This documentation

## Quick Start

### Prerequisites

```bash
# Install SIPp (macOS)
brew install sipp

# Or on Ubuntu/Debian
sudo apt-get install sipp

# Install audio tools (optional, for enhanced testing)
brew install sox          # macOS
sudo apt-get install sox  # Ubuntu/Debian
```

### Running Tests

```bash
# 1. Full bridge test suite (tests 2-way bridge_server.rs by default)
./sipp_scenarios/run_bridge_tests.sh

# 2. Test N-way conference bridge (tests multi_session_bridge_demo.rs)
./sipp_scenarios/run_bridge_tests.sh multi

# 3. Quick bridge test only
./sipp_scenarios/run_bridge_tests.sh quick

# 4. Set up test environment only
./sipp_scenarios/run_bridge_tests.sh setup

# 5. Run bridge server manually for debugging
./sipp_scenarios/run_bridge_tests.sh server

# 6. Show all available options
./sipp_scenarios/run_bridge_tests.sh help
```

### Bridge Server Options

The test suite can validate two different bridge implementations:

#### 🌉 **2-Way Bridge Server** (`bridge_server.rs`) - Default
- **Topology**: Simple 2-participant bridging
- **RTP Pairs**: 1 (Client A ↔ Client B via server)
- **Use Case**: Basic call bridging, call transfer scenarios

```bash
# Test 2-way bridge (default)
./sipp_scenarios/run_bridge_tests.sh
./sipp_scenarios/run_bridge_tests.sh all
BRIDGE_SERVER=bridge_server ./sipp_scenarios/run_bridge_tests.sh
```

#### 🎯 **N-Way Conference Server** (`multi_session_bridge_demo.rs`)
- **Topology**: Full-mesh conferencing (supports 3+ participants)
- **RTP Pairs**: N×(N-1)÷2 (e.g., 3 participants = 3 RTP pairs)
- **Use Case**: Conference calls, multi-party meetings

```bash
# Test N-way conference bridge
./sipp_scenarios/run_bridge_tests.sh multi
BRIDGE_SERVER=multi_session_bridge_demo ./sipp_scenarios/run_bridge_tests.sh
```

## Test Scenarios

### Basic Bridge Test (20 seconds)
- **Client A** connects to server (440Hz audio)
- **Client B** connects to server (880Hz audio)  
- Server automatically bridges the calls
- Both clients exchange audio for 20 seconds
- Calls terminate naturally

**For 2-way bridge**: Creates 1 RTP relay pair (A ↔ B)  
**For N-way conference**: Creates N×(N-1)÷2 RTP relay pairs (full-mesh topology)

### Quick Bridge Test (10 seconds)
- Same as basic test but shorter duration
- Useful for rapid validation during development

### Multi-Session Conference Test
When using `./sipp_scenarios/run_bridge_tests.sh multi`:
- Tests **multi_session_bridge_demo.rs** instead of **bridge_server.rs**
- Validates N-way conferencing capabilities
- Demonstrates full-mesh RTP forwarding topology
- Shows conference coordination and session management

## Environment Variables

- **`BRIDGE_SERVER`** - Choose which server example to test
  - `bridge_server` (default) - 2-way bridging
  - `multi_session_bridge_demo` - N-way conferencing

```bash
# Explicit server selection
BRIDGE_SERVER=bridge_server ./sipp_scenarios/run_bridge_tests.sh
BRIDGE_SERVER=multi_session_bridge_demo ./sipp_scenarios/run_bridge_tests.sh
```

## What Gets Tested

### ✅ Bridge Infrastructure (Both Bridge Types)
- Bridge creation and destruction
- Session-to-bridge association
- Bridge state management
- Bridge statistics and monitoring
- **2-way bridge**: Simple pairwise bridging
- **N-way conference**: Full-mesh RTP forwarding topology

### ✅ Real SIP Integration
- Complete SIP call flow: INVITE → 100 → 180 → 200 → ACK
- Dialog creation and management
- SDP negotiation with real media ports
- BYE handling and cleanup

### ✅ RTP Media Flow
- RTP packet capture and analysis
- Bidirectional audio flow validation
- Port allocation and routing
- Media session lifecycle
- **Multi-session**: Validates N×(N-1)÷2 RTP relay pairs for N participants

### ✅ Event System
- Bridge event notifications
- Session state changes
- Call lifecycle events
- Error handling and recovery

### ✅ Conference Coordination (N-way bridge only)
- Multi-participant session management
- Conference state transitions
- Automatic bridge partner discovery
- Full-mesh audio topology coordination

## Test Output

### Successful Bridge Test Output
```
=== Session-Core Bridge Test Suite ===
✓ SIPp found
✓ Cargo found  
✓ tcpdump found (RTP capture enabled)
✓ sox found (audio generation enabled)

✓ Created Client A audio file (440Hz)
✓ Created Client B audio file (880Hz)

✅ Bridge server started (PID: 12345)

=== Running Bridge Test: basic_bridge ===
Starting Client A...
Starting Client B...
✅ PASSED: basic_bridge (Both clients successful)

--- Bridge RTP Flow Analysis ---
Total RTP packets captured: 1247
✅ RTP media flow detected in bridge
✅ Bidirectional bridge flow detected

--- Server Bridge Activity Analysis ---
Bridge Statistics:
  Incoming calls: 2
  Bridges created: 1
  Bridges destroyed: 1
✅ Bridge creation detected in server logs

🎉 All bridge tests passed!
✅ Bridge infrastructure is working correctly
```

### Key Validation Points
- **RTP packets captured** > 0 (proves media flow)
- **Bidirectional flow detected** (proves bridge working)
- **Bridge creation in logs** (proves automatic bridging)
- **Both clients successful** (proves SIP call completion)

## Advanced Usage

### Command Reference
```bash
# Show all available commands and options
./sipp_scenarios/run_bridge_tests.sh help

# Test specific bridge implementations
./sipp_scenarios/run_bridge_tests.sh        # 2-way bridge (default)
./sipp_scenarios/run_bridge_tests.sh multi  # N-way conference bridge
./sipp_scenarios/run_bridge_tests.sh quick  # Quick test (any bridge)

# Environment variable control
BRIDGE_SERVER=bridge_server ./sipp_scenarios/run_bridge_tests.sh
BRIDGE_SERVER=multi_session_bridge_demo ./sipp_scenarios/run_bridge_tests.sh
```

### Manual Testing - 2-Way Bridge
```bash
# Terminal 1: Start 2-way bridge server
cargo run --example bridge_server

# Terminal 2: First client (waits for bridge partner)
sipp -sn uac 127.0.0.1:5060 -m 1 -d 30000 -rtp_echo

# Terminal 3: Second client (gets bridged with first)
sipp -sn uac 127.0.0.1:5060 -p 5062 -m 1 -d 30000 -rtp_echo
```

### Manual Testing - N-Way Conference Bridge
```bash
# Terminal 1: Start N-way conference server
cargo run --example multi_session_bridge_demo

# Terminal 2: First participant (joins conference)
sipp -sn uac 127.0.0.1:5060 -p 5061 -m 1 -d 60000 -rtp_echo

# Terminal 3: Second participant (joins conference)
sipp -sn uac 127.0.0.1:5060 -p 5062 -m 1 -d 60000 -rtp_echo

# Terminal 4: Third participant (creates full 3-way conference)
sipp -sn uac 127.0.0.1:5060 -p 5063 -m 1 -d 60000 -rtp_echo

# Expected: All participants hear each other (3 RTP relay pairs total)
```

### With Custom Audio Files
```bash
# Create custom audio files
sox -n -r 8000 -c 1 -b 16 client_a.wav synth 10 sine 440
sox -n -r 8000 -c 1 -b 16 client_b.wav synth 10 sine 880

# Use in SIPp
sipp -sn uac 127.0.0.1:5060 -ap client_a.wav -m 1 -d 10000
```

### Debugging Failed Tests

1. **Check server logs**: `bridge_results/bridge_server.log`
2. **Check SIPp logs**: `bridge_results/*_client_*.log`
3. **Analyze RTP capture**: `bridge_results/*_rtp.pcap`
4. **Verify bridge events**: Look for 📞 🌉 ✅ emojis in server log

## Integration with CI/CD

The bridge tests can be integrated into continuous integration:

```bash
# In CI script
./sipp_scenarios/run_bridge_tests.sh quick
exit_code=$?

if [ $exit_code -eq 0 ]; then
    echo "Bridge infrastructure validated ✅"
else
    echo "Bridge tests failed ❌"
    exit 1
fi
```

## Architecture Validation

These tests validate the complete session-core architecture:

```
🎯 call-engine (orchestration) ←── Future enhancement
         ↕
🧹 session-core (mechanics) ←────── ✅ TESTED HERE
    ├─ SessionManager
    ├─ DialogManager  
    ├─ BridgeCoordinator
    └─ MediaManager
         ↕
📡 transaction-core (SIP protocol) ←── ✅ TESTED HERE
         ↕
🚛 sip-transport (UDP/TCP) ←────── ✅ TESTED HERE
```

## Troubleshooting

### Common Issues

**"Bridge server failed to start"**
- Check if port 5060 is already in use: `lsof -i :5060`
- Ensure cargo can build the example: `cargo check --example bridge_server`

**"No RTP packets captured"**
- Verify tcpdump permissions: `sudo tcpdump --version`
- Check if firewall is blocking UDP ports 10000-20000
- Ensure SIPp has RTP enabled with `-rtp_echo` flag

**"Bridge creation not detected"**
- Check server logs for error messages
- Verify both SIPp clients connected successfully
- Ensure bridge coordinator is running

### Performance Notes

- Bridge tests are CPU-intensive due to real RTP processing
- Each test runs 1-2 SIPp clients + bridge server + packet capture
- Recommended to run on dedicated test machines for CI/CD

## Next Steps

After successful bridge tests, you can:

1. **Build call-engine** on top of session-core bridge infrastructure
2. **Add advanced bridge features** (conferencing, mixing, recording)
3. **Scale testing** with more concurrent calls
4. **Performance testing** with high call volumes

The bridge infrastructure is now **production-ready** and **fully validated**! 🎉 

## Bridge Implementation Comparison

| Feature | 2-Way Bridge (`bridge_server.rs`) | N-Way Conference (`multi_session_bridge_demo.rs`) |
|---------|-----------------------------------|--------------------------------------------------|
| **Topology** | Simple pairwise bridging | Full-mesh conferencing |
| **Max Participants** | 2 | Configurable (default: 10) |
| **RTP Relay Pairs** | 1 | N×(N-1)÷2 |
| **Use Cases** | Call transfer, basic bridging | Conference calls, multi-party meetings |
| **Complexity** | Simple coordinator logic | Conference management, participant discovery |
| **Test Command** | `./run_bridge_tests.sh` | `./run_bridge_tests.sh multi` |

### When to Use Each Test

**🌉 Use 2-Way Bridge Test When:**
- Validating basic bridge infrastructure
- Testing call transfer scenarios
- Verifying simple RTP forwarding
- CI/CD quick validation
- Learning bridge concepts

**🎯 Use N-Way Conference Test When:**
- Validating conference call capabilities
- Testing multi-participant scenarios
- Verifying full-mesh RTP topology
- Demonstrating advanced bridge features
- Performance testing with multiple sessions

### With Custom Audio Files