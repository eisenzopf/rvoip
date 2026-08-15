# rvoip examples

**Start here.** These are runnable, scenario-oriented examples for building with
rvoip — organized by *what you want to build*, not by which API you use. Each is
a standalone Cargo project with its own README and (for multi-process demos) a
`./run_demo.sh` that boots every process and checks the result.

## Maturity scope

Examples 01-10 target **`rvoip-sip`, the beta-qualified product** — the only
workspace product covered by the SIP release gate. Examples 11-14 demonstrate
available developer-preview products: the AI harness and vCon path,
cross-transport WebRTC-to-SIP escalation, Amazon Connect integration, and
hosted Vapi voice agents.

Beta media defaults to **PCMU/PCMA**; the fully integrated optional
**G.729A/G.729AB** and **AMR-NB/AMR-WB** paths are developer preview and are
not exercised here.
Transports are **UDP** (interop-tested) and **TCP/TLS** (supported), with
**SDES-SRTP** in the qualified SIP media envelope. **Opus/G.722** are
developer-preview additions. **DTLS-SRTP, ICE, external TURN configuration,
and WebRTC are available as developer previews outside the SIP beta gate.**
The source of truth is
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
| 11 | [ai-harness-demo](11-ai-harness-demo/) | Fake ASR/TTS/dialog + vCon evidence | `rvoip-harness` | `cargo run` |
| 12 | [customer-escalation-sip-webrtc](12-customer-escalation-sip-webrtc/) | Browser WebRTC chat escalates to Alice's SIP phone | `rvoip::app` gateway API | `cargo run -- --auto-proof` |
| 13 | [sip-to-amazon-connect](13-sip-to-amazon-connect/) | SIP headers become Amazon Connect attributes with a live audio bridge | `rvoip-amazon-connect` | `cargo run` |
| 14 | [vapi-agent](14-vapi-agent/) | One server accepts SIP or WebRTC callers and attaches a Vapi voice agent | `rvoip::app` + `rvoip::vapi` | `cargo run -- --transport sip\|webrtc` |

## Conventions

- **Self-contained projects.** Each example is its own Cargo workspace and uses
  local rvoip crates from this checkout through `path`. The paired `version`
  tracks the unified workspace train (`0.3.8`). When copying an example into
  your own project, drop `path` and select the published version you intend to
  use.
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
