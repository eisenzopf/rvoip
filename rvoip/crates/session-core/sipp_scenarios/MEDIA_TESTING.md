# Media Flow Testing Guide

This guide explains how to test actual RTP media exchange using the session-core SIPp scenarios, going beyond just SIP signaling validation.

## 🎯 What Media Testing Covers

### Current SIP Scenarios Test (Signaling Only)
- ✅ **SDP Negotiation** - Codec offers and answers in SIP messages
- ✅ **Media Parameters** - Port numbers, codec types, media direction
- ✅ **Call Flow** - INVITE → 200 OK → ACK sequences
- ✅ **Error Handling** - Rejection, cancellation, timeouts

### New Media Testing Adds (RTP Layer)
- 🆕 **RTP Packet Flow** - Actual media packet transmission
- 🆕 **Packet Timing** - RTP timestamp validation
- 🆕 **Codec Payload** - Real audio data transmission
- 🆕 **Media Interruption** - Hold/resume RTP flow validation

## 🛠️ Setup Requirements

### Essential Tools
```bash
# SIPp (required)
brew install sipp

# tcpdump (recommended - for packet capture)
# Usually pre-installed on macOS, may need sudo access

# Python 3 (for server connectivity check)
# Usually pre-installed on macOS
```

### Optional Tools (Enhanced Analysis)
```bash
# sox (for audio file generation)
brew install sox

# ffmpeg (alternative audio generation)
brew install ffmpeg

# wireshark (includes tshark for advanced RTP analysis)
brew install wireshark
```

## 🚀 Quick Start

### 1. Start the Session-Core Server
```bash
# In one terminal
cd rvoip/crates/session-core
cargo run --example sipp_server
```

### 2. Run Media Tests
```bash
# In another terminal
cd rvoip/crates/session-core/sipp_scenarios

# Setup test environment
./run_media_tests.sh setup

# Run all media tests
./run_media_tests.sh

# Run basic test only
./run_media_tests.sh basic
```

## 📊 Test Scenarios

### 1. **Media Flow Test** (`media_flow_test.xml`)
**Purpose**: Comprehensive RTP media validation

**Features Tested**:
- Initial RTP stream establishment
- 5-second media exchange period
- Codec re-negotiation via re-INVITE
- 3-second media exchange with new codec
- Graceful media termination

**Expected Results**:
- RTP packets captured on both directions
- Successful codec negotiation (PCMU → PCMA)
- No packet loss during media exchange

### 2. **Basic Call with RTP Monitoring**
**Purpose**: Validate RTP flow during standard call

**Uses**: `basic_call.xml` with RTP capture enabled

**Features Tested**:
- RTP establishment after 200 OK
- Media flow during 2-second call duration
- Clean RTP termination after BYE

### 3. **Hold/Resume with Media Validation**
**Purpose**: Test media pause/resume functionality

**Uses**: `hold_resume.xml` with RTP monitoring

**Features Tested**:
- Initial RTP flow (sendrecv)
- Media pause during hold (sendonly)
- Media resume (sendrecv)
- RTP flow changes during re-INVITEs

## 🔍 Understanding Results

### SIP Message Logs
Located in `media_results/{test_name}.log`:
```
Timestamp    Direction  SIP Message
12:34:56.123 ->         INVITE sip:test@127.0.0.1:5060
12:34:56.125 <-         SIP/2.0 100 Trying
12:34:56.127 <-         SIP/2.0 200 OK
```

### RTP Packet Capture
Located in `media_results/{test_name}_rtp.pcap`:
- View with Wireshark: `wireshark {test_name}_rtp.pcap`
- Analyze with tshark: `tshark -r {test_name}_rtp.pcap -Y rtp`

### Test Statistics
Located in `media_results/{test_name}.csv`:
```csv
CallID,Duration,ResponseTime,Status
1,5.234,0.123,Success
```

## 🔬 Advanced Analysis

### Manual RTP Analysis
```bash
# View RTP packet details
tshark -r media_results/basic_call_rtp_rtp.pcap -T fields \
       -e rtp.ssrc -e rtp.timestamp -e rtp.seq -Y "rtp"

# Check for packet loss
tshark -r media_results/basic_call_rtp_rtp.pcap -Y "rtp" \
       -T fields -e rtp.seq | sort -n | uniq | wc -l
```

### SIPp RTP Options
```bash
# Run with custom RTP settings
sipp -sf basic_call.xml \
     -rtp_echo \
     -ap audio_files/test_audio.wav \
     -trace_rtp \
     127.0.0.1:5060
```

## 🐛 Troubleshooting

### Common Issues

#### 1. **No RTP Packets Captured**
**Symptoms**: pcap file is empty or contains no RTP packets

**Causes**:
- Server not properly handling RTP
- Firewall blocking RTP ports
- Wrong network interface for capture

**Solutions**:
```bash
# Check RTP port range
sudo tcpdump -i any 'udp and portrange 10000-20000'

# Try different network interface
sudo tcpdump -i en0 'udp and portrange 10000-20000'
```

#### 2. **Audio File Generation Failed**
**Symptoms**: Warning about missing audio tools

**Solutions**:
```bash
# Install sox
brew install sox

# Or install ffmpeg
brew install ffmpeg

# Manual audio file creation
sox -n -r 8000 -c 1 -b 16 audio_files/test_audio.wav synth 10 sine 440
```

#### 3. **Permission Denied for tcpdump**
**Symptoms**: tcpdump requires sudo but fails

**Solutions**:
```bash
# Run with explicit sudo
sudo ./run_media_tests.sh

# Or give user tcpdump permissions (advanced)
# See: https://apple.stackexchange.com/questions/232529
```

#### 4. **Server Connectivity Check Fails**
**Symptoms**: Script says server is not running when it is

**Debugging**:
```bash
# Manual server check
python3 -c "
import socket
s = socket.socket()
s.settimeout(3)
try:
    s.connect(('127.0.0.1', 5060))
    print('Server is running')
except:
    print('Server connection failed')
s.close()
"

# Check what's listening on port 5060
lsof -i :5060
```

## 📈 Performance Expectations

### Typical Results

| Test | Duration | RTP Packets | Success Rate |
|------|----------|-------------|--------------|
| Basic Call | 2-3s | 100-150 | 100% |
| Media Flow | 8-10s | 400-500 | 100% |
| Hold/Resume | 15-18s | 600-800 | 100% |

### Quality Indicators

#### ✅ **Good Media Flow**
- Consistent RTP packet timing (20ms intervals)
- No packet sequence gaps
- Proper SSRC synchronization
- Expected packet count (50 packets/second)

#### ❌ **Poor Media Flow**
- Irregular packet timing
- Missing sequence numbers
- Multiple SSRC values (unexpected)
- Significantly fewer packets than expected

## 🎵 Audio File Details

### Test Audio Specifications
- **Sample Rate**: 8000 Hz (standard for telephony)
- **Channels**: Mono (1 channel)
- **Bit Depth**: 16-bit
- **Format**: WAV (uncompressed)
- **Duration**: 10 seconds
- **Tone**: 440 Hz sine wave (A4 note)

### Custom Audio Files
You can replace `audio_files/test_audio.wav` with your own:
```bash
# Requirements
# - 8000 Hz sample rate
# - Mono channel
# - WAV format
# - Reasonable duration (5-30 seconds)

# Example conversion
ffmpeg -i your_audio.mp3 -ar 8000 -ac 1 audio_files/custom_test.wav
```

## 🚀 Next Steps

### Integration with Session-Core Development
1. **Automated Testing** - Include media tests in CI/CD pipeline
2. **Performance Benchmarks** - Establish baseline RTP metrics
3. **Regression Testing** - Detect media flow regressions
4. **Load Testing** - Multiple concurrent RTP streams

### Enhanced Media Testing
1. **Codec Quality Tests** - Validate different codec implementations
2. **Network Simulation** - Test under packet loss/delay conditions
3. **Echo Cancellation** - Test acoustic echo handling
4. **DTMF Testing** - Validate RFC 2833 DTMF transmission

This comprehensive media testing framework ensures that session-core not only handles SIP signaling correctly but also successfully coordinates actual media transmission between endpoints. 