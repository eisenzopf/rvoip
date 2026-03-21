# RVOIP Production Readiness Plan

**Document ID**: TASK-001
**Created**: 2026-03-21
**Status**: Approved
**Priority**: Critical

---

## 1. Executive Summary

This document outlines the roadmap to bring rvoip from its current alpha state to production-grade quality. The single largest blocker is **3,788 unwrap() calls in sip-core** (the network boundary layer), where any malformed SIP packet can crash the entire process. This plan prioritizes eliminating that risk, then addresses secondary issues across the stack.

### Current State After Audit & Remediation

| Metric | Value |
|--------|-------|
| Total crates | 17 |
| Total lines of Rust | ~480,000 |
| Tests passing | 2,358 |
| Security fixes applied | 24 Critical/High |
| Code quality fixes applied | 34 Medium |
| Features implemented | 9 (TLS, WSS, DTLS cipher, PUBLISH/PRACK/MESSAGE, NOTIFY send, hold/resume/DTMF) |
| Architecture cleanup | session-core-v2/v3 merged into session-core |

### Remaining Blockers

| Issue | Count | Impact |
|-------|-------|--------|
| sip-core `.unwrap()` | 3,788 | Any malformed SIP packet crashes the process |
| sip-core `panic!()` | 510 | Same — type assertion failures kill the server |
| dialog-core `.unwrap()` | 416 | Dialog/transaction state corruption on bad input |
| rtp-core `.unwrap()` | 383 | Media path crashes on malformed RTP |
| `println!()` across project | 2,800+ | No structured logging, debug noise in production |
| NAT traversal (STUN/TURN/ICE) | 0 lines | Cannot operate across NAT boundaries |

---

## 2. SIP Interoperability Assessment

### What Works Today

| Scenario | Status | Conditions |
|----------|--------|------------|
| rvoip <-> rvoip (UDP, G.711) | Working | Controlled environment |
| REGISTER + Digest Auth | Working | Standard SIP registrar |
| Basic call (INVITE -> 200 -> ACK -> BYE) | Working | Standard flow |
| RTP audio (G.711 mu-law/A-law) | Working | Direct UDP |
| TLS transport | Implemented | Not yet field-tested |
| WSS transport | Implemented | Not yet field-tested |
| DTLS-SRTP | Implemented | Cipher activation complete, not field-tested |

### Risk Matrix for External SIP Integration

| Target | Feasibility | Blocking Issues |
|--------|-------------|-----------------|
| rvoip <-> rvoip (UDP) | Ready | None |
| rvoip <-> SIP softphone (UDP) | High risk | Malformed packets trigger unwrap panics |
| rvoip <-> FreeSWITCH/Asterisk | Medium risk | TLS untested, DTMF only via INFO, no STUN/NAT |
| rvoip <-> Carrier SIP trunk | Not feasible | Requires TLS/SRTP, NAT traversal, registration renewal |
| rvoip <-> WebRTC browser | Not feasible | DTLS-SRTP untested, no ICE/STUN, WSS untested |

---

## 3. Unwrap Elimination Strategy

### 3.1 Why sip-core Is Highest Priority

sip-core is the **network boundary layer** — it directly processes untrusted SIP messages from external sources. Of the 3,788 unwrap calls, many are on parser paths that handle arbitrary network input. A single malformed SIP message triggering any of these unwraps will panic and crash the server process. This is equivalent to a **zero-day DoS vulnerability**.

### 3.2 Layered Risk Model

```
Layer 1: Network Input Path (HIGHEST RISK)
  Direct exposure to untrusted external data
  ├── parser/message.rs      — SIP message parse entry point
  ├── parser/request.rs      — Request-line parsing
  ├── parser/response.rs     — Status-line parsing
  ├── parser/headers/*.rs    — All header parsers
  ├── parser/uri/*.rs        — URI parsing
  └── sdp/parser/*.rs        — SDP parsing

Layer 2: Message Construction/Serialization (MEDIUM RISK)
  Constructs messages from partially-validated internal data
  ├── builder/request.rs     — Request builder
  ├── builder/response.rs    — Response builder
  ├── builder/headers/*.rs   — Header builders
  └── types/sip_request.rs   — Serialization

Layer 3: Type System Internals (LOWER RISK)
  Internal conversions, usually from already-parsed data
  ├── types/headers/*.rs     — Typed header conversions
  ├── types/uri.rs           — URI type methods
  ├── types/address.rs       — Address operations
  └── types/*.rs             — Other type utilities

Layer 4: Auxiliary/Macros (LOWEST RISK)
  Developer-facing APIs, compile-time or controlled inputs
  ├── macros.rs              — Convenience macros
  ├── json/                  — JSON path utilities
  └── other utilities
```

### 3.3 Replacement Rules

All replacements must follow the PRX Rust Production Code Standards:

```rust
// BANNED in production code:
.unwrap()
.expect("...")
panic!("...")

// REQUIRED replacements (in order of preference):
value?                                    // Propagate with ?
value.ok_or(ParseError::MissingField)?    // Convert Option to Result
value.ok_or_else(|| Error::new(...))?     // Lazy error construction
value.unwrap_or_default()                 // Safe fallback (non-critical)
value.unwrap_or(fallback)                 // Explicit fallback
if let Some(v) = value { ... }           // Pattern match
value.map(|v| ...).unwrap_or(default)    // Transform with fallback

// ONLY exception:
LazyLock::new(|| Regex::new(r"...").expect("BUG: invalid hardcoded regex"))
// .expect("BUG: ...") allowed ONLY for compile-time constant values
```

### 3.4 Execution Plan

#### Phase A: Layer 1 — Parser Paths (Critical, ~800 unwraps)

| Batch | Files | Est. unwraps | Parallel agents |
|-------|-------|-------------|-----------------|
| A1 | `parser/message.rs`, `parser/request.rs`, `parser/response.rs` | ~50 | 1 |
| A2 | `parser/headers/` (a-m) | ~100 | 1 |
| A3 | `parser/headers/` (n-z) + `parser/headers/auth/` | ~100 | 1 |
| A4 | `parser/uri/` | ~100 | 1 |
| A5 | `sdp/parser/` | ~150 | 1 |
| A6 | `parser/multipart.rs` + remaining parser files | ~100 | 1 |

**Deliverable**: Zero unwrap/panic in any code path reachable from `parse_message()`.
**Verification**: `cargo test -p rvoip-sip-core --lib` must pass, plus fuzz test with malformed inputs.

#### Phase B: Layer 2 — Builder Paths (~500 unwraps)

| Batch | Files | Parallel agents |
|-------|-------|-----------------|
| B1 | `builder/request.rs`, `builder/response.rs` | 1 |
| B2 | `builder/headers/` | 1 |

**Deliverable**: Message construction never panics on invalid input.

#### Phase C: Layer 3 — Type System (~2,000 unwraps)

| Batch | Files | Parallel agents |
|-------|-------|-----------------|
| C1 | `types/headers/typed_header.rs` (unsafe + unwrap hotspot) | 1 |
| C2 | `types/uri.rs`, `types/address.rs` | 1 |
| C3 | `types/` remaining files (a-m) | 1 |
| C4 | `types/` remaining files (n-z) | 1 |

**Deliverable**: Type conversions return Result/Option instead of panicking.

#### Phase D: Layer 4 — Auxiliary + println Cleanup

| Batch | Scope | Parallel agents |
|-------|-------|-----------------|
| D1 | `macros.rs`, `json/` | 1 |
| D2 | `println!` → `tracing` in call-engine (1,673 instances) | 1 |
| D3 | `println!` → `tracing` in client-core (489) + sip-core (268) | 1 |
| D4 | `println!` → `tracing` in dialog-core (253) + remaining crates | 1 |

**Deliverable**: Zero println in production code; all logging via tracing with structured fields.

---

## 4. Secondary Crate Unwrap Elimination

After sip-core, apply the same treatment to other network-facing crates:

### dialog-core (416 unwraps)

| Priority | Files | Est. count |
|----------|-------|-----------|
| High | `transaction/manager/handlers.rs` | ~80 |
| High | `protocol/*.rs` (request/response handlers) | ~60 |
| Medium | `manager/*.rs` | ~100 |
| Lower | `subscription/`, `events/` | ~176 |

### rtp-core (383 unwraps)

| Priority | Files | Est. count |
|----------|-------|-----------|
| High | `packet/rtp.rs`, `packet/header.rs` (packet parsing) | ~60 |
| High | `srtp/*.rs` (crypto paths) | ~50 |
| High | `dtls/*.rs` | ~80 |
| Medium | `transport/*.rs` | ~50 |
| Lower | `api/`, `security/` | ~143 |

### media-core (321 unwraps)

| Priority | Files | Est. count |
|----------|-------|-----------|
| High | `processing/audio/*.rs` (DSP paths) | ~100 |
| Medium | `rtp_processing/` | ~80 |
| Lower | `relay/`, `codec/` | ~141 |

---

## 5. Feature Completion Roadmap

### Milestone 1: Crash Resistance (Phase A)

**Goal**: rvoip can receive arbitrary SIP packets without crashing.
**Scope**: sip-core Layer 1 unwrap elimination.
**Estimated agents**: 6 parallel.
**Exit criteria**:
- Zero unwrap/panic on parser paths
- `cargo test -p rvoip-sip-core --lib` passes (1,972 tests)
- Manual test: send 1,000 malformed SIP packets, zero crashes

### Milestone 2: Internal Robustness (Phases B-D + secondary crates)

**Goal**: Full stack resilience to bad input at any layer.
**Scope**: All remaining unwrap elimination + println cleanup.
**Estimated agents**: 14 parallel (across batches).
**Exit criteria**:
- Workspace-wide unwrap count < 200 (test code only)
- Zero println in src/ (non-test, non-example)
- `cargo clippy --workspace -- -W clippy::unwrap_used` passes

### Milestone 3: Feature Complete for Basic SIP

**Goal**: Full SIP method support + RFC 2833 DTMF.
**Scope**:
- RFC 2833/4733 DTMF telephone-event in RTP (Task P3-4, already created)
- Basic STUN client for NAT binding discovery
- Field-test TLS against FreeSWITCH
- Field-test SRTP end-to-end

**Exit criteria**:
- Can register with FreeSWITCH via TLS
- Can complete a call with SRTP media
- DTMF works via both INFO and RFC 2833
- Works behind a NAT with STUN

### Milestone 4: Production Validation

**Goal**: Verified for real-world deployment.
**Scope**:
- Load testing (target: 100 concurrent calls)
- Fuzz testing on all parsers (cargo-fuzz)
- Interop testing matrix (FreeSWITCH, Asterisk, Ooh SIP Phone, Ooh WebRTC)
- Security review (SRTP, DTLS, TLS configuration)
- Memory leak testing (long-running sessions)

**Exit criteria**:
- 100 concurrent calls sustained for 1 hour
- Zero crashes under fuzz testing (24 hours)
- Interop with at least 2 commercial SIP systems
- No memory growth under sustained load

---

## 6. Effort Estimation

| Phase | Scope | Agents | Priority |
|-------|-------|--------|----------|
| A (sip-core L1) | 800 unwraps | 6 | P0 — Immediate |
| B (sip-core L2) | 500 unwraps | 2 | P1 — Next |
| C (sip-core L3) | 2,000 unwraps | 4 | P1 — Next |
| D (println cleanup) | 2,800 printlns | 4 | P2 — Soon |
| Secondary crates | 1,120 unwraps | 6 | P2 — Soon |
| RFC 2833 DTMF | New feature | 1 | P2 — Soon |
| STUN client | New feature | 2 | P3 — Later |
| Field testing | Validation | Manual | P3 — Later |
| Fuzz testing | Validation | 1 | P3 — Later |

**Total estimated parallel agents**: ~26
**Phases A-D can be fully automated via subagent dispatch.**

---

## 7. Success Criteria

### Minimum Viable Production (MVP)

- [ ] sip-core parser paths: zero unwrap/panic
- [ ] TLS transport: field-tested with FreeSWITCH
- [ ] SRTP: field-tested end-to-end
- [ ] 100 concurrent calls without crash
- [ ] Structured logging (tracing) throughout

### Full Production Ready

- [ ] All crates: unwrap count < 200 (test-only)
- [ ] All SIP methods: fully implemented end-to-end
- [ ] NAT traversal: STUN working
- [ ] RFC 2833 DTMF: working
- [ ] 24-hour fuzz test: zero crashes
- [ ] Interop matrix: 3+ external SIP systems verified
- [ ] Documentation: accurate README, API docs, deployment guide

---

## 8. Completed Work Reference

### Audit & Remediation (2026-03-20 — 2026-03-21)

**Round 1 — Baseline Security Fixes (14 items)**
1. SRTP key separation (inbound/outbound contexts)
2. SRTP ROC tracking + replay protection + state commit fix
3. DTLS certificate verification (signature + time + fingerprint)
4. DTLS AEAD nonce derivation (epoch+sequence XOR iv)
5. Compilation error fixes (session-core, session-core-v2)
6. SIP/SDP parser limit enforcement (DoS protection)
7. Box::leak / process::exit / panic removal
8. Unsafe Arc mutation replaced with OnceLock/Mutex
9. Async deadlock fix (two-phase locking in session store)
10. API key hashing SHA-256 → Argon2id
11. Dependency version unification (14 Cargo.toml files)
12. Dead code cleanup + facade fix + axum-server upgrade

**Round 2 — Module-Level Deep Fixes (12 items)**
13. User self-privilege escalation + API key IDOR + list_users auth
14. WSS plaintext → reject + SRTP fallback → fail-closed
15. DTLS secret key logging removal (~80 println eliminated)
16. Call-engine non-atomic assignment → SQLite transaction
17. Subscription dead command queue → consumer loop
18. PooledAudioFrame unsafe UB → copy-on-write
19. Dialog transaction map lock-across-await → clone-under-lock
20. DTLS fingerprint fail-open → fail-closed
21. SRTP AEAD placeholder → NotImplemented error
22. CipherSuiteId From<u16> panic → TryFrom

**Round 3 — Medium-Level Hardening (34 items across 6 modules)**
23. sip-core: Request framing, duplicate Content-Length, multipart limits, macro panics
24. dialog-core: Terminated dialog cleanup, INVITE Completed retransmit, >=300 response scope, timer unregister, state checks, panic removal
25. media-core: channels/sample_rate validation, AGC/AEC bounds, i16::MIN overflow, Drop safety, CPAL lock safety
26. session-core: v2 index cleanup, terminated session unregister, register rollback, transfer case match, v3 lock consolidation
27. call-engine: Overflow implementation, priority normalization, concurrent call limit, DB pool config, parking_lot RwLock
28. users-core: Lockout before Argon2, refresh token storage, active check, JwtConfig, proxy trust list

**Architecture — Session Core Merge**
29. Deleted session-core-v3 (zero dependencies, dead code)
30. Merged session-core-v2 into session-core behind `state-table` feature
31. Switched intermediary-core to use session-core
32. Deleted session-core-v2

**Feature Implementation (9 items)**
33. PUBLISH handler in dialog-core
34. PRACK handler in dialog-core
35. MESSAGE inbound dispatch in dialog-core
36. NOTIFY send path wired in session-core
37. Hold/resume/DTMF wired in sip-client
38. TLS transport implemented in sip-transport
39. WSS transport implemented in sip-transport
40. DTLS record-layer cipher activation after CCS
41. README.md rewritten to match actual code state

**Infrastructure**
42. CLAUDE.md updated with PRX Rust Production Code Standards
43. Global memory rules established (workflow + Rust dev rules)
44. 25 artifact files deleted (.bak, .rs-e, orphan duplicates)
