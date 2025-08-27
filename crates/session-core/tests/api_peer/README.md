# SimplePeer API Integration Tests

This test suite provides comprehensive testing for the SimplePeer API in session-core, including full peer-to-peer communication with audio exchange and recording.

## Test Modules

### 1. `peer_audio_tests.rs`
Tests actual audio exchange between peers:
- **test_peer_to_peer_audio_exchange**: Full bidirectional audio exchange with recording
  - Alice sends 440Hz tone (A4 note)
  - Bob sends 880Hz tone (A5 note)
  - Both peers record input and output to WAV files
- **test_peer_audio_with_hold_resume**: Tests hold/resume during active calls
- **test_peer_audio_with_mute_unmute**: Tests mute/unmute functionality

### 2. `peer_call_tests.rs`
Tests call control and signaling:
- **test_successful_call_establishment**: Basic call setup and teardown
- **test_call_rejection**: Call rejection scenarios
- **test_multiple_incoming_calls**: Handling multiple simultaneous incoming calls
- **test_call_with_custom_call_id**: Custom call ID usage
- **test_call_duration_tracking**: Call duration monitoring
- **test_dtmf_sending**: DTMF tone sending

### 3. `peer_concurrent_tests.rs`
Tests concurrent call scenarios:
- **test_concurrent_bidirectional_calls**: Multiple peers calling each other
- **test_call_bridging**: 3-way call bridging
- **test_many_concurrent_peers**: Stress test with 10+ concurrent peers
- **test_rapid_call_setup_teardown**: Rapid call establishment/teardown cycles
- **test_presence_coordinator_access**: Presence system integration

## Running the Tests

Run all peer tests:
```bash
cargo test --test api_peer_integration
```

Run specific test module:
```bash
cargo test --test api_peer_integration peer_audio_tests
```

Run single test with output:
```bash
cargo test --test api_peer_integration test_peer_to_peer_audio_exchange -- --nocapture
```

Run tests serially to avoid port conflicts:
```bash
cargo test --test api_peer_integration -- --test-threads=1
```

## Audio File Output

The audio exchange tests save WAV files to temporary directories:
- `{temp_dir}/rvoip_peer_audio_test/alice/input.wav` - Alice's sent audio
- `{temp_dir}/rvoip_peer_audio_test/alice/output.wav` - Alice's received audio
- `{temp_dir}/rvoip_peer_audio_test/bob/input.wav` - Bob's sent audio
- `{temp_dir}/rvoip_peer_audio_test/bob/output.wav` - Bob's received audio

You can verify correct audio routing by checking that:
- Alice's output.wav contains Bob's 880Hz tone
- Bob's output.wav contains Alice's 440Hz tone

## Port Allocation

Tests use dynamic port allocation based on process ID to avoid conflicts:
- Base port: 15000 + (process_id % 100)
- Each peer in a test uses consecutive ports

## Debugging

Enable detailed logging:
```bash
RUST_LOG=rvoip=debug,test=info cargo test --test api_peer_integration -- --nocapture
```

## Coverage

This test suite verifies:
- ✅ SIP signaling between peers
- ✅ Audio channel establishment
- ✅ Bidirectional audio exchange
- ✅ Audio recording and playback
- ✅ Call control operations (hold, mute, transfer, DTMF)
- ✅ Multiple concurrent calls
- ✅ Call bridging and conferencing
- ✅ Error handling and edge cases
- ✅ Presence coordinator integration