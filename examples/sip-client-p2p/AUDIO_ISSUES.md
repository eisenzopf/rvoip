# Audio Issues in SIP Client P2P Example

## Current Status
The SIP signaling works correctly - calls connect and sessions are established. However, no audio flows between peers.

## Root Causes Identified

### 1. ✅ Fixed: Duplicate Session Creation
- **Issue**: Events were being handled twice, creating duplicate sessions
- **Fix**: Removed redundant event subscription in sip-client

### 2. ✅ Fixed: Hardcoded Localhost SDP
- **Issue**: SDP was using 127.0.0.1 instead of actual IP
- **Fix**: Updated to use actual local IP address

### 3. ❌ Critical: No RTP Endpoint Configuration
- **Issue**: RTP transport is not configured with remote peer's address
- **Symptom**: Audio is captured but has nowhere to send to
- **Required Fix**: 
  1. Extract remote RTP address/port from incoming SDP
  2. Configure RTP transport to send to remote address
  3. Bind RTP to local port specified in our SDP

### 4. ❌ Missing: SDP Processing Pipeline
- **Issue**: Remote SDP is not being parsed and used
- **Current Flow**:
  - Incoming INVITE has SDP with remote RTP endpoint
  - This SDP is stored in session-core but not processed
  - RTP layer doesn't know where to send packets
- **Required Flow**:
  1. Parse incoming SDP to extract:
     - Remote IP address (from c= line)
     - Remote RTP port (from m= line)
  2. Configure media session with remote endpoint
  3. Start RTP transport with proper addressing

## Technical Details

### Current Audio Path
```
Microphone → Capture (✅ Working) → Format Convert (✅ Working) → G.711 Encode (✅ Working) → RTP Send (❌ No destination)
RTP Receive (❌ No packets) → G.711 Decode → Format Convert → Speaker Playback
```

### What's Missing
The RTP layer needs to:
1. Bind to the local RTP port we advertise in SDP
2. Know the remote peer's IP:port to send packets to
3. Actually send/receive UDP packets

### Log Evidence
From the receiver log:
- ✅ "Captured audio frame #1: 160 samples"
- ❌ No "Received RTP packet" messages
- ❌ No "Playing audio frame" messages

## Next Steps

1. **Implement SDP parsing** in sip-client to extract remote RTP endpoint
2. **Pass remote endpoint** to media-core/RTP layer
3. **Configure RTP transport** with:
   - Local bind address/port
   - Remote destination address/port
4. **Verify UDP packets** are actually being sent/received

## Testing
Once fixed, you should see:
- RTP packets being sent to remote peer
- RTP packets being received from remote peer  
- Audio frames flowing through the playback pipeline
- Actual audio output from speakers