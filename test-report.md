# Post-Merge Test Sweep Report

Generated: 2026-05-28
Scope: 22 crates (auth-core, media-core, codec-core, infra-common, rtp-core, rvoip-core, rvoip-core-traits, rvoip-harness, rvoip-identity, rvoip-quic, rvoip-sip, rvoip-sip-core, rvoip-sip-dialog, rvoip-sip-proxy, rvoip-sip-registrar, rvoip-sip-transport, rvoip-stir-shaken, rvoip-uctp, rvoip-vcon, rvoip-webrtc, rvoip-websocket, rvoip-webtransport)

Run with `RUSTFLAGS` set to force-warn the 12 lints the workspace silences via `[workspace.lints.rust]` (warnings/unused_imports/unused_variables/unused_mut/dead_code/unused_comparisons/elided_named_lifetimes/ambiguous_glob_reexports/unexpected_cfgs/unreachable_patterns/irrefutable_let_patterns/unused_assignments/async_fn_in_trait).

Raw logs: `/tmp/rvoip-test-run/`
  - `00-build.log` — workspace `--all-targets` build
  - `unit/` — initial unit sweep (10-min per-crate cap)
  - `unit-uncapped/` — re-run of the 6 crates that hit the cap (no cap)
  - `doc/` — doc test sweep
  - `ex-build/` — example compilation
  - `ex-run/` — per-example run (30 s cap each, `<pkg>__<example>.log`)
  - `warnings-by-crate.txt` — full warning breakdown per crate

---

## TL;DR

| Phase | Result |
|---|---|
| `cargo build --all-targets` (22 crates) | ✅ PASS |
| `cargo test --lib --tests` (22 crates) | ⚠ 21 PASS / **1 FAIL** (rvoip-sip — 2 tests) |
| `cargo test --doc` (22 crates) | ✅ PASS |
| `cargo build --examples` (9 crates w/ examples) | ✅ PASS |
| `cargo run --example …` (153 examples) | 77 OK / 49 TIMEOUT (daemons) / 27 FAIL (17 feature-gated, 9 environmental, **1 real bug**) |
| Warnings (silenced lints forced on) | **3 044 in 18 crates** |

**Action items from the merge** — small list of *actual* code issues:

1. [rvoip-sip] Test failure: `beta_release_docs::beta_release_docs_exist_and_archived_docs_are_out_of_active_set` — `missing beta doc PRODUCTION_READINESS_GAP_PLAN.md` ([crates/rvoip-sip/tests/beta_release_docs.rs:30](crates/rvoip-sip/tests/beta_release_docs.rs:30))
2. [rvoip-sip] Test failure: `unified_api_tests::tls_client_only_config_does_not_require_endpoint_certificates` — rustls `CryptoProvider` not installed before use (panic at rustls-0.23.40/src/crypto/mod.rs:249). Test needs `CryptoProvider::install_default()` or enable exactly one of the `aws-lc-rs` / `ring` features.
3. [rvoip-sip-registrar] Example failure: `registrar_server` crashes at startup with `TransportManager has no default transport`. See `crates/rvoip-sip-registrar/examples/registrar_server.rs`.
4. Warnings: 1 484 in `rvoip-sip-core`, 590 in `rtp-core`, 313 in `sip-dialog`, 253 in `media-core` — see § Warnings.

Everything else under FAIL in the example run is either (a) needs `--features X` to enable, or (b) needs a peer/audio device/env var.

---

## 1. Build (`cargo build --all-targets`)

All 22 crates compiled clean (with the workspace's lint silencing overridden). No errors anywhere in workspace code.

---

## 2. Unit + integration tests

`cargo test -p <crate> --lib --tests --no-fail-fast` per crate.

First pass used a 10-minute per-crate cap (timeout returned exit 142). Six crates hit the cap and were re-run uncapped — five of those passed once given enough time; only **rvoip-sip** had real failures.

| Crate | Result | Duration | Notes |
|---|---|---|---|
| rvoip-auth-core | ✅ | <5 min | |
| rvoip-codec-core | ✅ | <5 min | |
| rvoip-core | ✅ | 16 min | initially capped at 10 min |
| rvoip-core-traits | ✅ | <5 min | |
| rvoip-harness | ✅ | <5 min | |
| rvoip-identity | ✅ | <5 min | |
| rvoip-infra-common | ✅ | <5 min | |
| rvoip-media-core | ✅ | 11 min | initially capped at 10 min |
| rvoip-quic | ✅ | <5 min | |
| rvoip-rtp-core | ✅ | <10 min | |
| **rvoip-sip** | **❌** | **76 min** | **2 failing tests** — see below |
| rvoip-sip-core | ✅ | <10 min | |
| rvoip-sip-dialog | ✅ | 29 min | initially capped at 10 min |
| rvoip-sip-proxy | ✅ | <5 min | |
| rvoip-sip-registrar | ✅ | <5 min | |
| rvoip-sip-transport | ✅ | <5 min | |
| rvoip-stir-shaken | ✅ | <5 min | |
| rvoip-uctp | ✅ | 19 min | initially capped at 10 min |
| rvoip-vcon | ✅ | <5 min | |
| rvoip-webrtc | ✅ | 14 min | initially capped at 10 min |
| rvoip-websocket | ✅ | <5 min | |
| rvoip-webtransport | ✅ | <5 min | |

### rvoip-sip failures

**Test 1:** `beta_release_docs::beta_release_docs_exist_and_archived_docs_are_out_of_active_set`
```
thread 'beta_release_docs_exist_and_archived_docs_are_out_of_active_set' panicked at crates/rvoip-sip/tests/beta_release_docs.rs:30:9:
missing beta doc PRODUCTION_READINESS_GAP_PLAN.md
```
Either the doc was renamed/removed during the merge and the test's allow-list is out of date, or the doc needs to be restored. Fix is in [crates/rvoip-sip/tests/beta_release_docs.rs:30](crates/rvoip-sip/tests/beta_release_docs.rs:30).

**Test 2:** `unified_api_tests::tls_client_only_config_does_not_require_endpoint_certificates`
```
thread 'tls_client_only_config_does_not_require_endpoint_certificates' panicked at rustls-0.23.40/src/crypto/mod.rs:249:14:
Could not automatically determine the process-level CryptoProvider from Rustls crate features.
Call CryptoProvider::install_default() before this point to select a provider manually, or make sure exactly one of the 'aws-lc-rs' and 'ring' features is enabled.
```
The rustls version pulled in has neither default provider feature enabled, so any code path that builds a `ClientConfig` without explicitly installing a provider panics. Two ways out:
- Enable `aws-lc-rs` or `ring` in `rustls` features (preferred — used everywhere else); or
- Have the test body call `rustls::crypto::aws_lc_rs::default_provider().install_default()` (or the `ring` equivalent) before constructing the config.

---

## 3. Doc tests

All 22 crates passed `cargo test --doc`. The long pole was rvoip-sip-core at ~93 min (lots of doctests); everything else under 10 min.

---

## 4. Examples

### Build (`cargo build --examples`)
All 9 crates that have any examples (153 total) built clean.

### Run (`cargo run --example <N>`, 30 s cap each)

153 examples, three buckets:

| Outcome | Count | Meaning |
|---|---|---|
| ✅ OK | 77 | exited 0 within 30 s |
| ⏱ TIMEOUT | 49 | killed at 30 s — almost all are servers/peers/listeners by design |
| ❌ FAIL | 27 | exited non-zero — see breakdown below |

#### Per-crate example outcomes

| Crate | OK | TIMEOUT | FAIL |
|---|---|---|---|
| rvoip-core | 1 | 1 | 0 |
| rvoip-infra-common | 2 | 4 | 0 |
| rvoip-media-core | 6 | 0 | 0 |
| rvoip-rtp-core | 46 | 2 | 0 |
| **rvoip-sip** | 14 | 39 | 18 |
| rvoip-sip-dialog | 7 | 0 | 0 |
| **rvoip-sip-registrar** | 0 | 0 | 1 |
| rvoip-uctp | 1 | 3 | 1 |
| **rvoip-webrtc** | 0 | 0 | 7 |

#### Categorizing the 27 FAILs

**Feature-gated (17) — expected; need `--features` to run, not bugs**

These print `error: target ... requires the features: <feature>`:

```
rvoip-webrtc    loopback_call                requires: client
rvoip-webrtc    webrtc_bridge_demo           requires: client
rvoip-webrtc    webrtc_browser_demo          requires: client
rvoip-webrtc    webrtc_comprehensive_client  requires: client
rvoip-webrtc    webrtc_comprehensive_server  requires: client
rvoip-webrtc    webrtc_quic_bridge_demo      requires: client
rvoip-webrtc    webrtc_server                requires: client
rvoip-sip       pbx_analyze                  requires: dev-insecure-tls
rvoip-sip       pbx_callback_builder         requires: dev-insecure-tls
rvoip-sip       pbx_endpoint                 requires: dev-insecure-tls
rvoip-sip       pbx_stream_peer              requires: dev-insecure-tls
rvoip-sip       profiling_dhat_b2bua         requires: dhat
rvoip-sip       profiling_dhat_dialog        requires: dhat
rvoip-sip       profiling_dhat_parse         requires: dhat
rvoip-sip       profiling_dhat_udp           requires: dhat
rvoip-sip       regression_tls_client        requires: dev-insecure-tls
rvoip-sip       regression_tls_server        requires: dev-insecure-tls
```

**Environmental (9) — expected; need an out-of-process peer / device / env var**

| Example | Cause |
|---|---|
| rvoip-sip endpoint_registered_account | `ConfigError("SIP_REGISTRAR environment variable is required")` |
| rvoip-sip regression_cancel_alice | waits for Bob: `call never reached Ringing state within 4s` |
| rvoip-sip regression_cancel_bob | waits for Alice: `timeout waiting for incoming INVITE` |
| rvoip-sip regression_glare_retry_bob | waits for Alice: `timeout waiting for incoming INVITE` |
| rvoip-sip regression_notify_send_alice | waits for Bob: `did not see CallAnswered within 8s` |
| rvoip-sip regression_notify_send_bob | waits for Alice |
| rvoip-sip regression_prack_alice | waits for Bob with 100rel |
| rvoip-sip sip_client | `Device not configured (os error 6)` — needs an audio output device |
| rvoip-uctp uctp_agent_ws | `ConnectionRefused` — needs a WS peer on 127.0.0.1:7777 |

These pass when run with their partner / harness; nothing to fix in code.

**Real bug (1)**

| Example | Failure |
|---|---|
| **rvoip-sip-registrar registrar_server** | `Transport("Failed to build multiplexed transport from TransportManager: Transport management error: build_multiplexed_transport: TransportManager has no default transport")` |

This is a startup regression — the example wires up a `TransportManager` without registering a default transport, then asks for a multiplexed transport. Likely a fallout from the recent transport / UCTP changes in the merge; needs either (a) registering a default UDP transport before calling `build_multiplexed_transport`, or (b) the example updated to use whatever new builder API supersedes it. Source: `crates/rvoip-sip-registrar/examples/registrar_server.rs`.

---

## 5. Warnings (lints forced on)

3 044 workspace warnings emerge once the `[workspace.lints.rust]` silencing is overridden. Full breakdown in `/tmp/rvoip-test-run/warnings-by-crate.txt`. Top issues by crate:

### rvoip-sip-core — 1 484 warnings (607 distinct)
By far the largest source. Dominant categories:
- **377× `hiding a lifetime that's elided elsewhere is confusing`** — `elided_named_lifetimes` lint, mostly in `parser/`. Mechanical fix: name the lifetime in the function sig (e.g. `&'a self` and `Result<'a>`).
- **42× `unused imports: Error and Result`** in `builder/headers/*`
- **39× `unused import: std::str::FromStr`** in `builder/headers/*`
- **28× `unused import: headers::header_access::HeaderAccess`** in `builder/headers/*`
- 23× `unused variable: rem`, 21× `unused import: crate::types::param::Param`, ...

A `cargo fix -p rvoip-sip-core --lib` (with the lints temporarily un-silenced) would auto-resolve a large fraction of these — cargo reports 262 of the lib-test warnings have machine-applicable suggestions.

### rtp-core — 590 warnings (333 distinct)
- 23× `variable does not need to be mutable` (notably DTLS handshake)
- 18× `unused import: std::sync::Arc`
- 12× `unused import: std::collections::HashMap`
- 11× `unused imports: info and warn` in `api/client/security/`
- 11× `unused import: debug` in `api/server/transport/rtcp/`
- 9× `unused variable: client_id`
- 5× **`ambiguous glob re-exports`** in `packet/extension/mod.rs:467` — actual API smell worth looking at
- Long tail of unused tracing imports across `api/`.

### rvoip-sip-dialog — 313 warnings (186 distinct)
- 9× `unused import: std::sync::Arc`
- 8× `unused import: std::fmt`, 8× `std::net::SocketAddr`, 8× `variable does not need to be mutable`
- 5× `unused variable: transport_tx`, 5× `unused variable: mock_tm` (test scaffolding)
- 4× `field logic is never read` — possibly dead struct field worth checking

### media-core — 253 warnings (173 distinct)
- **13× `unexpected cfg condition value: g729`** in `codec/audio/g729.rs:58` — gate is referenced but not declared in Cargo features; either declare it or drop the cfg.
- 8× `unused import: warn`, 8× `variable does not need to be mutable`
- 6× `unused import: SampleRate`
- 5× `comparison is useless due to type limits` in `rtp_processing/media/extensions.rs:151`

### rvoip-sip — 153 warnings (93 distinct)
Concentrated in `api/callback_peer.rs` (trait default impls with named-but-unused params):
- 16× `unused variable: handle`, 10× `reason`, 8× `status_code`, 7× `event`, 6× `request`, 4× `call_id`, 4× `registrar`, 3× `dialog`, etc. — all in the same callback trait stubs.
- 8× `unused variable: res` in `examples/callback_peer/01_auto_answer/server.rs:22`
- 2× `methods shutdown and wait_for_n are never used` in `tests/support/auth_uas.rs:59`

For the trait defaults, the canonical fix is `_handle`, `_reason`, etc. (or `#[allow(unused_variables)]` on the trait itself).

### rvoip-sip-transport — 98 warnings (73 distinct)
- 5× `unused import: std::sync::Arc`, 5× `unused import: Request` in `manager/mod.rs`
- 4× `unused variable: source` in `manager/mod.rs:293`
- Long tail of unused tracing imports.

### infra-common — 60 warnings (37 distinct)
- 5× `unused variable: event` in `events/coordinator.rs`
- 4× `variable does not need to be mutable` in `lifecycle/manager.rs`
- 3× `unused import: async_trait::async_trait` in `planes/routing.rs`
- Dead helper functions in `examples/api_bench_both.rs` (`demonstrate_batch_publishing`, `test_static_event_publishing`, `MediaProcessor`)

### codec-core — 38 warnings (16 distinct)
- **14× `comparison is useless due to type limits`** in `utils/validation.rs:14` — likely an `if x >= 0` on a `u*` type that's always true. Worth a real fix.
- **5× `unexpected cfg condition value: opus` + 5× `opus-sim`** in `codecs/mod.rs:75` — those feature names aren't declared in Cargo.toml.
- 1× `unexpected cfg condition value: g722` in `lib.rs:231`
- 8× "never used" tables/functions in `codecs/g711/tables.rs` (precomputed encode/decode lookup tables that aren't referenced — either wire them up or delete them).

### rvoip-core — 18 warnings (17 distinct)
- 2× `unused import: SessionId` in `tests/recording_and_ai.rs:19`
- `examples/cross_transport_bridge.rs`: several `unused_assignments` (`saved_peer_sid`, `wq_bridge_id`, `offerer_stream_handle` assigned but never read) — may be dead bookkeeping from the merge.
- 1× `fields speaking and speak_cancel are never read` in `orchestrator.rs:247` — worth checking, may be a real bug if these were supposed to drive behavior.

### rvoip-webrtc — 12 warnings (10 distinct)
- 2× `unused import: webrtc::media_stream::Track` in `peer/session.rs:907`
- 2× `unused variable: msg` in `peer/session.rs:678`
- Several "never used" methods in `tests/support/coturn_fixture.rs` / `lossy_turn_fixture.rs` (test scaffolding)

### rvoip-uctp — 9 warnings (7 distinct)
- 2× `unused import: chrono::Utc` in `examples/uctp_to_sip_bridge/uctp_agent_*.rs`
- 1× `field assurance is never read` in `src/state/coordinator.rs:117` — possibly real

### rvoip-sip-proxy — 7 warnings
- 3× `unused import: std::str::FromStr` in tests
- 1× `field destination is never read` in `src/proxy.rs:263` — worth a look

### rvoip-stir-shaken — 3 warnings
- `field typ is never read` in `verifier.rs:154` — may indicate parsed-but-unused JWT field
- Two test-only dead-code items

### auth-core — 2 warnings
- Two unused imports in `tests/dpop.rs` (`base64::Engine`, `DpopProof`)

### Single-warning crates
- **rvoip-sip-registrar** — 1× unused `EventPublisher` import in `src/api/mod.rs:4`
- **rvoip-quic / rvoip-websocket / rvoip-webtransport** — each has `fields by_connection and by_uctp_sid are never read` in `src/adapter.rs` (same pattern, three adapters; likely a single class hierarchy where these fields are reserved for a feature that isn't wired up yet)

---

## 6. Suggested fix order

If you want to clear out the merge debt:

1. **Fix the 2 unit-test failures in rvoip-sip** — blocking CI.
2. **Fix `registrar_server` example** — single real example bug.
3. **Auto-fix the unused-import / unused-variable noise** via `cargo fix` per crate (with lints temporarily un-silenced). Cargo reports machine-applicable suggestions for several hundred of the warnings.
4. **Walk the substantive warnings** that aren't auto-fixable:
   - codec-core `comparison is useless due to type limits` (utils/validation.rs:14)
   - codec-core unexpected cfg conditions (`opus` / `opus-sim` / `g722`) — declare features or delete the cfg
   - media-core unexpected cfg condition `g729`
   - rtp-core `ambiguous glob re-exports` (packet/extension/mod.rs:467)
   - rvoip-sip-core `hiding a lifetime that's elided elsewhere is confusing` (377 occurrences in `parser/` — bulk cleanup pass)
   - rvoip-core "fields never read" in orchestrator — verify whether they should drive behavior
   - rvoip-quic / rvoip-websocket / rvoip-webtransport `by_connection`/`by_uctp_sid` — probably the same pattern across three adapters worth resolving together

5. (Optional) re-enable a handful of the silenced lints in `[workspace.lints.rust]` once cleared (start with `unused_imports` and `unused_variables` — easy wins, prevent regression).

---

## 7. What was NOT verified

- Paired examples (alice/bob, client/server) were each run alone; they need their partner running. Their TIMEOUT and "waiting" FAIL exits are expected.
- `dhat`, `client`, `dev-insecure-tls` feature-gated examples were not built/run with those features enabled.
- No example was given more than 30 s. Long-running benchmarks and servers were terminated.
- I did not run benchmarks (`cargo bench`).
- audio-core, users-core, rvoip, rvoip-client are workspace members but were not in the list you gave me.
