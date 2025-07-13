# Audio Streaming SIP Demo

This example demonstrates **real-time audio streaming** between two equivalent SIP peers using actual microphone capture and speaker playback. Each peer can both make and receive calls, creating a truly symmetric VoIP system.

## Overview

The demo consists of **unified audio peers** that are completely symmetric:

- **Audio Peer** (`audio_peer.rs`) - Can both make and receive calls with full audio streaming
- **Two operating modes**: Listener (waits for calls) or Caller (initiates calls)
- **Network deployment**: Designed to run on different computers

Each peer demonstrates:

- ✅ **Full SIP call establishment** (INVITE/180/200/ACK)
- ✅ **Real-time microphone capture** using system audio devices
- ✅ **Real-time speaker playback** using system audio devices
- ✅ **Bidirectional RTP audio streaming** with frame-level processing
- ✅ **Audio device integration** via the client-core audio subsystem
- ✅ **Symmetric peer architecture** - either peer can call the other
- ✅ **Cross-computer deployment** with IP address configuration

## Key Features

### 🎤 Microphone Integration
- Automatic discovery of available input devices
- Real-time audio capture from default microphone
- Frame-by-frame processing and RTP transmission
- Configurable audio quality settings

### 🔊 Speaker Integration
- Automatic discovery of available output devices
- Real-time audio playback through default speakers
- Frame-by-frame processing from RTP reception
- Synchronized audio streaming

### 🎵 Audio Streaming Pipeline
```
Microphone → AudioFrame → Client-Core → RTP → Network
                                          ↓
Network → RTP → Client-Core → AudioFrame → Speaker
```

### 🔧 Audio Processing
- **Echo Cancellation** - Reduces audio feedback
- **Noise Suppression** - Improves audio quality
- **Auto Gain Control** - Maintains consistent volume
- **Voice Activity Detection** - Optimizes bandwidth usage

## Demo Flow

1. **Peer 1** starts in listener mode and waits for calls
2. **Peer 2** starts in caller mode and initiates a call to Peer 1
3. **Peer 1** auto-answers the call after 1 second
4. **Both peers** configure audio streaming (8kHz, PCMU codec)
5. **Both peers** start microphone capture and speaker playback
6. **Real-time audio** flows bidirectionally for configured duration
7. **Calling peer** terminates the call
8. **Both peers** shut down gracefully

**Key Difference**: Either peer can be the caller or listener - they're completely symmetric!

## Quick Start

### Prerequisites

- **Rust 1.70+** with `cargo` build tool
- **Working microphone** and **speakers/headphones**
- **Audio permissions** (may require granting microphone access)
- **Local network access** (uses localhost by default)

### Running the Demo

```bash
# Navigate to the audio-streaming directory
cd examples/audio-streaming

# Quick demo on same computer (localhost)
./run_peer_demo.sh

# Cross-computer demo (set IP addresses)
LOCAL_IP=192.168.1.100 REMOTE_IP=192.168.1.200 ./run_peer_demo.sh

# Or run peers manually:
# Computer A (listener):
cargo run --bin audio_peer -- --local-ip 0.0.0.0 --display-name Alice

# Computer B (caller):
cargo run --bin audio_peer -- --call 192.168.1.100 --display-name Bob
```

## Command Line Configuration

The unified audio peer supports extensive command line configuration:

### Audio Peer (Unified)
```bash
cargo run --bin audio_peer -- [OPTIONS]

OPTIONS:
  --local-ip <IP>          Local IP address to bind to [default: 127.0.0.1]
  --local-port <PORT>      Local SIP port to bind to [default: 5060]
  --rtp-port-start <PORT>  Local RTP port range start [default: 20000]
  --display-name <NAME>    Your display name [default: Peer]
  --answer-delay <SECONDS> Auto-answer delay in seconds [default: 1]
  --call <IP>              Call a remote peer (provide their IP address)
  --remote-port <PORT>     Remote peer's SIP port (when calling) [default: 5060]
  --duration <SECONDS>     Call duration in seconds (when calling) [default: 30]
  --help                   Print help information
```

### Operating Modes

**Listener Mode (Default)**:
```bash
# Wait for incoming calls
cargo run --bin audio_peer -- --local-ip 0.0.0.0 --display-name Alice
```

**Caller Mode**:
```bash
# Make a call to a remote peer
cargo run --bin audio_peer -- --call 192.168.1.100 --display-name Bob --duration 60
```

## Network Deployment Examples

### Same Computer (Default)
```bash
# Default localhost setup
./run_peer_demo.sh
```

### Different Computers on Same Network
```bash
# On Computer A (192.168.1.100) - automatic setup:
LOCAL_IP=192.168.1.100 REMOTE_IP=192.168.1.200 ./run_peer_demo.sh

# On Computer B (192.168.1.200) - automatic setup:
LOCAL_IP=192.168.1.200 REMOTE_IP=192.168.1.100 ./run_peer_demo.sh
```

### Manual Peer Control (Preferred)
```bash
# Computer A (192.168.1.100) - Start listener:
cargo run --bin audio_peer -- \
    --local-ip 0.0.0.0 \
    --display-name "Alice" \
    --answer-delay 1

# Computer B (192.168.1.200) - Start caller:
cargo run --bin audio_peer -- \
    --local-ip 0.0.0.0 \
    --call 192.168.1.100 \
    --display-name "Bob" \
    --duration 60
```

### Symmetric Calling (Either Direction)
```bash
# Alice can call Bob:
cargo run --bin audio_peer -- --call 192.168.1.200 --display-name Alice

# OR Bob can call Alice:
cargo run --bin audio_peer -- --call 192.168.1.100 --display-name Bob

# Both peers are equivalent - no "server" or "client" distinction!
```

### Environment Variables with run_peer_demo.sh
```bash
# Custom configuration using environment variables
PEER1_NAME=Alice PEER2_NAME=Bob CALL_DURATION=45 ./run_peer_demo.sh

# Cross-network deployment
LOCAL_IP=192.168.1.100 REMOTE_IP=10.0.0.50 ./run_peer_demo.sh
```

### Expected Output

```
🎵 RVOIP Audio Streaming Demo
============================
🎤 This demo shows real-time audio streaming between two SIP peers
🔊 Both peers will use microphone input and speaker output
⚠️  Make sure your microphone and speakers are working!

🔨 Building Audio Peer A and Audio Peer B...
✅ Build successful

▶️  Starting Audio Peer B (Receiver)...
   SIP Port: 5061
   Media Ports: 21000-21100
   Log: logs/audio_peer_b.log
✅ Audio Peer B is ready

▶️  Starting Audio Peer A (Caller)...
   SIP Port: 5060
   Media Ports: 20000-20100
   Log: logs/audio_peer_a.log

📋 Demo Progress:
   1. Audio Peer A will wait 3 seconds, then call Audio Peer B
   2. Audio Peer B will auto-answer after 1 second
   3. Both peers will start capturing audio from microphone
   4. Both peers will play received audio through speakers
   5. Audio streaming will continue for 30 seconds
   6. Audio Peer A will terminate the call

🎤 IMPORTANT: Speak into your microphone during the demo!
🔊 You should hear audio from the other peer through your speakers!
⚠️  If you hear feedback/echo, use headphones or lower the volume!

🎵 Audio streaming demo is now running...
📊 Monitoring progress (will complete in ~35 seconds)...

⏰ Demo running... (5s elapsed)
⏰ Demo running... (10s elapsed)
⏰ Demo running... (15s elapsed)
⏰ Demo running... (20s elapsed)
⏰ Demo running... (25s elapsed)
⏰ Demo running... (30s elapsed)
⏰ Demo running... (35s elapsed)

📊 Demo Results:
================================
✅ Audio Peer A log file created
✅ Audio Peer B log file created

📊 Call Statistics:
===================
📤 Audio Peer A (Caller): Sent: 1500 packets (240000 bytes), Received: 1500 packets (240000 bytes)
📥 Audio Peer B (Receiver): Sent: 1500 packets (240000 bytes), Received: 1500 packets (240000 bytes)
✅ SIP call successfully established
✅ Audio Peer A: Microphone and speaker streaming active
✅ Audio Peer B: Microphone and speaker streaming active

🎉 AUDIO STREAMING DEMO SUCCESSFUL!
   Both peers connected and exchanged real-time audio successfully
   🎤 Microphone capture worked on both sides
   🔊 Speaker playback worked on both sides
   📊 RTP media exchange was successful
```

## Architecture

### Network Configuration

```
┌─────────────────────────────────────────┐                    ┌─────────────────────────────────────────┐
│              Audio Peer A               │                    │              Audio Peer B               │
│              (Caller)                   │                    │              (Receiver)                 │
├─────────────────────────────────────────┤                    ├─────────────────────────────────────────┤
│ SIP: 5060                              │ ────► INVITE ────► │ SIP: 5061                              │
│ RTP: 20000-20100                       │ ◄─── 200 OK ◄──── │ RTP: 21000-21100                       │
│                                         │ ────► ACK ───────► │                                         │
│ 🎤 Microphone ─► AudioFrame ─► RTP ─────┤                    │ 🔊 Speaker ◄─ AudioFrame ◄─ RTP ◄──────┤
│                                         │ ◄──── RTP ──────► │                                         │
│ 🔊 Speaker ◄─ AudioFrame ◄─ RTP ◄──────┤                    │ 🎤 Microphone ─► AudioFrame ─► RTP ─────┤
│                                         │ ────► BYE ───────► │                                         │
│                                         │ ◄─── 200 OK ◄──── │                                         │
└─────────────────────────────────────────┘                    └─────────────────────────────────────────┘
```

### Audio Processing Pipeline

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   System Mic    │───▶│  AudioDevice    │───▶│  AudioFrame     │───▶│  send_audio_    │
│   (CPAL)        │    │  Manager        │    │  Conversion     │    │  frame()        │
└─────────────────┘    └─────────────────┘    └─────────────────┘    └─────────────────┘
                                                                                │
                                                                                ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│  System Speaker │◄───│  AudioDevice    │◄───│  AudioFrame     │◄───│  subscribe_to_  │
│   (CPAL)        │    │  Manager        │    │  Conversion     │    │  audio_frames() │
└─────────────────┘    └─────────────────┘    └─────────────────┘    └─────────────────┘
```

## Code Structure

### Audio Peer A (`audio_peer_a.rs`)
- **Main Role**: Initiates SIP calls
- **Audio Functions**:
  - `start_microphone_capture()` - Captures audio from microphone
  - `start_speaker_playback()` - Plays audio through speakers
  - `stop_audio_streaming()` - Cleanup audio resources
- **Key Features**:
  - Automatic device discovery
  - Real-time audio frame processing
  - RTP streaming integration
  - Call lifecycle management

### Audio Peer B (`audio_peer_b.rs`)
- **Main Role**: Receives and answers SIP calls
- **Audio Functions**:
  - `start_microphone_capture()` - Captures audio from microphone
  - `start_speaker_playback()` - Plays audio through speakers
  - `stop_audio_streaming()` - Cleanup audio resources
- **Key Features**:
  - Auto-answer incoming calls
  - Identical audio capabilities to Peer A
  - Bidirectional audio streaming

## Audio Configuration

### Default Settings
- **Sample Rate**: 8000 Hz (narrowband voice)
- **Channels**: 1 (mono)
- **Codec**: PCMU (G.711 μ-law)
- **Frame Size**: 20ms (160 samples at 8kHz)
- **Packet Rate**: ~50 packets/second per direction

### Audio Processing
- **Echo Cancellation**: Enabled
- **Noise Suppression**: Enabled
- **Auto Gain Control**: Enabled
- **Voice Activity Detection**: Enabled

## Generated Logs

The demo creates detailed log files in the `logs/` directory:

### Primary Logs
- **`audio_peer_a_stdout.log`** - Audio Peer A console output
- **`audio_peer_b_stdout.log`** - Audio Peer B console output

### Sample Log Content

**Audio Streaming Setup:**
```
🎤 [PEER A] Using microphone: Built-in Microphone
🔊 [PEER A] Using speaker: Built-in Speakers
🎤 [PEER A] Starting microphone capture loop...
🔊 [PEER A] Starting speaker playback loop...
🎤 [PEER A] Sent 250 audio frames from microphone
🔊 [PEER A] Played 250 audio frames to speaker
```

**RTP Statistics:**
```
📊 [PEER A] Final RTP Stats - Sent: 1500 packets (240000 bytes), Received: 1500 packets (240000 bytes)
📊 [PEER B] Final RTP Stats - Sent: 1500 packets (240000 bytes), Received: 1500 packets (240000 bytes)
```

## Troubleshooting

### Common Issues

**No audio devices found:**
```
❌ [PEER A] No audio devices found! Please ensure microphone and speakers are connected.
```
- **Solution**: Check that your microphone and speakers are properly connected and recognized by the system
- **Test**: Run audio device discovery examples in client-core

**Audio feedback/echo:**
```
⚠️  If you hear feedback/echo, use headphones or lower the volume!
```
- **Solution**: Use headphones to separate microphone and speakers
- **Alternative**: Lower speaker volume or increase distance between mic and speakers

**Audio permission denied:**
```
❌ Failed to start microphone: Permission denied
```
- **Solution**: Grant microphone permissions to your terminal/application
- **macOS**: System Preferences → Security & Privacy → Privacy → Microphone
- **Linux**: Check PulseAudio/ALSA permissions

**Build failures:**
```
❌ Build failed
```
- **Solution**: Ensure all dependencies are available:
  - `cargo build --release` in the project root
  - Check that CPAL audio backend is supported on your platform

### Debug Mode

For more verbose logging, set environment variables:

```bash
RUST_LOG=debug ./run_demo.sh
```

## Integration Examples

### Custom Audio Processing

```rust
// Example: Add audio effects between capture and transmission
while let Some(device_frame) = audio_receiver.recv().await {
    // Apply custom audio processing
    let processed_frame = apply_audio_effects(device_frame);
    
    // Convert and send
    let session_frame = processed_frame.to_session_core();
    client.send_audio_frame(&call_id, session_frame).await?;
}
```

### Custom Audio Devices

```rust
// Example: Use specific audio devices instead of defaults
let microphone = audio_manager.get_device_by_name("USB Microphone").await?;
let speaker = audio_manager.get_device_by_name("Bluetooth Speaker").await?;
```

### Audio Quality Settings

```rust
// Example: High-quality audio configuration
let config = AudioStreamConfig {
    sample_rate: 16000,    // Wideband
    channels: 1,
    codec: "Opus".to_string(),
    frame_size_ms: 20,
    enable_aec: true,
    enable_agc: true,
    enable_vad: true,
};
```

## Platform Support

### Supported Platforms
- **macOS** - CoreAudio backend via CPAL
- **Linux** - ALSA/PulseAudio backend via CPAL
- **Windows** - WASAPI backend via CPAL

### Audio Requirements
- **Input Device**: Any microphone (built-in, USB, Bluetooth)
- **Output Device**: Any speakers or headphones
- **Permissions**: Microphone access permissions
- **Drivers**: Platform-specific audio drivers

## Performance Metrics

### Expected Performance
- **Latency**: ~40-60ms end-to-end (depends on system)
- **Bandwidth**: ~64 kbps per direction (PCMU codec)
- **CPU Usage**: Low (efficient audio processing)
- **Memory**: Minimal (streaming frame processing)

### Optimization Tips
- Use headphones to prevent audio feedback
- Ensure stable network connection for best quality
- Close unnecessary applications to reduce system load
- Use lower sample rates for bandwidth-constrained scenarios

## Next Steps

1. **Explore the code** - Understand the audio streaming integration
2. **Modify settings** - Experiment with different audio configurations
3. **Add features** - Implement audio effects, recording, or custom devices
4. **Scale up** - Try multiple concurrent audio calls
5. **Integrate** - Use as foundation for softphone or conferencing applications

For more advanced audio examples, see the client-core audio examples directory.

## Related Examples

- **`audio_device_discovery.rs`** - Discover available audio devices
- **`audio_loopback.rs`** - Test audio device functionality
- **`peer-to-peer`** - Basic SIP call without audio streaming
- **`call-center`** - More complex call routing scenarios

The audio streaming demo showcases the full potential of real-time audio communication using the RVOIP stack! 