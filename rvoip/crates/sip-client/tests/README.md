# RVOIP SIP Client Integration Tests

This directory contains comprehensive integration tests for the RVOIP SIP client, focusing on peer-to-peer SIP communication with audio verification.

## Quick Start

```bash
# Run the full integration test
./integration_test.sh

# View help
./integration_test.sh --help

# Clean up test files
./integration_test.sh --cleanup

# Analyze existing logs only
./integration_test.sh --logs-only
```

## Test Overview

The integration test simulates a real-world SIP communication scenario:

1. **Alice (Receiver)**: Starts on port 5061, waits for incoming calls with auto-answer
2. **Bob (Caller)**: Starts on port 5062, makes a call to Alice
3. **Audio Testing**: Uses test WAV files to verify audio transmission (future)
4. **Monitoring**: Tracks call setup, connection, and completion
5. **Analysis**: Generates detailed results with pass/fail status

## Test Architecture

```
┌─────────────┐     SIP INVITE      ┌─────────────┐
│   Bob       │ ───────────────────> │   Alice     │
│ (Caller)    │                     │ (Receiver)  │
│ Port 5062   │ <─────────────────── │ Port 5061   │
└─────────────┘     SIP 200 OK      └─────────────┘
       │                                    │
       │            RTP Audio               │
       └────────────────────────────────────┘
```

## Test Components

### 1. Network Configuration
- **Alice SIP Port**: 5061
- **Bob SIP Port**: 5062  
- **Alice Media Port**: 6001 (future)
- **Bob Media Port**: 6002 (future)
- **Protocol**: SIP over UDP, RTP for audio

### 2. Test Files
- `tests/logs/alice.log` - Alice's detailed log
- `tests/logs/bob.log` - Bob's detailed log
- `tests/results/test_result.json` - Structured test results
- `tests/audio/alice_says.wav` - Alice's test audio file
- `tests/audio/bob_says.wav` - Bob's test audio file

### 3. Test Phases

#### Phase 1: Environment Setup
- Build rvoip-sip-client binary
- Create test directories
- Generate test audio files (with ffmpeg if available)

#### Phase 2: Client Startup
- Start Alice in receive mode with auto-answer
- Start Bob in call mode targeting Alice
- Monitor process health

#### Phase 3: SIP Communication
- Bob initiates SIP INVITE to Alice
- Alice auto-answers the call
- Both parties maintain call for configured duration
- Bob hangs up after timeout

#### Phase 4: Results Analysis
- Parse logs for success/failure indicators
- Generate JSON results file
- Display human-readable summary

## Expected Results

### Successful Test Output
```
🧪 RVOIP SIP CLIENT INTEGRATION TEST RESULTS
════════════════════════════════════════
📅 Test completed: Wed May 30 19:45:00 UTC 2025

📊 Test Results:
   Alice registered:    ✅ YES
   Bob registered:      ✅ YES  
   Call initiated:      ✅ YES
   Call connected:      ✅ YES
   Call completed:      ✅ YES
   Audio transmitted:   🚧 TODO
   Errors found:        ✅ NO

🎉 SIP COMMUNICATION TEST PASSED!
   ✅ Peer-to-peer SIP communication is working
   ✅ Call setup and teardown successful
```

## Test Verification Points

### SIP Protocol Compliance
- [x] **UDP Transport**: Real SIP messages over UDP
- [x] **SIP Headers**: Proper Via, From, To, Call-ID headers
- [x] **Transaction Handling**: RFC 3261 transaction state machine
- [x] **Message Flow**: INVITE → 100 Trying → 180 Ringing → 200 OK → ACK → BYE

### Call Lifecycle
- [x] **Registration**: Optional SIP registration (for future)
- [x] **Call Setup**: INVITE request/response handling
- [x] **Call Connection**: 200 OK and ACK exchange
- [x] **Call Teardown**: BYE request/response

### Media Handling (Future)
- [ ] **RTP Setup**: Media session establishment
- [ ] **Audio Transmission**: WAV file playback/recording
- [ ] **Codec Support**: PCMU/PCMA codec negotiation
- [ ] **Audio Verification**: Confirm received audio matches sent

## Troubleshooting

### Common Issues

#### Port Conflicts
If ports 5061/5062 are in use:
```bash
# Check what's using the ports
lsof -i :5061
lsof -i :5062

# Kill conflicting processes
pkill -f rvoip-sip-client
```

#### Build Failures
```bash
# Clean and rebuild
cd .. && cargo clean && cargo build --bin rvoip-sip-client
```

#### Test Hangs
```bash
# Force cleanup
./integration_test.sh --cleanup
pkill -f rvoip-sip-client
```

### Log Analysis

#### Check Alice's Log
```bash
# Look for key events
grep -E "(registered|incoming|answered|ended)" tests/logs/alice.log

# Check for errors
grep -E "(ERROR|Failed|❌)" tests/logs/alice.log
```

#### Check Bob's Log  
```bash
# Look for key events
grep -E "(registered|calling|connected|completed)" tests/logs/bob.log

# Check for errors
grep -E "(ERROR|Failed|❌)" tests/logs/bob.log
```

## Audio Testing (Future Enhancement)

The test infrastructure is designed to support audio verification:

1. **Test Audio Generation**: Creates 440Hz and 880Hz sine wave tones
2. **Audio Playback**: CLI client plays audio file during call
3. **Audio Recording**: CLI client records received audio
4. **Audio Verification**: Compare sent vs received audio characteristics

### Requirements for Audio Testing
- `ffmpeg` for audio file generation
- Audio device access for playback/recording
- Extended CLI commands for audio file handling

## Development Notes

### Adding New Test Cases
1. Create test functions in `integration_test.sh`
2. Add result verification in `analyze_results()`
3. Update expected results in this README

### Extending Audio Support
1. Add audio CLI parameters to sip-client
2. Integrate with media-core for file playback/recording
3. Implement audio comparison algorithms

### Performance Testing
- Call volume testing (multiple simultaneous calls)
- Network condition simulation (packet loss, jitter)
- Load testing with call center scenarios

## Integration with CI/CD

The test can be integrated into automated builds:

```yaml
# Example GitHub Actions workflow
- name: Run SIP Integration Tests
  run: |
    cd crates/sip-client
    ./tests/integration_test.sh
    
- name: Upload Test Results
  uses: actions/upload-artifact@v3
  with:
    name: sip-test-results
    path: crates/sip-client/tests/results/
```

## Related Documentation

- [SIP Client API Documentation](../src/lib.rs)
- [Configuration Guide](../src/config.rs)
- [Call Engine Integration](../../call-engine/README.md)
- [Media Core Documentation](../../media-core/README.md) 