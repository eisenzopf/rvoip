# Session-Core API Improvements Plan

## Overview
This document tracks improvements to the session-core API to ensure consistent usage between UAC and UAS implementations and to prevent direct access to internal implementation details.

## Phase 1: Add New Reference Examples (High Priority)
*Goal: Create new examples that demonstrate best practices without modifying existing ones*

### New Reference Examples
- [x] Create `examples/api_best_practices/uac_client_clean.rs` - demonstrates UAC using only public API
- [x] Create `examples/api_best_practices/uas_server_clean.rs` - demonstrates UAS using only public API
- [x] Create `examples/api_best_practices/README.md` - explains the patterns and why they're preferred

### Documentation Updates
- [ ] Add comments to existing examples noting that newer patterns are available
- [ ] Create migration guide showing how to move from direct access to API usage
- [ ] Document the pattern for storing coordinator reference in handlers

### API Compatibility Layer
- [ ] Ensure new API methods work alongside existing patterns
- [ ] Add deprecation warnings (non-breaking) for internal access patterns
- [ ] Create compatibility shims where needed

## Phase 2: API Enhancements (High Priority)
*Goal: Add missing methods to fully support UAS scenarios through the API*

### New MediaControl Methods (Required for Clean UAS Implementation)
- [x] Add `create_media_session(&self, session_id: &SessionId) -> Result<()>`
  - Creates media session without generating SDP
  - Required to replace `coordinator.media_manager.create_media_session()` usage

- [x] Add `update_remote_sdp(&self, session_id: &SessionId, remote_sdp: &str) -> Result<()>`
  - Updates session with remote SDP without starting transmission
  - Required to replace `coordinator.media_manager.update_media_session()` usage

- [x] Add `generate_sdp_answer(&self, session_id: &SessionId, offer: &str) -> Result<String>`
  - Generates SDP answer based on received offer
  - Provides proper offer/answer negotiation without internal access

### New SessionControl Methods
- [ ] Add `accept_incoming_call(&self, call: &IncomingCall, sdp_answer: Option<String>) -> Result<CallSession>`
  - Programmatic way to accept calls outside of CallHandler
  - Useful for more complex decision logic

- [ ] Add `reject_incoming_call(&self, call: &IncomingCall, reason: &str) -> Result<()>`
  - Programmatic way to reject calls
  - Complements accept method

### Helper Types
- [x] Create `SdpInfo` struct for parsed SDP data
  ```rust
  pub struct SdpInfo {
      pub ip: String,
      pub port: u16,
      pub codecs: Vec<String>,
  }
  ```

- [x] Add SDP parsing utilities to the API
  - `parse_sdp_connection(sdp: &str) -> Result<SdpInfo>`
  - Removes need for manual SDP parsing in examples

## Phase 3: Future Encapsulation Considerations (Optional)
*Goal: Consider stronger encapsulation in a future major version*

### Potential Future Changes
- [ ] Evaluate making `SessionCoordinator` fields private in next major version
- [ ] Consider adding `#[deprecated]` attributes to guide users
- [ ] Assess user feedback on API completeness before enforcing encapsulation

### Soft Deprecation Strategy
- [ ] Add documentation comments discouraging internal access
- [ ] Provide examples of migrating to public API
- [ ] Monitor usage patterns before making breaking changes

### Builder Improvements
- [ ] Create `UasServerBuilder` for UAS-specific configurations
  ```rust
  pub struct UasServerBuilder {
      auto_answer: bool,
      max_concurrent_calls: usize,
      codec_preferences: Vec<String>,
  }
  ```

- [ ] Add convenience methods to `SessionManagerBuilder`
  - `with_uas_defaults()` - sets up common UAS configuration
  - `with_uac_defaults()` - sets up common UAC configuration

## Phase 4: Documentation & Testing (Ongoing)
*Goal: Ensure API is well-documented and tested*

### Documentation
- [ ] Add module-level docs explaining UAC vs UAS usage patterns
- [ ] Create a migration guide for moving from direct access to API usage
- [ ] Add inline examples to all new API methods
- [ ] Create a cookbook with common scenarios

### Testing
- [x] Add integration tests using only the public API
- [ ] Create tests that verify internal fields cannot be accessed
- [ ] Add examples that compile but don't run (doc tests)

## Implementation Notes

### Non-Breaking Approach
- All changes are additive - no breaking changes to existing code
- Existing examples continue to work as-is
- New API methods provide cleaner alternatives to internal access
- Deprecation warnings guide users to better patterns over time

### Backwards Compatibility
- New methods work alongside existing patterns
- Internal access remains available but discouraged
- Migration is optional and can be done gradually
- Compatibility maintained for existing users

### Success Metrics
- New reference examples demonstrate best practices
- API methods available for all current internal access patterns
- Clear migration path documented for users who want to upgrade
- Both old and new patterns coexist without conflicts

## Timeline Estimate
- Phase 1: 1-2 days (new reference examples)
- Phase 2: 2-3 days (API additions - high priority)
- Phase 3: Future consideration (only if needed)
- Phase 4: Ongoing

## Priority Order
1. **Phase 2 API methods** - These enable clean implementations
2. **Phase 1 reference examples** - Show how to use the new APIs
3. **Documentation** - Help users migrate if they choose
4. **Phase 3** - Only if we decide to enforce encapsulation later

## Related Issues
- Link to any GitHub issues or PRs here
- Track user feedback about API usage
- Document any discovered edge cases 