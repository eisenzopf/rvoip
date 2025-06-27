# Call Center End-to-End Test Suite

**Quick Summary**: This test simulates a complete call center with customers calling in, agents handling calls, and full audio bridging - all running on localhost (127.0.0.1).

## Quick Start (TL;DR)

```bash
# Just run this:
./run_e2e_test.sh

# Watch the magic happen! ✨
```

## Overview

This directory contains a complete end-to-end test for the RVoIP call center system. It demonstrates:

1. A working call center server accepting incoming calls
2. Agent clients that register and handle calls  
3. Automated customer call simulation with SIPp
4. Packet capture for debugging
5. Comprehensive logging and analysis

## Overview

The test simulates a real call center environment:
- **Customers** (SIPp) call into the call center support line
- **Call Center Server** receives calls and routes them to available agents
- **Agents** (Alice, Bob) register with the server and handle incoming calls
- All communication uses SIP over UDP on localhost (127.0.0.1)

## Components

### 1. Call Center Server (`server/basic_call_center_server.rs`)
- **Address**: Listens on `0.0.0.0:5060`
- **Domain**: `127.0.0.1` (for test environment)
- **Database**: In-memory SQLite with test agents
- **Features**:
  - Accepts SIP REGISTER from agents
  - Receives calls to `sip:support@127.0.0.1`
  - Routes calls to available agents
  - Manages call queues when agents are busy
  - Bridges customer and agent audio streams

### 2. Agent Client (`agent/agent_client.rs`)
- **Purpose**: Simulates call center agents (Alice, Bob, etc.)
- **Features**:
  - Registers as `sip:alice@127.0.0.1` (or bob, charlie, etc.)
  - Auto-answers incoming calls
  - Maintains calls for configurable duration (default: 15 seconds)
  - Handles multiple concurrent calls (configurable)
- **Ports**: Alice on 5071, Bob on 5072, etc.

### 3. SIPp Test Scenarios (`sipp_scenarios/`)
- **customer_uac.xml**: Simulates customers calling the call center
- **Target**: Calls `sip:support@127.0.0.1`
- **Features**:
  - SDP negotiation for G.711 audio codecs
  - Optional audio playback from PCAP file
  - Configurable call rate and duration
  - Statistics collection

### 4. Test Runner Script (`run_e2e_test.sh`)
- **Automation**: Orchestrates the entire test flow
- **Steps**:
  1. Builds all Rust components
  2. Starts packet capture (tcpdump)
  3. Launches call center server
  4. Starts agent clients (Alice, Bob)
  5. Runs SIPp to make test calls
  6. Collects and analyzes results
  7. Cleanup on exit

## Prerequisites

Before running the tests, ensure you have:

```bash
# Required tools
cargo      # Rust build tool
sipp       # SIP testing tool
tcpdump    # Packet capture (requires sudo)

# Install on macOS
brew install sipp

# Install on Ubuntu/Debian
sudo apt-get install sip-tester tcpdump
```

## Running the Tests

### Quick Start

```bash
cd rvoip/crates/call-engine/examples/e2e_test
./run_e2e_test.sh
```

The script will:
1. Build all components
2. Start the call center server
3. Start two agents (Alice and Bob)
4. Run 5 test calls via SIPp
5. Capture all SIP and RTP traffic
6. Analyze and report results

### Manual Testing

You can also run components individually:

```bash
# Terminal 1: Start the server
cargo run --example e2e_test_server

# Terminal 2: Start Alice agent
cargo run --example e2e_test_agent -- --username alice --port 5071 --domain 127.0.0.1

# Terminal 3: Start Bob agent  
cargo run --example e2e_test_agent -- --username bob --port 5072 --domain 127.0.0.1

# Terminal 4: Make test calls with SIPp
cd sipp_scenarios
sipp -sf customer_uac.xml -s support -i 127.0.0.1 -p 5080 127.0.0.1:5060
```

**Note**: The `--domain 127.0.0.1` parameter is now the default, so it can be omitted.

## Analyzing Results

### Log Files

After running the test, check the logs in `logs/`:

- **`server.log`**: Call center server activity
  - Agent registrations
  - Incoming calls and routing decisions
  - Call state changes
  - Bridge creation/destruction
- **`alice.log`**: Alice agent activity
  - Registration status
  - Incoming call notifications
  - Call state changes
- **`bob.log`**: Bob agent activity (same as Alice)
- **`sipp.log`**: SIPp test results and statistics

### Packet Capture

The test captures all SIP and RTP traffic in `pcaps/test_capture.pcap`.

To analyze:

```bash
# View in Wireshark (recommended)
wireshark pcaps/test_capture.pcap

# Quick command-line analysis
tcpdump -r pcaps/test_capture.pcap -A | grep -E "(INVITE|REGISTER|200 OK|BYE)"

# Filter SIP messages only
tcpdump -r pcaps/test_capture.pcap -A port 5060

# See call flow
tcpdump -r pcaps/test_capture.pcap -nn -q | grep "SIP"
```

### Success Criteria

The test is considered successful if:

1. ✅ Both agents successfully register with the server
2. ✅ At least one call is established (customer → server → agent)
3. ✅ Calls are distributed between available agents
4. ✅ Audio is bridged between customer and agent
5. ✅ Calls complete cleanly without errors

## Troubleshooting

### Common Issues

1. **Port already in use**: Kill any processes using port 5060
   ```bash
   lsof -i :5060
   kill -9 <PID>
   ```

2. **Agent registration fails**: 
   - Check server log for "Updated agent ... status to available"
   - Ensure agents use domain `127.0.0.1` not `localhost`
   - Verify network connectivity

3. **No calls routed**: 
   - Verify agents show as "available" in server log
   - Check for "Assigning call to agent" messages
   - Ensure there are no DNS resolution errors

4. **SIPp fails**: 
   - Ensure SIPp is installed: `sipp -v`
   - Check SIPp can bind to port 5080
   - Verify scenario file path is correct

5. **DNS/Network errors**:
   - Always use IP addresses (127.0.0.1) not hostnames
   - Check for IPv6 vs IPv4 issues (use 127.0.0.1 not localhost)

### Debug Mode

For more verbose output:

```bash
# Run with debug logging
RUST_LOG=debug ./run_e2e_test.sh

# Or for specific components
RUST_LOG=rvoip_call_engine=debug,rvoip_session_core=debug ./run_e2e_test.sh

# Just SIP messages
RUST_LOG=rvoip_sip_core=debug ./run_e2e_test.sh
```

## Extending the Tests

### Adding More Agents

The server already creates charlie in the database. To add charlie as an active agent:

```bash
cargo run --example e2e_test_agent -- --username charlie --port 5073 --domain 127.0.0.1
```

To add more agents, edit `basic_call_center_server.rs` to include them in the test agents list.

### Custom Call Scenarios

Create new SIPp XML files in `sipp_scenarios/` for different test cases:

- Long duration calls
- High call volume
- Call transfers
- Failed call scenarios

### Performance Testing

Modify the SIPp parameters in `run_e2e_test.sh`:

```bash
# Increase call rate and total calls
sipp ... -r 10 -m 100  # 10 calls/sec, 100 total calls
```

## Architecture

```
┌─────────────────┐    REGISTER         ┌──────────────────────┐
│  Agent Alice    │ sip:alice@127.0.0.1 │                      │
│  (Port 5071)    ├────────────────────►│   Call Center Server │
└─────────────────┘                     │    (0.0.0.0:5060)   │
                                        │                      │
┌─────────────────┐    REGISTER         │  • Manages agents    │
│   Agent Bob     │ sip:bob@127.0.0.1   │  • Routes calls      │
│  (Port 5072)    ├────────────────────►│  • Creates bridges   │
└─────────────────┘                     │  • Handles queues    │
                                        └──────────┬───────────┘
                                                   │
┌─────────────────┐      INVITE                   │
│    Customer     │  sip:support@127.0.0.1        │
│     (SIPp)      ├───────────────────────────────┘
│  (Port 5080)    │    
└─────────────────┘

Call Flow:
1. Agents register with server
2. Customer calls support line
3. Server accepts call (200 OK)
4. Server creates outgoing call to available agent
5. Server bridges customer ↔ agent audio
6. Either party can hang up (BYE)
```

## Next Steps

1. Add authentication to agent registration
2. Implement call recording verification
3. Add media quality analysis
4. Create stress test scenarios
5. Add WebRTC agent support 