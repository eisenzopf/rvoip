# Post-Merge Fix Plan

Companion to [test-report.md](test-report.md). Sequenced from "blocks CI" → "real code smells" → "auto-fixable noise" → "lock it in."

## Phase 0 — Set up for cleanup

Override the workspace lint silencing while doing the work, so warnings show up in `cargo` output. Two options:

- **Per-shell (recommended for the cleanup session):**
  ```
  export RUSTFLAGS='--force-warn unused_imports --force-warn unused_variables --force-warn unused_mut --force-warn dead_code --force-warn unused_comparisons --force-warn elided_named_lifetimes --force-warn ambiguous_glob_reexports --force-warn unexpected_cfgs --force-warn unreachable_patterns --force-warn irrefutable_let_patterns --force-warn unused_assignments --force-warn async_fn_in_trait'
  ```
- **Or temporarily edit** `Cargo.toml` `[workspace.lints.rust]` so `cargo fix` can also see them (it doesn't honor `RUSTFLAGS` for what it considers "applicable"). Revert before commit if you don't intend to land it as Phase 6.

---

## Phase 1 — Unblock CI (3 fixes, must land first)

### 1.1 `rvoip-sip` test: `beta_release_docs_exist_and_archived_docs_are_out_of_active_set`
**File:** [crates/rvoip-sip/tests/beta_release_docs.rs:30](crates/rvoip-sip/tests/beta_release_docs.rs:30)
**Symptom:** `missing beta doc PRODUCTION_READINESS_GAP_PLAN.md`
**Fix:** Open the test, look at the expected-docs list and the archived-docs list. The merge either deleted, renamed, or moved `PRODUCTION_READINESS_GAP_PLAN.md`. Run:
```
git log --diff-filter=DR --all -- '*PRODUCTION_READINESS*'
```
to see what happened, then either:
- restore the doc (or its archive entry) at the expected path, or
- update the test's expected-list to reflect the new doc layout.

### 1.2 `rvoip-sip` test: `tls_client_only_config_does_not_require_endpoint_certificates`
**File:** `crates/rvoip-sip/tests/unified_api_tests.rs` (search for the test name)
**Symptom:** panic in `rustls-0.23.40/src/crypto/mod.rs:249` — `Could not automatically determine the process-level CryptoProvider`.
**Fix (preferred):** ensure rustls's crypto provider feature is enabled in the dependency chain. Check `crates/rvoip-sip/Cargo.toml` and parent transport crates for the `rustls` dependency; add `features = ["ring"]` or `["aws-lc-rs"]` to match what the rest of the workspace uses.
**Fix (fallback):** at the top of the test (or in a `#[ctor]`/once block), call:
```rust
rustls::crypto::aws_lc_rs::default_provider()
    .install_default()
    .ok();
```
Use whichever provider matches the rest of the workspace — grep for `install_default` to find the existing pattern.

### 1.3 `rvoip-sip-registrar` example: `registrar_server` startup crash
**File:** `crates/rvoip-sip-registrar/examples/registrar_server.rs`
**Symptom:** `TransportManager has no default transport` when `build_multiplexed_transport` is called.
**Fix:** Diff the example against the pre-merge version (`git log -p crates/rvoip-sip-registrar/examples/registrar_server.rs`). The merge changed `TransportManager`'s API — the example now needs to either:
- register a default UDP transport on the manager before calling `build_multiplexed_transport`, or
- switch to whatever new builder superseded that call path. Check `crates/rvoip-sip-transport/src/manager/mod.rs` for the current API.

**Verify Phase 1:**
```
cargo test -p rvoip-sip --test beta_release_docs
cargo test -p rvoip-sip --test unified_api_tests
cargo run -p rvoip-sip-registrar --example registrar_server  # should start and stay up
```

---

## Phase 2 — Substantive warnings (likely-real code smells)

These warrant a human look. Small list; each one is a half-hour at most.

### 2.1 codec-core: `comparison is useless due to type limits` (14×)
**File:** [crates/codec-core/src/utils/validation.rs:14](crates/codec-core/src/utils/validation.rs:14) and nearby
**Fix:** these are usually `if x >= 0` on an unsigned type. Either drop the redundant check or change the type. Look at all 14; bulk fix or refactor the validator.

### 2.2 codec-core: undeclared `cfg` values (`opus`, `opus-sim`, `g722`)
**Files:**
- [crates/codec-core/src/codecs/mod.rs:75](crates/codec-core/src/codecs/mod.rs:75) — `opus`, `opus-sim`
- [crates/codec-core/src/lib.rs:231](crates/codec-core/src/lib.rs:231) — `g722`
**Fix:** decide if these are real planned features or dead code. If planned, declare them in `crates/codec-core/Cargo.toml` under `[features]`. If dead, delete the `cfg`-gated blocks.

### 2.3 media-core: undeclared `cfg` value `g729` (13×)
**File:** [crates/media-core/src/codec/audio/g729.rs:58](crates/media-core/src/codec/audio/g729.rs:58)
**Fix:** same as 2.2 — add `g729` to `crates/media-core/Cargo.toml` `[features]` or remove.

### 2.4 rtp-core: `ambiguous glob re-exports`
**File:** [crates/rtp-core/src/packet/extension/mod.rs:467](crates/rtp-core/src/packet/extension/mod.rs:467) (5×)
**Fix:** two `pub use foo::*;` statements re-export the same identifier. Pick one as canonical and either remove the duplicate or rename. Real API hazard — fix even if it's "working" today.

### 2.5 rvoip-core: dead orchestrator fields
**File:** [crates/rvoip-core/src/orchestrator.rs:247](crates/rvoip-core/src/orchestrator.rs:247)
**Symptom:** `fields speaking and speak_cancel are never read`
**Fix:** these were probably meant to drive a state-machine branch. Either:
- wire them up (`git blame` to see who added them and why), or
- delete them if the feature was removed during the merge.

### 2.6 rvoip-core: unused assignments in `cross_transport_bridge` example
**File:** [crates/rvoip-core/examples/cross_transport_bridge.rs:722](crates/rvoip-core/examples/cross_transport_bridge.rs:722), :729, :929, :933, :1054, :1109
**Symptom:** `saved_peer_sid`, `wq_bridge_id`, `offerer_stream_handle` assigned but never read
**Fix:** dead bookkeeping from the merge. Either delete or use.

### 2.7 rvoip-quic / rvoip-websocket / rvoip-webtransport: identical dead fields
**Files:**
- [crates/rvoip-quic/src/adapter.rs:184](crates/rvoip-quic/src/adapter.rs:184)
- [crates/rvoip-websocket/src/adapter.rs:136](crates/rvoip-websocket/src/adapter.rs:136)
- [crates/rvoip-webtransport/src/adapter.rs:137](crates/rvoip-webtransport/src/adapter.rs:137)
**Symptom:** all three have `fields by_connection and by_uctp_sid are never read`
**Fix:** Same pattern in 3 adapters — they're tracking maps that nobody reads. This is suspicious: either the lookups got removed during the merge and the bookkeeping is dead, OR the lookups should exist and got dropped. Investigate one adapter, then fix all three the same way.

### 2.8 rvoip-sip-proxy: `field destination is never read`
**File:** [crates/rvoip-sip-proxy/src/proxy.rs:263](crates/rvoip-sip-proxy/src/proxy.rs:263)
**Fix:** check whether this field should be used in routing/logging. If not, delete.

### 2.9 rvoip-stir-shaken: `field typ is never read`
**File:** [crates/rvoip-stir-shaken/src/verifier.rs:154](crates/rvoip-stir-shaken/src/verifier.rs:154)
**Fix:** likely a parsed JWT header field that should validate the token type. If verification should check it, add the check; otherwise stop parsing it.

### 2.10 rvoip-uctp: `field assurance is never read`
**File:** [crates/rvoip-uctp/src/state/coordinator.rs:117](crates/rvoip-uctp/src/state/coordinator.rs:117)
**Fix:** check whether assurance-level was meant to gate something in the coordinator.

**Effort for Phase 2:** ~3-5 hours, one PR per crate or one rolled-up "fix dead fields & cfg values" PR.

---

## Phase 3 — Auto-fixable bulk cleanup (`cargo fix`)

Most of the 3 044 warnings are `unused_imports`, `unused_variables`, `unused_mut`, and `dead_code` for trivially deletable items. `cargo fix` machine-applies a large fraction.

**Important:** `cargo fix` only operates on lints currently active. You must un-silence them first — either via the temporary Cargo.toml edit (Phase 0 option 2) or by passing the lint flags to rustc via `RUSTFLAGS` *and* using `cargo +nightly fix` (stable cargo ignores RUSTFLAGS for fix suggestion eligibility). Simplest: temporarily edit `[workspace.lints.rust]` to set the targeted lint to `warn`.

Per-crate fix order (smallest first to validate the workflow):

```
cargo fix -p rvoip-sip-registrar --lib --tests --allow-dirty
cargo fix -p auth-core --lib --tests --allow-dirty
cargo fix -p rvoip-stir-shaken --lib --tests --allow-dirty
cargo fix -p rvoip-sip-proxy --lib --tests --allow-dirty
cargo fix -p rvoip-uctp --lib --tests --examples --allow-dirty
cargo fix -p rvoip-webrtc --lib --tests --allow-dirty
cargo fix -p rvoip-core --lib --tests --examples --allow-dirty
cargo fix -p codec-core --lib --tests --allow-dirty
cargo fix -p infra-common --lib --tests --examples --allow-dirty
cargo fix -p rvoip-sip-transport --lib --tests --allow-dirty
cargo fix -p rvoip-sip --lib --tests --examples --allow-dirty
cargo fix -p media-core --lib --tests --examples --allow-dirty
cargo fix -p rvoip-sip-dialog --lib --tests --examples --allow-dirty
cargo fix -p rtp-core --lib --tests --examples --allow-dirty
cargo fix -p rvoip-sip-core --lib --tests --allow-dirty   # 262 suggestions reported
```

After each crate: run `cargo test -p <crate> --lib --tests` to make sure nothing regressed (cargo fix occasionally renames a still-used import). Commit per crate so it's easy to bisect.

**Expected reduction:** ~500-1 000 warnings auto-removed, mostly in rvoip-sip-core, rtp-core, sip-dialog, and media-core.

**Effort:** ~2-3 hours mostly waiting for the per-crate builds.

---

## Phase 4 — rvoip-sip-core lifetime cleanup (large but mechanical)

**Symptom:** 377× `hiding a lifetime that's elided elsewhere is confusing` in `crates/rvoip-sip-core/src/parser/`.

These are parser functions where some lifetimes are named and others elided in the same signature. Pattern:
```rust
// before
fn parse(input: &str) -> Result<Foo<'_>> { ... }
// after
fn parse(input: &str) -> Result<Foo<'_>> { ... }  // wait — what changes?
```
Actual change: usually re-name the elided lifetime explicitly so all references match, or use `'_` consistently. Look at one offender first:
```
cargo build -p rvoip-sip-core 2>&1 | grep -B1 -A12 "hiding a lifetime" | head -30
```
to see the suggested fix; many are mechanical and can be done with a targeted find-replace per file (e.g. all of `parser/headers/`).

**Effort:** ~4-8 hours. Probably worth splitting across a few PRs by parser subdirectory (`common`, `headers/*`, `address`, `sdp`).

---

## Phase 5 — rvoip-sip `callback_peer.rs` trait stubs

**Symptom:** ~50 `unused variable` warnings in `crates/rvoip-sip/src/api/callback_peer.rs:1044-1230` — all in default trait-method implementations.

**Fix options:**

1. **Quick:** prefix each parameter with `_` (`handle` → `_handle`, etc.). Mechanical, but loses self-documenting parameter names.
2. **Better:** put `#[allow(unused_variables)]` on the trait's `impl` block (or on individual default methods). Keeps names readable.
3. **Best:** if you intend external implementors to override these, the unused-warning is harmless — go with option 2 at the impl-block level.

**Effort:** ~30 min.

---

## Phase 6 — Lock it in (prevent regression)

Once Phases 1-5 land and the warning count is near zero:

Edit `Cargo.toml` `[workspace.lints.rust]` and switch the cleared lints from `"allow"` back to `"warn"`. Suggested first wave (lowest risk):
```toml
unused_imports = "warn"
unused_variables = "warn"
unused_mut = "warn"
unused_assignments = "warn"
ambiguous_glob_reexports = "warn"
unexpected_cfgs = "warn"
```

Hold off on `dead_code = "warn"` until you've audited the "never used" tables in `codecs/g711/tables.rs` and similar — if those are intentionally precomputed for future use, mark them `#[allow(dead_code)]` individually.

Run `cargo build --workspace --all-targets` to confirm warning count is acceptable. CI will catch any regressions from this point on.

**Effort:** ~30 min plus whatever individual `#[allow]` annotations you decide to add.

---

## Suggested PR layout

| PR | Phase(s) | Why grouped |
|---|---|---|
| 1 | 1.1, 1.2, 1.3 | unblocks CI; small and reviewable |
| 2 | 2.1-2.4 (cfg + comparison + glob) | mechanical, low risk |
| 3 | 2.5-2.10 (dead fields) | investigative, one commit per crate |
| 4 | 3 (cargo fix) | one commit per crate so bisects cleanly |
| 5 | 4 (sip-core lifetimes) | split by parser subdirectory if it grows |
| 6 | 5 (callback_peer) | tiny, can ride with PR 4 |
| 7 | 6 (re-enable lints) | last, after warnings are at zero |

---

## Estimated total effort

- Phase 1: 1-2 hours (3 focused fixes)
- Phase 2: 3-5 hours (10 small investigations)
- Phase 3: 2-3 hours (mostly cargo time)
- Phase 4: 4-8 hours (largest task, parser-wide)
- Phase 5: 0.5 hours
- Phase 6: 0.5 hours

**Total: ~11-19 hours of engineering time** to get from 3 044 warnings + 3 failures to a clean workspace with lints back on.

---

## Verification at the end

```
# All clean — should match what test-report.md showed but with 0 warnings and 0 fails:
cargo build --workspace --all-targets
cargo test --workspace --lib --tests --no-fail-fast
cargo test --workspace --doc
cargo build --workspace --examples
```
