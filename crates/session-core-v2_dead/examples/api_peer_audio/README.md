# Audio Peer Example

This example demonstrates bidirectional audio exchange between two peers using the session-core-v2 simple API.

## What it does

- Alice and Bob both make outgoing calls to each other
- Each peer sends 5 seconds of audio (250 frames of 20ms each)
- Alice sends a 440Hz tone, Bob sends an 880Hz tone
- Each peer receives the other's audio stream
- Audio is saved as WAV files for verification

## Running the example

```bash
# Run with audio recording enabled
./run_audio_test.sh --record

# Run with debug logging
./run_audio_test.sh --debug

# Run with trace logging
./run_audio_test.sh --trace
```

## Output

When recording is enabled, the following WAV files are created:
- `alice_sent.wav` - 440Hz tone sent by Alice
- `alice_received.wav` - 880Hz tone received from Bob
- `bob_sent.wav` - 880Hz tone sent by Bob  
- `bob_received.wav` - 440Hz tone received from Alice

## Architecture

The example uses:
- **SimplePeer API**: Simplified interface for making/receiving calls
- **Real networking**: UDP ports 5060/5061 for SIP, 10000-10200 for RTP
- **Real codecs**: PCMU (G.711 μ-law) at 8kHz sample rate
- **Global event bus**: Cross-crate communication via infra-common
- **State machine**: Session lifecycle managed by YAML state table

## Known Limitations

1. **Incoming calls**: Currently not working due to SignalingInterceptor not being wired up. 
   The `peer2.rs` example shows how incoming calls should work once fixed.

2. **Call setup**: No actual SIP signaling between peers yet - they both make outgoing calls
   which creates independent media sessions that happen to exchange audio.

3. **Timing**: Relies on sleep() calls for synchronization rather than proper signaling.

## Files

- `peer1.rs` - Alice implementation (makes call, sends 440Hz)
- `peer2.rs` - Bob implementation for incoming calls (not working yet)
- `peer2_simple.rs` - Bob implementation that makes outgoing call (working)
- `run_audio_test.sh` - Test runner script
- `audio_utils.rs` - Legacy audio utilities (not used by current examples)
