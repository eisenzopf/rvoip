# SIPp Integration Example

This example demonstrates a complete SIP call lifecycle test using SIPp (SIP testing tool) to test the RVOIP client-core library. It shows real SIP protocol exchange, SDP negotiation, and RTP audio transmission.

## Overview

The test consists of:
1. **RVOIP Test Server** - A SIP server built with rvoip-client-core that accepts incoming calls
2. **SIPp UAC** - A SIP client that makes calls to the server and sends RTP audio
3. **Test Script** - Orchestrates the entire test lifecycle

## Features Demonstrated

- ✅ Full SIP call flow (INVITE → 200 OK → ACK → BYE)
- ✅ SDP offer/answer negotiation
- ✅ Media port allocation
- ✅ RTP audio transmission (G.711)
- ✅ Multiple concurrent calls
- ✅ Call state tracking
- ✅ Event-driven architecture
- ✅ Clean resource management

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

Simply execute the test script:

```bash
cd examples/sipp_integration
./run_test.sh
```

The script will:
1. Check dependencies
2. Download required audio files
3. Build the RVOIP test server
4. Start the server
5. Run SIPp tests
6. Display results and statistics

## Test Configuration

You can modify the test parameters in `run_test.sh`:

```bash
SIP_PORT=5060        # RVOIP server SIP port
MEDIA_PORT=20000     # RVOIP server base RTP port
NUM_CALLS=5          # Number of test calls
CALL_RATE=1          # Calls per second
CALL_DURATION=10     # Duration of each call in seconds
```

## Architecture

```
┌─────────────┐         SIP Messages          ┌─────────────────┐
│    SIPp     │ ←──────────────────────────→ │  RVOIP Server   │
│   (UAC)     │                               │    (UAS)        │
└─────────────┘                               └─────────────────┘
      │                                              │
      │              RTP Audio Stream                │
      └──────────────────────────────────────────────┘
```

## Files

- `sip_test_server.rs` - RVOIP server implementation
- `uac_with_media.xml` - SIPp scenario file
- `run_test.sh` - Test orchestration script
- `pcap/` - Directory for RTP audio files
  - `g711a.pcap` - G.711 A-law audio sample

## Example Output

```
🚀 RVOIP Client Core - SIPp Integration Test
================================================
✅ All dependencies found
✅ Build successful
✅ Server is ready

📞 Running SIPp test scenario...
   Target: 127.0.0.1:5060
   Calls: 5
   Rate: 1 call/s
   Duration: 10 seconds per call

📊 Test Progress:
================================
[SERVER] 📞 Incoming call from: sip:sipp@127.0.0.1:5061
[SERVER] ✅ Auto-accepting incoming call
[SERVER] 📞 Call connected
[SERVER] 🎵 Starting RTP audio transmission
...
================================

✅ SIPp test completed successfully!

📈 Test Statistics:
Call-Id  Start Time  End Time  Status  Duration
...

🎉 Test Complete!
```

## Troubleshooting

1. **Port already in use**
   - Change `SIP_PORT` and `MEDIA_PORT` in the script

2. **SIPp not found**
   - Ensure SIPp is installed and in your PATH

3. **Build errors**
   - Check that all dependencies are up to date
   - Run `cargo update` in the project root

4. **No audio transmission**
   - Ensure the PCAP file exists in `pcap/g711a.pcap`
   - Check that RTP ports are not blocked by firewall

## Advanced Usage

### Running Server Manually

```bash
# Build
cargo build --example sipp_integration_sip_test_server

# Run with custom ports
./target/debug/examples/sipp_integration_sip_test_server 5060 20000 auto
```

### Custom SIPp Scenarios

You can create your own SIPp scenarios. Place them in this directory and run:

```bash
sipp -sf your_scenario.xml -s service 127.0.0.1:5060
```

### Analyzing Results

- Check `server.log` for detailed server behavior
- SIPp creates `*_messages.log` with all SIP messages
- Statistics are saved in CSV files

## Learn More

- [SIPp Documentation](http://sipp.sourceforge.net/doc/reference.html)
- [RVOIP Client Core Documentation](../../README.md)
- [SIP Protocol RFC 3261](https://www.ietf.org/rfc/rfc3261.txt) 