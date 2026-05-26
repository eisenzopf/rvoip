# rvoip browser-smoke

Playwright-driven smoke for the UCTP browser surface (gap plan §3.2).
Spawns the `orchestrator_bridge` example, then drives a headless
Chromium against the WebSocket signaling path and asserts the round-trip
auth handshake works from real browser code.

## Prerequisites

- Node.js ≥ 20 (the harness uses ESM + `node:child_process.spawn`).
- A working `cargo` in `PATH` (the smoke compiles + runs
  `crates/rvoip-uctp/examples/uctp_to_sip_bridge/orchestrator_bridge.rs`).

## Setup

```bash
cd tests/browser-smoke
npm install
npx playwright install chromium
```

## Run

```bash
npm test
```

The first run may take a couple of minutes — `cargo` compiles the demo
binary if it isn't already built. Subsequent runs reuse the cached
build artifact and complete in a few seconds plus the Playwright
session warmup.

## What it covers

- `ws_smoke.spec.mjs` — opens a `WebSocket("ws://127.0.0.1:7777")` from
  a Chromium page, sends one `auth.hello` envelope, asserts the server
  replies with `auth.challenge`. This is the same wire path the
  `uctp_agent_ws` Rust example exercises; running it from a real
  browser proves the server hasn't drifted from browser-WS semantics.
- `wt_smoke.spec.mjs` — gap plan §3.2 v1 punch list. Opens
  `new WebTransport("https://127.0.0.1:4433/uctp")` with the
  self-signed demo cert pinned via Chromium's
  `--ignore-certificate-errors-spki-list` flag. The orchestrator_bridge
  writes the SPKI hash (SHA-256 of the SubjectPublicKeyInfo block,
  base64-encoded) to `/tmp/uctp_demo_cert.spki`; the Playwright config
  reads it at runtime. Asserts WT session readiness; the full
  envelope round-trip is best-effort (see below).

  **Opt-in:** set `RVOIP_WT_SMOKE=1` before running `npm test` to
  include the WT project. Default CI does NOT enable it, preserving
  current pass-rate while the harness stabilizes.

## What it does NOT cover

- **WT bidi-stream envelope round-trip.** Chromium's
  `createBidirectionalStream()` and the server-side
  `web_transport_quinn::Session::accept_bi()` API have a known
  interop gap today — the browser opens a stream that the server
  doesn't detect. The smoke currently logs this as a known follow-up
  rather than failing, because the cert-pinning path (the actual
  §3.2 deliverable) is already proven. Track resolution upstream as
  `web_transport_quinn` matures.
- **WSS** can be added by pointing the smoke at `wss://...` and
  generating a trusted cert; today's demo binds plain `ws://`.

## Why outside the Cargo workspace

This is a Node.js project. `cargo metadata` would choke on its
`package.json`, and `node_modules` shouldn't show up in workspace
member discovery. The repo's root `Cargo.toml` lists `tests/browser-smoke`
in the workspace `exclude` array.

## CI

`.github/workflows/browser-smoke.yml` runs this suite on every PR.
