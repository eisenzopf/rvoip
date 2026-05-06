# RVOIP Client-Server RTP Demo

This example demonstrates a complete SIP call with RTP media exchange between a UAC (client) and UAS (server) using the RVOIP client-core library.

## Overview

- **UAS Server (`uas_server.rs`)**: A SIP server that auto-answers incoming calls and processes RTP media
- **UAC Client (`uac_client.rs`)**: A SIP client that makes calls and sends RTP media
- Both use the `client-core` library with full media capabilities

## Features

- ✅ Complete SIP signaling (INVITE, 200 OK, ACK, BYE)
- ✅ SDP negotiation for media setup
- ✅ RTP port allocation and media session establishment
- ✅ RTP packet transmission and reception
- ✅ Detailed logging of media events
- ✅ Support for multiple concurrent calls

## Quick Start

```bash
# Run the complete demo
./run_test.sh

# Or run components separately:

# Terminal 1: Start the server
cargo run --release --bin uas_server -- --port 5070 --rtp-debug

# Terminal 2: Make calls
cargo run --release --bin uac_client -- --server 127.0.0.1:5070 --test-audio
```

## Command Line Options

### UAS Server
- `--port`: SIP listening port (default: 5070)
- `--media-port`: RTP port range start (default: 30000)
- `--rtp-debug`: Enable detailed RTP packet logging

### UAC Client
- `--server`: Server address to call (default: 127.0.0.1:5070)
- `--port`: Local SIP port (default: 5071)
- `--num-calls`: Number of calls to make (default: 1)
- `--duration`: Call duration in seconds (default: 10)
- `--test-audio`: Generate test audio tone
- `--rtp-debug`: Enable detailed RTP packet logging

## Expected Output

When running the demo, you should see:

1. **Server starts and waits for calls**:
   ```
   🚀 Starting UAS Server
   ✅ UAS Server ready on port 5070
   🎯 Waiting for incoming calls...
   ```

2. **Client makes calls**:
   ```
   🚀 Starting UAC Client
   📞 Making call 1 of 2
   ✅ Call initiated successfully
   ```

3. **RTP packets flow** (with `--rtp-debug`):
   ```
   📦 RTP packet received - SSRC: 12345, Seq: 1, Timestamp: 160
   📤 RTP packet sent - SSRC: 67890, Seq: 1, Timestamp: 160
   ```

## Architecture

This demo uses the RVOIP client-core library which provides:
- High-level SIP client API
- Automatic SDP negotiation
- RTP session management
- Media engine integration
- Event-driven architecture

The actual RTP packet processing happens in the underlying `media-core` and `rtp-core` libraries.

## Troubleshooting

1. **Port conflicts**: Make sure ports 5070, 5071, and 30000-31000 are free
2. **No RTP packets**: Check that both `--test-audio` and `--rtp-debug` are enabled
3. **Build errors**: Run `cargo clean` and rebuild

## Next Steps

- Implement actual audio generation/playback
- Add support for different codecs
- Test with external SIP clients
- Add DTMF support
- Implement call transfer and hold 