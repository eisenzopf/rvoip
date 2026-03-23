# AUDIT-003: rvoip Comprehensive Audit Report

**Date**: 2026-03-23
**Scope**: Full workspace audit — completeness, code quality, testing, complexity, edition compliance
**Version**: 0.1.26
**Workspace Edition**: Rust 2024 (rust-version = "1.85")
**Total Codebase**: ~482,250 lines / 1,319 files / 19 crates

---

## 1. WORKSPACE STRUCTURE

### 1.1 Crate Inventory

**Default Members (14):**

| Crate | Package Name | Lines | Layer |
|-------|-------------|-------|-------|
| crates/sip-core | rvoip-sip-core | 120,836 | Foundation |
| crates/rtp-core | rvoip-rtp-core | 64,927 | Transport |
| crates/dialog-core | rvoip-dialog-core | 43,864 | Protocol |
| crates/session-core | rvoip-session-core | 42,214 | Session |
| crates/media-core | rvoip-media-core | 34,616 | Protocol |
| crates/call-engine | rvoip-call-engine | 28,204 | Application |
| crates/client-core | rvoip-client-core | 19,918 | Application |
| crates/infra-common | rvoip-infra-common | 9,546 | Support |
| crates/codec-core | rvoip-codec-core | 7,043 | Support |
| crates/audio-core | rvoip-audio-core | 6,525 | Support |
| crates/sip-client | rvoip-sip-client | 6,166 | Application |
| crates/registrar-core | rvoip-registrar-core | 2,287 | Application |
| crates/rvoip | rvoip | 94 | Facade |
| crates/infra-common | rvoip-infra-common | 9,546 | Support |

**Non-Default Members (5):**

| Crate | Package Name | Lines | Status |
|-------|-------------|-------|--------|
| crates/users-core | users-core | 3,240 | Not in default-members |
| crates/intermediary-core | rvoip-intermediary-core | 1,323 | Not in default-members |
| crates/integration-tests | rvoip-integration-tests | — | Test-only crate |
| crates/auth-core | rvoip-auth-core | ~82 | Orphan — NOT listed in members or default-members |

**Removed:**
- transaction-core — merged into dialog-core (documented in Cargo.toml comments)

### 1.2 Architecture Layers (bottom-up)

```
┌─────────────────────────────────────────────┐
│  call-engine (B2BUA, queuing, routing)      │  Application
│  client-core / sip-client                   │
├─────────────────────────────────────────────┤
│  session-core (lifecycle, call control)     │  Session
├─────────────────────────────────────────────┤
│  dialog-core (dialog state + transactions)  │  Protocol
│  media-core (AEC, AGC, VAD, codecs)         │
├─────────────────────────────────────────────┤
│  sip-transport (UDP/TCP/TLS/WS)             │  Transport
│  rtp-core (RTP/RTCP/SRTP/DTLS)             │
├─────────────────────────────────────────────┤
│  sip-core (RFC 3261 parsing, SDP, headers)  │  Foundation
├─────────────────────────────────────────────┤
│  codec-core, audio-core, infra-common       │  Support
│  registrar-core, users-core, auth-core      │
└─────────────────────────────────────────────┘
```

---

## 2. EDITION COMPLIANCE

### 2.1 Workspace-Level Setting

```toml
# rvoip/Cargo.toml line 47
edition = "2024"
rust-version = "1.85"
```

### 2.2 Per-Crate Edition Status

| Crate | Edition | Method | Compliant? |
|-------|---------|--------|-----------|
| sip-core | 2024 | `edition.workspace = true` | ✅ |
| sip-transport | 2024 | `edition.workspace = true` | ✅ |
| dialog-core | 2024 | `edition.workspace = true` | ✅ |
| rtp-core | 2024 | `edition.workspace = true` | ✅ |
| media-core | 2024 | `edition.workspace = true` | ✅ |
| session-core | 2024 | `edition.workspace = true` | ✅ |
| call-engine | 2024 | `edition.workspace = true` | ✅ |
| client-core | 2024 | `edition.workspace = true` | ✅ |
| codec-core | 2024 | `edition.workspace = true` | ✅ |
| audio-core | 2024 | `edition.workspace = true` | ✅ |
| sip-client | 2024 | `edition.workspace = true` | ✅ |
| rvoip (facade) | 2024 | `edition.workspace = true` | ✅ |
| infra-common | 2024 | `edition.workspace = true` | ✅ |
| users-core | 2024 | `edition.workspace = true` | ✅ |
| integration-tests | 2024 | `edition.workspace = true` | ✅ |
| **registrar-core** | **2021** | **Hardcoded** | ❌ |
| **intermediary-core** | **2021** | **Hardcoded** | ❌ |
| **auth-core** | — | **Not in workspace members** | ❌ orphan |

### 2.3 Edition Findings

- **FINDING-ED-001**: `registrar-core` hardcodes `edition = "2021"` instead of inheriting workspace 2024
- **FINDING-ED-002**: `intermediary-core` hardcodes `edition = "2021"` instead of inheriting workspace 2024
- **FINDING-ED-003**: `auth-core` exists on disk but is NOT listed in `[workspace.members]` — orphan crate, unreachable by `cargo build --workspace`
- **FINDING-ED-004**: No Rust 2024-specific features (e.g., `gen` blocks, `unsafe extern`, edition-gated `use` changes) were observed being actively utilized — the edition upgrade appears to be declarative only

---

## 3. COMPLETENESS

### 3.1 SIP Protocol Coverage

**SIP Methods (RFC 3261 + extensions):**

| Method | RFC | Implemented? | Tested? |
|--------|-----|-------------|---------|
| INVITE | 3261 | ✅ | ✅ Heavy |
| ACK | 3261 | ✅ | ✅ |
| BYE | 3261 | ✅ | ✅ |
| CANCEL | 3261 | ✅ | ✅ |
| REGISTER | 3261 | ✅ | ✅ |
| OPTIONS | 3261 | ✅ | ✅ |
| SUBSCRIBE | 6665 | ✅ | ✅ |
| NOTIFY | 6665 | ✅ | ✅ |
| MESSAGE | 3428 | ✅ | ✅ |
| UPDATE | 3311 | ✅ | ✅ |
| INFO | 6086 | ✅ | ✅ |
| PRACK | 3262 | ✅ | ✅ |
| REFER | 3515 | ✅ | ✅ |
| PUBLISH | 3903 | ✅ | ✅ |
| Extension(String) | — | ✅ Custom | — |

**Verdict**: ✅ All 14 standard methods + custom extensions supported

### 3.2 SDP (Session Description Protocol)

- RFC 8866 compliant (obsoletes RFC 4566)
- Session-level + media-level parsing
- Field order validation + mandatory field enforcement
- WebRTC attributes: ICE candidates, SSRC, RID
- Limits: 16KB max, 500 lines, 32 media sections
- **Files**: `sip-core/src/types/sdp.rs` (1,066 lines), `sip-core/src/sdp/parser/sdp_parser.rs` (1,036 lines)

**Verdict**: ✅ Complete

### 3.3 Codecs

| Codec | Status | Notes |
|-------|--------|-------|
| G.711 PCMU (μ-law) | ✅ Full | Production-ready |
| G.711 PCMA (A-law) | ✅ Full | Production-ready |
| G.729 Base | ✅ Full | Including Annex A (reduced complexity) |
| G.729 Annex B | ✅ Full | VAD/DTX/CNG |
| Opus | ⚠️ **Stub** | `.pack()`/`.unpack()` copy bytes without actual encoding/decoding |
| DTMF RFC 4733 | ✅ Full | **Not wired to client API** |

**FINDING-COMP-001**: Opus codec is a pass-through stub — no actual Opus encoding/decoding
**FINDING-COMP-002**: DTMF RFC 4733 implemented but not accessible from client-core API

### 3.4 Transport

| Transport | Status | Notes |
|-----------|--------|-------|
| UDP | ✅ Full | Primary transport |
| TCP | ✅ Full | Connection pooling |
| TLS | ✅ Full | rustls 0.23 |
| WebSocket | ✅ Full | RFC 7118, **not wired to client-core builder** |
| SCTP | ⚠️ Adapter | Experimental adapter layer |

**FINDING-COMP-003**: WebSocket transport implemented but not accessible from client-core builder API

### 3.5 Media Security

| Feature | RFC | Status | Notes |
|---------|-----|--------|-------|
| SRTP | 3711 | ✅ | Encryption/decryption |
| DTLS-SRTP | 5764 | ✅ | **Critical bug — see SEC-001** |
| ZRTP | 6189 | ✅ | With test coverage |
| MIKEY | 3830 | ✅ | Key exchange |
| SDES | 4568 | ✅ | Older method |

### 3.6 Features Implemented But Not Integrated

| Feature | RFC | Code Complete | Integrated into Call Flow |
|---------|-----|--------------|--------------------------|
| ICE Agent | 8445 | ✅ | ❌ Not wired |
| TURN Client | 5766 | ✅ | ❌ Not wired |
| STUN Client | 5389 | ✅ | ❌ Not wired |
| DTLS-SRTP Bridge | 5764 | ✅ | ❌ Not wired |
| WebSocket Client | 7118 | ✅ | ❌ Not wired |
| DTMF Codec | 4733 | ✅ | ❌ Not wired |
| Attended Transfer | — | TODO Phase 2 | ❌ Only blind transfer |
| Conference/Mixing | — | Stub | ❌ Multi-party impossible |
| RTP Statistics | — | Always returns zero | ❌ TODO markers in code |

**FINDING-COMP-004**: 6 fully implemented features (ICE, TURN, STUN, DTLS-SRTP, WebSocket, DTMF) are isolated from the actual call flow — unreachable from client APIs

---

## 4. CODE QUALITY — PRX RULE COMPLIANCE

### 4.1 Rule #1: NO panic-capable unwrapping

**FINDING-QUAL-001 [CRITICAL]**: 5,767 panic-capable calls in production code

| Crate | `.unwrap()` | `.expect()` | Total |
|-------|-----------|-----------|-------|
| sip-core | 3,750 | 78 | 3,828 |
| dialog-core | 392 | 88 | 480 |
| rtp-core | 354 | 89 | 443 |
| media-core | 270 | 12 | 282 |
| sip-transport | 192 | 12 | 204 |
| client-core | 191 | 7 | 198 |
| session-core | 106 | 14 | 120 |
| audio-core | 52 | 0 | 52 |
| codec-core | 47 | 6 | 53 |
| call-engine | 40 | 1 | 41 |
| infra-common | 32 | 5 | 37 |
| users-core | 14 | 0 | 14 |
| registrar-core | 6 | 0 | 6 |
| intermediary-core | 5 | 0 | 5 |
| sip-client | 4 | 0 | 4 |
| **TOTAL** | **5,455** | **312** | **5,767** |

**Severity**: This is the single largest quality debt in the project.

### 4.2 Rule #2: NO dead code

- 10 instances of `#[allow(dead_code)]` found
- Workspace lint config allows `dead_code` globally — masks violations
- **FINDING-QUAL-002**: Workspace lint `dead_code = "allow"` defeats Rule #2

### 4.3 Rule #3: NO incomplete implementations

- **FINDING-QUAL-003**: 26 `unimplemented!()` macros found in dialog-core (within doc comments — not executable but concerning)
- **FINDING-QUAL-004**: 291 TODO markers across codebase (informational but indicates unfinished work)

### 4.4 Rule #6: Explicit error handling

**FINDING-QUAL-005 [HIGH]**: 271 instances of `let _ =` silently discarding Results

| Crate | `let _ =` Count | Severity |
|-------|-----------------|----------|
| session-core | 82 | HIGH — event publications silently dropped |
| rtp-core | 48 | MEDIUM |
| dialog-core | 33 | MEDIUM |
| media-core | 31 | MEDIUM |
| sip-transport | 26 | MEDIUM |
| sip-client | 16 | LOW |
| client-core | 16 | LOW |
| call-engine | 5 | LOW |
| infra-common | 4 | LOW |
| audio-core | 2 | LOW |

Typical pattern: `let _ = self.publish_event(...);` — caller never knows if event delivery failed.

### 4.5 Rule #7: Minimize allocations

Not systematically audited. No egregious violations observed, but `Arc<Mutex>` usage (294 instances) suggests over-cloning.

### 4.6 Safety Rules

**std::sync::Mutex in async context:**

**FINDING-QUAL-006 [MEDIUM]**: 7 instances of `std::sync::Mutex` in production async code:
- `dialog-core/src/transaction/timer/factory.rs:374` (1)
- `media-core/src/relay/controller/mod.rs` (6)

PRX rules require `parking_lot::Mutex` (sync) or `tokio::sync::Mutex` (async).

**Unsafe blocks:**
- 43 total unsafe blocks across sip-core (26), rtp-core (9), dialog-core (6), codec-core (2)
- ✅ All examined blocks have `// SAFETY:` comments

**FINDING-QUAL-007 [MEDIUM]**: Unsafe pointer casts in `sip-core/src/types/headers/typed_header.rs:259-269`:
```rust
Some(unsafe { &*(h as *const _ as *const T) })
```
~7 instances without runtime type validation. Type confusion possible if TypeId check is bypassed.

### 4.7 Workspace Lint Configuration

**FINDING-QUAL-008 [MEDIUM]**: Lint config is overly permissive — only `correctness` and `suspicious` set to `warn`. All other clippy groups (`pedantic`, `style`, `complexity`, `perf`, `cargo`, `nursery`, `restriction`) set to `allow`. Additionally 49 individual lints explicitly allowed. This masks real issues.

---

## 5. SECURITY FINDINGS

### 5.1 SEC-001 [CRITICAL]: DTLS-SRTP Plaintext Fallback

**File**: `rtp-core/src/transport/security_transport.rs:120`
```rust
debug!("SRTP decryption failed, treating as plain RTP: {}", e);
```

**Issue**: After successfully negotiating DTLS-SRTP encryption, if decryption fails on any packet, the code falls back to processing the packet as **unencrypted plaintext RTP**.

**Violation**: RFC 5764 Section 3 — once DTLS-SRTP is negotiated, all RTP MUST be encrypted. Plaintext fallback enables eavesdropping.

**Impact**: An attacker can inject unencrypted packets that will be accepted, or force decryption key misalignment to downgrade the entire call to plaintext.

**Remediation**: Drop the packet and log an error. Never fall back to plaintext after DTLS-SRTP negotiation.

### 5.2 SEC-002 [CRITICAL]: OAuth TLS Certificate Bypass

**File**: `session-core/src/auth/oauth.rs:140`
```rust
.danger_accept_invalid_certs(true)
```

**Issue**: TLS certificate validation is unconditionally disabled for OAuth token exchange. Not gated behind `#[cfg(test)]` or a feature flag.

**Impact**: Man-in-the-middle attacks on OAuth token exchange. Attacker can intercept and steal authentication tokens.

**Remediation**: Remove `danger_accept_invalid_certs(true)`. If needed for development, gate behind `#[cfg(test)]` or a `dangerous-tls-bypass` feature flag.

### 5.3 SEC-003 [HIGH]: Production panic!() Calls

**6 `panic!()` calls in production code:**

| Location | Trigger |
|----------|---------|
| `sip-core/src/macros.rs:391` | Macro compile-time error (acceptable) |
| `audio-core/src/device/mod.rs:27` | `as_any` not implemented |
| `session-core/src/state_table/mod.rs:40` | Invalid YAML config |
| `session-core/src/state_table/mod.rs:89` | Invalid YAML config |
| `call-engine/src/api/admin.rs:815` | Missing engine instance |
| `call-engine/src/api/supervisor.rs:1229` | Missing engine instance |

**Impact**: Invalid configuration or missing initialization crashes the entire process. No graceful degradation.

### 5.4 SEC-004 [MEDIUM]: No Graceful Shutdown

- 341 `tokio::spawn` calls without `JoinHandle` tracking
- No shutdown coordination mechanism observed
- In-flight calls will be abruptly terminated on process exit

---

## 6. TEST COVERAGE

### 6.1 Overall Statistics

- **Total test functions**: 4,387
- **Crates with tests**: 17 / 18
- **Integration test files**: 201 (tests/ directories)
- **Source files with inline tests**: 513

### 6.2 Per-Crate Test Count

| Crate | Unit (src/) | Integration (tests/) | Total | Assessment |
|-------|-----------|-------------------|-------|-----------|
| sip-core | 2,007 | 59 | 2,066 | ✅ Excellent |
| session-core | 86 | 566 | 658 | ✅ Excellent — 75 test files |
| rtp-core | 395 | 4 | 399 | ✅ Good |
| media-core | 255 | 68 | 323 | ✅ Good |
| dialog-core | 174 | 124 | 298 | ✅ Good |
| codec-core | 126 | 0 | 126 | ✅ Adequate |
| audio-core | 63 | 63 | 126 | ✅ Adequate |
| client-core | 27 | 98 | 125 | ✅ Adequate |
| sip-transport | 50 | 13 | 63 | ⚠️ Low for transport layer |
| sip-client | 40 | 22 | 62 | ⚠️ Low |
| users-core | 5 | 49 | 54 | ⚠️ Low |
| infra-common | 29 | 0 | 29 | ⚠️ Low — no dedicated test files |
| call-engine | 7 | 20 | 27 | ⚠️ Low for B2BUA engine |
| intermediary-core | 10 | 0 | 10 | ⚠️ Very low |
| integration-tests | 0 | 10 | 10 | ⚠️ Only 3 cross-crate test files |
| registrar-core | 6 | 0 | 6 | ❌ Minimal |
| rvoip (facade) | 0 | 4 | 4 | — Doc examples only |
| **auth-core** | **0** | **0** | **0** | ❌ **Zero tests** |

### 6.3 Cross-Crate Integration Tests

**Dedicated crate**: `crates/integration-tests/` with 3 test files:

1. `sctp_sip_integration.rs` (4 tests) — SIP over SCTP
2. `dialog_transport_integration.rs` (3 tests) — dialog-core + sip-transport
3. `session_media_integration.rs` (3 tests) — SDP negotiation → media flow E2E

**FINDING-TEST-001**: Cross-crate integration coverage is thin — only 10 tests in 3 files. Critical integration paths not covered:
- No sip-core → sip-transport → dialog-core → session-core full-stack test
- No call-engine → session-core → media-core → rtp-core orchestration test
- No registrar-core → dialog-core registration flow test

### 6.4 E2E Test Scenarios (within session-core)

Session-core has strong E2E coverage internally (75 test files):
- `e2e_register_and_call.rs` — Full registration + call flow
- `e2e_call_with_audio.rs` — Call with audio exchange
- `e2e_encrypted_call.rs` — Encrypted media calls
- Conference tests, presence tests, transfer tests, hold/resume tests

### 6.5 Test Infrastructure

- **Frameworks**: tokio::test (primary), serial_test, proptest (property-based), criterion (benchmarks)
- **Utilities**: Comprehensive test helpers in `session-core/tests/common/` (6 utility modules)
- **Mocks**: Test request builders in dialog-core, port allocation helpers in integration-tests

### 6.6 Test Gaps

| Gap | Severity | Details |
|-----|----------|---------|
| auth-core zero tests | HIGH | Authentication module completely untested |
| registrar-core 6 tests | HIGH | SIP registration barely tested |
| call-engine 27 tests | MEDIUM | B2BUA/call center orchestration under-tested |
| intermediary-core 10 tests | MEDIUM | SIP proxy/gateway under-tested |
| Cross-crate integration | MEDIUM | Only 10 tests across 3 files |
| No coverage reports | LOW | No LCOV/tarpaulin reports or CI integration |
| No mutation testing | LOW | No cargo-mutants or similar |

---

## 7. COMPLEXITY ANALYSIS

### 7.1 Large Files (>1,000 lines)

| File | Lines | Risk |
|------|-------|------|
| `client-core/src/client/media.rs` | 3,454 | HIGH — media session monolith |
| `dialog-core/src/transaction/manager/mod.rs` | 2,486 | MEDIUM — transaction state machine |
| `session-core/src/media/manager.rs` | 2,252 | MEDIUM — media lifecycle |
| `client-core/src/client/manager.rs` | 2,124 | MEDIUM — client lifecycle |
| `sip-core/src/builder/multipart.rs` | 2,067 | MEDIUM — MIME multipart |
| `dialog-core/src/api/client.rs` | 1,889 | MEDIUM — dialog API |
| `sip-core/src/builder/response.rs` | 1,761 | MEDIUM — response builder |
| `client-core/src/client/controls.rs` | 1,708 | MEDIUM — call control |
| `call-engine/src/orchestrator/calls.rs` | 1,598 | MEDIUM — orchestration |
| `session-core/src/coordinator/event_handler.rs` | 1,590 | MEDIUM — event dispatch |
| `rtp-core/src/dtls/connection.rs` | 1,578 | MEDIUM — DTLS state |
| `sip-core/src/json/path.rs` | 1,572 | LOW — JSON access |
| `sip-client/src/simple.rs` | 1,572 | MEDIUM — client API |
| `media-core/src/codec/audio/g729_engine.rs` | 1,507 | LOW — codec impl |
| `rtp-core/src/srtp/crypto.rs` | 1,504 | MEDIUM — crypto |
| `session-core/src/dialog/coordinator.rs` | 1,463 | MEDIUM — coordination |
| `dialog-core/src/transaction/client/invite.rs` | 1,411 | MEDIUM — INVITE FSM |
| `call-engine/src/orchestrator/routing.rs` | 1,380 | MEDIUM — routing |
| `client-core/src/client/recovery.rs` | 1,379 | MEDIUM — recovery |

**FINDING-CMPLX-001**: 19 files exceed 1,000 lines. `client-core/src/client/media.rs` at 3,454 lines is the most concerning — should be decomposed.

### 7.2 Concurrency Complexity

- **294 `Arc<Mutex>` instances** — creates lock contention under load
- **341 `tokio::spawn` without JoinHandle tracking** — no graceful shutdown
- **7 `std::sync::Mutex` in async context** — can block tokio worker threads

### 7.3 State Machine Complexity

- **Transaction FSM** (dialog-core): ~15 states, ~50 transitions — well-structured with explicit enums
- **Dialog FSM**: 5 states (Initial, Early, Confirmed, Recovering, Terminated) — clean
- **Session lifecycle** (session-core): Complex but manageable, good event-driven design

### 7.4 Dependency Health

**Core dependencies** — all well-maintained:
- tokio 1.36, serde 1.0, rustls 0.23, ring 0.17, nom 7.1
- dashmap 5.5, parking_lot 0.12, tracing 0.1

**Concerning:**
- `once_cell 1.19` — should migrate to `std::sync::OnceLock` (stable since Rust 1.70)
- `webrtc-util 0.11` — uncertain maintenance status
- 4 build profiles defined (release, release-small, release-fast, flamegraph) — well-organized

---

## 8. FINDINGS SUMMARY

### Critical (P0)

| ID | Finding | Location |
|----|---------|----------|
| SEC-001 | DTLS-SRTP falls back to plaintext on decryption failure | `rtp-core/src/transport/security_transport.rs:120` |
| SEC-002 | OAuth TLS certificate validation unconditionally disabled | `session-core/src/auth/oauth.rs:140` |
| QUAL-001 | 5,767 panic-capable unwrap/expect calls in production code | Workspace-wide, worst in sip-core (3,828) |

### High (P1)

| ID | Finding | Location |
|----|---------|----------|
| SEC-003 | 6 production panic!() calls — config/init errors crash process | audio-core, session-core, call-engine |
| QUAL-005 | 271 silent error swallowing via `let _ =` | session-core (82), rtp-core (48), dialog-core (33) |
| COMP-004 | 6 features implemented but not integrated into call flow | ICE, TURN, STUN, DTLS-SRTP bridge, WebSocket, DTMF |
| TEST-001 | Cross-crate integration tests inadequate (10 tests / 3 files) | crates/integration-tests/ |

### Medium (P2)

| ID | Finding | Location |
|----|---------|----------|
| ED-001 | registrar-core hardcodes edition = "2021" | `crates/registrar-core/Cargo.toml` |
| ED-002 | intermediary-core hardcodes edition = "2021" | `crates/intermediary-core/Cargo.toml` |
| ED-003 | auth-core is orphan crate — not in workspace members | `crates/auth-core/` |
| QUAL-006 | 7 std::sync::Mutex in async context | dialog-core (1), media-core (6) |
| QUAL-007 | Unsafe pointer casts without runtime type validation | `sip-core/src/types/headers/typed_header.rs:259-269` |
| QUAL-008 | Workspace lint config too permissive | `rvoip/Cargo.toml [workspace.lints]` |
| SEC-004 | No graceful shutdown — 341 untracked tokio::spawn | Workspace-wide |
| CMPLX-001 | 19 files exceed 1,000 lines (max 3,454) | See Section 7.1 |
| COMP-001 | Opus codec is stub — no actual encoding | `codec-core/src/codecs/opus/` |

### Low (P3)

| ID | Finding | Location |
|----|---------|----------|
| ED-004 | No Rust 2024 edition-specific features actively used | Workspace-wide |
| QUAL-002 | Workspace allows dead_code lint globally | `rvoip/Cargo.toml` |
| QUAL-003 | 26 unimplemented!() in dialog-core doc comments | `dialog-core/src/api/` |
| QUAL-004 | 291 TODO markers across codebase | Workspace-wide |
| COMP-002 | DTMF not wired to client API | codec-core → client-core gap |
| COMP-003 | WebSocket not wired to client-core builder | sip-transport → client-core gap |

---

## 9. METRICS DASHBOARD

| Metric | Value | Target | Status |
|--------|-------|--------|--------|
| Total LOC | 482,250 | — | — |
| Total crates | 19 | — | — |
| Edition 2024 compliance | 17/19 (89%) | 100% | ⚠️ |
| Total tests | 4,387 | — | — |
| Crates with tests | 17/18 (94%) | 100% | ⚠️ |
| unwrap/expect in prod | 5,767 | 0 | ❌ |
| Silent error swallows | 271 | 0 | ❌ |
| Production panic!() | 6 | 0 | ❌ |
| Unsafe blocks (documented) | 43/43 | 100% | ✅ |
| std::sync::Mutex in async | 7 | 0 | ⚠️ |
| Files > 1000 lines | 19 | 0 | ⚠️ |
| Security critical findings | 2 | 0 | ❌ |
| Features not integrated | 6 | 0 | ⚠️ |

---

## 10. RECOMMENDED REMEDIATION PRIORITY

### Phase 1 — Security (Immediate)
1. Fix DTLS-SRTP plaintext fallback (SEC-001) — drop packets, never fall back
2. Remove OAuth TLS bypass (SEC-002) — gate behind `#[cfg(test)]` or feature flag
3. Replace 6 production panic!() with proper error returns (SEC-003)

### Phase 2 — Stability (1-2 weeks)
4. Systematic unwrap/expect elimination — start with sip-core (3,828), prioritize network-facing code paths
5. Audit and fix 271 `let _ =` patterns — at minimum log errors
6. Replace std::sync::Mutex with parking_lot/tokio alternatives (7 instances)

### Phase 3 — Integration (2-4 weeks)
7. Wire ICE/TURN/STUN into session-core call flow
8. Wire DTLS-SRTP bridge into rtp-core transport
9. Wire WebSocket transport into client-core builder
10. Wire DTMF codec to client API

### Phase 4 — Quality (ongoing)
11. Upgrade registrar-core and intermediary-core to edition 2024
12. Resolve auth-core orphan status (add to members or delete)
13. Expand cross-crate integration tests (target: 50+ tests)
14. Add auth-core and registrar-core test coverage
15. Tighten workspace lint config — enable `style` and `complexity` at minimum
16. Decompose files > 2000 lines

---

## 11. CODEX RE-AUDIT INSTRUCTIONS

This report should be verified by Codex with the following specific checks:

1. **Verify unwrap/expect counts**: Run `grep -r '\.unwrap()' --include='*.rs' crates/*/src/ | grep -v test | wc -l` and compare against reported 5,455
2. **Verify SEC-001**: Read `rtp-core/src/transport/security_transport.rs` around line 120 and confirm plaintext fallback behavior
3. **Verify SEC-002**: Read `session-core/src/auth/oauth.rs` around line 140 and confirm `danger_accept_invalid_certs(true)`
4. **Verify edition overrides**: Read `registrar-core/Cargo.toml` and `intermediary-core/Cargo.toml` for hardcoded edition
5. **Verify auth-core orphan**: Confirm `auth-core` is not listed in workspace `[members]` in root Cargo.toml
6. **Verify integration test count**: Count `#[test]` + `#[tokio::test]` in `crates/integration-tests/tests/`
7. **Spot-check let _ = patterns**: Sample 10 instances from session-core and confirm they discard Results
8. **Verify Opus stub**: Read `codec-core/src/codecs/opus/` and confirm no actual Opus FFI or encoding logic
9. **Check for any findings this audit may have missed**
10. **Assess whether any findings have been remediated since last audit**

---

*End of AUDIT-003*
