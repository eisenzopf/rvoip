# AUDIT-004: Remediation Report

**Date**: 2026-03-23
**Scope**: Full remediation of AUDIT-003 findings + Codex re-audit verification
**Version**: 0.1.26
**Verified by**: Claude Opus 4.6 + OpenAI Codex (2 rounds)

---

## Summary

82 files changed, 1,329 insertions, 12,509 deletions (net -11,180 lines).
3,248 unit tests passed, 0 failed. 30 integration tests (20 new).

---

## 1. Security Fixes

### 1.1 SRTP Architecture Unification (P0)

**Problem**: Two independent SRTP implementations (session-core SrtpMediaBridge + rtp-core SecurityRtpTransport) never connected. All calls negotiating DTLS-SRTP transmitted in plaintext.

**Fix**: SecurityRtpTransport is now the single SRTP layer:
- `RtpSession::with_transport()` accepts external transport (rtp-core)
- `MediaSessionController::start_media_with_transport()` passes transport to RTP session (media-core)
- `MediaManager::initiate_srtp_for_session()` creates SecurityRtpTransport, performs DTLS, installs keys (session-core)
- `SrtpContext::from_dtls_key_material()` bridges DTLS keys to SecurityRtpTransport (rtp-core)
- Dormant `protect_rtp`/`unprotect_rtp`/`send_rtp_with_srtp`/`receive_rtp_with_srtp` removed

**Data flow**:
```
Before: Audio -> RtpSession -> UdpRtpTransport -> socket.send_to(plaintext)
After:  Audio -> RtpSession -> SecurityRtpTransport -> encrypt -> socket.send_to(ciphertext)
```

### 1.2 DTLS-SRTP Downgrade Prevention (P0)

- `SrtpSecurityDowngrade` error variant added to MediaError
- `srtp_required_sessions` tracks sessions where SRTP was negotiated
- Coordinator UAC/UAS paths check SDP for DTLS params; hard error if SRTP setup fails
- Event handler terminates sessions on SRTP security failure via `SessionError::is_srtp_security_failure()`
- re-INVITE downgrade properly cleans up `srtp_bridges`, `srtp_required_sessions`, and `security_transports`

### 1.3 Previously Fixed (confirmed by Codex)

- SEC-001: DTLS-SRTP plaintext fallback in rtp-core — already fixed
- SEC-002: OAuth TLS bypass — already fixed
- SEC-003: Production panic!() calls — already fixed

---

## 2. Edition & Workspace

| Change | Details |
|--------|---------|
| registrar-core | `edition = "2021"` → `edition.workspace = true` (2024) |
| intermediary-core | `edition = "2021"` → `edition.workspace = true` (2024), `version.workspace = true` |
| auth-core | Added to `[workspace.members]`, `edition.workspace = true`, `version.workspace = true` |
| registrar-core | `version = "0.1.0"` → `version.workspace = true` (0.1.26) |
| **Result** | 19/19 crates on Edition 2024, all versions unified |

---

## 3. Error Handling

### 3.1 Silent Error Swallowing (`let _ =`)

~145 instances of `let _ =` discarding Results replaced with `if let Err(e) = ... { tracing::warn/debug!(...) }`:

| Crate | Fixed | Log Levels |
|-------|-------|------------|
| session-core | 82 | warn: event delivery, state updates. debug: shutdown, channel closed |
| dialog-core | 21 | warn: timeout, transport error. debug: state transitions |
| rtp-core | 20 | warn: errors, handshake. debug: broadcast events |
| media-core | 15 | warn: codec reset, cleanup. debug: packet events |
| client-core | 7 | warn: incoming call, cleanup. debug: state broadcasts |

3 instances intentionally kept: `tracing_subscriber::try_init()`, doc comment, non-Result discard.

### 3.2 G711Codec Infallibility

`G711Codec::new()`/`mu_law()`/`a_law()` changed from `Result<Self>` to `Self` (constructors are infallible). Eliminated `expect("BUG: ...")` and `?` at 7 call sites.

---

## 4. Code Quality

### 4.1 once_cell Migration

13 instances of `once_cell::sync::Lazy`/`OnceCell` migrated to `std::sync::LazyLock`/`OnceLock`. `once_cell` dependency removed from 5 crate Cargo.toml files and workspace root.

### 4.2 Lint Configuration

Workspace lints tightened:
- `dead_code`: `"allow"` → `"warn"`
- `unused_imports`: `"allow"` → `"warn"`
- `unused_variables`: `"allow"` → `"warn"`

Build passes with zero warnings.

### 4.3 TODO Audit

277 markers audited: 3 stale removed, 268 valid kept, 6 in doc comments untouched.

---

## 5. Large File Decomposition

Top 5 files split into 15 sub-modules:

| Original File | Lines | Split Into |
|--------------|-------|-----------|
| client-core/client/media.rs | 3,454 | media/{mod,mute_codec,transmission,session,sdp_stats}.rs |
| dialog-core/transaction/manager/mod.rs | 2,488 | manager/{mod,constructors,operations,creation}.rs |
| session-core/media/manager.rs | 2,256 | manager/{mod,rtp_processing,session_lifecycle,audio_control,srtp_setup}.rs |
| client-core/client/manager.rs | 2,126 | manager.rs + manager/registration_ops.rs |
| sip-core/builder/multipart.rs | 2,067 | multipart/{mod,part_builder,builder,tests}.rs |

All public APIs re-exported; no external API changes.

---

## 6. Testing

### 6.1 auth-core (0 → 41 tests)

Tests cover: AuthError variants, UserContext construction/serialization, TokenType/AuthMethod enum serialization, re-export verification.

### 6.2 Cross-Crate Integration Tests (10 → 30 tests)

4 new test files:
- `sip_call_flow_integration.rs` (5) — SIP message round-trip, INVITE+SDP
- `codec_media_integration.rs` (5) — G.711 PCMU/PCMA encode/decode, SNR verification
- `registrar_dialog_integration.rs` (5) — REGISTER flow, multi-device, unregister
- `security_transport_integration.rs` (5) — SecurityRtpTransport RFC 5764 enforcement

---

## 7. Codex Audit Results

### Round 1 (pre-remediation)
4 findings CONFIRMED, 4 DISPUTED (already fixed), 1 NEW (DTLS-SRTP downgrade in session-core).

### Round 2 (post-remediation)
- SRTP architecture: **PASS**
- Security fixes: **PASS** (with 2 CONCERNs → fixed)
- File splits: **PASS**
- Error handling: **PASS**
- Regressions: **PASS**
- Codex CONCERN fixes applied:
  - re-INVITE downgrade now cleans up `security_transports`
  - SRTP error detection uses typed `is_srtp_security_failure()` instead of string matching

---

## 8. Remaining Items

| Item | Status | Notes |
|------|--------|-------|
| 268 valid TODO markers | Kept | Genuine future work |
| 62 files >1000 lines (after splits) | Reduced from 67 | Further splitting optional |
| SrtpMediaBridge protect/unprotect | Kept | Still needed for DTLS handshake phase |
| DTLS binds separate socket | By design | DTLS requires independent UDP socket |
| 2 example compilation errors | Pre-existing | rtp-core examples, not from this change |

---

## 9. Verification

```
cargo check --workspace          ✅ 0 errors, 0 warnings
cargo test --workspace --lib     ✅ 3,248 passed / 0 failed
cargo test -p rvoip-integration-tests  ✅ 30 passed (20 new)
cargo test -p rvoip-auth-core    ✅ 41 passed (all new)
```
