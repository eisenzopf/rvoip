# rvoip Production-Hardening Roadmap

**Status:** P0 + P1 implemented (2026-07-01); P2/P3 open. Target: open-internet / hostile-network exposure for `rtp-core`.
**Scope:** `rvoip-sip` (beta-ready within its documented envelope) and `rtp-core` (media transport). Both beta-tier (0.2.x).

## Implementation status (2026-07-01)

**Done — P0 (parser panic-safety + fuzzing):**
- `crates/media/fuzz/` stood up mirroring `crates/sip/fuzz` — 6 targets: `rtp_packet`, `rtcp_packet`,
  `srtp_unprotect`, `dtls_record`, `stun_response`, `g711_unpack`. All build; 5 clean at 500k runs.
- **Fuzzing found a real remote-reachable panic** in `RtcpPacket::parse`: the XR StatisticsSummary /
  VoipMetrics block parsers under-guarded their mandatory fixed fields (16 vs 18, 24 vs 32 bytes),
  so a truncated XR block over-read and panicked (`bytes` `panic_advance`). Fixed in
  `packet/rtcp/xr.rs`; the exact crash input is a permanent regression test.
- SRTP auth-tag slice panic guarded (`srtp/auth.rs`, `srtp/crypto.rs`) + `SrtpCryptoSuite::validate()`
  at context construction (`srtp/mod.rs`).
- `crates/media/rtp-core/tests/malformed_input.rs` — 7 malformed/truncated-input regression tests
  (all green; 233 existing rtp-core lib tests still green).

**Done — P1 (CI enforcement):**
- `.github/workflows/workspace-test-clippy.yml` — `cargo test` (service-free core) + `cargo clippy`.
  Fixed a latent lint-group-priority misconfig in the root `Cargo.toml` that made `cargo clippy` error.
  Clippy is not `-D warnings` yet (~500 pre-existing pedantic/style warnings — see P3).
- `deny.toml` + `.github/workflows/cargo-deny.yml` — advisories now enforced in CI
  (`check advisories bans sources`; all three green), with the beta-gate accepted-advisory ignores
  mirrored and unmaintained treated as informational (matching `cargo audit`). Fixed the pre-existing
  `bans` failure by grandfathering `rvoip-amazon-connect` into the wrappers allow-list (a media adapter
  that drives the orchestrator impl directly, like `rvoip-webrtc`).
- **The new advisories gate immediately paid off:** it caught two live quick-xml DoS CVEs
  (`RUSTSEC-2026-0194` / `-0195` — quadratic attribute scan + unbounded namespace allocation)
  reachable via PIDF presence bodies parsed in `sip-registrar`. Remediated by bumping quick-xml
  0.36 -> 0.41 (`presence/pidf.rs` migration: `BytesText::unescape` -> `decode()` + `escape::unescape`).
- `beta_gate.sh` `run_fuzz_smoke_gates()` now runs the 6 media fuzz targets too.

**Open — P2/P3:** DTLS handshake fragmentation (P2.8, large), SRTP/DTLS-SRTP security review (P2.9,
external), the ~500-warning clippy cleanup to reach `-D warnings` (part of P1.5 follow-up), the
`unimplemented!()`/`todo!` hygiene (P3.10), CHANGELOGs (P3.11), and the rvoip-sip feature tracks (P3.12).

## Summary

An external production-readiness audit rated `rvoip-sip` 🟡 (ready within scope) and `rtp-core` 🟠
(conditional — SDES-SRTP only, panic-on-malformed-input flagged as the #1 blocker). A follow-up code
review verified the audit and found it **directionally correct on the process gaps but overstating the
panic/DoS severity**. This roadmap prioritizes the work to make `rtp-core` safe for unbounded traffic
and to close the workspace CI/process gaps.

## Corrected findings (review of the audit)

### Real, confirmed gaps
- **No `rtp-core` fuzz harness.** `crates/sip/fuzz/` has 4 targets (`sip_message`, `header`, `sdp`,
  `uri`); `crates/media/rtp-core` has none.
- **No CI `cargo test` / clippy gate.** CI runs only `fmt` / `cargo-deny check bans sources` / example
  builds / nightly browser-interop. Workspace testing depends on the local `beta_gate.sh`.
- **cargo-deny CI omits `advisories`.** `[advisories]` *is* configured in `deny.toml`, but the CI job
  runs `check bans sources`; RUSTSEC scanning currently only happens in `beta_gate`'s `cargo audit`.
- **DTLS handshake fragmentation is incomplete.** `dtls/connection.rs` hardcodes `fragment_offset=0` /
  `fragment_length=<full>` on send and has no inbound reassembly, so large-certificate handshakes fail.
  (DTLS-SRTP is currently post-beta.)

### Overstated: the "460 unwraps → malformed packet → remote panic/DoS" framing
- The RTP/RTCP/extension parsers are **bounds-checked (guard-before-index) and `Result`-returning** —
  verified in `packet/header.rs`, `packet/rtp.rs`, `packet/extension/mod.rs`, `packet/rtcp/mod.rs`.
  No reachable panic from malformed RTP/RTCP was found.
- Of ~460 `unwrap()`: ~130 are test code, ~120 are infallible (`mutex.lock()`, `SystemTime`), ~45 are
  config/init-time; only **~2 are reachable production panics.**
- The one concrete reachable panic — the SRTP auth-tag slice `result[..tag_length]` in `srtp/auth.rs:61`
  and `srtp/crypto.rs:687` — requires `tag_length > 20`, but every standard suite uses 4 or 10 bytes
  (GCM's 16 goes through a separate AEAD path). It is a **misconfiguration-gated robustness bug, not an
  attacker-controlled wire DoS.**
- **Not yet deep-audited:** ~163 `unwrap()` in payload codecs (`payload/{opus,g711,g722,vp8,vp9}.rs`),
  DTLS, and STUN parsing. Fuzzing is the correct tool to find any real panics here — a blanket unwrap
  purge is not warranted.

### Minor corrections to the audit's numbers
`rvoip-sip` has **0** `unsafe` blocks (the "1" was a comment); **3** `unimplemented!()` exist in
`rvoip-sip/src/api/types.rs` (audit said 0); `unwrap()` count is 139 (not 135). `rtp-core`'s 9 `unsafe`
blocks are all `libc::setsockopt` in `transport/validation.rs` (confirmed).

## Roadmap (prioritized)

### P0 — Parser panic-safety + fuzzing
1. **Stand up `crates/media/fuzz/`** mirroring `crates/sip/fuzz/` (libfuzzer-sys; `[package.metadata]
   cargo-fuzz = true`; dep `rvoip-rtp-core`). Targets on the existing `Result`-returning entry points:
   `rtp_packet` → `RtpPacket::parse` (`packet/rtp.rs:46`); `rtcp_packet` → `RtcpPacket::parse`
   (`packet/rtcp/mod.rs:105`); `srtp_unprotect` → `SrtpContext::unprotect`/`unprotect_rtcp`
   (`srtp/mod.rs:252`); plus payload (`opus`/`g711`/`vp8`/`vp9`), DTLS record/handshake, and STUN parse
   (the un-audited ~163). Template: `crates/sip/fuzz/fuzz_targets/sip_message.rs`.
2. **Run the fuzzers with real budgets** (minutes–hours per target) to shake out panics across the
   payload/DTLS/STUN surface, then **fix each reachable panic** — convert to `Result` / guarded `.get()`
   using the existing `Error` enum (`BufferTooSmall` / `InvalidPacket` / `ParseError`, `src/error.rs`).
   Each crash artifact becomes a regression test.
3. **Fix the SRTP auth-tag panic** (`srtp/auth.rs:61`, `srtp/crypto.rs:687`): bounds-check before the
   slice (return `Error::SrtpError` when `tag_length > result.len()`), and validate `tag_length` at
   `SrtpCryptoSuite` construction (`srtp/mod.rs`).
4. **Add malformed/truncated-input unit tests** as regression guards: truncated RTP (0–11 bytes),
   over-claimed CSRC count, bad extension length fields, bad RTCP report counts, oversized SRTP tag.

### P1 — CI enforcement
5. **New `.github/workflows/workspace-test-clippy.yml`** (mirror `examples.yml`'s
   `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2`): `cargo test --workspace --all-features`
   plus `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Low-lift — the root
   `[workspace.lints.clippy]` is already permissive (`correctness`/`suspicious` = warn, rest = allow).
6. **Extend `cargo-deny.yml`**: `check bans sources` → `check advisories bans sources`, mirroring the
   accepted-advisory ignores already used by `beta_gate` (the transitive `quinn-proto`/`rustls-webpki` set).
7. **Wire the new rtp-core fuzz targets into `beta_gate.sh`** (`run_fuzz_smoke_gates()`, ~L978; a second
   `BETA_FUZZ_CRATE_DIR` for `crates/media/fuzz`), and add a nightly longer-budget fuzz CI job.

### P2 — DTLS-SRTP + crypto review (gate on offering DTLS-SRTP over open internet)
8. **Implement DTLS handshake fragmentation** in `dtls/connection.rs`: outbound split of messages > MTU
   with correct `fragment_offset`/`fragment_length`, and an inbound reassembly buffer keyed by
   `message_seq`. Fixes large-certificate handshakes.
9. **Formal SRTP + DTLS-SRTP security review** before promoting key exchange beyond the tested SDES path
   (DTLS-SRTP / MIKEY / ZRTP are implemented but unaudited / post-beta).

### P3 — Hygiene & scoped gaps
10. Resolve the 3 `unimplemented!()` in `rvoip-sip/src/api/types.rs` (return a typed error, not a panic)
    and the `rtp-core` `todo!` markers.
11. Add `CHANGELOG.md` to the published beta crates (currently only `sip-dialog` and `rvoip-webrtc` have one).
12. Optional documented `rvoip-sip` feature gaps: attended transfer, carrier-SBC / Kamailio / OpenSIPS
    validation matrix, DTLS-SRTP / ICE / WebRTC track.

## Reuse map (existing infrastructure to build on)
- **Fuzz template:** `crates/sip/fuzz/Cargo.toml` + `fuzz_targets/sip_message.rs` (and `sdp.rs` for
  structured-input mutation patterns).
- **Fuzz gate:** `beta_gate.sh` — `run_fuzz_smoke_gates()` / `run_fuzz_smoke_target` / `BETA_FUZZ_*` knobs.
- **Error type:** `crates/media/rtp-core/src/error.rs` (`Error` enum + `Result`).
- **CI toolchain pattern:** `.github/workflows/examples.yml`; deny config in `deny.toml` + `cargo-deny.yml`.
- **Clippy config:** root `Cargo.toml` `[workspace.lints.clippy]`.

## Verification
- New fuzz targets build and run clean for a defined budget (e.g. `-runs=1_000_000` / `-max_total_time`)
  with no crashes; any crash artifact is committed as a regression test.
- `cargo test --workspace --all-features` green; `cargo clippy … -- -D warnings` clean.
- `beta_gate.sh --full` (including the new fuzz targets) passes; malformed-input unit tests pass.
- `cargo deny check advisories bans sources` passes (with the documented ignores).

## Effort & sequencing
P0 first (the fuzz harness is ~1 day to stand up; fixing what it finds is the variable cost) → P1
alongside (~1 day) → P2 (weeks; gated on the DTLS-SRTP decision) → P3 ongoing. P0 + P1 are the concrete,
high-value core; P2 is the real open-internet lift; P3 is polish.
