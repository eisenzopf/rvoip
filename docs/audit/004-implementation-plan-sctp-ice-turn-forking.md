# Implementation Plan: SCTP, TURN→ICE, ICE Improvements, Dialog Forking

**Document ID**: PLAN-004
**Date**: 2026-03-21
**Status**: Pending Review
**Scope**: 4 features across rtp-core, dialog-core, sip-transport, session-core

---

## Phase 1: TURN → ICE Integration (Priority: HIGH)

### 1.1 Problem
TURN client is implemented but not connected to ICE candidate gathering. ICE only gathers host + server-reflexive candidates, not relay candidates. Behind symmetric NAT, calls fail.

### 1.2 Changes

#### A. Add TURN credentials to IceConfig
**File**: `crates/session-core/src/media/types.rs`
```rust
pub struct IceConfig {
    pub enabled: bool,
    pub stun_servers: Vec<SocketAddr>,
    pub turn_servers: Vec<TurnServerConfig>,  // Changed from Vec<SocketAddr>
}

pub struct TurnServerConfig {
    pub server: SocketAddr,
    pub username: String,
    pub password: String,
    pub transport: TurnTransport,  // Udp, Tcp, Tls
}
```

#### B. Add `gather_relay_candidates()` to ICE gather module
**File**: `crates/rtp-core/src/ice/gather.rs`
- New function: `pub async fn gather_relay_candidates(turn_configs: &[TurnServerConfig], local_socket: &UdpSocket, component: ComponentId, ufrag: &str) -> Vec<IceCandidate>`
- For each TURN server:
  1. Create `TurnClient::with_socket()` (share socket with STUN/RTP)
  2. Call `allocate()` → get `TurnAllocation`
  3. Call `create_permission()` for any known peer addresses
  4. Create `IceCandidate` with `CandidateType::Relay`, `related_address = allocation.mapped_address`
  5. Store `TurnClient` handle for later data relay

#### C. Extend `IceAgent::gather_candidates()`
**File**: `crates/rtp-core/src/ice/agent.rs` (line ~188)
- Add parameter: `turn_configs: &[TurnServerConfig]`
- After srflx gathering, if turn_configs non-empty: call `gather_relay_candidates()`
- Store `TurnClient` handles in agent for relay data path
- Add field: `turn_clients: Vec<TurnClient>`

#### D. Relay data path for relay candidates
**File**: `crates/rtp-core/src/ice/agent.rs`
- When selected pair uses a relay candidate: route RTP through TURN Send Indication or ChannelData
- Add method: `send_via_relay(&self, data: &[u8], peer_addr: SocketAddr) -> Result<()>`
- Add method: `receive_relay_data(&mut self) -> Result<Option<(Vec<u8>, SocketAddr)>>`

#### E. Wire into session-core
**File**: `crates/session-core/src/media/manager.rs`
- Pass `ice_config.turn_servers` to `IceAgent::gather_candidates()`
- After ICE completes with relay pair: use TURN relay path for media

### 1.3 Dependencies
- `rtp-core/src/turn/` (already implemented)
- `rtp-core/src/ice/` (already implemented)

### 1.4 Tests
- Unit: relay candidate priority calculation
- Unit: gather with mock TURN server
- Integration (`#[ignore]`): real TURN server allocation

### 1.5 Estimated Effort: 1 agent, ~45 min

---

## Phase 2: ICE Keepalive + Trickle ICE (Priority: MEDIUM)

### 2.1 ICE Keepalive (RFC 7675 Consent Freshness)

#### A. Add consent tracking to IceAgent
**File**: `crates/rtp-core/src/ice/agent.rs`
```rust
pub struct IceAgent {
    // ... existing fields
    last_consent_response: Option<Instant>,
    consent_check_interval: Duration,  // default 15s per RFC 7675
    consent_timeout: Duration,         // default 30s per RFC 7675
}
```

#### B. Keepalive methods
- `needs_keepalive(&self) -> bool` — true if 15s since last consent response
- `build_keepalive(&self) -> Result<(Vec<u8>, SocketAddr)>` — STUN Binding Indication (type 0x0011, no response expected)
- `handle_consent_response(&mut self)` — update `last_consent_response`
- `check_consent_expired(&mut self) -> bool` — if 30s without response, transition to Disconnected

#### C. Integration
- session-core media manager: periodically call `needs_keepalive()` + send keepalive
- On consent expired: tear down media session

### 2.2 Trickle ICE (RFC 8838)

#### A. Enable trickle in SDP
**File**: `crates/session-core/src/media/manager.rs` (SDP generation)
- Add `a=ice-options:trickle` to SDP offer when ICE enabled
- Parse remote `a=ice-options:trickle` from SDP answer

#### B. IceAgent trickle support
**File**: `crates/rtp-core/src/ice/agent.rs`
```rust
pub struct IceAgent {
    // ... existing
    trickle_enabled: bool,
    end_of_candidates: bool,
}

// Already exists:
pub fn add_remote_candidate(&mut self, candidate: IceCandidate)

// New:
pub fn end_of_candidates(&mut self)
pub fn is_trickle_enabled(&self) -> bool
```

#### C. SIP transport for trickle candidates
Use SIP INFO method within established dialog:
- Body type: `application/trickle-ice-sdpfrag` (RFC 8840)
- Body content: `a=candidate:...` lines
- `a=end-of-candidates` when gathering complete

**File**: `crates/session-core/src/coordinator/session_ops.rs`
- Add `send_ice_candidate(session_id, candidate)` method
- Add `handle_ice_candidate_info(session_id, body)` for incoming SIP INFO

#### D. session-core integration
- After `gather_candidates()` returns: if trickle enabled, send each candidate via SIP INFO
- On receiving SIP INFO with candidates: forward to `IceAgent::add_remote_candidate()`
- On `a=end-of-candidates`: call `IceAgent::end_of_candidates()`

### 2.3 Tests
- Unit: keepalive timer logic
- Unit: consent expiry detection
- Unit: trickle candidate addition mid-check
- Unit: end-of-candidates handling

### 2.4 Estimated Effort: 1 agent, ~45 min

---

## Phase 3: Dialog Forking (RFC 3261 §16.7) (Priority: MEDIUM)

### 3.1 Problem
UAC sends INVITE through a proxy. Proxy forks to multiple UAS. UAC receives multiple 1xx/2xx with different To tags. Current code creates only one dialog per Call-ID, losing forked responses.

### 3.2 Architecture Changes

#### A. Early dialog group tracking
**File**: `crates/dialog-core/src/manager/core.rs`
```rust
pub struct DialogManager {
    // ... existing
    /// Call-ID → Vec<DialogId> for early dialog groups (forking)
    pub early_dialog_groups: Arc<DashMap<String, Vec<DialogId>>>,
}
```

#### B. Modify response handling for forked responses
**File**: `crates/dialog-core/src/protocol/response_handler.rs`

When receiving 1xx or 2xx for an INVITE:
1. Extract To tag from response
2. Look up existing early dialogs for this Call-ID
3. If To tag doesn't match any existing dialog → **new forked early dialog**
4. Create new dialog with same Call-ID but different To tag
5. Add to early_dialog_groups[call_id]

```rust
async fn handle_invite_response(&self, response: &Response) -> Result<()> {
    let call_id = response.call_id()?;
    let to_tag = response.to_tag()?;

    // Check if this is a new fork
    let existing = self.find_early_dialog(&call_id, &to_tag);
    if existing.is_none() && response.status().is_provisional_or_success() {
        // New forked response — create additional early dialog
        let dialog_id = self.create_forked_early_dialog(&call_id, &to_tag, response).await?;
        self.add_to_early_dialog_group(&call_id, dialog_id);
    }
    // ... proceed with normal response handling
}
```

#### C. Dialog confirmation with fork cleanup
When first 2xx received:
1. Confirm the matching early dialog
2. For all OTHER early dialogs in same group: transition to Terminated
3. Send BYE (or CANCEL for provisional) to terminated dialogs
4. Remove from early_dialog_groups

```rust
async fn confirm_dialog_with_fork_cleanup(&self, call_id: &str, confirmed_dialog: DialogId) {
    if let Some(group) = self.early_dialog_groups.get(call_id) {
        for dialog_id in group.iter() {
            if *dialog_id != confirmed_dialog {
                self.terminate_dialog(dialog_id).await;
                self.send_bye_or_cancel(dialog_id).await;
            }
        }
        self.early_dialog_groups.remove(call_id);
    }
}
```

#### D. Modify `find_dialog_for_request()` for forking
**File**: `crates/dialog-core/src/manager/dialog_operations.rs` (line ~378)

Add parallel/sequential fork handling:
- **Parallel forking**: UAC creates early dialog for each 1xx with unique To tag
- **Sequential forking**: Proxy retries with different UAS after timeout/failure

```rust
// New helper:
fn find_early_dialogs_for_call_id(&self, call_id: &str) -> Vec<DialogId> {
    self.early_dialog_groups
        .get(call_id)
        .map(|v| v.clone())
        .unwrap_or_default()
}
```

#### E. Session-core integration
**File**: `crates/session-core/src/coordinator/event_handler.rs`
- Handle `ForkedResponse` event from dialog-core
- Create separate media sessions for each fork (for early media)
- On confirmation: clean up non-selected forks' media sessions

### 3.3 Tests
- Unit: early dialog group creation
- Unit: fork detection from multiple 180 Ringing
- Unit: 2xx confirmation + BYE to other forks
- Unit: sequential forking (redirect → new INVITE)
- Integration: 3-party forking scenario

### 3.4 Estimated Effort: 1 agent, ~60 min

---

## Phase 4: SCTP Transport (Priority: LOW)

### 4.1 Scope Decision

SCTP has two distinct use cases:
1. **SCTP as SIP transport** (RFC 4168) — Rare, mainly for IMS networks
2. **DTLS-SCTP for WebRTC Data Channels** (RFC 8831) — Common for WebRTC

**Recommendation**: Implement DTLS-SCTP for WebRTC data channels first (higher value). SIP-over-SCTP can follow later.

### 4.2 DTLS-SCTP for WebRTC Data Channels

#### A. SCTP association over DTLS
**File**: New `crates/rtp-core/src/sctp/`

Use the `webrtc-sctp` crate (part of webrtc-rs project) as the SCTP implementation:

```toml
# crates/rtp-core/Cargo.toml
webrtc-sctp = { version = "0.9", optional = true }
```

Module structure:
```
crates/rtp-core/src/sctp/
├── mod.rs           # Public API
├── association.rs   # SCTP association management
├── channel.rs       # Data channel abstraction
└── dtls_sctp.rs     # DTLS-SCTP integration
```

#### B. `DtlsSctpTransport` struct
```rust
pub struct DtlsSctpTransport {
    dtls_connection: DtlsConnection,    // From existing crates/rtp-core/src/dtls/
    sctp_association: SctpAssociation,   // From webrtc-sctp
    data_channels: HashMap<u16, DataChannel>,
}

impl DtlsSctpTransport {
    pub async fn new(socket: UdpSocket, remote: SocketAddr, dtls_role: DtlsRole) -> Result<Self>
    pub async fn create_data_channel(&mut self, label: &str, options: DataChannelOptions) -> Result<DataChannel>
    pub async fn accept_data_channel(&mut self) -> Result<DataChannel>
    pub async fn send(&self, channel_id: u16, data: &[u8]) -> Result<()>
    pub async fn receive(&self) -> Result<(u16, Vec<u8>)>
}
```

#### C. SDP negotiation for data channels
**File**: `crates/sip-core/src/sdp/builder.rs`
```
m=application 9 UDP/DTLS/SCTP webrtc-datachannel
c=IN IP4 0.0.0.0
a=sctp-port:5000
a=max-message-size:262144
a=setup:actpass
a=fingerprint:sha-256 XX:XX:...
```

#### D. Session-core integration
- Add `DataChannelConfig` to `MediaConfig`
- In SDP offer: include `m=application` section when data channels requested
- In SDP answer: negotiate data channel parameters
- Expose `send_data(channel, data)` and `on_data_received(channel, data)` to application

### 4.3 SIP-over-SCTP (RFC 4168) — Deferred

#### A. Transport implementation
**File**: New `crates/sip-transport/src/transport/sctp/`

```rust
pub struct SctpTransport {
    association: SctpAssociation,
    local_addr: SocketAddr,
    streams: HashMap<u16, SctpStream>,
}

impl Transport for SctpTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()>;
    async fn close(&self) -> Result<()>;
    fn supports_sctp(&self) -> bool { true }
}
```

- Use `sctp-rs` or `lksctp` FFI bindings
- Implement multi-streaming: different SIP transactions on different SCTP streams
- Handle association failure → re-establish

#### B. Via header support
- Add `transport=sctp` to Via header generation
- Parse `transport=sctp` in incoming Via headers

### 4.4 Tests
- Unit: SCTP association setup/teardown
- Unit: Data channel create/accept/send/receive
- Unit: SDP negotiation with m=application
- Integration (`#[ignore]`): browser ↔ rvoip data channel

### 4.5 Estimated Effort: 1-2 agents, ~60 min (DTLS-SCTP only)

---

## Implementation Order

| Phase | Feature | Priority | Deps | Agent Est. |
|-------|---------|----------|------|-----------|
| 1 | TURN → ICE relay candidates | HIGH | turn/ + ice/ | 1 agent |
| 2a | ICE Keepalive (RFC 7675) | MEDIUM | ice/ | 1 agent |
| 2b | Trickle ICE (RFC 8838) | MEDIUM | ice/ + dialog-core | (same agent) |
| 3 | Dialog Forking (RFC 3261) | MEDIUM | dialog-core | 1 agent |
| 4 | DTLS-SCTP Data Channels | LOW | dtls/ + webrtc-sctp | 1 agent |

**Total**: 4 agents, ~3 hours

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|-----------|
| TURN shared socket conflicts | Media packet loss | Use pending_packets buffer (already fixed) |
| ICE trickle timing | Missed candidates | Buffer candidates, process on timer |
| Dialog forking state explosion | Memory growth | Limit max early dialogs per Call-ID (e.g., 10) |
| SCTP library maturity | Compile issues | Gate behind feature flag `sctp` |
| Multi-homed ICE with TURN | Wrong relay selected | Use foundation-based pairing |

---

## Success Criteria

| Feature | Criterion |
|---------|----------|
| TURN → ICE | SDP offer includes relay candidate; call succeeds behind symmetric NAT |
| ICE Keepalive | Call stays alive > 30s; drops if peer disappears |
| Trickle ICE | Candidates arrive after SDP exchange; connectivity established |
| Dialog Forking | UAC receives multiple 180 Ringing; accepts first 200 OK; BYEs others |
| DTLS-SCTP | Browser can open data channel to rvoip; send/receive messages |

---

*End of Implementation Plan*
