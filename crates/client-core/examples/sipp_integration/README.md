# SIPp Integration Example

This example demonstrates a complete SIP call lifecycle test using SIPp (SIP testing tool) to test the RVOIP client-core library. It shows real SIP protocol exchange, SDP negotiation, and RTP audio transmission with verified bidirectional media flow.

## Overview

The test consists of:
1. **RVOIP Test Server** - A SIP server built with rvoip-client-core that auto-accepts incoming calls
2. **SIPp UAC** - A SIP client that makes calls to the server and sends RTP audio
3. **Test Script** - Orchestrates the entire test lifecycle with multiple test modes

## Features Demonstrated

- ✅ Full SIP call flow (INVITE → 100 Trying → 200 OK → ACK → BYE → 200 OK)
- ✅ SDP offer/answer negotiation with codec selection (PCMU/PCMA)
- ✅ Dynamic media port allocation per call
- ✅ Bidirectional RTP audio transmission (G.711 PCMU/PCMA)
- ✅ Sequential call handling (no port conflicts)
- ✅ Real-time call statistics and monitoring
- ✅ Event-driven architecture with session coordination
- ✅ Clean resource management and graceful shutdown

## Prerequisites

1. **Install SIPp**
   ```bash
   # macOS
   brew install sipp

   # Ubuntu/Debian
   sudo apt-get install sip-tester

   # Or build from source
   git clone https://github.com/SIPp/sipp.git
   cd sipp
   cmake . && make
   ```

2. **Install Rust** (if not already installed)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

## Running the Test

Execute the test script with different modes:

```bash
cd examples/sipp_integration

# Run media test (default - with RTP audio)
./run_test.sh media

# Run simple signaling test (no media)
./run_test.sh simple

# Run both tests in sequence
./run_test.sh both
```

### Test Modes

- **`simple`** - SIP signaling only (no RTP media transmission)
- **`media`** - Full test with RTP audio transmission and statistics
- **`both`** - Runs both simple and media tests sequentially

## Test Configuration

Current optimized settings in `run_test.sh`:

```bash
SIP_PORT=5060                # RVOIP server SIP port
MEDIA_PORT=20000             # RVOIP server base RTP port
NUM_CALLS=5                  # Number of test calls
CALL_RATE=0.1               # 0.1 calls/sec (sequential, no overlap)
CALL_DURATION=8             # Duration of each call in seconds
```

**Why Sequential?** The test uses `CALL_RATE=0.1` (one call every 10 seconds) with 8-second duration to ensure calls don't overlap, eliminating media port conflicts and providing cleaner test results.

## Architecture

```
┌─────────────┐    SIP Messages (UDP:5060)     ┌─────────────────┐
│    SIPp     │ ←──────────────────────────→  │  RVOIP Server   │
│   (UAC)     │                                │    (UAS)        │
│   Port:5061 │    RTP Audio (Dynamic Ports)   │ Ports:20000+    │
│             │ ←──────────────────────────→  │                 │
└─────────────┘                                └─────────────────┘
```

## Files Structure

```
sipp_integration/
├── sip_test_server.rs      # RVOIP server implementation
├── run_test.sh             # Test orchestration script
├── uac_with_media.xml      # SIPp scenario with RTP media
├── simple_uac.xml          # SIPp scenario without media
├── minimal_media.xml       # Minimal media scenario
├── README.md              # This file
├── Cargo.toml             # Rust package configuration
├── audio/                 # Audio files for RTP transmission
│   └── client_a_440hz_pcma.wav
├── pcap/                  # Downloaded PCAP files
│   ├── g711a.pcap
│   └── g711u.pcap
└── [Generated Log Files]
    ├── server.log         # RVOIP server logs
    ├── sipp_messages_*.log # SIP message traces
    ├── sipp_screen_*.log   # SIPp statistics
    └── sipp_errors_*.log   # SIPp errors (if any)
```

## Test Process

The script automatically:

1. **Dependency Check** - Verifies SIPp and Cargo are installed
2. **Audio File Setup** - Downloads G.711 PCAP files if missing
3. **Build** - Compiles the RVOIP test server
4. **Port Check** - Ensures SIP port is available
5. **Server Start** - Launches RVOIP server with auto-answer
6. **Test Execution** - Runs SIPp scenarios
7. **Results Analysis** - Shows statistics and logs
8. **Cleanup** - Gracefully shuts down server

## Example Output

### Successful Media Test Results

```
🚀 RVOIP Client Core - SIPp Integration Test
================================================
✅ SIPp found: SIPp v3.7.1
✅ Cargo found
✅ Audio files ready
✅ Build successful
✅ Server is ready

🎯 Test mode: media
📞 Running SIPp test scenario with RTP media...
   Target: 127.0.0.1:5060
   Calls: 5
   Rate: 0.1 call/s
   Duration: 8 seconds per call
   RTP: Audio transmission using G.711 PCAP

[SIPp Output - Sequential Calls]
Call limit 5 hit
Peak was 1 calls (sequential)
5 successful calls, 0 failed calls

📊 Media test completed with exit code: 0

📋 Server Log (RTP Statistics):
Call 1: Sent 200 packets (34,400 bytes), Received 151 packets (25,972 bytes)
Call 2: Sent 200 packets (34,400 bytes), Received 151 packets (25,972 bytes)
Call 3: Sent 200 packets (34,400 bytes), Received 146 packets (25,112 bytes)
Call 4: Sent 200 packets (34,400 bytes), Received 151 packets (25,972 bytes)
Call 5: Sent 200 packets (34,400 bytes), Received 151 packets (25,972 bytes)

✅ Test completed!
```

### Key Success Indicators

- **SIP Protocol**: All calls show proper INVITE → 200 → ACK → BYE flow
- **Media Negotiation**: Successful SDP exchange with PCMU codec selection
- **RTP Exchange**: Server both sends AND receives RTP packets (bidirectional)
- **Port Management**: Each call gets unique dynamic ports (no conflicts)

## Troubleshooting

### Common Issues

1. **Port already in use**
   ```bash
   # Check what's using the port
   lsof -i :5060
   
   # Solution: Script automatically kills conflicting processes
   # Or change SIP_PORT in run_test.sh
   ```

2. **SIPp not found**
   ```bash
   # Check installation
   which sipp
   sipp -v
   
   # Install if missing (see Prerequisites)
   ```

3. **Build errors**
   ```bash
   # Update dependencies
   cd ../../../../  # Go to project root
   cargo update
   cargo build --example sipp_integration_sip_test_server
   ```

4. **Server won't start**
   ```bash
   # Check server logs
   cat server.log
   
   # Common causes:
   # - Permission denied on ports < 1024
   # - Firewall blocking UDP traffic
   # - Dependencies missing
   ```

5. **No RTP media exchange (SIPp shows 0 packets sent)**
   - **This is expected** - SIPp has configuration limitations with dynamic ports
   - **Check server logs** for actual RTP statistics
   - Server receiving RTP packets confirms media exchange is working

### Log Analysis

- **`server.log`** - Contains detailed RVOIP server behavior and RTP statistics
- **`sipp_messages_*.log`** - Complete SIP message traces
- **`sipp_screen_*.log`** - SIPp test statistics and call flow
- **`sipp_errors_*.log`** - SIPp errors (created only if errors occur)

## Advanced Usage

### Running Server Manually

```bash
# Build first
cargo build --example sipp_integration_sip_test_server

# Run with custom configuration
./target/debug/examples/sipp_integration_sip_test_server <sip_port> <media_port> <mode>

# Example
./target/debug/examples/sipp_integration_sip_test_server 5060 20000 auto
```

### Custom SIPp Tests

```bash
# Use different scenario files
sipp -sf minimal_media.xml -s service 127.0.0.1:5060

# Run with custom parameters
sipp -sf uac_with_media.xml -s service 127.0.0.1:5060 -l 1 -m 1
```

### Performance Testing

```bash
# Modify run_test.sh for stress testing
NUM_CALLS=50
CALL_RATE=0.2  # Still sequential but faster
CALL_DURATION=5
```

## Technical Details

### RTP Media Flow

1. **SDP Negotiation**: Server selects PCMU codec from offered PCMU/PCMA
2. **Port Allocation**: Server dynamically assigns unique RTP ports (20000+ range)
3. **Media Session**: Bidirectional RTP stream with packet statistics
4. **Monitoring**: Real-time packet count and byte transfer tracking

### Expected Results

- **SIP Success Rate**: 100% (all calls complete successfully)
- **Media Sessions**: Active bidirectional RTP exchange
- **Packet Statistics**: ~200 sent, ~150 received per 8-second call
- **Codec**: PCMU (G.711 μ-law) selected automatically

## Learn More

- [SIPp Documentation](http://sipp.sourceforge.net/doc/reference.html)
- [RVOIP Client Core Documentation](../../README.md)
- [SIP Protocol RFC 3261](https://www.ietf.org/rfc/rfc3261.txt)
- [RTP Protocol RFC 3550](https://www.ietf.org/rfc/rfc3550.txt) 