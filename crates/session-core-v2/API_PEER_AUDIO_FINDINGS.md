# API Peer Audio Example Findings for Session-Core-V2

## Summary
Created and tested the api_peer_audio example for session-core-v2 to identify what works and what needs fixing in the new library compared to the old session-core.

## What Was Done

### 1. Created Example Files
- `examples/api_peer_audio/peer1.rs` - Alice (UAC) peer that makes calls
- `examples/api_peer_audio/peer2.rs` - Bob (UAS) peer that receives calls  
- `examples/api_peer_audio/run_audio_test.sh` - Test runner script

### 2. Created State Table YAML
- Created `state_tables/session_coordination.yaml` with all required transitions
- Defined states: Idle, Calling, Ringing, Answering, Active, OnHold, Terminating, Terminated, Failed
- Added transitions for UAC and UAS roles with proper events and actions

## Issues Identified

### 1. State Table Loading Issue ❌
**Problem**: The YAML state table is not being loaded at runtime. The system falls back to hardcoded transitions.

**Error**:
```
WARN rvoip_session_core_v2::state_table: Failed to load YAML state table, using hardcoded transitions
```

**Root Cause**: The YAML loader uses `include_str!` which requires the file at compile time, but the path resolution or file format may be incorrect.

### 2. Incomplete Hardcoded Transitions ❌
**Problem**: The fallback hardcoded transitions are incomplete - missing exit transitions for some states.

**Error**:
```
Invalid state table: ["State EarlyMedia has no exit transitions", "State Resuming has no exit transitions"]
```

**Impact**: The application panics on startup when YAML fails to load.

### 3. API Differences ⚠️
**Old API (session-core)**:
- Direct SessionBuilder that creates sessions
- Direct access to audio channels
- Simple call flow

**New API (session-core-v2)**:
- SessionBuilder creates UnifiedCoordinator
- UnifiedSession created from coordinator
- Audio delegated to media-core (no direct channel access)
- Event-driven architecture

### 4. Audio Channel Access ⚠️
**Problem**: The new API doesn't directly expose audio channels like the old API did.

**Impact**: Audio exchange examples can only simulate audio, not actually exchange it without additional integration with media-core.

## What Works ✅

### 1. Basic Structure
- The UnifiedCoordinator and UnifiedSession creation works
- Event subscription mechanism works
- Basic call flow structure is in place

### 2. Compilation
- Examples compile successfully with the new API
- Type system properly enforces role-based operations

### 3. Event System
- Event callbacks can be registered
- State change notifications work (when transitions are defined)

## What Needs Fixing

### Priority 1 - Critical
1. **Fix YAML Loading**: Debug why `include_str!` isn't finding the YAML file or parsing it correctly
2. **Complete Hardcoded Transitions**: Add missing transitions for EarlyMedia and Resuming states
3. **Fix State Table Validation**: Ensure all states have proper exit transitions

### Priority 2 - Important  
1. **Media Integration**: Document how to access media channels through media-core adapter
2. **Example Completion**: Update examples to show actual media flow when available
3. **Error Handling**: Add proper error handling for missing transitions

### Priority 3 - Nice to Have
1. **Documentation**: Add inline docs explaining the delegation model
2. **Helper Methods**: Add convenience methods for common operations
3. **Logging**: Add more detailed logging for debugging state transitions

## Recommendations

1. **Fix State Table Loading First**: This is blocking all functionality
2. **Add Integration Tests**: Test the full call flow with mock adapters
3. **Document Media Access Pattern**: Show how to get audio channels from media-core
4. **Consider Compatibility Layer**: Add methods that mimic old API for easier migration

## Code Size Comparison
- Old session-core: 32,330 lines
- New session-core-v2: 7,726 lines
- **76% reduction** in code size

## Architecture Benefits
- Clear separation of concerns (dialog-core handles SIP, media-core handles RTP)
- State table driven (declarative vs imperative)
- Better testability with adapters
- Reduced complexity through delegation

## Next Steps
1. Fix the YAML loading issue by investigating the include_str! macro path resolution
2. Complete the hardcoded state transitions as a fallback
3. Run successful peer-to-peer audio test
4. Document the media access pattern for real audio exchange