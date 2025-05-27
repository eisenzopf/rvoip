# Session-Core SIPp Test Scenarios

This directory contains comprehensive SIPp test scenarios for validating the `rvoip-session-core` library functionality. These scenarios test various aspects of SIP call handling, from basic call flows to advanced features and edge cases.

## 📋 Test Scenarios Overview

### 🔧 Basic Functionality Tests

| Scenario | File | Description | Key Features |
|----------|------|-------------|--------------|
| **Basic Call** | `basic_call.xml` | Standard INVITE → 200 OK → ACK → BYE flow | Core call establishment and termination |
| **Call Rejection** | `call_rejection.xml` | INVITE → 486 Busy Here → ACK | Call rejection handling |
| **Call Cancel** | `call_cancel.xml` | INVITE → 180 Ringing → CANCEL → 487 → ACK | Mid-setup cancellation |
| **OPTIONS Ping** | `options_ping.xml` | OPTIONS requests for keepalive/capabilities | Non-INVITE method handling |

### 🚀 Advanced Functionality Tests

| Scenario | File | Description | Key Features |
|----------|------|-------------|--------------|
| **Hold/Resume** | `hold_resume.xml` | re-INVITE with sendonly/sendrecv | Media direction changes |
| **Early Media** | `early_media.xml` | 183 Session Progress with SDP | Early media establishment |
| **Multiple Codecs** | `multiple_codecs.xml` | Codec negotiation and re-negotiation | SDP offer/answer with multiple formats |
| **Forking Test** | `forking_test.xml` | Multiple 180 responses, single 200 OK | Early dialog management |

### ⚡ Stress & Reliability Tests

| Scenario | File | Description | Key Features |
|----------|------|-------------|--------------|
| **Stress Test** | `stress_test.xml` | Rapid call setup/teardown | Performance under load |
| **Timeout Test** | `timeout_test.xml` | Extended timeouts and delays | Timeout handling robustness |

## 🚀 Quick Start

### Prerequisites

1. **Install SIPp**: Download from [SIPp GitHub](https://github.com/SIPp/sipp)
   ```bash
   # macOS with Homebrew
   brew install sipp
   
   # Ubuntu/Debian
   sudo apt-get install sipp
   
   # Build from source
   git clone https://github.com/SIPp/sipp.git
   cd sipp && make
   ```

2. **Start Session-Core Server**:
   ```bash
   cd rvoip/crates/session-core
   cargo run --example sipp_server
   ```

### Running Tests

#### Run All Tests
```bash
./run_tests.sh
```

#### Run Specific Test Categories
```bash
# Basic functionality only
./run_tests.sh basic

# Stress tests only  
./run_tests.sh stress
```

#### Run Individual Tests
```bash
# Basic call test
sipp -sf basic_call.xml 127.0.0.1:5060

# Hold/resume test
sipp -sf hold_resume.xml 127.0.0.1:5060

# Stress test with 100 calls at 10 cps
sipp -sf stress_test.xml -m 100 -r 10 127.0.0.1:5060
```

## 📊 Test Results

Test results are automatically saved to the `results/` directory:

- **Log files**: `results/{test_name}.log` - Detailed SIP message traces
- **CSV files**: `results/{test_name}.csv` - Call statistics and timing data

### Key Metrics Tracked

- **Call Setup Time**: Time from INVITE to 200 OK
- **Call Duration**: Total call time
- **Success Rate**: Percentage of successful calls
- **Response Times**: Individual message timing
- **Error Rates**: Failed call statistics

## 🎵 Media Flow Testing

In addition to SIP signaling tests, we provide **media flow testing** that validates actual RTP packet exchange:

### Media Test Scenarios

| Scenario | File | Media Features Tested |
|----------|------|----------------------|
| **Media Flow Test** | `media_flow_test.xml` | RTP packet exchange, codec negotiation, media parameter changes |
| **Basic Call with RTP** | Uses `basic_call.xml` | RTP flow during standard call |
| **Hold/Resume with RTP** | Uses `hold_resume.xml` | Media pause/resume validation |

### Running Media Tests

```bash
# Run all media tests with RTP validation
./run_media_tests.sh

# Setup media test environment
./run_media_tests.sh setup

# Run basic media test only
./run_media_tests.sh basic
```

### Media Testing Features

The media test runner provides:

1. **RTP Packet Capture** - Uses `tcpdump` to capture actual RTP traffic
2. **Audio File Generation** - Creates test tones using `sox` or `ffmpeg`
3. **Packet Analysis** - Analyzes RTP flow with `tshark` (if available)
4. **Media Statistics** - Tracks packet counts, timing, and quality metrics

### Prerequisites for Media Testing

- **SIPp** - SIP protocol testing tool
- **tcpdump** - Packet capture (requires sudo)
- **sox** or **ffmpeg** - Audio file generation (optional)
- **tshark** - Advanced RTP analysis (optional)

### What Gets Tested

#### ✅ **SIP Signaling Layer**
- SDP offer/answer negotiation
- Codec selection and parameters
- Media direction attributes (`sendrecv`, `sendonly`, etc.)
- Re-INVITE media parameter changes

#### ✅ **RTP Media Layer** (New!)
- Actual RTP packet transmission
- Packet timing and sequencing
- Audio payload delivery
- Media flow during hold/resume

#### ❌ **Limitations**
- **Audio Quality**: No codec decoding/analysis
- **Echo/Delay**: No acoustic quality measurements  
- **Jitter/Loss**: Basic packet analysis only

### Media Test Results

Results are saved to `media_results/` directory:
- **`{test}_rtp.pcap`** - Captured RTP packets
- **`{test}.log`** - SIP message traces with RTP info
- **`{test}.csv`** - Call statistics and timing data

## 🔍 Scenario Details

### Basic Call (`basic_call.xml`)
Tests the fundamental SIP call flow:
```
UAC                    UAS (session-core)
 |                            |
 |--- INVITE (SDP) ---------->|
 |<-- 100 Trying (optional) --|
 |<-- 180 Ringing (optional) -|
 |<-- 200 OK (SDP) -----------|
 |--- ACK ------------------->|
 |                            |
 |--- BYE ------------------->|
 |<-- 200 OK -----------------|
```

### Hold/Resume (`hold_resume.xml`)
Tests media direction changes via re-INVITE:
```
1. Initial INVITE with a=sendrecv
2. re-INVITE with a=sendonly (HOLD)
3. re-INVITE with a=sendrecv (RESUME)
4. BYE to terminate
```

### Early Media (`early_media.xml`)
Tests early media establishment:
```
UAC                    UAS (session-core)
 |                            |
 |--- INVITE (SDP) ---------->|
 |<-- 100 Trying --------------|
 |<-- 183 Session Progress ---|  (with SDP)
 |    (Early Media Period)    |
 |<-- 180 Ringing ------------|
 |<-- 200 OK (SDP) -----------|
 |--- ACK ------------------->|
```

### Multiple Codecs (`multiple_codecs.xml`)
Tests codec negotiation:
```
1. INVITE with multiple codecs (PCMU, PCMA, G729, DTMF)
2. Server selects preferred codec in 200 OK
3. re-INVITE to change codec preference
4. Server responds with new codec selection
```

### Forking Test (`forking_test.xml`)
Tests multiple early dialogs:
```
UAC                    UAS (session-core)
 |                            |
 |--- INVITE (SDP) ---------->|
 |<-- 100 Trying --------------|
 |<-- 180 Ringing (tag=1) ----|  Early Dialog 1
 |<-- 180 Ringing (tag=2) ----|  Early Dialog 2  
 |<-- 180 Ringing (tag=3) ----|  Early Dialog 3
 |<-- 200 OK (tag=2) ---------|  Winner: Dialog 2
 |--- ACK (tag=2) ----------->|
```

### Stress Test (`stress_test.xml`)
Optimized for high-volume testing:
- Minimal call duration (100ms)
- Rapid setup/teardown cycles
- Configurable call rate and volume
- Performance metrics collection

## 🛠️ Customization

### Modifying Test Parameters

Edit the test runner script variables:
```bash
# Server configuration
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"

# Client configuration  
CLIENT_IP="127.0.0.1"
CLIENT_PORT="5061"
```

### Creating Custom Scenarios

1. **Copy an existing scenario**:
   ```bash
   cp basic_call.xml my_custom_test.xml
   ```

2. **Modify the SIP flow** as needed

3. **Add to test runner**:
   ```bash
   # Add to run_tests.sh
   run_test "$SCENARIOS_DIR/my_custom_test.xml" "my_custom_test" 1 1
   ```

### SIPp Command Line Options

Common SIPp options for customization:
```bash
# Call volume and rate
-m 100          # 100 calls total
-r 10           # 10 calls per second

# Timing
-d 5000         # 5 second call duration
-i 1000         # 1 second interval between calls

# Network
-i 192.168.1.10 # Local IP address
-p 5061         # Local port

# Logging
-trace_msg      # Full message trace
-trace_shortmsg # Short message trace
-message_file   # Log file location
```

## 🔧 Troubleshooting

### Common Issues

1. **Server Not Running**:
   ```
   Error: Server is not running on 127.0.0.1:5060
   ```
   **Solution**: Start the session-core server:
   ```bash
   cargo run --example sipp_server
   ```

2. **SIPp Not Found**:
   ```
   Error: SIPp is not installed or not in PATH
   ```
   **Solution**: Install SIPp (see Prerequisites)

3. **Port Already in Use**:
   ```
   bind: Address already in use
   ```
   **Solution**: Change client port in test runner or kill existing processes

4. **Test Timeouts**:
   ```
   recv timeout
   ```
   **Solution**: Check server logs, increase timeout values, or verify scenario logic

### Debug Mode

Run individual tests with verbose output:
```bash
sipp -sf basic_call.xml -trace_msg -trace_shortmsg 127.0.0.1:5060
```

### Log Analysis

Check detailed logs in `results/` directory:
```bash
# View SIP message flow
cat results/basic_call.log

# View call statistics
cat results/basic_call.csv
```

## 📈 Performance Benchmarks

### Expected Performance Targets

| Test Type | Calls | Rate (cps) | Success Rate | Avg Setup Time |
|-----------|-------|------------|--------------|----------------|
| Basic Call | 1 | 1 | 100% | < 50ms |
| Hold/Resume | 1 | 1 | 100% | < 100ms |
| Stress Test | 100 | 10 | > 99% | < 100ms |
| Burst Test | 50 | 20 | > 95% | < 200ms |

### Monitoring

The test runner provides real-time feedback:
- ✅ **Green**: Test passed
- ❌ **Red**: Test failed  
- 📊 **Summary**: Overall results

## 🤝 Contributing

### Adding New Test Scenarios

1. **Identify the test case**: What specific functionality needs testing?
2. **Create the XML scenario**: Follow existing patterns
3. **Add to test runner**: Include in `run_tests.sh`
4. **Document the scenario**: Update this README
5. **Test thoroughly**: Verify against session-core server

### Best Practices

- **Keep scenarios focused**: One test per specific functionality
- **Use meaningful names**: Clear scenario and variable names
- **Include comments**: Document the test purpose and flow
- **Handle timeouts**: Set appropriate timeout values
- **Validate responses**: Check critical headers and status codes

## 📚 References

- [SIPp Documentation](http://sipp.sourceforge.net/doc/reference.html)
- [RFC 3261 - SIP Protocol](https://tools.ietf.org/html/rfc3261)
- [RFC 3264 - SDP Offer/Answer](https://tools.ietf.org/html/rfc3264)
- [Session-Core Documentation](../README.md) 