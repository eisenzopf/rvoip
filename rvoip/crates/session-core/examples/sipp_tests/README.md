# 🧪 SIPp Integration Tests

Comprehensive automated testing suite using SIPp to validate the session-core SIP implementation with real network traffic capture and audio verification.

## 📋 Overview

This testing infrastructure provides:

- **Interoperability Testing**: Validate session-core against industry-standard SIPp scenarios
- **RFC 3261 Compliance**: Ensure 100% compliance with external SIP implementations  
- **Performance Validation**: Test concurrent call handling and resource management
- **Audio Verification**: Confirm RTP stream establishment and audio quality
- **Regression Testing**: Automated testing for CI/CD integration

## 🏗️ Architecture

```
sipp_tests/
├── src/
│   ├── bin/
│   │   ├── sip_test_server.rs     # UAS - receives calls from SIPp
│   │   ├── sip_test_client.rs     # UAC - makes calls to SIPp UAS  
│   │   └── sip_echo_server.rs     # Advanced echo/conference server
│   ├── lib.rs                     # Common test utilities
│   └── config.rs                  # Test configuration
├── scenarios/
│   ├── sipp_to_rust/              # SIPp calls our Rust apps
│   │   └── basic_call.xml         # Simple INVITE/200/ACK/BYE
│   └── rust_to_sipp/              # Our Rust apps call SIPp
├── scripts/
│   ├── test_inbound.sh            # SIPp → Rust tests
│   └── test_outbound.sh           # Rust → SIPp tests
├── configs/
│   └── test_config.yaml           # Test configuration
└── audio/                         # Audio test files
```

## 🚀 Quick Start

### Prerequisites

1. **Install SIPp**:
   ```bash
   # macOS
   brew install sipp
   
   # Ubuntu
   sudo apt-get install sipp
   ```

2. **Build test applications**:
   ```bash
   cd examples/sipp_tests
   cargo build
   ```

### Basic Usage

1. **Run basic inbound test** (SIPp calls our server):
   ```bash
   ./scripts/test_inbound.sh basic_call
   ```

2. **Manual testing**:
   ```bash
   # Terminal 1: Start our SIP test server
   cargo run --bin sip_test_server -- --port 5062 --mode auto-answer
   
   # Terminal 2: Run SIPp scenario
   sipp -sf scenarios/sipp_to_rust/basic_call.xml 127.0.0.1:5062
   ```

## 🧪 Test Applications

### SIP Test Server (`sip_test_server`)

UAS that receives calls from SIPp with configurable response behavior.

```bash
cargo run --bin sip_test_server -- --help

# Examples:
cargo run --bin sip_test_server -- --port 5062 --mode auto-answer
cargo run --bin sip_test_server -- --port 5062 --mode busy
cargo run --bin sip_test_server -- --port 5062 --mode random
```

**Features**:
- Auto-answer, busy, not-found, or random responses
- Call statistics and metrics collection
- Clean shutdown and resource management
- Configurable timeouts and logging

### SIP Test Client (`sip_test_client`)

UAC that makes calls to SIPp UAS scenarios.

```bash
cargo run --bin sip_test_client -- --help

# Examples:
cargo run --bin sip_test_client -- --target 127.0.0.1:5060 --calls 10
cargo run --bin sip_test_client -- --target 127.0.0.1:5060 --rate 2.0
```

**Features** (planned):
- Configurable call patterns
- DTMF sequence generation
- Hold/resume operations
- Concurrent call generation
- Performance metrics

### SIP Echo Server (`sip_echo_server`)

Advanced test server for audio verification.

```bash
cargo run --bin sip_echo_server -- --help

# Examples:
cargo run --bin sip_echo_server -- --port 5063 --delay 100
```

**Features** (planned):
- Audio echo with configurable delay
- Multiple codec support
- Audio quality analysis
- Jitter and packet loss simulation

## 📝 Test Scenarios

### Available Scenarios

| Scenario | Description | SIPp Role | Our App Role |
|----------|-------------|-----------|--------------|
| `basic_call` | Simple INVITE/200/ACK/BYE | UAC | UAS (server) |
| `call_with_dtmf` | Call + INFO (DTMF) | UAC | UAS |
| `call_with_hold` | Call + UPDATE (hold/resume) | UAC | UAS |
| `call_rejection` | INVITE → 486 Busy Here | UAC | UAS |
| `early_media` | INVITE → 183 + early media | UAC | UAS |
| `stress_test` | Multiple concurrent calls | UAC | UAS |

### Creating New Scenarios

1. **Add SIPp XML file**:
   ```bash
   # For inbound tests (SIPp → Rust)
   touch scenarios/sipp_to_rust/my_scenario.xml
   
   # For outbound tests (Rust → SIPp)  
   touch scenarios/rust_to_sipp/my_scenario.xml
   ```

2. **Run the scenario**:
   ```bash
   ./scripts/test_inbound.sh my_scenario
   ```

## ⚙️ Configuration

### Test Configuration (`configs/test_config.yaml`)

```yaml
session_core:
  server:
    sip_port: 5062
    auto_answer: true
    log_level: "info"
  client:
    local_port: 5061
    default_target: "127.0.0.1:5060"

sipp:
  binary_path: "sipp"
  default_rate: 1
  max_concurrent: 100
  timeout: 30

capture:
  interface: "lo0"  # macOS loopback
  enabled: true

reporting:
  output_dir: "./reports"
  formats: ["Html", "Junit", "Json"]
```

### Environment Variables

```bash
# Test script configuration
export RUST_SERVER_PORT=5062
export SIPP_PORT=5060
export TEST_DURATION=30
export CALL_RATE=1
export MAX_CALLS=10
export CAPTURE_ENABLED=true
```

## 📊 Reports and Analysis

Test results are automatically generated in multiple formats:

- **HTML Report**: `reports/test_report.html` - Visual test results
- **JUnit XML**: `reports/junit_results.xml` - CI/CD integration
- **JSON Data**: `reports/test_data.json` - Programmatic analysis
- **Packet Captures**: `captures/*.pcap` - Network analysis

## 🔧 Development

### Implementation Status

- ✅ **Infrastructure**: Directory structure, build system, configuration
- ✅ **Basic Server**: SIP test server with response mode support
- ✅ **Test Scripts**: Automated test execution with packet capture
- ✅ **SIPp Scenarios**: Basic call flow scenario
- 🔄 **In Progress**: Session-core integration, additional scenarios
- 📋 **Planned**: Client implementation, echo server, audio testing

### Adding Features

1. **New Test Application**:
   ```bash
   touch src/bin/my_test_app.rs
   # Add binary entry to Cargo.toml
   ```

2. **New Test Utility**:
   ```rust
   // Add to src/lib.rs
   pub mod my_utils {
       // Implementation
   }
   ```

3. **New Configuration Option**:
   ```rust
   // Add to src/config.rs
   pub struct MyConfig {
       // New fields
   }
   ```

## 🎯 Testing Matrix

| Test Type | Priority | Status | Description |
|-----------|----------|--------|-------------|
| Basic Call Flow | P0 | ✅ | INVITE/200/ACK/BYE sequence |
| DTMF Handling | P0 | 📋 | INFO method DTMF reception |
| Hold/Resume | P1 | 📋 | UPDATE method SDP modification |
| Call Rejection | P1 | 📋 | Error response handling |
| Early Media | P2 | 📋 | 180/183 responses, early RTP |
| Concurrent Calls | P1 | 📋 | Performance, resource management |
| Stress Testing | P2 | 📋 | High-volume call processing |
| Audio Quality | P2 | 📋 | RTP streams, codec negotiation |

## 🚨 Troubleshooting

### Common Issues

1. **Port conflicts**:
   ```bash
   # Check what's using the port
   lsof -i :5062
   
   # Use different port
   cargo run --bin sip_test_server -- --port 5063
   ```

2. **SIPp not found**:
   ```bash
   # Install SIPp
   brew install sipp  # macOS
   sudo apt-get install sipp  # Ubuntu
   ```

3. **Permission denied (packet capture)**:
   ```bash
   # Run with sudo or disable capture
   CAPTURE_ENABLED=false ./scripts/test_inbound.sh basic_call
   ```

4. **Compilation errors**:
   ```bash
   # Clean rebuild
   cargo clean
   cargo build
   ```

## 📚 References

- [SIPp Documentation](http://sipp.sourceforge.net/doc/)
- [RFC 3261 - SIP Protocol](https://tools.ietf.org/html/rfc3261)
- [Session-core API](../README.md)
- [Test Plan](../TEST_PLAN.md)

---

*This testing infrastructure ensures comprehensive validation of session-core against industry-standard SIP implementations, providing confidence in production readiness and RFC compliance.* 