# Audio Streaming Demo Troubleshooting Guide

## Issues Fixed

### 1. Frame Size Mismatch
**Problem**: The microphone was producing very small audio chunks (43 samples) instead of the expected 160 samples (20ms at 8kHz) for RTP transmission.

**Solution**: Added frame accumulation logic in `start_microphone_capture()` to collect small chunks and assemble them into proper 20ms frames before transmission.

### 2. Port Binding Conflict
**Problem**: Both peers were trying to bind to port 5060, causing the first peer to fail with "Address already in use".

**Solution**: Modified `run_peer_demo.sh` to use different ports:
- Peer A (Alice): Port 5060
- Peer B (Bob): Port 5061

### 3. Conflicting Audio APIs
**Problem**: The demo was calling both `start_audio_stream()` (legacy API) and using the frame-based API (`send_audio_frame()`), which conflicted with each other.

**Solution**: Removed the call to `start_audio_stream()` and use only the frame-based API for manual audio frame control.

### 4. Audio Frame Type Conversion
**Problem**: Incorrect conversion between `session-core::AudioFrame` and `client-core::AudioFrame` types.

**Solution**: Fixed the frame conversion in `start_speaker_playback()` to properly create `DeviceAudioFrame` objects.

### 4. Frame Type Conversion
**Problem**: Fixed the conversion between session_core::AudioFrame (from RTP) and DeviceAudioFrame (for speakers).

**Solution**: Added proper timestamp conversions and frame type handling.

## RTP Transmission Issue (No Audio)

### Root Cause
The audio streaming demo was not transmitting RTP packets despite successfully sending audio frames. The issue was:

1. **Missing `start_audio_transmission()` call**: While `start_audio_stream()` enables the frame-based API, it doesn't activate the RTP transmitter (AudioTransmitter) in the MediaSessionController.

2. **MediaSessionController Architecture**: The MediaSessionController requires explicit activation of the AudioTransmitter to actually send RTP packets over the network. Without this, packets are queued but never transmitted.

### The Fix
Added a call to `start_audio_transmission()` after `start_audio_stream()`. This activates the RTP transmitter which:
- Creates an AudioTransmitter component
- Starts the RTP packet transmission loop
- Enables actual network transmission of RTP packets

Both APIs work together:
- `start_audio_stream()`: Enables the frame-based streaming API for `send_audio_frame()`
- `start_audio_transmission()`: Activates the RTP transmitter to send packets over the network

## How the Demo Works

1. **Audio Capture**: The microphone captures high-quality audio (e.g., 48kHz) and produces small chunks
2. **Frame Accumulation**: Small chunks are accumulated into 160-sample frames (20ms at 8kHz)
3. **RTP Transmission**: Complete frames are sent via `send_audio_frame()` which encodes them as G.711 μ-law and transmits via RTP
4. **Audio Reception**: The receiver gets frames via `subscribe_to_audio_frames()`
5. **Audio Playback**: Frames are upsampled to the speaker's native rate and played back

## Testing the Demo

1. **Build the demo**:
   ```bash
   cargo build --release --bin audio_peer
   ```

2. **Run the demo**:
   ```bash
   ./run_peer_demo.sh
   ```

3. **Monitor the logs**:
   - Check `logs/peer1.log` for Alice (listener)
   - Check `logs/peer2.log` for Bob (caller)

4. **Expected behavior**:
   - Bob should capture audio from the microphone
   - Alice should play the received audio through speakers
   - You should see "Sent X audio frames" messages in Bob's log
   - You should see "Played X audio frames" messages in Alice's log
   - RTP statistics should show packets being sent and received

## Common Issues

### No Audio Heard
- Check that audio devices are properly connected
- Ensure no other applications are using the microphone/speakers
- Verify that the logs show frames being sent and received

### Port Already in Use
- Kill any existing audio_peer processes: `pkill -f audio_peer`
- Check if another application is using ports 5060-5061

### Frame Count Mismatch
- The sender may send more frames than the receiver plays due to:
  - Initial buffering delay
  - Network jitter
  - Processing overhead

### Performance Issues
- The demo uses high-quality audio capture (48kHz) and downsamples to 8kHz
- This provides better quality but uses more CPU
- For lower CPU usage, you could modify the capture format to 8kHz directly 

## Additional Notes

- The demo uses the frame-based streaming API (`send_audio_frame`/`subscribe_to_audio_frames`) rather than the legacy transmission API
- Audio frames are manually accumulated to create proper 20ms frames for RTP
- Resampling is performed from device sample rates to 8kHz for telephony compatibility 