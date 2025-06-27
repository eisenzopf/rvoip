# 🎵 Quick RTP Media Exchange Test

This guide shows how to quickly test real RTP media exchange between SIPp and session-core.

## 🚀 Quick Start (2 Terminals)

### Terminal 1: Start RTP Test Server
```bash
cd rvoip/crates/session-core/examples/sipp_tests
cargo run --bin sip_rtp_test_server -- --port 5062 --media-mode echo --rtp-logging
```

**Expected Output:**
```
🧪 SIP RTP Test Server starting...
🔧 Configuration:
  📡 SIP Port: 5062
  🎵 Media Mode: Echo
  📊 RTP Logging: true
  🎯 Media Ports: 10000-20000
✅ SIP RTP Test Server ready and listening on port 5062
🎵 Media processing: Echo
📡 RTP packet logging: true
🔄 Waiting for incoming calls from SIPp...
```

### Terminal 2: Run SIPp RTP Test
```bash
cd rvoip/crates/session-core/examples/sipp_tests
sipp -sf scenarios/sipp_to_rust/rtp_media_test.xml -mi 127.0.0.1 -mp 6000 -rtp_echo 127.0.0.1:5062
```

**Expected Output:**
```
📞 Call answered! Server RTP: 127.0.0.1:10697
📡 Client RTP: 127.0.0.1:6000
✅ SIP call established successfully
🎵 Starting RTP media exchange test...
🎵 STARTED: Playing G.711A PCAP audio file
📡 SIPp is now SENDING RTP packets to server
🔄 RTP transmission in progress...
🏁 COMPLETED: PCAP audio playback finished
✅ RTP Media Exchange Test COMPLETED
```

## 🔍 What to Look For

### ✅ Success Indicators:
1. **SIP Signaling**: INVITE → 200 OK → ACK → BYE flow
2. **Media Negotiation**: SDP offer/answer with correct ports
3. **RTP Flow**: "SENDING RTP packets to server" messages
4. **Server Logs**: Media session creation and RTP processing

### 📊 Server Logs (Terminal 1):
```
📞 [SIPp-RTP-Test-Server] Incoming call from sip:client@127.0.0.1:5061 to sip:mediatest@127.0.0.1:5062
✅ [SIPp-RTP-Test-Server] Auto-accepting call with media processing
🎉 [SIPp-RTP-Test-Server] Call sess_xxx answered successfully
📡 [SIPp-RTP-Test-Server] Media Session Details for sess_xxx:
  🎯 RTP Port: Some(10697)
  📍 Local Bind: Some(127.0.0.1:10000)
  🎵 Codecs: ["PCMU", "PCMA"]
🎵 [SIPp-RTP-Test-Server] Media session active - ready for RTP packet exchange
```

## 🎯 Expected Results

### ✅ **PASS** Criteria:
- SIP call completes successfully
- Media session created with valid RTP port
- SIPp reports sending RTP packets
- Server receives and processes media session

### ❌ **FAIL** Indicators:
- SIP call fails or timeouts
- No media session created
- RTP ports not allocated
- "Connection refused" errors

## 🔧 Troubleshooting

### Port Conflicts:
```bash
# Check if port 5062 is in use
lsof -i :5062

# Use different port
cargo run --bin sip_rtp_test_server -- --port 5063
sipp -sf scenarios/sipp_to_rust/rtp_media_test.xml -mi 127.0.0.1 -mp 6000 -rtp_echo 127.0.0.1:5063
```

### Build Errors:
```bash
# Clean rebuild
cargo clean
cargo build --bin sip_rtp_test_server
```

### SIPp Not Found:
```bash
# macOS
brew install sipp

# Ubuntu
sudo apt-get install sipp
```

## 📊 Packet Capture Verification

For detailed RTP analysis, capture packets during the test:
```bash
# Terminal 3 (requires sudo)
sudo tcpdump -i lo0 -w rtp_test.pcap "port 5062 or port 6000 or portrange 10000-20000"

# Analyze after test
tshark -r rtp_test.pcap -Y "rtp" -T fields -e rtp.ssrc -e rtp.payload_type
```

## 🎉 Success!

If you see RTP packets flowing and media sessions being created, congratulations! 
Your session-core RTP media exchange is working correctly.

The test demonstrates:
- ✅ SIP protocol compliance
- ✅ SDP media negotiation  
- ✅ RTP session establishment
- ✅ Media packet processing
- ✅ Bidirectional communication

## 📚 Next Steps

1. **Analyze Logs**: Check detailed media session logs
2. **Test Different Modes**: Try `--media-mode analyze` for packet analysis
3. **Performance Testing**: Run with multiple concurrent calls
4. **Custom Scenarios**: Create your own SIPp scenarios
5. **Integration**: Integrate into your application's test suite 