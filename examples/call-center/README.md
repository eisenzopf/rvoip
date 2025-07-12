# Call Center Demo with Real Audio

This example demonstrates a complete call center system using the RVOIP libraries with **real audio streaming capabilities**. It showcases how customers can call a support line, get routed to available agents, and establish bidirectional audio conversations using real microphones and speakers.

## 🎵 Real Audio Features

- **Real-time Audio Streaming**: Live microphone capture and speaker playback
- **Echo Cancellation**: Prevents audio feedback between speakers and microphones
- **Noise Suppression**: Reduces background noise for clearer communication
- **Auto Gain Control**: Automatically adjusts audio levels
- **Voice Activity Detection**: Optimizes audio processing based on speech patterns
- **Audio Device Discovery**: Automatically detects and configures available audio devices
- **Cross-platform Support**: Works on Windows, macOS, and Linux

## Overview

The demo consists of three main components:

- **Call Center Server** (`server.rs`) - Uses `call-engine` to handle incoming calls and route them to agents
- **Agents** (`agent.rs`) - Use `client-core` with real audio to register with the server and handle customer calls
- **Customer** (`customer.rs`) - Uses `client-core` with real audio to call the support line

## Architecture

```
┌─────────────────┐    REGISTER         ┌──────────────────────┐
│  Agent Alice    │ sip:alice@domain    │                      │
│  (Real Audio)   ├────────────────────►│   Call Center Server │
│  🎤 Microphone  │                     │    (configurable)    │
│  🔊 Speaker     │                     │                      │
└─────────────────┘                     │  • Routes calls      │
                                        │  • Manages queues    │
┌─────────────────┐    REGISTER         │  • Bridges audio    │
│   Agent Bob     │ sip:bob@domain      │  • Distributed       │
│  (Real Audio)   ├────────────────────►│    deployment       │
│  🎤 Microphone  │                     └──────────┬───────────┘
│  🔊 Speaker     │                                │
└─────────────────┘                                │
                                                   │
┌─────────────────┐      INVITE                   │
│    Customer     │  sip:support@domain           │
│  (Real Audio)   ├───────────────────────────────┘
│  🎤 Microphone  │    
│  🔊 Speaker     │
└─────────────────┘
```

## Demo Flow

1. **Server** starts and listens on configurable address
2. **Agent Alice** registers with real audio devices
3. **Agent Bob** registers with real audio devices
4. **Customer** calls support line using microphone
5. **Server** routes the call to an available agent
6. **Agent** answers using real audio devices
7. **Customer** and **Agent** have real audio conversation
8. **Audio processing** (echo cancellation, noise suppression) active
9. **Call** completes with comprehensive statistics

## Quick Start

### Prerequisites

- Rust 1.70+
- `cargo` build tool
- **Audio hardware** (microphone and speakers/headphones)
- Network access (configurable for distributed deployment)

### Running the Demo

```bash
# Navigate to the call-center directory
cd examples/call-center

# Make the script executable
chmod +x run_demo.sh

# Run the complete demo with real audio
./run_demo.sh
```

The script will automatically:
- Build all components with audio support
- Discover and configure audio devices
- Start the call center server
- Start two agents with real audio
- Execute a customer call with real audio
- Monitor audio streaming and quality
- Generate comprehensive reports

### Expected Output

```
🏢 RVOIP Call Center Demo with Real Audio
==========================================

🔧 Configuration:
   Server Domain: 127.0.0.1
   Server Port: 5060
   Call Duration: 30s
   Demo Mode: local
   Verbose: false

🎵 Audio Device Information:
=================================
🎤 INPUT DEVICES (Microphones):
  1. Built-in Microphone (DEFAULT)
     ID: cpal-input-0

🔊 OUTPUT DEVICES (Speakers):
  1. Built-in Speakers (DEFAULT)
     ID: cpal-output-0

✅ Audio devices discovered

🎉 REAL AUDIO CALL CENTER DEMO SUCCESSFUL!
   ✅ Customer connected to agent
   ✅ Call routed successfully
   ✅ Real audio streaming established
   ✅ Audio devices configured
   ✅ Call completed cleanly

🎯 Next Steps:
   • Try running components on separate machines
   • Use --list-devices to see available audio devices
   • Configure specific audio devices with --input-device and --output-device
   • Enable verbose logging with --verbose for detailed audio info
   • Experiment with different call durations
```

## Components

### Call Center Server

**File**: `src/server.rs`  
**Default Port**: 5060  
**Features**:
- Configurable bind address and domain
- Accepts SIP REGISTER from agents
- Receives calls to `sip:support@domain`
- Routes calls to available agents
- Manages call queues
- Supports distributed deployment

**Usage**:
```bash
# Local deployment
cargo run --bin server

# Distributed deployment
cargo run --bin server -- --bind-addr 0.0.0.0:5060 --domain 192.168.1.100

# With verbose logging
cargo run --bin server -- --verbose
```

### Agent

**File**: `src/agent.rs`  
**Default Ports**: Alice (5071), Bob (5072)  
**Features**:
- Real audio device integration
- Echo cancellation and noise suppression
- Configurable audio devices
- Auto-accepts incoming calls
- Handles calls with real audio streaming
- Comprehensive audio statistics

**Usage**:
```bash
# Basic usage with real audio (use agent's own IP for --domain)
cargo run --bin agent -- --name alice --server 192.168.1.100:5060 --domain 192.168.1.101

# List available audio devices
cargo run --bin agent -- --list-devices

# Use specific audio devices
cargo run --bin agent -- --name alice --input-device cpal-input-1 --output-device cpal-output-1

# With verbose audio logging
cargo run --bin agent -- --name alice --verbose --audio-debug
```

### Customer

**File**: `src/customer.rs`  
**Default Port**: 5080  
**Features**:
- Real audio device integration
- Calls the support line using microphone
- Receives audio through speakers
- Audio quality monitoring
- Comprehensive call statistics

**Usage**:
```bash
# Basic usage with real audio (use customer's own IP for --domain)
cargo run --bin customer -- --server 192.168.1.100:5060 --domain 192.168.1.102

# Extended call duration
cargo run --bin customer -- --call-duration 60

# Use specific audio devices
cargo run --bin customer -- --input-device cpal-input-0 --output-device cpal-output-0
```

## Advanced Configuration

### Distributed Deployment

The system supports running components on separate machines:

**Server (192.168.1.100)**:
```bash
cargo run --bin server -- --bind-addr 0.0.0.0:5060 --domain 192.168.1.100
```

**Agent (192.168.1.101)**:
```bash
cargo run --bin agent -- --name alice --server 192.168.1.100:5060 --domain 192.168.1.101 --port 5071
```

**Customer (192.168.1.102)**:
```bash
cargo run --bin customer -- --server 192.168.1.100:5060 --domain 192.168.1.102 --port 5080
```

**Important**: Each machine should use its **own IP address** for the `--domain` parameter. This ensures proper SIP signaling and prevents `0.0.0.0` addresses in Contact headers and SDP.

### Audio Device Configuration

#### Discovering Audio Devices

```bash
# List all available audio devices
cargo run --bin agent -- --list-devices

# Expected output:
# 🎤 INPUT DEVICES (Microphones):
#   1. Built-in Microphone (DEFAULT)
#      ID: cpal-input-0
#   2. USB Microphone
#      ID: cpal-input-1
#
# 🔊 OUTPUT DEVICES (Speakers):
#   1. Built-in Speakers (DEFAULT)
#      ID: cpal-output-0
#   2. USB Headphones
#      ID: cpal-output-1
```

#### Using Specific Devices

```bash
# Use USB microphone and headphones
cargo run --bin agent -- --name alice --input-device cpal-input-1 --output-device cpal-output-1

# Use built-in microphone and external speakers
cargo run --bin customer -- --input-device cpal-input-0 --output-device cpal-output-1
```

### Environment Variables

```bash
# Server configuration
export SERVER_DOMAIN=192.168.1.100
export SERVER_PORT=5060
export CALL_DURATION=60
export VERBOSE=true

# Run with environment variables
./run_demo.sh
```

## Audio Quality Configuration

### MediaConfig Settings

The system uses optimized audio settings for VoIP:

```rust
MediaConfig {
    preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
    dtmf_enabled: true,
    echo_cancellation: true,   // Prevents feedback
    noise_suppression: true,   // Reduces background noise
    auto_gain_control: true,   // Normalizes audio levels
    ..Default::default()
}
```

### Audio Stream Configuration

```rust
AudioStreamConfig {
    sample_rate: 8000,         // Standard VoIP rate
    channels: 1,               // Mono audio
    codec: "PCMU".to_string(), // G.711 μ-law
    frame_size_ms: 20,         // 20ms frames
    enable_aec: true,          // Echo cancellation
    enable_agc: true,          // Auto gain control
    enable_vad: true,          // Voice activity detection
}
```

## Troubleshooting

### Common Issues

**No audio devices found:**
```bash
# Check audio system
cargo run --bin agent -- --list-devices

# Install audio drivers (Linux)
sudo apt-get install libasound2-dev
```

**Audio feedback/echo:**
```bash
# Use headphones instead of speakers
# Or ensure echo cancellation is enabled
cargo run --bin agent -- --verbose --audio-debug
```

**Port conflicts:**
```bash
# Kill processes using ports
lsof -i :5060
kill -9 <PID>

# Or use different ports
cargo run --bin server -- --bind-addr 0.0.0.0:5070
```

**Network connectivity:**
```bash
# Check firewall settings
sudo ufw allow 5060/udp
sudo ufw allow 5070-5080/udp

# Test connectivity
nc -u 192.168.1.100 5060
```

**Audio quality issues:**
```bash
# Enable verbose audio logging
cargo run --bin agent -- --verbose --audio-debug

# Check audio statistics in logs
grep "Audio stats" logs/agent.log
```

### Debug Mode

For detailed logging:

```bash
# Enable all debug logging
RUST_LOG=debug ./run_demo.sh

# Enable specific component debugging
RUST_LOG=rvoip_client_core=debug,rvoip_call_engine=debug ./run_demo.sh

# Enable audio-specific debugging
cargo run --bin agent -- --verbose --audio-debug
```

### Performance Optimization

**High-quality audio:**
```bash
# Use higher sample rates (requires codec support)
# Configure in MediaConfig for better quality
```

**Low-latency audio:**
```bash
# Reduce frame size for lower latency
# Configure frame_size_ms: 10 for 10ms frames
```

## Network Configuration

### Port Allocation

| Component | SIP Port | Media Port Range | RTP Port Range |
|-----------|----------|------------------|----------------|
| Server    | 5060     | N/A              | N/A            |
| Alice     | 5071     | 6071             | 7071-7171      |
| Bob       | 5072     | 6072             | 7072-7172      |
| Customer  | 5080     | 6080             | 7080-7180      |

### Firewall Configuration

```bash
# Allow SIP signaling
sudo ufw allow 5060/udp
sudo ufw allow 5070-5080/udp

# Allow RTP media
sudo ufw allow 6000-8000/udp
```

## Technical Details

### Audio Processing Pipeline

```
Microphone → Capture → Noise Suppression → AGC → Echo Cancellation → Encoder → RTP → Network
Network → RTP → Decoder → AGC → Noise Suppression → Playback → Speakers
```

### SIP Configuration

- **Codecs**: PCMU (G.711 μ-law) and PCMA (G.711 A-law)
- **RTP Payload**: 160 bytes per packet (20ms @ 8kHz)
- **Packet Rate**: ~50 packets/second per direction
- **Registration Expiry**: 300 seconds (agents)

### Audio Device Integration

The system uses the client-core audio device abstraction:

```rust
// Audio device discovery
let audio_manager = AudioDeviceManager::new().await?;
let devices = audio_manager.list_devices(AudioDirection::Input).await?;

// Start audio capture
client.start_audio_capture(&call_id, &device_id).await?;

// Start audio playback
client.start_audio_playback(&call_id, &device_id).await?;

// Subscribe to audio frames
let subscriber = client.subscribe_to_audio_frames(&call_id).await?;
```

## Generated Logs

The demo creates comprehensive logs in the `logs/` directory:

### Primary Logs

- **`server_stdout.log`** - Call center server activity
- **`alice_stdout.log`** - Alice agent detailed events with audio stats
- **`bob_stdout.log`** - Bob agent detailed events with audio stats
- **`customer_stdout.log`** - Customer call activity with audio stats
- **`call_flow.log`** - Combined timeline of all events including audio

### Audio-Specific Log Content

**Audio Device Discovery**:
```
[alice] 🎤 Selected input device: Built-in Microphone
[alice] 🔊 Selected output device: Built-in Speakers
[alice] ✅ Audio devices successfully configured
```

**Real Audio Setup**:
```
[customer] 🔧 Configuring audio stream for call 12345
[customer]    Sample Rate: 8000Hz
[customer]    Channels: 1
[customer]    Codec: PCMU
[customer]    Frame Size: 20ms
[customer] ✅ Audio streaming started for call 12345
```

**Audio Statistics**:
```
[alice] 📊 Audio stats: 500 frames received, 50.0 fps
[customer] 📊 Call Quality - MOS: 4.2, Jitter: 2.1ms, Packet Loss: 0.05%
```

## Extending the Demo

### Adding More Agents

Start additional agents with unique names and ports:
```bash
cargo run --bin agent -- --name charlie --server 192.168.1.100:5060 --port 5073
cargo run --bin agent -- --name diana --server 192.168.1.100:5060 --port 5074
```

### Multiple Customers

Run multiple customer calls simultaneously:
```bash
# Terminal 1
cargo run --bin customer -- --name customer1 --port 5081

# Terminal 2  
cargo run --bin customer -- --name customer2 --port 5082
```

### Custom Audio Scenarios

- **High-quality audio**: Configure higher sample rates
- **Low-latency audio**: Reduce frame sizes
- **Multi-codec support**: Add additional codecs
- **Audio effects**: Implement custom audio processing

### Integration Testing

```bash
# Test audio device compatibility
cargo run --bin agent -- --list-devices

# Test distributed deployment
./run_demo.sh with SERVER_DOMAIN=<remote-ip>

# Test audio quality
cargo run --bin customer -- --call-duration 120 --verbose
```

## Production Considerations

### Security
- Implement SIP authentication
- Use TLS for signaling (SIP over TLS)
- Use SRTP for media encryption
- Network segmentation and firewall rules

### Scalability
- Load balancing for multiple servers
- Database persistence for agent states
- Message queuing for high call volumes
- Monitoring and alerting systems

### Audio Quality
- Adaptive bitrate control
- Packet loss recovery
- Advanced echo cancellation
- Bandwidth optimization

## Next Steps

1. **Experiment with distributed deployment** - Run components on separate machines
2. **Test different audio devices** - Use USB headsets, professional microphones
3. **Optimize audio settings** - Adjust sample rates, frame sizes, and codecs
4. **Add authentication** - Implement SIP digest authentication
5. **Scale testing** - Try multiple agents and concurrent calls
6. **Custom features** - Add call recording, conferencing, or IVR

The real audio capabilities make this demo suitable for understanding and testing production-grade VoIP systems with actual voice communication. 