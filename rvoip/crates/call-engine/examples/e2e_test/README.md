# Call Center End-to-End Test Suite

This directory contains a complete end-to-end test for the RVoIP call center system. It demonstrates:

1. A working call center server accepting incoming calls
2. Agent clients that register and handle calls
3. Automated testing with SIPp
4. Packet capture for debugging
5. Comprehensive logging

## Components

### 1. Call Center Server (`server/basic_call_center_server.rs`)
- Runs on port 5060
- Creates a SQLite database with test agents (alice, bob, charlie)
- Accepts calls to `sip:support@callcenter.example.com`
- Routes calls to available agents using round-robin

### 2. Agent Client (`agent/agent_client.rs`)
- Registers with the call center server
- Automatically answers incoming calls
- Configurable call duration
- Supports multiple agents running simultaneously

### 3. SIPp Test Scenarios (`sipp_scenarios/`)
- `customer_uac.xml`: Customer calling the call center
- Makes test calls with SDP negotiation
- Configurable call duration and rate

### 4. Test Runner Script (`run_e2e_test.sh`)
- Orchestrates the entire test
- Starts server and agent clients
- Runs SIPp tests
- Captures packets with tcpdump
- Analyzes results

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
cargo run --example e2e_test_agent -- --username alice --port 5071

# Terminal 3: Start Bob agent  
cargo run --example e2e_test_agent -- --username bob --port 5072

# Terminal 4: Make test calls with SIPp
cd sipp_scenarios
sipp -sf customer_uac.xml -s support -i 127.0.0.1 -p 5080 127.0.0.1:5060
```

## Analyzing Results

### Log Files

After running the test, check the logs in `logs/`:

- `server.log`: Call center server activity
- `alice.log`: Alice agent activity
- `bob.log`: Bob agent activity
- `sipp.log`: SIPp test results

### Packet Capture

The test captures all SIP and RTP traffic in `pcaps/test_capture.pcap`.

To analyze:

```bash
# View in Wireshark
wireshark pcaps/test_capture.pcap

# Quick command-line analysis
tcpdump -r pcaps/test_capture.pcap -A | grep -E "(INVITE|REGISTER|200 OK)"

# Filter SIP messages only
tcpdump -r pcaps/test_capture.pcap -A port 5060
```

### Success Criteria

The test is considered successful if:

1. Both agents successfully register
2. At least one call is established
3. Calls are distributed between agents
4. Calls complete without errors

## Troubleshooting

### Common Issues

1. **Port already in use**: Kill any processes using port 5060
   ```bash
   lsof -i :5060
   kill -9 <PID>
   ```

2. **Agent registration fails**: Check that the database was created successfully

3. **No calls routed**: Verify agents are in "available" status in server log

4. **SIPp fails**: Ensure SIPp is installed and in PATH

### Debug Mode

For more verbose output:

```bash
# Run with debug logging
RUST_LOG=debug ./run_e2e_test.sh

# Or for specific components
RUST_LOG=rvoip_call_engine=debug,rvoip_session_core=debug ./run_e2e_test.sh
```

## Extending the Tests

### Adding More Agents

Edit the server code to add more test agents to the database, then start additional agent clients:

```bash
cargo run --example e2e_test_agent -- --username charlie --port 5073
```

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
┌─────────────┐     REGISTER      ┌──────────────┐
│ Agent Alice ├──────────────────►│              │
└─────────────┘                   │              │
                                  │ Call Center  │
┌─────────────┐     REGISTER      │   Server     │
│  Agent Bob  ├──────────────────►│              │
└─────────────┘                   │              │
                                  └──────┬───────┘
                                         │
┌─────────────┐      INVITE             │
│   Customer  ├─────────────────────────┘
│   (SIPp)    │    (to support@...)
└─────────────┘

The server routes the call to an available agent
```

## Next Steps

1. Add authentication to agent registration
2. Implement call recording verification
3. Add media quality analysis
4. Create stress test scenarios
5. Add WebRTC agent support 