# Next Steps — three workstreams from the rvoip vs Asterisk/FreeSWITCH comparison

> Companion to [`BENCHMARKING.md`](BENCHMARKING.md) (publishable-numbers methodology)
> and [`PROFILING.md`](PROFILING.md) (finding bottlenecks). This document
> describes the three concrete workstreams that fell out of the first
> three-way comparison attempt.

## Status snapshot (2026-05-21)

| Area | Status | Test count delta |
|---|---|---|
| **A** FreeSWITCH benchmarkable | Blocked — needs edits in `~/Developer/freeswitch/` (out-of-repo) | 0 |
| **B.1** flamegraph diagnose | Manual step — requires running sipp + samply against a built test binary | 0 |
| **B.2** fix | Gated on B.1 | 0 |
| **B.3** validate at 100/200/500 CPS | Gated on B.2 | 0 |
| **B.4** PROFILING.md Scenario 9 | Gated on B.2 | 0 |
| **C1.1** Port-zero rejection (RFC 3264 §6 / RFC 4568 §7.3) | Done | +2 |
| **C1.2** SDP version-increment audit + hold/resume test | Done — production paths already wired; test added | +1 |
| **C1.3** UAS-side glare / 491 Request Pending | Done — executor pre-table short-circuit + YAML cleanup rows | +2 |
| **C2** Opus codec wiring (SDP side) | Done — `Config::offered_codecs` + builder refactor; media-flow encode/decode still gated on media-core `opus` feature | +2 |
| **C3** DTLS-SRTP negotiator | SDP-side scaffold done (`adapters/dtls_negotiator.rs`, `SetupRole`, `detect_dtls_offer`). Handshake driver + SRTP keying (C3.2 / C3.3) still open | +8 |

Total lib-test delta: **101 → 116** (15 new tests). All passing.

### What remains for A + B

Areas A and B need a live environment — a running FreeSWITCH/Asterisk
container, sipp installed, and a samply flamegraph capture against the
`perf_sipp_parity` binary. The deliverables below are the executable
steps; running them is an operator task, not an in-tree code change.

## Context

Running `perf_sipp_parity` and direct sipp probes against an Asterisk
container + a FreeSWITCH container surfaced three concrete deliverables:

1. **FreeSWITCH never produced numbers.** Its exposed profiles
   (`5060` / `5062` / `5063`) all require digest auth and return
   `407 Proxy Authentication Required` to anonymous sipp. The
   three-way comparison table is blocked on this.
2. **rvoip hangs at 100 CPS over sipp** even though the internal
   benchmarks (alice→bob in-process) sustain 100+ CPS at 100 % ASR.
   sipp pegs 99 % CPU retransmitting; the rvoip test process sits at
   46 % CPU. The 30 CPS sipp-driven run succeeded cleanly (600/600).
   This is a real, reproducible degradation cliff between in-process
   driving (works) and sipp-driving (doesn't) — a serialization
   point we can find.
3. **The SDP audit catalogued real compliance gaps** (port-zero
   rejection, DTLS-SRTP, glare / 491, re-INVITE version increment,
   Opus codec wiring). Each is documented with file:line and
   acknowledged in source comments — they're well-scoped tickets,
   not investigation work.

---

## Area A — Make FreeSWITCH benchmarkable

**Goal:** drive sipp against FreeSWITCH with the same args we use
against Asterisk and rvoip, so the three-way comparison table has a
FreeSWITCH column.

### Findings from exploration

- `~/Developer/freeswitch/` is a Dockerfile-built container. The
  standard FreeSWITCH config tree is baked into the image; **custom
  rvoip profiles are generated at container start by
  `docker-entrypoint.sh`** (not bind-mounted).
- Existing custom profiles: `rvoip_udp.xml` (5062) and
  `rvoip_tls_srtp.xml` (5063). Both set `auth-calls=true`, both route
  inbound calls to context `rvoip`.
- Existing dialplan extensions: `100[1-3]` (TLS/SRTP bridges) and
  `200[1-3]` (UDP/RTP bridges). **No auto-answer extension.**
- The config tree is NOT volume-mounted in `docker-compose.yml`, so
  config tweaks need an image rebuild *OR* a runtime `docker cp` +
  `fs_cli reloadxml`.

### Deliverables

1. **`~/Developer/freeswitch/docker-entrypoint.sh`** — extend the
   `write_rvoip_profiles()` function to also generate
   `perf_bench.xml`:
   - Port `5064` (UDP only)
   - `auth-calls=false`
   - Context `perf_bench`
   - Codec `PCMU,PCMA`
2. **Same file** — extend `write_rvoip_dialplan()` to add a
   `perf_bench` context with a catch-all auto-answer extension:
   ```xml
   <context name="perf_bench">
     <extension name="perf_bench_catchall">
       <condition field="destination_number" expression=".*">
         <action application="answer"/>
         <action application="sleep" data="100"/>
         <action application="hangup"/>
       </condition>
     </extension>
   </context>
   ```
3. **`~/Developer/freeswitch/docker-compose.yml`** — add
   `5064:5064/udp` to the `ports:` list.
4. **`~/Developer/freeswitch/scripts/up.sh`** — verify it triggers a
   fresh build when the entrypoint script changes (Dockerfile
   `COPY docker-entrypoint.sh` should invalidate the layer cache).
5. **Run the comparison** at 30 / 100 / 300 CPS — same sipp args as
   Asterisk:
   ```bash
   sipp -sn uac -r <CPS> -m <CPS*30> -p 5083 -nostdin \
        -trace_stat -stf fs_<CPS>cps.csv 192.168.64.2:5064
   ```
6. **Capture numbers** in a comparison table extending
   `docs/BENCHMARKING.md §6.5` (industry calibration) with a new
   "in-house measured on this hardware" sub-section.

### Files touched

- `~/Developer/freeswitch/docker-entrypoint.sh` (extend two existing
  functions)
- `~/Developer/freeswitch/docker-compose.yml` (add one port)
- `crates/sip/rvoip-sip/docs/BENCHMARKING.md` (new sub-section with
  measured numbers)

### Verification

- After rebuild + restart, `fs_cli -x "sofia status"` shows the
  `perf_bench` profile listening on `5064`.
- `sipp -sn uac -r 1 -m 1 192.168.64.2:5064` returns
  `Successful call: 1, Failed call: 0`.
- Full 30 CPS run completes with ≥ 99 % success rate.

---

## Area B — rvoip 100-CPS sipp-path saturation

**Goal:** find and fix the serialization point that makes rvoip
handle 100 CPS internally but stall on 100 CPS over sipp.

### Findings from exploration

Top three suspects, ranked by confidence:

| # | Suspect | Where | Reason |
| --- | --- | --- | --- |
| 1 | `CallbackPeer::dispatch` single-task event consumption loop | `crates/sip/rvoip-sip/src/api/callback_peer.rs:1527, 1583` | `select!` loop pulls events one-at-a-time from a tokio mpsc and spawns a handler per event. At 100 INVITEs/sec, if any handler awaits a slow path (media setup, mutex), the recv loop falls behind. Internal benchmarks bypass this loop because `alice.invite()` does not flow through `CallbackPeer`. |
| 2 | `StateMachine::process_event` serial action execution | `crates/sip/rvoip-sip/src/state_machine/executor.rs:336` | Actions inside one session run sequentially. `Action::CreateSession` → `media_adapter.create_session()` allocates an RTP socket per call. Even 5 ms per call × 100 CPS = a full second of media-setup latency lined up behind the event loop. |
| 3 | UDP transport single-task recv loop | `crates/sip/rvoip-sip-transport/src/transport/udp/mod.rs:38` | One spawned task parses + dispatches every inbound packet. Has worked fine until now because the downstream layers were the bottleneck. |

Recent commits (`d32212a9`, `b32145f1`, `0b47a59e`) attacked locks in
the dialog / transaction layers but **left the rvoip-sip event loop
and state-machine executor serial**. That's the next target.

### Deliverables

1. **Diagnose with `samply`**:
   ```bash
   cargo build --profile flamegraph -p rvoip-sip --features perf-tests \
        --test perf_sipp_parity
   PATH=/opt/homebrew/bin:$PATH RVOIP_PERF_TARGET_CPS=100 \
        RVOIP_PERF_STEADY_SECS=20 \
        samply record -- target/flamegraph/deps/perf_sipp_parity-* --nocapture
   ```
   Read the flamegraph for: time in `event_rx.recv`, time in
   `process_event`, time in `MediaSessionController::start_media`.
   Note which is widest.

2. **Hypothesis-driven fix** (pick whichever the flamegraph confirms):
   - **If event-loop serialization** (suspect #1): refactor
     `CallbackPeer::dispatch` to use `tokio::sync::mpsc::recv_many`
     for batched recv, or move the on-incoming-call dispatch off the
     main event task entirely (each event spawned to a separate task
     on receipt, not on dispatch).
   - **If state-machine serialization** (suspect #2): move
     `Action::CreateSession`'s media-setup off the critical path.
     The handler can return `Accept` immediately, defer media-adapter
     `start_media()` to a `tokio::spawn`, and signal back via the
     existing event channel. This is a meaningful refactor; gate it
     behind a measurement that proves the win.
   - **If UDP recv-task serialization** (suspect #3): split parser
     dispatch from the recv task. The recv task does only `recv_from`
     + push to an unbounded channel; a parser-task pool drains.

3. **Validate** with `perf_sipp_parity` at 100, 200, 500 CPS.
   Acceptance: 100 CPS achieves ≥ 99 % ASR, parity delta < 5 %, no
   sipp retransmits.

4. **Document** the bottleneck and the fix in `docs/PROFILING.md`
   Scenario 9 (new sub-section). The narrative + samply screenshot is
   high-credibility content.

### Files touched (anticipated, contingent on flamegraph)

- `crates/sip/rvoip-sip/src/api/callback_peer.rs` (if suspect #1)
- `crates/sip/rvoip-sip/src/state_machine/executor.rs` +
  `crates/sip/rvoip-sip/src/adapters/media_adapter.rs` (if suspect #2)
- `crates/sip/rvoip-sip-transport/src/transport/udp/mod.rs` (if suspect #3)
- `crates/sip/rvoip-sip/docs/PROFILING.md` (new Scenario 9)

### Verification

- Smoke: `perf_sipp_parity` at 100 CPS completes within the
  steady-state window with ≥ 99 % ASR and parity delta within ± 5 %.
- Regression: `cargo test -p rvoip-sip --features perf-tests
  --test perf_call_setup_cps --release` internal numbers do **not**
  regress (the fix targets the external-driver path; internal
  driving was already lock-free).
- Lib tests: 101 / 101 still pass.
- The `samply` flamegraph before/after shows the previously-wide
  stack collapse.

---

## Area C — SDP compliance gaps

**Goal:** close the documented gaps from the SDP audit so rvoip's SDP
handling is RFC 3264 / 4566 / 4568 / 5763 complete for the cases that
matter for interop (Asterisk, FreeSWITCH, SIPp, then later WebRTC).

### Findings from the audit

The parser is already RFC 4566 complete. The negotiator does codec
intersection + direction reconciliation per RFC 3264 §6.1 correctly.
**The gaps are in the answer-builder side and the mid-call paths.**

Six concrete gaps, with file:line:

| # | Gap | File:line | Why it matters |
| --- | --- | --- | --- |
| 1 | **Port-zero rejection** (RFC 3264 §6) — declined m-lines must answer `m=audio 0 …` | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs:646-652` (acknowledged TODO) | Strict middleboxes reject the answer; will hit on interop |
| 2 | **DTLS-SRTP** (RFC 5763) — `a=fingerprint` / `a=setup` parsed but not negotiated | `crates/rvoip-media-core/src/.../negotiation.rs:19-25` | Required for WebRTC interop |
| 3 | **Glare / 491 Request Pending** (RFC 3261 §14.1) — re-INVITE collisions not detected | not implemented | Strict SBC interop |
| 4 | **Mid-call re-INVITE SDP version increment** — `next_sdp_origin` is referenced but not fully wired | `media_adapter.rs:1307` | Some peers reject re-INVITEs with unchanged version |
| 5 | **Opus / G.729 / G.722** — codec types exist in `audio-core` but not in the SDP builder | `media_adapter.rs:1334-1365` | Hard-coded PCMU/PCMA — voice-AI peers need Opus |
| 6 | **Multi-m-line / video** — parser handles them, adapter is audio-only | `media_adapter.rs:645` comment | Bigger ticket; defer |

### Deliverables (sequenced)

**Phase C1 — quick wins** (low risk, immediate interop value):

- **C1.1** Port-zero rejection for declined m-lines. Replace the
  "answer plaintext on the same port" placeholder at
  `media_adapter.rs:646-652` with `m=<media> 0 <proto> 0` per
  RFC 3264. Add a focused unit test that verifies the answer for an
  offer with `RTP/SAVP` when `accept_srtp=false`.
- **C1.2** Mid-call SDP version increment. Wire `next_sdp_origin`
  through every re-offer / re-answer build path. Add a test
  asserting `o=` line `version` field strictly increases across
  hold → resume.
- **C1.3** Glare detection. When a re-INVITE arrives while we have
  an outstanding re-INVITE on the same dialog, respond
  `491 Request Pending` per RFC 3261 §14.1. Caller-side:
  random-backoff retry on receiving 491. Add a test that drives the
  glare path between two coordinators.

**Phase C2 — Opus codec wiring** (codec-core already supports it):

- **C2.1** Extend `media_adapter.rs:1334-1365` to offer
  `opus/48000/2` (PT 96) when `Config::offered_codecs` includes
  Opus. Hook the codec-core Opus encoder/decoder into the
  media-session output path. Test against an Opus-capable peer
  (loopback alice with `offered_codecs = [Opus]`).
- **C2.2** Update `BENCHMARKING.md §6.5` industry table to mention
  Opus support unlocks WebRTC-style workloads.

**Phase C3 — DTLS-SRTP** (largest item — write the negotiator):

- **C3.1** In the SDES branch of `negotiation.rs`, add a parallel
  DTLS branch: detect `a=fingerprint` + `a=setup:active/passive`,
  generate the answer fingerprint + complementary setup.
- **C3.2** Hand off to a DTLS handshake driven by rtp-core's
  transport. (rtp-core's SRTP context construction already exists;
  we need to plumb in DTLS-derived keys instead of SDES-derived
  keys.)
- **C3.3** Add an integration test in
  `tests/dtls_srtp_integration.rs` using an in-process WebRTC-style
  peer.

**Phase C4 — out of scope, document only**:

- Multi-m-line / video: a Sprint-4+ ticket, scope it out explicitly
  in the methodology doc and `BENCHMARKING.md`.

### Verification

- Phase C1: unit tests for port-zero rejection, version increment,
  and glare path pass.
- Phase C2: `tests/sdp_matcher_integration.rs` gains an Opus codec
  test that passes.
- Phase C3: a new `tests/dtls_srtp_integration.rs` exercises the
  full DTLS-SRTP setup and passes.
- No regressions: `cargo test -p rvoip-sip --lib --all-features`
  stays 101 / 101.

---

## Execution order across the three areas

| Step | Area | Why this order |
| --- | --- | --- |
| 1 | **A** FreeSWITCH benchmarkable | Independent; unblocks the three-way comparison table that's already half-built. ~1 hour of work + image rebuild. |
| 2 | **B** flamegraph + diagnose | Independent; produces concrete fix scope from real data. ~1 hour to instrument + capture + analyse. |
| 3 | **B** fix | Depends on step 2. ~half-day to one day depending on which suspect the flamegraph indicts. |
| 4 | **A** re-run with the fix | The three-way comparison is then publishable. |
| 5 | **C1** quick-win SDP fixes | Independent of A / B; safe to interleave. ~half-day for all three fixes. |
| 6 | **C2** Opus | Builds on C1's changes to the codec offer side. |
| 7 | **C3** DTLS-SRTP | Largest item; can be deferred. |

A + B unblock immediate credibility (real comparison numbers). C1
unblocks interop credibility. C2 / C3 unlock new market segments
(WebRTC / voice-AI).

## Out of scope

- Multi-m-line / video — explicit Sprint-4 deferral, documented in
  the methodology doc.
- Setting up Kamailio / OpenSIPS containers — Area A delivers the
  FreeSWITCH column; SIP-proxy benchmarks are a separate workstream.
- Re-running the whole 18-scenario perf suite against the rvoip code
  post-fix — out-of-scope here; the regression check in step "B fix"
  is just scenario 1 internal numbers.

## See also

- [`BENCHMARKING.md`](BENCHMARKING.md) — publishable-numbers
  methodology (philosophy, KPI glossary, sweep tables, knee point,
  industry calibration table).
- [`PROFILING.md`](PROFILING.md) — flamegraph / dhat / criterion
  recipes for finding the root cause when a number regresses.
