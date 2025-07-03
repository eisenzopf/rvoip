# 🚀 Quick Start Guide - Session-Core SIPp Test Suite

## **One-Command Testing**

The enhanced SIPp test suite provides **one script that does everything** - automatic server management, audio generation, packet capture, and comprehensive reporting.

## **✨ Run Everything**

```bash
# Complete test suite (recommended)
sudo ./scripts/run_all_tests.sh

# Basic SIP tests only  
sudo ./scripts/run_all_tests.sh basic

# Bridge testing (2-party)
sudo ./scripts/run_all_tests.sh bridge

# Stress testing (high volume)
sudo ./scripts/run_all_tests.sh stress

# Setup environment only (no sudo needed)
./scripts/run_all_tests.sh setup
```

## **📋 Prerequisites**

1. **Install SIPp**:
   ```bash
   brew install sipp                # macOS
   sudo apt-get install sipp        # Ubuntu
   ```

2. **Install audio tools** (optional but recommended):
   ```bash
   brew install sox                 # macOS
   sudo apt-get install sox         # Ubuntu
   ```

3. **Ensure sudo access** (required for packet capture):
   ```bash
   sudo echo "Testing sudo access"
   ```

## **🎯 What You Get**

After running `sudo ./scripts/run_all_tests.sh`, you'll have:

### **📁 Organized Results**
```
sipp_tests/
├── logs/test_session_TIMESTAMP/     # All logs organized by test
├── captures/*.pcap                  # RTP packet captures for analysis
├── reports/test_summary_TIMESTAMP.html  # Complete HTML report
├── audio/generated/*.wav            # Test audio files (440Hz, 880Hz, 1320Hz)
└── audio/captured/                  # Captured audio streams
```

### **📊 Test Coverage**
- ✅ **Basic SIP Tests**: Core INVITE/ACK/BYE functionality
- ✅ **Bridge Tests**: 2-party call simulation
- 📋 **Conference Tests**: 3+ party (planned for Phase 3)
- ✅ **Stress Tests**: Concurrent call handling

### **📡 Comprehensive Capture**
- **Server Logs**: session-core application logs
- **SIPp Logs**: Industry-standard SIP client logs  
- **Packet Captures**: Complete RTP/SIP network traffic
- **Audio Files**: Generated test tones for different clients
- **Analysis Reports**: Automated tshark packet analysis

## **🔧 Test Modes**

| Mode | Description | Duration | Use Case |
|------|-------------|----------|----------|
| `basic` | Core SIP functionality | ~10s | Quick validation |
| `bridge` | 2-party bridging | ~20s | Bridge testing |
| `conference` | Multi-party calls | ~25s | Future feature |
| `stress` | High-volume testing | ~30s | Performance validation |
| `all` | Complete suite | ~90s | Comprehensive testing |

## **📈 Results Analysis**

### **Automatic Analysis**
- **HTML Report**: Visual summary of all tests
- **Packet Analysis**: tshark-based RTP flow analysis
- **Log Correlation**: Cross-reference server and SIPp logs
- **Audio Validation**: Frequency analysis of captured audio

### **Manual Analysis**
```bash
# View comprehensive HTML report
open reports/test_summary_TIMESTAMP.html

# Analyze packet captures
wireshark captures/basic_tests_TIMESTAMP.pcap

# Check server logs
tail -f logs/test_session_TIMESTAMP/basic_server.log
```

## **🐛 Troubleshooting**

### **Common Issues**

1. **"Missing sudo access"**:
   ```bash
   # Run with sudo for packet capture
   sudo ./scripts/run_all_tests.sh
   ```

2. **"SIPp not found"**:
   ```bash
   # Install SIPp
   brew install sipp  # macOS
   sudo apt-get install sipp  # Ubuntu
   ```

3. **"Port already in use"**:
   ```bash
   # Check what's using the port
   lsof -i :5062
   
   # Kill existing processes
   pkill -f "sip_test_server"
   ```

4. **"No audio files generated"**:
   ```bash
   # Install audio tools
   brew install sox  # macOS
   sudo apt-get install sox  # Ubuntu
   ```

### **Debug Mode**
```bash
# Run with extra logging
RUST_LOG=debug sudo ./scripts/run_all_tests.sh basic

# Setup only (no tests)
./scripts/run_all_tests.sh setup
```

## **🎵 Audio Testing**

The suite generates test audio files with different frequencies:
- **Client A**: 440Hz (A4 note) 
- **Client B**: 880Hz (A5 note)
- **Client C**: 1320Hz (E6 note)

This allows validation of multi-party audio mixing and bridge functionality.

## **🔗 Integration with Existing Tests**

This enhanced suite builds on and complements:
- ✅ **test_inbound.sh**: Enhanced with organized logging
- ✅ **sip_test_server.rs**: Used as foundation for all tests
- ✅ **basic_call.xml**: Working SIPp scenario
- 🔄 **Bridge patterns**: Integrated from existing bridge tests
- 🔄 **Media patterns**: Integrated from existing media tests

## **📚 Next Steps**

1. **Run the complete suite**:
   ```bash
   sudo ./scripts/run_all_tests.sh
   ```

2. **Review the HTML report** to understand test coverage

3. **Add custom scenarios** by creating new XML files in `scenarios/`

4. **Integrate with CI/CD** using the generated JUnit XML reports

---

**🎉 Ready to test? Just run:** `sudo ./scripts/run_all_tests.sh` 