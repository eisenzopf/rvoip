# Changelog for rvoip-session-core

## [Unreleased] - Transaction Core Integration

### Added
- Full integration with `rvoip-transaction-core` for SIP transaction handling
- Transaction event subscription and processing in SessionManager
- Proper transaction tracking and association with sessions/dialogs
- Transaction state synchronization with dialog states
- Support for forked SIP responses with multiple dialogs
- Correct handling of ACK for both 2xx responses (end-to-end) and non-2xx responses (transaction-layer)
- Enhanced BYE transaction handling with proper cleanup
- Support for CANCEL transactions with appropriate state transitions
- Transaction timeout handling
- Transaction transport error handling
- Comprehensive handling of transaction state changes
- SDP integration with transaction flow

### Fixed
- Dialog termination logic improved to handle transaction termination events
- Session state transitions now properly sync with transaction states
- Dialog-to-transaction mapping for bidirectional lookups
- Session termination on various transaction failure conditions
- Transaction resource management and cleanup

### Changed
- Session and dialog state machines now driven by transaction events
- Request sending now uses transaction layer for all SIP messages
- Response handling integrated with transaction layer
- Session state transitions now follow RFC 3261 more closely

### Examples
- Added `integrated_call.rs` example showing transaction-to-session flow
- Added `basic_integration.rs` example for simplified integration demonstration

## Next Steps
- Fix remaining compilation errors
- Add comprehensive tests for transaction integration
- Add detailed documentation on integration patterns
- Implement transaction recovery mechanisms for network failures 