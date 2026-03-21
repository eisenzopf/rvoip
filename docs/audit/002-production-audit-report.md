# RVOIP Production Audit Report

**Document ID**: AUDIT-002
**Date**: 2026-03-21
**Auditor**: Claude Code (Opus 4.6)
**Scope**: Full workspace (14 crates, ~480,000 LOC)
**Edition**: Rust 2024
**Branch**: prx

---

## 1. Executive Summary

This audit covers the complete rvoip workspace after the production readiness remediation session. The session addressed ~5,275 unwrap/expect/panic calls, ~2,800 println statements, and implemented 7 new modules (Digest Auth, REGISTER, STUN, ICE, TURN, DTLS-SRTP Bridge, WebSocket Client, DTMF).

**Current State**: 14/14 crates compile with **zero warnings, zero errors** on Rust Edition 2024.

### Remaining Risk Summary

| Severity | Count | Description |
|----------|-------|-------------|
| 🔴 CRITICAL | 4 | Crash/security vulnerabilities |
| 🟠 HIGH | 8 | Incorrect behavior in production |
| 🟡 MEDIUM | 5 | Issues under load/edge cases |
| 🟢 LOW | 3 | Code quality |

---

## 2. What Was Fixed This Session

### 2.1 Unwrap/Expect/Panic Elimination
- **Before**: ~5,275 production unwrap/expect + ~510 panic!
- **After**: 0 production unwrap + 6 panic! (see §3.1)
- **Method**: 28 parallel sub-agents across all 14 crates
- **Verification**: `cargo check --workspace` zero warnings

### 2.2 println → tracing Migration
- **Before**: ~2,800 println/eprintln in production code
- **After**: 0 (all converted to tracing macros with structured fields)

### 2.3 std::sync::Mutex → parking_lot Migration
- Migrated in: infra-common, audio-core, session-core
- **Remaining violations**: See §3.4

### 2.4 New Feature Implementations

| Feature | RFC | Module | Tests |
|---------|-----|--------|-------|
| Digest Auth | RFC 2617/7616 | `sip-core/src/auth/digest.rs` | 30 |
| REGISTER flow | RFC 3261 | `session-core/src/coordinator/registration.rs` | 7 |
| STUN Client | RFC 5389 | `rtp-core/src/stun/` | 16 |
| ICE Agent | RFC 8445 | `rtp-core/src/ice/` | 35 |
| TURN Client | RFC 5766 | `rtp-core/src/turn/` | 20 |
| DTLS-SRTP Bridge | RFC 5764 | `session-core/src/media/srtp_bridge.rs` | 8 |
| WebSocket Client | RFC 7118 | `sip-transport/src/transport/ws/` | 2 |
| DTMF | RFC 4733 | `media-core/src/dtmf/` | codec tests |

### 2.5 Edition 2024 Upgrade
- `gen` keyword escaping (3 files)
- Implicit borrow pattern fix (1 file)
- `rust-version` bumped to 1.85

---

## 3. Remaining CRITICAL Issues

### 3.1 Production panic! Calls (6 remaining)

| File | Line | Code | Fix |
|------|------|------|-----|
| `sip-core/src/macros.rs` | 391 | `panic!("BUG: sip_request! macro received invalid URI")` | Acceptable — compile-time macro error |
| `audio-core/src/device/mod.rs` | 27 | `panic!("AudioDevice::as_any not implemented")` | Return Error or use unreachable |
| `session-core/src/state_table/mod.rs` | 40 | `panic!("Invalid default state table")` | Return Error — embedded YAML could be corrupted |
| `session-core/src/state_table/mod.rs` | 89 | `panic!("Invalid default state table")` | Same |
| `call-engine/src/api/admin.rs` | 815 | `panic!("AdminApi requires an engine instance")` | Return Error |
| `call-engine/src/api/supervisor.rs` | 1229 | `panic!("SupervisorApi requires an engine instance")` | Return Error |

**Risk**: Invalid config or missing initialization crashes the process.

### 3.2 TLS Certificate Validation Bypass

**File**: `session-core/src/auth/oauth.rs:140`
```rust
.danger_accept_invalid_certs(true)
```

**Impact**: MITM attacks on OAuth token exchange. Not gated behind `#[cfg(test)]`.
**Fix**: Remove or gate behind explicit `insecure_tls` feature flag.

### 3.3 SRTP Fallback to Plain RTP

**File**: `rtp-core/src/transport/security_transport.rs:120`
```rust
debug!("SRTP decryption failed, treating as plain RTP: {}", e);
```

**Impact**: After DTLS-SRTP negotiation, decryption failure silently downgrades to unencrypted media. Violates RFC 5764 §3.
**Fix**: Drop packet on decryption failure, log error, do NOT process as plain RTP.

### 3.4 std::sync::Mutex in Async Code (7 remaining)

| File | Line | Context |
|------|------|---------|
| `dialog-core/src/transaction/timer/factory.rs` | 374 | Timer factory |
| `media-core/src/relay/controller/mod.rs` | 760 | Static RTP counters |
| `media-core/src/relay/controller/mod.rs` | 761 | Same |
| `media-core/src/relay/controller/mod.rs` | 809 | Static frame counters |
| `media-core/src/relay/controller/mod.rs` | 810 | Same |
| `media-core/src/relay/controller/mod.rs` | 836 | Static logged set |
| `media-core/src/relay/controller/mod.rs` | 837 | Same |

**Impact**: Can block tokio worker threads. Static Lazy<Mutex> are low-risk (short hold times) but still violate project rules.

---

## 4. HIGH Severity Issues

### 4.1 Silent Error Discarding (`let _ =`)

**Count**: 271 instances in production code
**Highest concentration**:
- `dialog-core/src/transaction/common_logic.rs` — ~26 `let _ = events_tx.send()`
- `media-core/src/relay/controller/` — multiple send errors swallowed
- `session-core/src/coordinator/` — event dispatch errors lost

**Impact**: Transaction state changes, media events, and session events silently lost. Callers never know operations failed.

### 4.2 New Modules Not Integrated into Call Flow

| Module | Status | Gap |
|--------|--------|-----|
| ICE Agent | Implemented | Not called from session-core SDP negotiation |
| TURN Client | Implemented | Not called from ICE candidate gathering |
| STUN | Implemented | SDP offer doesn't include server-reflexive address |
| DTLS-SRTP Bridge | Implemented | `protect_rtp()`/`unprotect_rtp()` not called from actual RTP send/recv |
| DTMF codec | Implemented | No API path from client-core → media-core DTMF |
| WebSocket Client | Implemented | client-core builder has no WS transport option |

**Impact**: Features exist as isolated modules but calls still use plain UDP with no ICE, no SRTP, no DTMF.

### 4.3 Registration Refresh Failure Recovery

**File**: `session-core/src/coordinator/registration.rs`
**Issue**: Refresh failure → no retry, no backoff, no re-registration
**Impact**: Temporary network outage → registration expires → incoming calls fail silently

### 4.4 Attended Transfer Not Implemented

**File**: `session-core/src/coordinator/transfer.rs:130`
**Issue**: `// TODO: Implement attended transfer in phase 2`
**Impact**: Only blind transfer works. Enterprise PBX scenarios requiring attended transfer will fail.

### 4.5 TODO Count in Production Code

**Total**: 291 TODO/FIXME markers
**Critical TODOs**:
- `session-core/src/coordinator/event_handler.rs:814` — media establishment
- `call-engine/src/orchestrator/core.rs:662` — DTMF IVR handling
- `session-core/src/coordinator/event_handler.rs:1411` — PIDF XML parsing
- `call-engine/src/orchestrator/calls.rs:1531` — call hold
- `call-engine/src/orchestrator/calls.rs:1550` — call resume

### 4.6 Presence/SUBSCRIBE/NOTIFY Callbacks Missing

**Files**: `session-core/src/coordinator/event_handler.rs` lines 1389, 1423, 1447, 1484
**Issue**: 4 TODO markers for subscription/presence callbacks to CallHandler trait
**Impact**: Application layer cannot receive presence updates

### 4.7 Unsafe Pointer Casts Without Type Validation

**File**: `sip-core/src/types/headers/typed_header.rs:259-269`
```rust
Some(unsafe { &*(h as *const _ as *const T) })
```
**Count**: ~7 instances of raw pointer casting
**Impact**: Type confusion if wrong header type is cast. Memory safety violation.

### 4.8 Conference/Mixing Not Implemented

**File**: `session-core/src/conference/` — all files are stubs
**Impact**: Multi-party calls impossible

---

## 5. MEDIUM Severity Issues

### 5.1 tokio::spawn Without Handle Tracking

**Count**: 341 `tokio::spawn` calls, most without storing JoinHandle
**Impact**: No graceful shutdown, orphaned tasks, resource leaks
**Evidence**: `session-core/tests/transfer_debug_test.rs:154` uses `std::process::exit(0)` because tasks won't terminate

### 5.2 RTP Statistics Always Zero

**File**: `rtp-core/src/events/adapter.rs:179-184`
```rust
packets_sent: 0,     // TODO: Get actual stats
packets_received: 0, // TODO: Get actual stats
```
**Impact**: Monitoring dashboards show zero metrics

### 5.3 IPv6 Not Tested

**Finding**: IPv6 code exists in sip-transport and rtp-core but no test coverage
**Impact**: IPv6-only environments may have undiscovered bugs

### 5.4 Opus Codec Stub Only

**File**: `media-core/src/rtp_processing/payload/opus.rs`
**Issue**: `.pack()` and `.unpack()` just copy bytes, no actual Opus encoding
**Impact**: Only G.711 PCMU/PCMA available. No wideband audio.

### 5.5 OAuth `allow_insecure` Available in Production

**File**: `session-core/src/auth/oauth.rs:35`
**Issue**: Config field not gated behind `#[cfg(test)]` or feature flag
**Impact**: Can be accidentally enabled in production config

---

## 6. LOW Severity Issues

### 6.1 MIKEY Certificate Chain Not Supported
**File**: `rtp-core/src/security/mikey/mod.rs:441`
**Impact**: Minor — DTLS-SRTP is the primary path

### 6.2 Minimal Error Context in Error Returns
**Pattern**: `headers: HashMap::new() // TODO` in coordinator responses
**Impact**: Debugging production issues is harder

### 6.3 Inconsistent Error Handling Patterns
**Pattern**: Mix of `let _ =`, `.ok()`, `if let Err(_)`, `?` across crates
**Impact**: Code review and maintenance burden

---

## 7. Test Coverage Summary

| Crate | Lib Tests | Status |
|-------|-----------|--------|
| sip-core | 1,357 | ✅ All pass |
| rtp-core | 282 (16 stun + 35 ice + 20 turn + 211 others) | ✅ All pass |
| session-core | 82 | ✅ Lib pass |
| dialog-core | ~50 | ✅ Lib pass |
| media-core | — | ⚠️ 3 pre-existing test compile errors |
| infra-common | — | ⚠️ 4 pre-existing test compile errors |
| call-engine | — | Not verified |

**New tests added this session**: 118 (auth 30 + stun 16 + ice 35 + turn 20 + srtp 8 + registration 7 + dtmf + ws 2)

---

## 8. Architecture Assessment

### Strengths
- Clean layered architecture (sip-core → transport → dialog → session → client)
- Comprehensive SDP parsing with WebRTC attribute support
- Buffer pooling for RTP performance
- Structured logging throughout (post-remediation)

### Weaknesses
- New modules (ICE, TURN, DTLS-SRTP, DTMF) are isolated — not wired into call flow
- No graceful shutdown mechanism
- Arc<Mutex> over-usage (294 instances) creates lock contention under load
- Conference/mixing is pure stub

---

## 9. Compliance Matrix

| Standard | Status | Gaps |
|----------|--------|------|
| RFC 3261 (SIP) | ⚠️ Partial | CSeq monotonic validation missing, attended transfer TODO |
| RFC 2617/7616 (Digest Auth) | ✅ Complete | — |
| RFC 5389 (STUN) | ✅ Complete | Not integrated into SDP |
| RFC 8445 (ICE) | ✅ Complete | Not integrated into call flow |
| RFC 5766 (TURN) | ✅ Complete | Not integrated into ICE gathering |
| RFC 5764 (DTLS-SRTP) | ⚠️ Partial | Fallback to plain RTP violates spec |
| RFC 4733 (DTMF) | ✅ Complete | Not wired to client API |
| RFC 7118 (SIP over WS) | ✅ Complete | Not wired to client builder |

---

## 10. Recommended Fix Priority

### Batch 1: Critical Security + Crash (Est: 4 agents)
1. Fix 5 production panic! → return Error
2. Remove `danger_accept_invalid_certs(true)`
3. Fix SRTP plain-text fallback → drop packet
4. Replace 7 std::sync::Mutex → parking_lot

### Batch 2: Integration Wiring (Est: 4 agents)
5. Wire ICE → session-core SDP negotiation
6. Wire DTMF → client-core/call-engine API
7. Wire DTLS-SRTP → actual RTP send/receive
8. Wire WebSocket → client-core builder

### Batch 3: Error Handling + Robustness (Est: 6 agents)
9. Audit 271 `let _ =` — add tracing::warn for critical paths
10. Registration refresh retry with exponential backoff
11. Graceful shutdown for spawned tasks
12. Fix unsafe pointer casts in typed_header.rs

### Batch 4: Feature Completion (Est: 4 agents)
13. Attended transfer implementation
14. Presence callbacks wiring
15. Conference/mixing basic implementation
16. RTP statistics collection

---

*End of Audit Report*
