# Peer-to-Peer SIP Demo

This example demonstrates a minimal peer-to-peer SIP call between two User Agents (UAs) using the RVOIP `client-core` library. It showcases a complete SIP call flow with bidirectional RTP media exchange.

## Overview

The demo consists of two simple SIP peers:

- **Peer A** (`peer_a.rs`) - Acts as the caller (UAC - User Agent Client)
- **Peer B** (`peer_b.rs`) - Acts as the receiver (UAS - User Agent Server)

Both peers use the `client-core` library exclusively and demonstrate:

- ✅ Full SIP call establishment (INVITE/180/200/ACK)
- ✅ Bidirectional RTP media streaming
- ✅ Proper call termination (BYE)
- ✅ Detailed logging and statistics
- ✅ Minimal code footprint

## Demo Flow

1. **Peer B** starts and listens on port 5061
2. **Peer A** starts and waits 3 seconds
3. **Peer A** initiates a SIP call to Peer B
4. **Peer B** auto-answers the call after 1 second
5. Both peers establish RTP media sessions
6. Media flows bidirectionally for 15 seconds
7. **Peer A** terminates the call
8. Both peers shut down gracefully

## Quick Start

### Prerequisites

- Rust 1.70+
- `cargo` build tool
- Local network access (uses localhost)

### Running the Demo

```bash
# Navigate to the peer-to-peer directory
cd examples/peer-to-peer

# Make the script executable
chmod +x run_demo.sh

# Run the demo
./run_demo.sh
```

The script will:
- Build both peer binaries
- Start Peer B (receiver)
- Start Peer A (caller)
- Monitor the call progress
- Generate detailed logs
- Report success/failure

### Expected Output

```
🚀 RVOIP Peer-to-Peer Demo
============================

🔨 Building Peer A and Peer B...
✅ Build successful

▶️  Starting Peer B (Receiver)...
   SIP Port: 5061
   Media Ports: 21000-21100
   Log: logs/peer_b.log
✅ Peer B is ready

▶️  Starting Peer A (Caller)...
   SIP Port: 5060
   Media Ports: 20000-20100
   Log: logs/peer_a.log

📋 Demo Progress:
   1. Peer A will wait 3 seconds, then call Peer B
   2. Peer B will auto-answer after 1 second
   3. Both peers will exchange RTP media for 15 seconds
   4. Peer A will terminate the call

⏳ Waiting for demo to complete...

📊 Demo Results:
================================
✅ Peer A completed successfully
✅ Peer A log file created
✅ Peer B log file created

📊 Call Statistics:
===================
📤 Peer A (Caller): Final RTP Stats - Sent: 750 packets (120000 bytes), Received: 750 packets (120000 bytes)
📥 Peer B (Receiver): Final RTP Stats - Sent: 750 packets (120000 bytes), Received: 750 packets (120000 bytes)
✅ SIP call successfully established
✅ RTP media exchange successful

🎉 DEMO SUCCESSFUL!
   Both peers connected and exchanged media successfully
```

## Architecture

### Network Configuration

```
┌─────────────────┐                    ┌─────────────────┐
│     Peer A      │                    │     Peer B      │
│   (Caller)      │                    │   (Receiver)    │
├─────────────────┤                    ├─────────────────┤
│ SIP: 5060       │ ────► INVITE ────► │ SIP: 5061       │
│ RTP: 20000-20100│ ◄─── 200 OK ◄──── │ RTP: 21000-21100│
│                 │ ────► ACK ───────► │                 │
│                 │                    │                 │
│                 │ ◄──── RTP ──────► │                 │
│                 │                    │                 │
│                 │ ────► BYE ───────► │                 │
│                 │ ◄─── 200 OK ◄──── │                 │
└─────────────────┘                    └─────────────────┘
```

### Code Structure

- **`peer_a.rs`** - 150 lines of clean, focused code
- **`peer_b.rs`** - 150 lines of clean, focused code
- **`run_demo.sh`** - Automated test runner and reporter
- **`README.md`** - This documentation

## Generated Logs

The demo creates several log files in the `logs/` directory:

### Primary Logs

- **`peer_a.log`** - Detailed Peer A events and RTP statistics
- **`peer_b.log`** - Detailed Peer B events and RTP statistics
- **`sip_messages.log`** - Combined SIP signaling timeline

### Debug Logs (stdout/stderr)

- **`peer_a_stdout.log`** - Peer A console output
- **`peer_b_stdout.log`** - Peer B console output

### Sample Log Content

**SIP Call Flow (`sip_messages.log`):**
```
[PEER A] 📞 Initiating call to Peer B...
[PEER B] 📞 Incoming call: call-123 from sip:alice@127.0.0.1:5060 to sip:bob@127.0.0.1:5061
[PEER A] 🔄 Call call-123 state: None → Initiating
[PEER B] 🔔 Call call-123 state: None → IncomingPending
[PEER B] ✅ Call call-123 state: IncomingPending → Connected
[PEER A] ✅ Call call-123 state: Initiating → Connected
[PEER A] 📴 Call call-123 state: Connected → Terminated
[PEER B] 📴 Call call-123 state: Connected → Terminated
```

**RTP Statistics:**
```
[PEER A] 📊 Final RTP Stats - Sent: 750 packets (120000 bytes), Received: 750 packets (120000 bytes)
[PEER B] 📊 Final RTP Stats - Sent: 750 packets (120000 bytes), Received: 750 packets (120000 bytes)
```

## Technical Details

### SIP Configuration

- **Codec**: PCMU (G.711 μ-law) and PCMA (G.711 A-law)
- **RTP Payload**: 160 bytes per packet (20ms @ 8kHz)
- **Packet Rate**: ~50 packets/second per direction
- **Call Duration**: 15 seconds
- **Expected Packets**: ~750 packets per direction

### Client-Core Integration

The demo uses the `client-core` library's high-level API:

```rust
// Client setup
let client = ClientManager::new(config).await?;
client.set_event_handler(handler).await;
client.start().await?;

// Making a call
let call_id = client.make_call(from_uri, to_uri, None).await?;

// Answering a call
client.answer_call(&call_id).await?;

// Media control
client.start_audio_transmission(&call_id).await?;

// Statistics
let stats = client.get_rtp_statistics(&call_id).await?;

// Call termination
client.hangup_call(&call_id).await?;
```

### Event Handling

Both peers implement the `ClientEventHandler` trait to respond to:

- **Incoming calls** - Auto-answer with configurable delay
- **Call state changes** - Track call progress and start media
- **Media events** - Monitor RTP session lifecycle
- **Errors** - Handle and log any failures

## Customization

### Modifying Call Duration

Edit the sleep duration in `peer_a.rs`:

```rust
// Let the call run for 30 seconds instead of 15
tokio::time::sleep(Duration::from_secs(30)).await;
```

### Changing Ports

Modify the client configurations:

```rust
// Use different ports
let config = ClientConfig::new()
    .with_sip_addr("127.0.0.1:6060".parse()?)  // Custom SIP port
    .with_media_addr("127.0.0.1:30000".parse()?)  // Custom media port
```

### Adding Codecs

Extend the codec preferences:

```rust
preferred_codecs: vec![
    "OPUS".to_string(),    // Add Opus
    "G722".to_string(),    // Add G.722
    "PCMU".to_string(),
    "PCMA".to_string(),
],
```

## Troubleshooting

### Common Issues

**Port conflicts:**
```
Error: Address already in use (os error 48)
```
- Solution: Change the SIP ports in the peer configurations

**No RTP packets received:**
```
⚠️  No RTP packets were received from the server!
```
- Check firewall settings
- Verify RTP port ranges don't conflict
- Ensure both peers start successfully

**Call fails to establish:**
```
❌ SIP call failed to establish
```
- Check the detailed logs in `logs/`
- Verify network connectivity
- Ensure both peers are running

### Debug Mode

For more verbose logging, set environment variables:

```bash
RUST_LOG=debug ./run_demo.sh
```

## Integration

This example can be used as a foundation for:

- **Softphone applications** - Extend with UI frameworks
- **Call testing tools** - Add automated validation
- **Performance benchmarks** - Scale to multiple concurrent calls
- **Protocol testing** - Add custom SIP scenarios

The minimal codebase makes it easy to understand and modify for specific use cases.

## Next Steps

1. **Review the logs** - Understand the SIP and RTP flow
2. **Modify the code** - Experiment with different configurations
3. **Scale the demo** - Try multiple concurrent calls
4. **Add features** - Implement hold, transfer, DTMF, etc.

For more advanced scenarios, see the other examples in the RVOIP project. 