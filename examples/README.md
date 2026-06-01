# rvoip examples

**Start here.** These are runnable, scenario-oriented examples for building with
rvoip — organized by *what you want to build*, not by which API you use. Each is
a standalone Cargo project with its own README and (for multi-process demos) a
`./run_demo.sh` that boots every process and checks the result.

## Beta scope

Every example here targets **`rvoip-sip`, the beta-candidate crate** — the only
crate in the workspace under the beta contract. Everything else in rvoip
(WebRTC, UCTP, the orchestrator, the identity stack) is **experimental** and is
intentionally *not* shown here; for those, see the in-crate examples under each
crate and the `scripts/demo-*.sh` runners at the repo root.

Beta media is **PCMU/PCMA**; transports are **UDP** (interop-tested) and
**TCP/TLS** (supported); **SDES-SRTP** (limited suites). **Opus/G.722/G.729,
DTLS-SRTP, ICE/TURN, and WebRTC are post-beta.** The source of truth is
[`crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md`](../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Recommended path

1. [01-quickstart-p2p](01-quickstart-p2p/) — your first SIP call.
2. [02-softphone-audio](02-softphone-audio/) — add real PCMU media.
3. Then jump to whatever you're building below.

## The examples

| # | Example | Scenario | API surface | Run |
|---|---------|----------|-------------|-----|
| 01 | [quickstart-p2p](01-quickstart-p2p/) | Minimal peer-to-peer call | `StreamPeer` | `./run_demo.sh` |
| 02 | [softphone-audio](02-softphone-audio/) | Bidirectional PCMU media (verified) | `Endpoint` + audio | `./run_demo.sh` |
| 03 | [register-to-pbx](03-register-to-pbx/) | REGISTER + call via a PBX | `Endpoint` | `cargo run` (env-driven) |
| 04 | [call-control](04-call-control/) | Hold / resume / DTMF | `SessionHandle` | `./run_demo.sh` |
| 05 | [blind-transfer](05-blind-transfer/) | 3-party REFER transfer | `SessionHandle` | `./run_demo.sh` |
| 06 | [attended-transfer](06-attended-transfer/) | Consult + REFER w/ Replaces | `SessionHandle` | `./run_demo.sh` |
| 07 | [secure-call-srtp](07-secure-call-srtp/) | Mandatory SDES-SRTP | `Config` SRTP | `./run_demo.sh` |
| 08 | [tls-transport](08-tls-transport/) | SIP over TLS (`sips:`) | `Config` TLS | `./run_demo.sh` (needs openssl) |
| 09 | [ivr-server](09-ivr-server/) | Reactive inbound server | `CallbackPeer` | `./run_demo.sh` |
| 10 | [call-center-b2bua](10-call-center-b2bua/) | B2BUA bridge + routing | `UnifiedCoordinator` + `server::b2bua` | `./run_demo.sh` |

## Conventions

- **Self-contained projects.** Each example is its own Cargo workspace and
  depends on the local crate via `rvoip-sip = { version = "0.2.0-beta.1", path =
  "../../crates/sip/rvoip-sip" }`. That builds against the live tree today and
  records the crates.io version for when you copy it into your own project
  (drop the `path`, keep the `version`).
- **`./run_demo.sh`** builds release binaries, boots every process with port
  readiness checks, prints the combined logs, and exits non-zero on failure.
  Logs land in each example's `logs/`.
- **`RUST_LOG`** controls stack tracing (`info`, `debug`).

## Looking for the API reference?

These scenario examples are the productized, multi-process front door. For
**per-API-surface reference examples** (one lane each for `endpoint`,
`stream_peer`, `callback_peer`, `unified`, plus protocol regression fixtures and
PBX interop), see the in-crate suite:
[`crates/sip/rvoip-sip/examples/`](../crates/sip/rvoip-sip/examples/). Each
example here notes the in-crate example it was built from.
