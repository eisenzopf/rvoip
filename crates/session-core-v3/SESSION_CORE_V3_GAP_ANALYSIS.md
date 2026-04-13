# Session-Core-v3 Gap Analysis

## Executive Summary

This document analyzes **only session-core-v3** to identify gaps for its intended purpose: **single session management** with support for:
1. ✅ Basic call control (make, answer, hangup, hold, transfer)
2. ✅ Audio send/receive
3. ⚠️ **SIP Registration** (REGISTER with server)
4. ⚠️ **SIP Authentication** (digest auth, credentials)
5. ⚠️ **Audio bridging primitive** (for b2bua crate to use)
6. ✅ Multi-session coordination (for transfers, needed by b2bua)

**Scope:** session-core-v3 is for **single session use cases** (softphones, IVR, simple agents).
**Out of Scope:** B2BUA and call center features belong in separate crates (rvoip-b2bua, rvoip-call-center).

**Current Status:**
- ✅ **80% Ready** for single session scenarios
- ⚠️ **20% Gaps** - Missing registration, authentication, audio bridge primitive

---

## Architecture Context

Per the **Layered Architecture Proposal**, session-core-v3 is ONE layer:

```
rvoip-call-center        ← Queue, agents, ACD (separate crate)
    ↓
rvoip-b2bua              ← Two-leg bridging (separate crate)
    ↓
session-core-v3          ← Single session management (THIS CRATE)
    ↓
dialog-core + media-core
```

**session-core-v3 responsibility:** Manage ONE session at a time.
**NOT responsible for:** B2BUA logic, call center logic.

---

## Current Infrastructure Assessment

### ✅ What EXISTS and WORKS

1. **Session Management**
   - ✅ SessionStore with DashMap (lock-free, concurrent)
   - ✅ Multiple concurrent sessions supported
   - ✅ State machine with YAML state tables
   - ✅ Per-session state tracking

2. **Call Control**
   - ✅ `make_call()` - Create outbound calls
   - ✅ `accept_call()` / `reject_call()` - Handle incoming
   - ✅ `hangup()` - Terminate calls
   - ✅ `hold()` / `resume()` - Call features

3. **Audio**
   - ✅ `send_audio()` - Send audio to session
   - ✅ `subscribe_to_audio()` - Receive audio from session
   - ✅ Per-session audio channels

4. **Transfer (Blind)**
   - ✅ `send_refer()` - Initiate transfer
   - ✅ Transfer events (ReferReceived)
   - ✅ `complete_blind_transfer()` helper (Phase 1 implemented)

5. **Adapters**
   - ✅ DialogAdapter - SIP integration
   - ✅ MediaAdapter - Media integration
   - ✅ Event routing

6. **APIs (Planned)**
   - ✅ SimplePeer (exists, has limitations)
   - 📋 PolicyPeer (planned, 11h)
   - 📋 CallbackPeer (planned, 9h)
   - 📋 EventStreamPeer (planned, 13h)

---

## Critical Gaps for session-core-v3

### 🔴 Gap 1: SIP Registration Support

**Problem:** Cannot register with SIP servers to receive calls at registered address.

**Current State:**
- ❌ No REGISTER message support
- ❌ No credential storage
- ❌ No registration refresh/renewal
- ❌ No unregistration

**Impact:**
- Can't receive calls via SIP proxy/registrar
- Can't use hosted PBX systems
- Limited to direct IP calling

**Needed in session-core-v3:**

#### 1.1 Registration State & Types
```rust
// In src/session_store/state.rs
pub struct SessionState {
    // ... existing fields ...
    
    // Registration fields
    pub registrar_uri: Option<String>,
    pub registration_expires: Option<u32>,
    pub registration_contact: Option<String>,
    pub credentials: Option<Credentials>,
}

// In src/types.rs (already exists)
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub realm: Option<String>,
}
```

#### 1.2 Registration Methods (UnifiedCoordinator)
```rust
impl UnifiedCoordinator {
    /// Register with SIP server
    pub async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
    ) -> Result<RegistrationHandle> {
        // Create registration session
        // Send REGISTER
        // Handle 401 challenge
        // Store credentials
    }
    
    /// Unregister (expires=0)
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()>;
    
    /// Refresh registration
    pub async fn refresh_registration(&self, handle: &RegistrationHandle) -> Result<()>;
}
```

#### 1.3 DialogAdapter::send_register()
```rust
impl DialogAdapter {
    pub async fn send_register(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        contact_uri: &str,
        expires: u32,
        credentials: Option<&Credentials>,
    ) -> Result<()> {
        // Build REGISTER request
        // Send via dialog-core
    }
}
```

#### 1.4 State Table Transitions
```yaml
# Registration states and transitions
- role: "UAC"
  state: "Idle"
  event:
    type: "StartRegistration"
  next_state: "Registering"
  actions:
    - type: "SendREGISTER"
  
- role: "UAC"
  state: "Registering"
  event:
    type: "Registration200OK"
  next_state: "Registered"
```

#### 1.5 Registration Events
```rust
// In src/api/events.rs
pub enum Event {
    // ... existing events ...
    
    RegistrationSuccess {
        registrar: String,
        expires: u32,
    },
    
    RegistrationFailed {
        registrar: String,
        status_code: u16,
        reason: String,
    },
}
```

**Estimated Work:** 12-16 hours

**Priority:** 🔴 **CRITICAL** - Required for production SIP clients

---

### 🔴 Gap 2: SIP Authentication (Digest Auth)

**Problem:** Cannot authenticate with SIP servers (401/407 challenges).

**Current State:**
- ❌ No digest authentication implementation
- ❌ No credential management
- ❌ No auth challenge handling

**Impact:**
- Can't use most production SIP servers
- Can't register with authentication required
- Security risk (no auth)

**Needed in session-core-v3:**

#### 2.1 Auth Challenge Handling
```rust
// In DialogAdapter
async fn handle_401_challenge(
    &self,
    session_id: &SessionId,
    challenge_header: &str,
) -> Result<()> {
    // Parse WWW-Authenticate header
    // Extract realm, nonce, algorithm
    
    // Get credentials from session
    let session = self.store.get_session(session_id).await?;
    let credentials = session.credentials.ok_or("No credentials")?;
    
    // Compute digest response
    let auth_header = compute_digest_auth(
        &credentials.username,
        &credentials.password,
        challenge_header,
        "REGISTER",  // or INVITE
    )?;
    
    // Retry request with Authorization header
    self.send_register_with_auth(session_id, &auth_header).await?;
}
```

#### 2.2 Digest Auth Computation
```rust
// In src/auth/ (new module)
pub fn compute_digest_auth(
    username: &str,
    password: &str,
    challenge: &str,
    method: &str,
) -> Result<String> {
    // Parse challenge
    let realm = extract_param(challenge, "realm")?;
    let nonce = extract_param(challenge, "nonce")?;
    let algorithm = extract_param(challenge, "algorithm").unwrap_or("MD5");
    
    // Compute response
    // HA1 = MD5(username:realm:password)
    // HA2 = MD5(method:uri)
    // response = MD5(HA1:nonce:HA2)
    
    format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", response=\"{}\"",
        username, realm, nonce, response
    )
}
```

**Alternative:** Use `rvoip-auth-core` if it has digest auth utilities.

#### 2.3 Credential Storage
```rust
// Already exists in src/types.rs
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub realm: Option<String>,
}
```

**Estimated Work:** 10-14 hours

**Priority:** 🔴 **CRITICAL** - Required for production SIP clients

**Note:** Can leverage auth-core if it provides digest auth helpers.

---

### 🟡 Gap 3: Audio Bridge Primitive (for B2BUA)

**Problem:** `create_bridge()` only updates state, doesn't actually bridge audio.

**Current State:**
```rust
// In media_adapter.rs line 332
pub async fn create_bridge(&self, session1: &SessionId, session2: &SessionId) -> Result<()> {
    // Just updates session state - NO ACTUAL BRIDGING!
    session.bridged_to = Some(session2.clone());
}
```

**What's Missing:**
- ❌ No call to media-core's `create_relay()`
- ❌ No audio forwarding

**Needed (session-core-v3 provides primitive only):**
```rust
pub async fn bridge_audio_between_sessions(
    &self,
    session1: &SessionId,
    session2: &SessionId,
) -> Result<()> {
    // Get dialog IDs
    let dialog1 = self.session_to_dialog.get(session1)
        .ok_or("No dialog for session1")?;
    let dialog2 = self.session_to_dialog.get(session2)
        .ok_or("No dialog for session2")?;
    
    // Call media-core to create relay
    self.controller.create_relay(
        dialog1.to_string(),
        dialog2.to_string()
    ).await.map_err(|e| SessionError::MediaError(e.to_string()))?;
    
    // Media-core handles audio forwarding internally via RTP relay
    // No need to manually forward - media-core does it!
    
    Ok(())
}
```

**Add to UnifiedCoordinator:**
```rust
pub async fn bridge_sessions(&self, s1: &SessionId, s2: &SessionId) -> Result<()> {
    self.media_adapter.bridge_audio_between_sessions(s1, s2).await
}
```

**Usage by b2bua crate:**
```rust
// In rvoip-b2bua/src/simple.rs
coordinator.bridge_sessions(&inbound_session, &outbound_session).await?;
```

**Estimated Work:** 8-12 hours

**Priority:** 🟡 **HIGH** - Required for b2bua crate, but not for single-session use cases

**Note:** This is a PRIMITIVE for b2bua. Session-core-v3 provides the capability, b2bua crate provides the API.

---

### 🟡 Gap 4: Attended Transfer Support

**Problem:** Only blind transfer implemented (Phase 1).

**Current State:**
- ✅ Blind transfer works (send REFER, recipient completes)
- ❌ Attended transfer not supported

**Attended Transfer Flow:**
1. Agent calls customer (call 1 active)
2. Agent puts customer on hold
3. Agent calls expert (call 2 active)
4. Agent consults with expert
5. Agent transfers customer to expert (via REFER with Replaces header)
6. Agent hangs up

**Needed:**
```rust
impl UnifiedCoordinator {
    /// Initiate attended transfer
    pub async fn attended_transfer(
        &self,
        held_call: &SessionId,      // Customer (on hold)
        active_call: &SessionId,     // Expert (consultation)
    ) -> Result<()> {
        // Get dialog IDs and call-ids
        // Send REFER with Replaces header
        // Refer-To: <sip:expert@...>?Replaces=call-id
    }
}
```

**Estimated Work:** 8-12 hours

**Priority:** 🟡 **MEDIUM** - Nice to have, but blind transfer works for most cases

---

### 🟢 Gap 5: Presence/Subscription (SUBSCRIBE/NOTIFY)

**Problem:** No presence support.

**Current State:**
- ❌ No SUBSCRIBE support
- ❌ No NOTIFY handling
- ❌ No presence state

**Impact:**
- Can't subscribe to buddy status
- Can't publish own status
- No busy lamp field (BLF)

**Needed:**
```rust
impl UnifiedCoordinator {
    /// Subscribe to presence
    pub async fn subscribe_presence(
        &self,
        target_uri: &str,
        expires: u32,
    ) -> Result<SubscriptionHandle>;
    
    /// Publish own presence
    pub async fn publish_presence(&self, status: PresenceStatus) -> Result<()>;
}
```

**Estimated Work:** 10-14 hours

**Priority:** 🟢 **LOW** - Nice to have, but not critical for core functionality

**Note:** Detailed in SIMPLEPEER_COMPLETION_PLAN.md Phase 3

---

### 🟢 Gap 6: SIP MESSAGE Support

**Problem:** No instant messaging support.

**Current State:**
- ❌ No MESSAGE request/response

**Needed:**
```rust
impl UnifiedCoordinator {
    pub async fn send_message(&self, to: &str, message: &str) -> Result<()>;
}

// Event
pub enum Event {
    MessageReceived { from: String, body: String },
    MessageDelivered { to: String },
}
```

**Estimated Work:** 4-6 hours

**Priority:** 🟢 **LOW** - Nice to have

---

## API-Level Gaps

### 🔴 Gap 7: PolicyPeer Implementation

**Problem:** PolicyPeer doesn't exist yet (only planned).

**Current State:** Planning document only

**Needed:** Full implementation per POLICY_PEER_IMPLEMENTATION_PLAN.md

**Estimated Work:** 11 hours (enhanced with session-core patterns)

**Priority:** 🔴 **CRITICAL** - Core API for production use

---

### 🔴 Gap 8: CallbackPeer Implementation

**Problem:** CallbackPeer doesn't exist yet (only planned).

**Current State:** Planning document only

**Needed:** Full implementation per CALLBACK_PEER_IMPLEMENTATION_PLAN.md

**Estimated Work:** 9 hours (trait-based)

**Priority:** 🔴 **CRITICAL** - Core API for session-core users

---

### 🟡 Gap 9: EventStreamPeer Implementation

**Problem:** EventStreamPeer doesn't exist yet (only planned).

**Current State:** Planning document only

**Needed:** Full implementation per EVENT_STREAM_PEER_IMPLEMENTATION_PLAN.md

**Estimated Work:** 13 hours (with helpers)

**Priority:** 🟡 **HIGH** - Needed for IVR and advanced use cases

---

## Integration Gaps

### 🟡 Gap 10: Registration Integration with SimplePeer/PolicyPeer/CallbackPeer

**Problem:** No registration methods exposed in high-level APIs.

**Current State:**
- SimplePeer: No registration methods
- PolicyPeer: Not implemented yet
- CallbackPeer: Not implemented yet

**Needed:**

#### For SimplePeer:
```rust
impl SimplePeer {
    pub async fn register(
        &mut self,
        registrar: &str,
        username: &str,
        password: &str,
    ) -> Result<()>;
    
    pub async fn wait_for_registration(&mut self) -> Result<()>;
    pub async fn unregister(&mut self) -> Result<()>;
}
```

#### For PolicyPeer:
```rust
impl PolicyPeer {
    pub async fn register(
        &self,
        registrar: &str,
        credentials: Credentials,
    ) -> Result<RegistrationHandle>;
}
```

#### For CallbackPeer:
```rust
#[async_trait]
pub trait PeerHandler {
    // ... existing methods ...
    
    async fn on_registration_success(&self, registrar: String, expires: u32) {
        // Default implementation
    }
    
    async fn on_registration_failed(&self, registrar: String, reason: String) {
        // Default implementation
    }
}
```

**Estimated Work:** 6-8 hours (after Gap #1 fixed)

**Priority:** 🟡 **HIGH** - Users need registration in production

---

### 🟢 Gap 11: Audio Bridge Cleanup

**Problem:** Bridges not cleaned up when sessions terminate.

**Current State:**
- ✅ `DestroyBridge` action exists
- ⚠️ Not automatically called when bridged session ends

**Needed:**
```rust
// In state machine or adapter
async fn on_session_terminated(&self, session_id: &SessionId) {
    // Check if session was bridged
    let session = store.get_session(session_id).await?;
    
    if let Some(bridged_to) = session.bridged_to {
        // Destroy the bridge
        media_adapter.destroy_bridge(session_id).await?;
    }
}
```

**Estimated Work:** 2-4 hours

**Priority:** 🟢 **MEDIUM** - Cleanup is important but can be manual initially

---

## Detailed Gap Analysis by Component

### UnifiedCoordinator Gaps

| Gap | Method | Hours | Priority |
|-----|--------|-------|----------|
| Registration | `register()`, `unregister()`, `refresh_registration()` | 4 | 🔴 Critical |
| Authentication | Handled in DialogAdapter | 0 | N/A |
| Bridge Sessions | `bridge_sessions()` - high-level API | 2 | 🟡 High |
| Attended Transfer | `attended_transfer()` | 8-12 | 🟡 Medium |
| Presence | `subscribe_presence()`, `publish_presence()` | 10-14 | 🟢 Low |
| Messaging | `send_message()` | 4-6 | 🟢 Low |

**Total UnifiedCoordinator gaps:** 28-38 hours

---

### DialogAdapter Gaps

| Gap | Method | Hours | Priority |
|-----|--------|-------|----------|
| Send REGISTER | `send_register()` | 4-6 | 🔴 Critical |
| Handle 401 Challenge | `handle_auth_challenge()` | 6-8 | 🔴 Critical |
| Digest Auth Computation | `compute_digest_response()` | 4-6 | 🔴 Critical |
| Send SUBSCRIBE | `send_subscribe()` | 4-6 | 🟢 Low |
| Send NOTIFY | `send_notify()` | 2-4 | 🟢 Low |
| Send MESSAGE | `send_message()` | 2-4 | 🟢 Low |

**Total DialogAdapter gaps:** 22-34 hours

**Note:** May leverage auth-core for digest computation if available.

---

### MediaAdapter Gaps

| Gap | Method | Hours | Priority |
|-----|--------|-------|----------|
| Fix create_bridge() | Actually call media-core relay | 8-12 | 🟡 High (for b2bua) |
| Bridge cleanup | Auto-cleanup on termination | 2-4 | 🟢 Medium |

**Total MediaAdapter gaps:** 10-16 hours

---

### State Machine / State Table Gaps

| Gap | Component | Hours | Priority |
|-----|-----------|-------|----------|
| Registration transitions | Add to default.yaml | 2 | 🔴 Critical |
| Auth states | Add Authenticating state | 1 | 🔴 Critical |
| SendREGISTER action | Implement in actions.rs | 2 | 🔴 Critical |
| Presence transitions | Add to default.yaml | 2 | 🟢 Low |
| SendSUBSCRIBE action | Implement in actions.rs | 2 | 🟢 Low |

**Total State Machine gaps:** 9 hours

---

### API Gaps (New APIs)

| API | Status | Hours | Priority |
|-----|--------|-------|----------|
| PolicyPeer | Planned | 11 | 🔴 Critical |
| CallbackPeer | Planned | 9 | 🔴 Critical |
| EventStreamPeer | Planned | 13 | 🟡 High |

**Total API gaps:** 33 hours

**Note:** These are NEW features, not gaps in existing code.

---

## Integration with auth-core and registrar-core

### auth-core (Authentication)

**What auth-core provides:**
- JWT token generation/validation
- OAuth2 flows
- Credential management

**What session-core-v3 needs:**
- ✅ **SIP Digest Auth** - Different from JWT/OAuth!
- ✅ **Credential storage** - Simple username/password

**Conclusion:**
- ⚠️ auth-core is for **application-level auth** (JWT, OAuth)
- ⚠️ session-core-v3 needs **SIP digest auth** (protocol-level)
- ✅ Implement SIP digest auth in session-core-v3 (or dialog-core)
- ✅ Can use auth-core for credential storage helpers

**Action:** Check if auth-core has MD5 digest helpers. If yes, use them. If no, implement locally.

---

### registrar-core (Registrar Server)

**What registrar-core provides:**
- ✅ Registrar server implementation
- ✅ Location service (AOR → contact mappings)
- ✅ Registration database

**What session-core-v3 needs:**
- ✅ **Registrar CLIENT** - Send REGISTER to servers
- ❌ NOT a registrar server

**Conclusion:**
- ✅ registrar-core is for **building registrar servers**
- ✅ session-core-v3 needs to **connect TO registrar servers**
- ❌ No code reuse between them (different roles)

**Action:** Implement REGISTER client in dialog-core or session-core-v3.

---

## Gap Summary by Priority

### 🔴 Critical (Blocking Production Use) - 47-63 hours

| Gap # | Component | Description | Hours |
|-------|-----------|-------------|-------|
| #1 | Registration | REGISTER support, credential storage | 12-16 |
| #2 | Authentication | Digest auth, 401 challenge handling | 10-14 |
| #7 | PolicyPeer | New API implementation | 11 |
| #8 | CallbackPeer | New API implementation | 9 |
| #10 | Registration API | Expose in high-level APIs | 6-8 |

**Total Critical:** 48-58 hours (6-7 days)

---

### 🟡 High Priority (Needed for Advanced Use Cases) - 31-45 hours

| Gap # | Component | Description | Hours |
|-------|-----------|-------------|-------|
| #3 | Audio Bridge | Fix create_bridge() for b2bua | 8-12 |
| #9 | EventStreamPeer | New API implementation | 13 |
| #4 | Attended Transfer | REFER with Replaces | 8-12 |
| #11 | Bridge Cleanup | Auto-cleanup on termination | 2-4 |

**Total High Priority:** 31-41 hours (4-5 days)

---

### 🟢 Low Priority (Nice to Have) - 16-26 hours

| Gap # | Component | Description | Hours |
|-------|-----------|-------------|-------|
| #5 | Presence | SUBSCRIBE/NOTIFY support | 10-14 |
| #6 | Messaging | SIP MESSAGE support | 4-6 |

**Total Low Priority:** 14-20 hours (2 days)

---

## Implementation Roadmap

### Sprint 1: Core APIs (2 weeks) - 48-58 hours

**Goal:** PolicyPeer and CallbackPeer working

1. ✅ Implement PolicyPeer (11h)
2. ✅ Implement CallbackPeer (9h)
3. ✅ Add SIP Registration support (12-16h)
4. ✅ Add SIP Digest Authentication (10-14h)
5. ✅ Expose registration in APIs (6-8h)

**Deliverable:** Production-ready APIs with registration

---

### Sprint 2: Advanced Features (1 week) - 31-41 hours

**Goal:** EventStreamPeer and audio bridging

1. ✅ Implement EventStreamPeer (13h)
2. ✅ Fix audio bridge primitive (8-12h)
3. ✅ Add attended transfer (8-12h)
4. ✅ Add bridge cleanup (2-4h)

**Deliverable:** Full feature set, ready for b2bua crate

---

### Sprint 3: Extended Features (Optional) - 14-20 hours

**Goal:** Presence and messaging

1. ⏳ Add presence support (10-14h)
2. ⏳ Add messaging support (4-6h)

**Deliverable:** Complete SIP client capabilities

---

## What session-core-v3 SHOULD Have

Per the layered architecture, session-core-v3 should focus on **single session management**:

### ✅ Core Session Features (MUST Have):
1. ✅ Make/receive calls
2. ✅ Hold/resume
3. ✅ Blind transfer
4. ✅ Audio send/receive
5. ⚠️ **SIP registration** ← GAP #1
6. ⚠️ **SIP authentication** ← GAP #2
7. ⚠️ **PolicyPeer/CallbackPeer/EventStreamPeer** ← GAPS #7, #8, #9

### ✅ Primitives for Higher Layers (SHOULD Have):
1. ⚠️ **Audio bridge primitive** ← GAP #3 (for b2bua)
2. ✅ Multi-session support (already exists)
3. ✅ Event system (already exists)

### ⚠️ Extended SIP Features (NICE to Have):
1. ⚠️ Attended transfer ← GAP #4
2. ⚠️ Presence/subscription ← GAP #5
3. ⚠️ Instant messaging ← GAP #6

### ❌ NOT for session-core-v3 (Separate Crates):
1. ❌ B2BUA two-leg coordination → `rvoip-b2bua`
2. ❌ Call queuing → `rvoip-call-center`
3. ❌ Agent management → `rvoip-call-center`
4. ❌ ACD logic → `rvoip-call-center`
5. ❌ Supervisor features → `rvoip-call-center`

---

## Minimal session-core-v3 for Production

### Must-Fix Gaps (48-58 hours):

1. ✅ SIP Registration (Gap #1) - 12-16h
2. ✅ SIP Authentication (Gap #2) - 10-14h
3. ✅ PolicyPeer (Gap #7) - 11h
4. ✅ CallbackPeer (Gap #8) - 9h
5. ✅ Registration API integration (Gap #10) - 6-8h

**Total:** 48-58 hours (6-7 days)

**Deliverable:** Production-ready session-core-v3 for softphones and simple agents

---

### Should-Fix for B2BUA Support (8-12 hours):

6. ✅ Audio bridge primitive (Gap #3) - 8-12h

**Deliverable:** session-core-v3 ready for b2bua crate to build on

---

### Nice-to-Have (31-45 hours):

7. ⏳ EventStreamPeer (Gap #9) - 13h
8. ⏳ Attended Transfer (Gap #4) - 8-12h
9. ⏳ Presence Support (Gap #5) - 10-14h
10. ⏳ Messaging Support (Gap #6) - 4-6h

**Deliverable:** Full-featured SIP client library

---

## Testing Requirements

### Unit Tests Needed:

1. **Registration:**
   - REGISTER message construction
   - 200 OK handling
   - 401 challenge response
   - Refresh logic
   - Unregister (expires=0)

2. **Authentication:**
   - Digest MD5 computation
   - Challenge parsing
   - Credential storage
   - Re-authentication

3. **Audio Bridging:**
   - Bridge creation
   - Audio forwarding
   - Bridge termination
   - Cleanup

**Estimated Test Work:** 12-16 hours

---

## Risk Assessment

### Low Risk (Clear Implementation):
- ✅ Registration state machine - Standard SIP
- ✅ Audio bridge primitive - Media-core already has relay
- ✅ API implementation - Clear plans exist

### Medium Risk (Some Complexity):
- ⚠️ Digest authentication - Need to get it right for security
- ⚠️ Registration refresh - Timing and renewal logic
- ⚠️ Bridge cleanup - Race conditions possible

### High Risk (Integration):
- 🔴 Dialog-core integration - May have limitations
- 🔴 Auth-core usage - Unknown if compatible with SIP digest

---

## Dependencies on Other Crates

### dialog-core

**Needed from dialog-core:**
- ✅ Send REGISTER requests
- ✅ Handle REGISTER responses (200, 401, 403)
- ⚠️ Support auth headers (Authorization, WWW-Authenticate)

**Action:** Verify dialog-core supports REGISTER method

---

### media-core

**Needed from media-core:**
- ✅ `create_relay()` - Already exists
- ✅ Conference mixing - Already exists
- ✅ Per-session audio - Already exists

**No gaps in media-core!**

---

### auth-core

**Potentially useful from auth-core:**
- ⚠️ MD5 hashing utilities (for digest auth)
- ⚠️ Credential storage helpers

**Action:** Check if auth-core has SIP digest auth support. If yes, use it. If no, implement locally.

---

### registrar-core

**Not needed by session-core-v3:**
- ❌ registrar-core is for building registrar SERVERS
- ❌ session-core-v3 needs registrar CLIENT

**No dependency!**

---

## Conclusion

### session-core-v3 Current Status: 80% Ready

**Strong Foundation:**
- ✅ State machine, adapters, session management
- ✅ Multi-session support
- ✅ Audio per session
- ✅ Blind transfer working

**Critical Gaps (Must Fix):**
- ❌ SIP Registration (Gap #1) - 12-16h
- ❌ SIP Authentication (Gap #2) - 10-14h
- ❌ PolicyPeer API (Gap #7) - 11h
- ❌ CallbackPeer API (Gap #8) - 9h

**Total Must-Fix:** 42-51 hours (5-6 days)

---

### Recommended Implementation Order

**Week 1-2: Core Infrastructure**
1. SIP Registration (Gap #1) - 12-16h
2. SIP Authentication (Gap #2) - 10-14h
3. State machine updates (registration/auth) - 4h

**Week 2-3: Core APIs**
4. CallbackPeer (Gap #8) - 9h
5. PolicyPeer (Gap #7) - 11h
6. Registration API integration (Gap #10) - 6-8h

**Week 4: B2BUA Primitive**
7. Fix audio bridge (Gap #3) - 8-12h
8. Bridge cleanup (Gap #11) - 2-4h

**Week 5+: Optional**
9. EventStreamPeer (Gap #9) - 13h
10. Attended transfer (Gap #4) - 8-12h
11. Presence (Gap #5) - 10-14h

---

### What About auth-core and registrar-core?

**auth-core:**
- ✅ Use if it has MD5/digest utilities
- ❌ Don't use for SIP protocol auth (different from JWT/OAuth)
- ✅ Might be useful for credential storage patterns

**registrar-core:**
- ✅ Completely separate concern (server vs client)
- ❌ No code sharing
- ✅ session-core-v3 connects TO registrars built with registrar-core

---

## Final Recommendation

### For session-core-v3 to be production-ready:

**Minimum (48-58 hours):**
1. ✅ Add SIP Registration support
2. ✅ Add SIP Digest Authentication
3. ✅ Implement PolicyPeer
4. ✅ Implement CallbackPeer

**Recommended (79-99 hours):**
- Above + EventStreamPeer + Audio bridge primitive

**Full-Featured (110-144 hours):**
- Above + Attended transfer + Presence + Messaging

---

**The infrastructure is solid. session-core-v3 just needs registration, authentication, and the new APIs. Everything else belongs in separate crates (b2bua, call-center).**

**Timeline:** 6-7 days for minimum production-ready, 10-12 days for recommended.

