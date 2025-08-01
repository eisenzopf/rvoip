# Audio Record-Playback Test

This test verifies that the audio implementation is working correctly by recording audio from your microphone and playing it back through your speakers. It also demonstrates the automatic audio format conversion that enables VoIP compatibility.

## Prerequisites

Build the test first:
```bash
cd tests/record-playback
cargo build --release
```

## Quick Start

```bash
# Basic test - record for 5 seconds and play back
./target/release/record-playback

# Quick test - record for 3 seconds
./target/release/record-playback --duration 3

# Save your recording
./target/release/record-playback --save my-recording.wav
```

## Usage Guide

### Basic Test (Default: 5-second recording)
```bash
./target/release/record-playback
```
This will:
1. Record from your default microphone for 5 seconds
2. Show a real-time audio level meter
3. Play back the recording through your default speakers
4. Display format conversion information (e.g., 44.1kHz → 8kHz)

### List Available Audio Devices
```bash
./target/release/record-playback --list-devices
```
Shows all available input/output devices with their capabilities:
- Device names and IDs
- Supported sample rates
- Supported channel configurations
- Which device is the system default

### Custom Recording Duration
```bash
# Record for 10 seconds
./target/release/record-playback --duration 10

# Quick 2-second test
./target/release/record-playback --duration 2
```

### Save Recording to File
```bash
# Record and save as WAV file
./target/release/record-playback --save recording.wav

# Record for 10 seconds and save
./target/release/record-playback --duration 10 --save long-recording.wav
```

### Use Specific Devices
```bash
# First, list devices to get exact names
./target/release/record-playback --list-devices

# Then use specific devices by name
./target/release/record-playback \
  --input-device "MacBook Pro Microphone" \
  --output-device "External Headphones"
```

### Skip Playback (Recording Only)
```bash
# Just record and save, no playback
./target/release/record-playback --playback false --save recording.wav
```

### All Options
```bash
./target/release/record-playback --help
```

## What to Expect

### During Recording
```
🎤 RECORDING... Speak now!
🎤 Level: [████████████████                                  ] 2.5s
```
- Speak into your microphone
- The level meter shows your input volume in real-time
- Adjust your microphone position/volume if the meter doesn't move

### During Playback
```
🔊 PLAYING BACK...
🔊 Playing: 73%
```
- Your recording will play through your speakers
- Progress percentage shows playback status

### Format Conversion Messages
```
🎵 Hardware format: 44100Hz 1 ch (requested: 8000Hz 1 ch)
📐 Creating format converter for capture: 44100Hz → 8000Hz
```
These messages show the automatic format conversion in action:
- Hardware typically supports 44.1kHz or 48kHz
- VoIP (G.711) requires 8kHz
- The library automatically converts between formats

## Troubleshooting

### No Audio Devices Found
- Ensure your microphone/speakers are properly connected
- On macOS: Check System Preferences → Security & Privacy → Microphone
- Try unplugging and reconnecting USB devices

### No Sound During Recording
- Check your microphone isn't muted (system level)
- Speak louder or move closer to the microphone
- Try a different input device with `--input-device`
- Look for the level meter - it should move when you speak

### No Sound During Playback
- Check your system volume isn't muted
- Verify the correct output device is selected
- Try a different output device with `--output-device`
- Check the recording was successful (non-zero RMS values in debug mode)

### Permission Errors
- **macOS**: Grant microphone permission when prompted
- **macOS**: Go to System Preferences → Security & Privacy → Microphone → Allow Terminal/iTerm
- **Linux**: Ensure your user is in the `audio` group: `sudo usermod -a -G audio $USER`

### Format Conversion Issues
If you see errors about unsupported formats:
- The test will show what formats your hardware supports
- Most modern devices support multiple rates (8kHz, 16kHz, 44.1kHz, 48kHz)
- The library automatically converts between hardware and VoIP formats

## Debug Mode

For detailed information about the audio pipeline:
```bash
# See all audio processing details
RUST_LOG=debug ./target/release/record-playback

# Even more verbose output
RUST_LOG=trace ./target/release/record-playback
```

Debug mode shows:
- Device enumeration details
- Format negotiation process
- Real-time audio frame information
- Format conversion statistics
- Buffer levels and timing

## Technical Details

This test demonstrates:
1. **Real-time audio capture** from system microphones
2. **Real-time audio playback** to system speakers
3. **Automatic format conversion** between hardware and VoIP formats
4. **Cross-platform audio** using CPAL (CoreAudio on macOS, ALSA on Linux, WASAPI on Windows)
5. **VoIP-ready audio** with 8kHz/16kHz sample rates for G.711/G.729 codecs

The test uses the same audio pipeline as the VoIP application, ensuring that if this test works, your VoIP calls will have working audio.