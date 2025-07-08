# Audio Device Examples

This directory contains comprehensive examples demonstrating the real hardware audio functionality in `rvoip-client-core`. These examples show how to use the audio device abstraction layer for VoIP applications.

## Prerequisites

- **Audio Hardware**: You'll need working microphone and speaker/headphone hardware
- **Permissions**: On some platforms, you may need to grant microphone permissions
- **Dependencies**: All examples use the `audio-cpal` feature which provides real hardware support

## Examples Overview

### 1. Audio Device Discovery (`audio_device_discovery.rs`)

**Purpose**: Discover and inspect available audio devices on your system.

```bash
cargo run --example audio_device_discovery
```

**What it does**:
- Lists all available input devices (microphones)
- Lists all available output devices (speakers/headphones)
- Shows device capabilities (sample rates, channels, formats)
- Tests format compatibility for each device
- Identifies default devices

**Sample Output**:
```
🎵 Audio Device Discovery Example
================================

🎤 INPUT DEVICES (Microphones):
------------------------------
  1. Built-in Microphone (cpal-input-0)
     Default: Yes
     Supported Sample Rates: [44100, 48000] Hz
     Supported Channels: [1, 2]
     Supported Formats: VoIP (16kHz, Mono), CD Quality (44.1kHz, Stereo)

🔊 OUTPUT DEVICES (Speakers):
----------------------------
  1. Built-in Speakers (cpal-output-0)
     Default: Yes
     Supported Sample Rates: [44100, 48000] Hz
     Supported Channels: [2]
     Supported Formats: CD Quality (44.1kHz, Stereo), Studio Quality (48kHz, Stereo)
```

### 2. Audio Loopback (`audio_loopback.rs`)

**Purpose**: Real-time audio capture and playback demonstration.

```bash
cargo run --example audio_loopback
```

**⚠️ WARNING**: This may cause audio feedback! Use headphones or keep volume low.

**What it does**:
- Captures audio from default microphone
- Plays it back through default speakers in real-time
- Shows real-time performance statistics
- Demonstrates low-latency audio processing

**Sample Output**:
```
🎵 Audio Loopback Example
=========================
⚠️  WARNING: This may cause feedback! Use headphones or keep volume low.

🎤 Input Device: Built-in Microphone (cpal-input-0)
🔊 Output Device: Built-in Speakers (cpal-output-0)

✅ Using format: 48000Hz, 1 channels, 20ms frames

▶️  Audio loopback is now active!
   Press Ctrl+C to stop...

📊 Stats: 50.0 frames/sec, 48000 samples/sec (5.0s elapsed)
```

### 3. VoIP Audio Demo (`voip_audio_demo.rs`)

**Purpose**: Demonstrates VoIP-style audio session management.

```bash
cargo run --example voip_audio_demo
```

**What it does**:
- Shows integration with `ClientManager`
- Simulates VoIP call scenarios
- Demonstrates session lifecycle management
- Shows concurrent call handling
- Tests session cleanup

**Sample Output**:
```
📞 VoIP Audio Demo
==================
🎯 This demo simulates a VoIP call using real audio devices

🚀 Starting VoIP client...
📱 Found 1 input device(s) and 1 output device(s)

📞 Simulating VoIP call...
🎙️  Starting audio capture for call 12345...
🔊 Starting audio playback for call 12345...
✅ Audio sessions active:
   Capture: true
   Playback: true

📡 Simulating 30-second call...
⏱️  Call time: 5s / 30s (25s remaining)
   Active sessions: 1 capture, 1 playback
```

### 4. Audio Performance Benchmark (`audio_benchmark.rs`)

**Purpose**: Comprehensive performance testing and measurement.

```bash
cargo run --example audio_benchmark
```

**What it does**:
- Measures audio capture/playback latency
- Tests throughput (frames per second)
- Evaluates format compatibility performance
- Benchmarks concurrent session handling
- Provides detailed performance metrics

**Sample Output**:
```
📊 Audio Performance Benchmark
==============================

🧪 Format Compatibility Benchmark
----------------------------------
  VoIP 8kHz: 0/1 input, 0/1 output devices (2.1ms)
  VoIP 16kHz: 0/1 input, 0/1 output devices (1.8ms)
  CD 44.1kHz: 1/1 input, 1/1 output devices (2.3ms)
  Studio 48kHz: 1/1 input, 1/1 output devices (1.9ms)

🎤 Audio Capture Benchmark
---------------------------
Using format: 48000Hz, 1 channels
Setup time: 45.2ms
Results:
  Frames received: 250
  Frame rate: 50.0 fps
  Sample rate: 48000 samples/sec
  Average frame interval: 20.0ms
  Frame jitter: 2.1ms
```

## Usage Tips

### Running Examples

1. **Basic Usage**:
   ```bash
   cd crates/client-core
   cargo run --example <example_name>
   ```

2. **With Logging**:
   ```bash
   RUST_LOG=debug cargo run --example audio_device_discovery
   ```

3. **Release Mode** (for benchmarks):
   ```bash
   cargo run --release --example audio_benchmark
   ```

### Platform-Specific Notes

#### macOS
- May require microphone permissions (System Preferences → Security & Privacy → Microphone)
- Built-in devices typically support 44.1kHz and 48kHz

#### Windows
- WASAPI backend provides low-latency audio
- May require running as administrator for some devices

#### Linux
- Uses ALSA backend
- May require `pulseaudio-dev` or `alsa-dev` packages
- Check audio permissions with `groups $USER`

### Troubleshooting

**No Audio Devices Found**:
- Check hardware connections
- Verify system audio settings
- Try running with `RUST_LOG=debug` for detailed logs

**Permission Denied**:
- Grant microphone permissions in system settings
- Check user groups on Linux (`audio` group)

**Audio Feedback in Loopback**:
- Use headphones instead of speakers
- Reduce system volume
- Increase distance between mic and speakers

**High Latency**:
- Try different audio formats (lower sample rates)
- Check system audio buffer settings
- Use exclusive mode if available

## Integration with Your Application

These examples show how to integrate real hardware audio into your VoIP application:

1. **Device Discovery**: Use `AudioDeviceManager::list_devices()` to let users select devices
2. **Session Management**: Use `ClientManager` audio methods for VoIP calls
3. **Format Selection**: Implement format negotiation for compatibility
4. **Error Handling**: Handle device unavailability and format mismatches
5. **Performance**: Monitor latency and throughput for quality assurance

## Next Steps

- **Real VoIP Integration**: Connect with SIP signaling and RTP transport
- **Advanced Features**: Add echo cancellation, noise reduction, gain control
- **Multi-device Support**: Handle device switching during calls
- **Network Integration**: Connect audio streams with session-core for real VoIP calls

For more information, see the main project documentation and the `AUDIO_STREAM_INTEGRATION_PLAN.md` file. 