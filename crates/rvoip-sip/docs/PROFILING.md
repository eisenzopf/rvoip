# Profiling & performance benchmarks

A developer runbook for measuring the SIP stack under load. Pairs three
tools — **Criterion** (throughput / latency numbers), **samply** (CPU
flamegraphs), and **dhat** (heap profiling) — with four canonical
scenarios:

| Scenario | What it stresses |
|---|---|
| Parser micro-bench | `parse_message` + `Message::to_bytes` in isolation |
| Call setup throughput | Full INVITE → 200 → ACK → BYE between two peers |
| Registration storm | N clients REGISTER → 200 against one registrar |
| Concurrent dialog count | Steady-state perf at N held-open dialogs |

All benches and profiling examples are committed; this doc just tells
you how to run them and how to read the output.

---

## Prereqs

```bash
# CPU profiling (macOS-friendly, no sudo).
cargo install samply

# Linux fallback (uses perf; needs sudo / kernel.perf_event_paranoid).
cargo install flamegraph

# Optional: tokio-console for async runtime introspection on long-running
# scenarios (registration_storm, dialog_steady_state).
cargo install --locked tokio-console
```

Heap profiles produced by dhat (`dhat-heap.json`) are opened in a
browser at <https://nnethercote.github.io/dh_view/dh_view.html>.

Helpful — but optional — env vars:

```bash
# Keep frame pointers so samply / perf can walk stacks cleanly.
export RUSTFLAGS="-C force-frame-pointers=yes"
```

The workspace root `Cargo.toml` already defines a `[profile.flamegraph]`
inheriting from release with `debug = true` and `strip = false`, so
symbols are intact in `target/flamegraph/`.

---

## Quick start

```bash
# 1. Criterion micro-benches (fast — sip-core only)
cargo bench -p rvoip-sip-core

# 2. Save a workspace-wide baseline on the current branch
cargo bench --workspace -- --save-baseline main

# 3. CPU profile of a real workload
cargo build --profile flamegraph -p rvoip-sip --example profiling_call_setup_loop
samply record target/flamegraph/examples/profiling_call_setup_loop

# 4. Heap profile of the parser
cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_parse
# → open dhat-heap.json at https://nnethercote.github.io/dh_view/dh_view.html
```

---

## Per-scenario recipes

### 1. Parser micro-bench

The parser owns ~100% of the inbound CPU path. Start here when a fix or
refactor touches anything in `crates/rvoip-sip-core/src/parser/`.

| Tool | Command | What to look for |
|---|---|---|
| Criterion | `cargo bench -p rvoip-sip-core` | Throughput in MiB/s per fixture; regression > 5% on any `core_parse_message/*` line is a red flag. |
| samply | `cargo build --profile flamegraph -p rvoip-sip --example profiling_parser_loop` then `samply record target/flamegraph/examples/profiling_parser_loop` | `parse_message` should dominate. Wide stacks under `header_value_better` or `nom::sequence::tuple` are the byte-by-byte scan / combinator overhead. |
| dhat | `cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_parse` | Target ≤ 2 allocations per `parse + to_bytes` cycle. The current `Vec<Header>` + per-header `format!()` path is well above that — this is where zero-copy / `bytes::Bytes` slicing would pay off. |

Run duration of the long-running loops is controlled via
`RVOIP_PROFILE_DURATION=<secs>` (default 30; `inf` means run forever).

### 2. Call setup throughput

End-to-end measure of `INVITE → 100 → 180 → 200 → ACK → BYE → 200` cost
between two `StreamPeer`s on loopback.

| Tool | Command | What to look for |
|---|---|---|
| Criterion | `cargo bench -p rvoip-sip --bench call_setup` | `e2e_call_setup/sequential` and `e2e_call_setup/concurrent/<N>` lines. Target ≥ 1k calls/sec/core on `concurrent/16` — substantially below means the transaction-manager `Arc<Mutex<HashMap>>` is the bottleneck. |
| samply | `cargo build --profile flamegraph -p rvoip-sip --example profiling_call_setup_loop` then `samply record target/flamegraph/examples/profiling_call_setup_loop` | `Mutex::lock` > 10% of self time → confirmed contention in `TransactionManager`. Walk the `DashMap` plateaus to spot per-shard lock pressure. |

Pair with the `dialog_txn_contended/*` bench in
`crates/rvoip-sip-dialog/benches/transaction_manager.rs` to validate
whether the contention is in the manager's lock specifically.

### 3. Registration storm

Models the "WAN outage recovery — all phones REGISTER at once" scenario.

| Tool | Command | What to look for |
|---|---|---|
| Criterion | `cargo bench -p rvoip-sip --bench registration_storm` | `e2e_register_storm/<fanout>` lines. Throughput should scale roughly linearly with fanout up to core count. If it plateaus early, the registrar's auth/hash path is contended. |
| samply | `cargo build --profile flamegraph -p rvoip-sip --example profiling_registration_storm` then `samply record target/flamegraph/examples/profiling_registration_storm` | Auth/hash insertion > 25% of samples → shard the registrar map. `parse_message` > 30% → see the parser scenario above. |
| dhat | `cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_b2bua` | Per-REGISTER alloc count should be O(10), not O(100). Anything in the hundreds is likely per-header `format!()` in the response serializer. |
| tokio-console | Build with `--features tokio-console`, `RUSTFLAGS="--cfg tokio_unstable"`, run, then open `tokio-console` in another shell. | Tasks stuck in `Scheduled` / `Idle` waiting on a lock surface as long `busy` bars. |

Fanout and duration: `RVOIP_PROFILE_FANOUT=<n>` (default 16),
`RVOIP_PROFILE_DURATION=<secs>` (default 30).

### 4. Concurrent dialog count

Pre-establishes N held-open dialogs and measures the cost of one more
INVITE/BYE against that backlog. The latency curve as N grows is the
signal.

| Tool | Command | What to look for |
|---|---|---|
| Criterion | `cargo bench -p rvoip-sip --bench dialog_steady_state` | Per-call latency at backlog 0, 50, 250, 1000. Sub-linear growth is fine; super-linear (e.g. 4× from 250 → 1000) means a per-call O(N) scan somewhere. |
| dhat | `cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_dialog` | "Max bytes at t-gmax" is the steady-state heap. Multiply by your target dialog count to estimate RSS. |
| samply | `cargo build --profile flamegraph -p rvoip-sip --example profiling_dialog_steady_state` then `samply record target/flamegraph/examples/profiling_dialog_steady_state` | Wide `parking_lot` / `tokio::sync::Mutex` stacks = lock contention. Wide `DashMap::get` stacks = call-id `String` hash overhead. |

Backlog and duration: `RVOIP_PROFILE_BACKLOG=<n>` (default 250),
`RVOIP_PROFILE_DURATION=<secs>` (default 60).

---

## RTP / Media scenarios

The four scenarios above stress signalling. The two below stress the
per-packet / per-frame media plane in `rvoip-rtp-core` and
`rvoip-media-core`. Both crates expose criterion benches under their
own `benches/` directories.

### 5. Per-packet forward storm

Every received packet goes through SSRC demux → SRTP unprotect → header
parse → broadcast dispatch (`rtp-core/src/transport/udp.rs:269`), then
on the media side through a contended codec mutex
(`media-core/src/relay/controller/mod.rs:150`). This is the load-bearing
hot path for any active call. We measure it isolated (rtp-core) and at
the per-session controller level (media-core).

| Tool | Command | What to look for |
|---|---|---|
| Criterion (rtp-core) | `cargo bench -p rvoip-rtp-core --bench packet_parse_serialize` and `--bench srtp_protect_unprotect` and `--bench udp_loopback` | Bytes/sec at 80, 160, 200, 1200 B payloads. `udp_loopback`'s `srtp` vs `plain` delta tells you the SRTP Mutex tax above the actual crypto work measured by `srtp_protect_unprotect`. |
| Criterion (rtp-core) | `cargo bench -p rvoip-rtp-core --bench session_demux` | `std_mutex` vs `dashmap` lines at threads {1, 4, 8, 16}. Anything where `std_mutex` >2× slower at 16 threads means the Phase C2 swap is a clear win. |
| Criterion (media-core) | `cargo bench -p rvoip-media-core --bench audio_frame_pipeline` | `pipeline_concurrent/zero_copy` at tasks {1, 4, 8, 16}. If 16-task throughput is < 4× 1-task throughput, the shared `g711_codec` Mutex is the cap; Phase C5 (per-session codec) is the fix. |
| Criterion (media-core) | `cargo bench -p rvoip-media-core --bench bridge_forward` | `tokio_rwlock` vs `dashmap` lines. The current `rtp_sessions: RwLock<HashMap>` serialises every bridged forward — the `dashmap` line is the post-C7 ceiling. |
| samply | `cargo build --profile flamegraph -p rvoip-sip --example profiling_call_setup_loop` then `samply record …` | `tokio::sync::Mutex::lock` aggregate self-time on the inbound RTP path should drop sharply after Phases C1, C5, C7. SRTP `protect`/`unprotect` itself should stay flat (it's the actual crypto, not the lock). |

### 6. Jitter-buffer steady-state

Both `rtp-core` and `media-core` carry an adaptive jitter buffer. The
media-core one currently takes ~7 `tokio::RwLock.write().await`
acquisitions per inserted frame (`buffer/jitter.rs:152–189`); Phase C6
collapses that to one `parking_lot::Mutex` + a few atomics.

| Tool | Command | What to look for |
|---|---|---|
| Criterion (rtp-core) | `cargo bench -p rvoip-rtp-core --bench jitter_buffer` | `jitter_add_in_order` and `jitter_add_out_of_order` ns/op at depths {0, 10, 100, 1000}. Curve should grow as `log(depth)` — super-linear means BTreeMap insertion is sharing a cache line with stat updates. |
| Criterion (media-core) | `cargo bench -p rvoip-media-core --bench jitter_buffer_frames` | Same shape; track `add_in_order` ns/op pre vs post C6. Target ≥3× improvement (collapsing 7 RwLock awaits to 1 + atomics). |
| Criterion (media-core) | `cargo bench -p rvoip-media-core --bench mixer` | `mixer_mix_cycle` at participant counts {2, 4, 8, 16}. Throughput per participant should stay roughly constant; if it drops past N=8 the `output_cache` / `stats` Mutex chain is serialising the cycle (Phase C7). |
| samply | `cargo build --profile flamegraph -p rvoip-media-core --example profiling_jitter_steady_state` (when added) | `RwLock::write` self-time inside `JitterBuffer::add_frame` should compress to a single `parking_lot::Mutex::lock` after Phase C6. |

The four-scenario template (Criterion → samply → dhat) generalises: add
a long-running profiling example under `crates/rvoip-sip/examples/`
mirroring `profiling_call_setup_loop` whenever you need a sustained
workload that's CPU- or heap-profilable. The `[profile.flamegraph]`
inheritance keeps symbols intact.

---

## Reading flamegraphs

- **X-axis is sample count, not time.** A wide function used `self time`
  proportional to its width — that's where the CPU went.
- **Y-axis is call depth.** Plateaus near the top are leaf functions
  (the actual hot loops); wide bases are entry points.
- **Inverted / icicle view** in samply / Firefox Profiler often reads
  better — caller-side aggregation makes the bottleneck obvious.
- Look for *plateaus*, not peaks. A single 5%-wide function called from
  many places is usually a bigger lever than a 20%-wide function called
  once.

## Reading dhat output

dhat reports two main numbers:

- **Total bytes** — every byte ever allocated. Dominated by
  short-lived allocations. Useful for "how much GC pressure".
- **Max bytes at t-gmax** — peak live heap. Multiply by target
  concurrency for an RSS estimate.

Filter the tree by program-point to see which call site allocated. Hot
spots in `format!`, `Vec::new`, or `BytesMut::with_capacity` are
candidates for pooling / `SmallVec` / `Cow<'static, str>`.

---

## macOS pitfalls

- `cargo flamegraph` on macOS uses `dtrace`, which requires `sudo` and
  is blocked by System Integrity Protection on binaries that link
  system frameworks (codec-core, UDP sockets). Prefer `samply` —
  it works without sudo and produces the same SVG-style output.
- If you must use `cargo flamegraph`, the `--root` flag re-signs the
  binary, but it still trips SIP intermittently on Darwin 25.
- On Linux, `cargo flamegraph --profile flamegraph --example <name>` is
  the simplest path; ensure `kernel.perf_event_paranoid <= 1`.

## tokio-console

When the hypothesis is "tasks are blocked on a lock", neither CPU nor
heap profiling will tell you. tokio-console shows per-task busy / idle /
scheduled time and highlights tasks waiting on locks.

Requires:

```bash
RUSTFLAGS="--cfg tokio_unstable" \
    cargo run --release --features tokio-console \
              -p rvoip-sip --example profiling_registration_storm
```

In another shell:

```bash
tokio-console
```

Scoped here to `profiling_registration_storm` and
`profiling_dialog_steady_state` — the two scenarios where lock
contention is the working hypothesis. The parser and call-setup
scenarios are CPU-bound; tokio-console adds noise without insight.

---

## Tracking regressions

```bash
# On main, before your change:
cargo bench --workspace -- --save-baseline main

# On your branch, after the change:
cargo bench --workspace -- --baseline main
```

Criterion prints per-benchmark deltas. Per the workspace memory note,
always include `--all-features` for the validation pass — default
`cargo bench` skips feature-gated targets and reports false-green:

```bash
cargo bench --workspace --all-features --no-run
```

If a regression appears, re-run the matching profiling recipe (CPU and
heap) to localise the cause before chasing a fix.
