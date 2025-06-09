# 🧪 **SIPp Integration Testing Plan** - Enhanced

## **Overview**
This document outlines a comprehensive automated testing suite using SIPp to validate the session-core SIP implementation with real network traffic capture and audio verification. The plan builds on our existing excellent `sipp_tests` infrastructure to provide **one-script-runs-everything** testing with comprehensive capture and analysis.

## **🎯 Objectives**

1. **Interoperability Testing**: Validate session-core against industry-standard SIPp scenarios
2. **RFC 3261 Compliance**: Ensure 100% compliance with external SIP implementations  
3. **Performance Validation**: Test concurrent call handling and resource management
4. **Audio Verification**: Confirm RTP stream establishment and audio quality
5. **Bridge/Conference Testing**: Multi-party call and conferencing validation
6. **Automated Regression Testing**: One-command CI/CD integration

## **🚀 Single-Script Architecture**

### **Core Philosophy: One Command Does Everything**
```bash
# Complete test suite with automatic everything
sudo ./scripts/run_all_tests.sh

# Specific test modes
sudo ./scripts/run_all_tests.sh basic      # Basic SIP flows
sudo ./scripts/run_all_tests.sh bridge     # 2-party bridging  
sudo ./scripts/run_all_tests.sh conference # Multi-party conferencing
sudo ./scripts/run_all_tests.sh stress     # High-volume testing
sudo ./scripts/run_all_tests.sh all        # Everything (default)
```

### **What The Single Script Does**
1. **✅ Prerequisites Check**: SIPp, cargo, sudo, tcpdump, sox/ffmpeg
2. **🚀 Server Management**: Auto-start/stop session-core test servers
3. **🎵 Audio Generation**: Create test tones at different frequencies
4. **📡 Packet Capture**: tcpdump for comprehensive RTP analysis
5. **🧪 Test Execution**: Run all SIPp scenarios with logging
6. **📊 Result Analysis**: Parse logs, pcap, and generate reports
7. **🧹 Cleanup**: Automatic cleanup even on failures

## **📋 Enhanced Directory Structure**

### **Current Excellent Foundation**
```
examples/sipp_tests/                 # ✅ Already excellent structure
├── src/
│   ├── bin/
│   │   ├── sip_test_server.rs      # ✅ Working UAS with session-core
│   │   ├── sip_test_client.rs      # 🔄 Complete UAC implementation
│   │   ├── sip_echo_server.rs      # 🔄 Audio echo/conference server
│   │   └── sip_bridge_server.rs    # 🆕 Multi-party bridge server
│   ├── lib.rs                      # ✅ Common utilities
│   └── config.rs                   # ✅ Configuration management
├── scenarios/
│   ├── sipp_to_rust/               # SIPp calls our Rust apps
│   │   ├── basic_call.xml          # ✅ Already exists
│   │   ├── call_with_dtmf.xml      # 🆕 INFO method DTMF
│   │   ├── call_with_hold.xml      # 🆕 UPDATE hold/resume
│   │   ├── call_rejection.xml      # 🆕 Busy/Not Found responses
│   │   ├── early_media.xml         # 🆕 183 Session Progress
│   │   ├── concurrent_calls.xml    # 🆕 Stress testing
│   │   ├── bridge_2party.xml       # 🆕 2-party bridge test
│   │   └── conference_3party.xml   # 🆕 3-party conference test
│   └── rust_to_sipp/               # Our Rust apps call SIPp
│       ├── outbound_call.xml       # 🆕 Basic outbound scenario
│       ├── outbound_dtmf.xml       # 🆕 Outbound with DTMF
│       └── load_test.xml           # 🆕 High-volume load testing
├── scripts/
│   ├── run_all_tests.sh            # 🆕 MAIN: One script runs everything
│   ├── test_inbound.sh             # ✅ Excellent - minor enhancements
│   ├── test_outbound.sh            # 🆕 Rust → SIPp tests
│   ├── test_bridge.sh              # 🆕 Bridge/conference tests
│   ├── test_audio.sh               # 🆕 Audio verification tests
│   └── setup_environment.sh        # 🆕 Prerequisites check
├── logs/                           # ✅ Working - organized by test
│   ├── basic_test_TIMESTAMP_server.log
│   ├── basic_test_TIMESTAMP_sipp.log
│   ├── bridge_test_TIMESTAMP_server.log
│   ├── conference_test_TIMESTAMP_server.log
│   └── test_execution_TIMESTAMP.log
├── captures/                       # ✅ Working - RTP pcap files
│   ├── basic_test_TIMESTAMP.pcap
│   ├── bridge_test_TIMESTAMP.pcap
│   ├── conference_test_TIMESTAMP.pcap
│   └── network_analysis/
├── audio/                          # 🆕 Generated and captured audio
│   ├── generated/
│   │   ├── client_a_440hz.wav      # Test tone A (440Hz)
│   │   ├── client_b_880hz.wav      # Test tone B (880Hz)
│   │   ├── client_c_1320hz.wav     # Test tone C (1320Hz)
│   │   └── dtmf_sequence.wav       # DTMF tones
│   └── captured/
│       ├── bridge_mixed_audio.wav  # Bridge output
│       └── conference_audio.wav    # Conference mixing
├── reports/                        # ✅ Working - enhanced reporting
│   ├── test_summary_TIMESTAMP.html # Complete test report
│   ├── basic_test_TIMESTAMP.csv    # SIPp statistics
│   ├── bridge_analysis_TIMESTAMP.html
│   └── junit_results.xml           # CI/CD integration
└── configs/
    ├── test_config.yaml            # ✅ Working configuration
    └── sipp_defaults.yaml          # SIPp scenario defaults
```

## **🎯 Enhanced Test Applications**

### **1. SIP Test Server (`sip_test_server.rs`)** ✅ Excellent Foundation
**Current Status**: Working excellently with session-core integration

**Enhancements**:
- ✅ Auto-answer, busy, not-found, random responses (already working)
- 🔄 Add DTMF INFO request handling and logging
- 🔄 Add UPDATE hold/resume support
- 🔄 Enhanced statistics and metrics
- ✅ Clean shutdown and resource management (already working)

### **2. SIP Test Client (`sip_test_client.rs`)** 🔄 Complete Implementation
**Purpose**: UAC that makes calls to SIPp UAS scenarios

**Features** (to implement):
- Make calls to SIPp UAS configurations  
- Configurable call patterns (single, burst, sustained load)
- Send DTMF sequences via INFO requests
- Initiate hold/resume via UPDATE requests
- Concurrent call generation for stress testing
- Performance metrics collection

### **3. SIP Bridge Server (`sip_bridge_server.rs`)** 🆕 New Application
**Purpose**: Multi-party bridge/conference server for advanced testing

**Features**:
- 2-party bridge calls (like existing bridge tests)
- N-way conferencing (3+ participants)
- Audio mixing and routing
- Bridge creation/destruction logging
- Performance metrics for concurrent bridges

## **🧪 Comprehensive Test Scenarios Matrix**

| Test Scenario | Priority | Implementation | Validation Focus |
|---------------|----------|----------------|------------------|
| **Basic Call Flow** | P0 | ✅ Working | SIP compliance, call establishment |
| **DTMF Handling** | P0 | 🔄 Implement | INFO method, DTMF reception |
| **Hold/Resume** | P1 | 🔄 Implement | UPDATE method, SDP modification |
| **Call Rejection** | P1 | 🔄 Implement | Error response handling |
| **2-Party Bridge** | P1 | 🆕 New | Bridge creation, audio routing |
| **3-Party Conference** | P2 | 🆕 New | N-way conferencing, audio mixing |
| **Concurrent Calls** | P1 | 🔄 Implement | Performance, resource management |
| **Stress Testing** | P2 | 🔄 Implement | High-volume call processing |
| **Audio Quality** | P2 | 🆕 New | RTP streams, codec negotiation |
| **Early Media** | P2 | 🔄 Implement | 180/183 responses, early RTP |

## **🎵 Audio Testing Strategy**

### **Audio Generation** (Building on Bridge Test Patterns)
```bash
# Different frequency test tones for multi-party testing
sox -n -r 8000 -c 1 -b 16 "client_a_440hz.wav" synth 30 sine 440 vol 0.5   # A4 note
sox -n -r 8000 -c 1 -b 16 "client_b_880hz.wav" synth 30 sine 880 vol 0.5   # A5 note  
sox -n -r 8000 -c 1 -b 16 "client_c_1320hz.wav" synth 30 sine 1320 vol 0.5 # E6 note

# DTMF sequence generation
# Generate standard DTMF tones for INFO testing
```

### **Audio Validation**
- **RTP Flow Analysis**: Parse pcap with tshark for RTP streams
- **Bridge Verification**: Confirm bidirectional audio in bridge scenarios
- **Conference Validation**: Verify N-way audio mixing
- **Quality Metrics**: Jitter, packet loss, codec negotiation

## **📡 Comprehensive Capture Strategy**

### **Per-Test Organized Logging** (Enhanced from Current)
```bash
# Current excellent pattern (keep and enhance):
logs/server_TIMESTAMP.log

# Enhanced organized pattern:
logs/
├── ${TEST_TYPE}_${TIMESTAMP}_server.log     # Session-core server output
├── ${TEST_TYPE}_${TIMESTAMP}_sipp.log       # SIPp client/server output
├── ${TEST_TYPE}_${TIMESTAMP}_execution.log  # Test orchestration
└── test_summary_${TIMESTAMP}.log            # Complete test results
```

### **RTP Packet Capture** (Enhanced from Current)
```bash
# Current working pattern (keep and enhance):
sudo tcpdump -i lo0 -w "$capture_file" "port $SIPP_PORT or port $RUST_SERVER_PORT"

# Enhanced comprehensive pattern:
sudo tcpdump -i lo0 -w "captures/${TEST_TYPE}_${TIMESTAMP}.pcap" \
    "port 5060 or port 5061 or port 5062 or portrange 10000-20000"
```

### **Analysis and Reporting**
```bash
# Automatic analysis pipeline:
1. Parse SIPp CSV statistics → HTML reports
2. Analyze pcap with tshark → RTP flow validation  
3. Correlate server logs → Call success metrics
4. Generate comprehensive HTML summary report
5. Create JUnit XML for CI/CD integration
```

## **🚀 Main Test Runner (`run_all_tests.sh`)**

### **Architecture**
```bash
#!/bin/bash
# 🧪 Session-Core Complete SIPp Test Suite
# One script to rule them all!

main() {
    case "${1:-all}" in
        "basic")      run_basic_tests ;;
        "bridge")     run_bridge_tests ;;  
        "conference") run_conference_tests ;;
        "stress")     run_stress_tests ;;
        "setup")      setup_environment_only ;;
        "all"|*)      run_complete_suite ;;
    esac
}

run_complete_suite() {
    log_header "🧪 Session-Core Complete Test Suite"
    
    # Prerequisites and setup
    check_prerequisites_and_sudo
    setup_test_environment
    generate_audio_files
    
    # Start infrastructure
    start_packet_capture_master
    
    # Run test phases
    run_basic_tests           # P0: Core SIP functionality
    run_bridge_tests          # P1: 2-party bridging
    run_conference_tests      # P2: N-way conferencing  
    run_stress_tests          # P2: Performance validation
    
    # Analysis and reporting
    analyze_all_results
    generate_comprehensive_report
    cleanup_everything
    
    log_success "🎉 Complete test suite finished!"
}
```

### **Test Execution Flow**
1. **Environment Check**: Verify SIPp, cargo, sudo, tcpdump, audio tools
2. **Audio Generation**: Create test tones at different frequencies
3. **Infrastructure Start**: Master packet capture, test directories
4. **Test Phases**: 
   - Basic SIP flows (INVITE/BYE, DTMF, hold/resume)
   - Bridge testing (2-party audio bridging)
   - Conference testing (3+ party conferencing)
   - Stress testing (concurrent calls, high volume)
5. **Analysis**: Parse logs, analyze pcap, generate reports
6. **Cleanup**: Stop capture, archive results, clean processes

## **⚙️ Enhanced Configuration**

### **Test Configuration (`configs/test_config.yaml`)**
```yaml
# Build on existing excellent config
session_core:
  servers:
    basic_server:
      binary: "sip_test_server"
      port: 5062
      mode: "auto-answer"
      auto_shutdown: 60
    bridge_server:
      binary: "sip_bridge_server"  
      port: 5063
      bridge_timeout: 30
    conference_server:
      binary: "sip_conference_server"
      port: 5064
      max_participants: 10

sipp:
  binary_path: "sipp"
  scenarios_dir: "./scenarios"
  default_rate: 1
  max_concurrent: 100
  timeout: 30

capture:
  interface: "lo0"              # macOS loopback
  master_filter: "port 5060 or port 5061 or port 5062 or port 5063 or portrange 10000-20000"
  per_test_capture: true
  analysis_enabled: true

audio:
  generation:
    client_a_freq: 440          # Hz - A4 note
    client_b_freq: 880          # Hz - A5 note  
    client_c_freq: 1320         # Hz - E6 note
    duration: 30                # seconds
    sample_rate: 8000
  validation:
    quality_threshold: 95       # percent
    jitter_threshold: 50        # ms
    packet_loss_threshold: 1    # percent

bridge_testing:
  two_party:
    enabled: true
    duration: 20                # seconds
    stagger_delay: 3            # seconds between calls
  conference:
    enabled: true
    max_participants: 3
    duration: 25
    join_delay: 3               # seconds between each join

reporting:
  output_dir: "./reports"
  formats: ["html", "junit", "json"]
  include_pcap_analysis: true
  archive_old_results: true
```

## **📊 Success Metrics & Validation**

### **Functional Validation**
- **✅ Basic SIP**: 100% RFC 3261 compliant call flows
- **✅ DTMF**: Accurate INFO method DTMF reception
- **✅ Hold/Resume**: Proper UPDATE method SDP modification
- **✅ Bridge**: Successful 2-party audio bridging
- **✅ Conference**: N-way audio mixing and routing
- **✅ Error Handling**: Proper rejection and timeout handling

### **Performance Validation**  
- **Concurrent Calls**: Handle 100+ simultaneous calls
- **Call Rate**: Process 10+ calls per second
- **Bridge Performance**: 50+ concurrent bridges
- **Memory Usage**: No memory leaks during stress tests
- **Response Times**: Sub-100ms SIP response times

### **Audio Validation**
- **RTP Establishment**: 100% successful media stream setup
- **Bridge Audio**: Bidirectional audio in bridge scenarios
- **Conference Audio**: Multi-party audio mixing verification
- **Quality Metrics**: <50ms jitter, <1% packet loss
- **Codec Support**: PCMU, PCMA, Opus compatibility

## **🎯 Implementation Roadmap**

### **Phase 1: Enhanced Single-Script Infrastructure** (Current Sprint)
- [x] ✅ Excellent foundation in `sipp_tests/` (already done)
- [ ] 🔄 Create comprehensive `run_all_tests.sh`
- [ ] 🔄 Enhance `test_inbound.sh` with organized logging
- [ ] 🔄 Add missing SIPp scenarios (DTMF, hold, bridge)
- [ ] 🔄 Audio generation and capture integration

### **Phase 2: Core Test Scenarios** (Next Sprint)
- [ ] 🔄 Complete DTMF testing (INFO method)
- [ ] 🔄 Complete hold/resume testing (UPDATE method)
- [ ] 🔄 Implement bridge testing scenarios
- [ ] 🔄 Error handling and rejection scenarios
- [ ] 🔄 Basic stress testing

### **Phase 3: Advanced Bridge/Conference** (Future)
- [ ] 🆕 Multi-party conference testing
- [ ] 🆕 Advanced audio verification
- [ ] 🆕 Performance benchmarking
- [ ] 🆕 Advanced SIP features testing

### **Phase 4: CI/CD Integration** (Future)
- [ ] 🆕 GitHub Actions workflow
- [ ] 🆕 Automated regression detection
- [ ] 🆕 Performance baseline tracking
- [ ] 🆕 Release validation pipeline

## **🚀 Getting Started (Enhanced)**

### **Prerequisites**
```bash
# 1. Install SIPp
brew install sipp                    # macOS
sudo apt-get install sipp            # Ubuntu

# 2. Install audio tools (for audio generation)
brew install sox                     # macOS
sudo apt-get install sox            # Ubuntu

# 3. Ensure sudo access for packet capture
sudo echo "Sudo access confirmed"
```

### **Quick Start**
```bash
# 1. Navigate to test directory
cd examples/sipp_tests

# 2. Build test applications  
cargo build

# 3. Run complete test suite (ONE COMMAND!)
sudo ./scripts/run_all_tests.sh

# 4. Run specific test types
sudo ./scripts/run_all_tests.sh basic      # Basic SIP only
sudo ./scripts/run_all_tests.sh bridge     # Bridge testing
sudo ./scripts/run_all_tests.sh conference # Conference testing

# 5. View results
open reports/latest_test_summary.html      # Complete report
ls -la logs/                               # All logs organized by test
ls -la captures/                           # All pcap files
ls -la audio/                              # Generated and captured audio
```

### **What You Get After One Command**
```
sipp_tests/
├── logs/
│   ├── complete_suite_20250108_143022/    # Organized by test run
│   │   ├── basic_test_server.log
│   │   ├── bridge_test_server.log
│   │   ├── conference_test_server.log
│   │   └── test_execution.log
├── captures/  
│   ├── basic_test_20250108_143022.pcap     # RTP analysis ready
│   ├── bridge_test_20250108_143045.pcap
│   └── conference_test_20250108_143112.pcap
├── reports/
│   ├── test_summary_20250108.html          # Complete visual report
│   ├── junit_results.xml                   # CI/CD integration
│   └── performance_metrics.json           # Structured data
└── audio/
    ├── generated/                          # Test tones created
    └── captured/                           # Audio streams captured
```

## **📚 References**

- [RFC 3261 - SIP: Session Initiation Protocol](https://tools.ietf.org/html/rfc3261)
- [SIPp Documentation](http://sipp.sourceforge.net/doc/)
- [session-core API Documentation](../README.md)
- [Simple Peer-to-Peer Example](./simple_peer_to_peer.rs)
- [Existing Bridge Tests](./run_bridge_tests.sh) ✅ Excellent patterns
- [Existing Media Tests](./run_media_tests.sh) ✅ Great foundation

---

*This enhanced test plan builds on our excellent existing `sipp_tests` infrastructure to provide comprehensive one-script testing with complete capture, analysis, and reporting. The goal: `sudo ./scripts/run_all_tests.sh` does everything!* 