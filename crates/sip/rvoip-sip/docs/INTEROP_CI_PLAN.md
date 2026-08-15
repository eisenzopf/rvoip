# rvoip-sip Beta Interop CI Plan

Date: 2026-05-25

The beta release needs repeatable external-peer evidence. This plan defines the
minimum lab matrix. It can run in CI, nightly CI, or a documented release-gate
workflow, but results must be archived before beta release notes are cut.

The release-gate entry point is:

```sh
crates/sip/rvoip-sip/scripts/beta_gate.sh --interop
```

By default, missing PBX or strict-UA lab dependencies are recorded as `SKIP`
artifacts. For a release candidate, run with
`BETA_GATE_REQUIRE_EXTERNAL=1` so those skips fail the gate. The real
Kamailio/OpenSIPS stateful-proxy matrix is stricter: it is mandatory in both
`--interop` and `--full`, and missing Docker, either pinned peer, SIPp, packet
capture, either adjacency order, UDP, TCP, or verified TLS is always a hard
failure rather than a skip.

## Required Peers

| Peer | Role | Beta requirement |
|------|------|------------------|
| SIPp | Deterministic UAC/UAS and load generator | Required release gate. |
| Asterisk `res_pjsip` | PBX interop | Required release gate. |
| FreeSWITCH Sofia | PBX/B2BUA interop | Required release gate. |
| PJSIP or baresip | Strict SIP user agent | Required release gate. |
| Kamailio | Transaction-stateful proxy interoperability peer | Required for the `0.3.8` proxy release gate. |
| OpenSIPS | Independent transaction-stateful proxy interoperability peer | Required for the `0.3.8` proxy release gate. |

## Current Automation Status

| Gate | Current status | Command |
|------|----------------|---------|
| Local Asterisk/FreeSWITCH matrix | Scripted; manages `~/Developer/asterisk` and `~/Developer/freeswitch` sequentially | `BETA_RUN_LOCAL_PBX=1 crates/sip/rvoip-sip/scripts/beta_gate.sh --interop` |
| Already-running PBX matrix | Scripted; requires PBX containers already running | `BETA_RUN_PBX=1 crates/sip/rvoip-sip/scripts/beta_gate.sh --interop` |
| SIPp standalone | Scripted; requires SIPp and target host/port | `BETA_RUN_SIPP=1 BETA_SIPP_TARGET_HOST=<host> BETA_SIPP_TARGET_PORT=<port> crates/sip/rvoip-sip/scripts/beta_gate.sh --interop` |
| PJSIP/baresip | Scripted; final gate archived baresip strict-UA evidence | `BETA_RUN_STRICT_UA=1 crates/sip/rvoip-sip/scripts/beta_gate.sh --interop` or the final full gate. |
| Kamailio/OpenSIPS | Mandatory release entry point implemented; the command remains fail-closed until every scenario and verified-TLS row is implemented | `crates/sip/sip-proxy/tests/interop/scripts/beta_gate.sh`; the enclosing beta gate invokes it once with the fixed release matrix. |

The local PBX gate stops Asterisk and FreeSWITCH before switching providers
because both bind overlapping SIP ports. It restores the PBX that was running
when the gate started unless `BETA_RESTORE_LOCAL_PBX=0` is set.

The beta gate writes audit evidence under `BETA_GATE_ARTIFACT_DIR` or
`target/beta-gate/<timestamp>/`:

- `summary.md`: gate status, durations, and log links.
- `environment/environment.md`: host, toolchain, git, Docker state, redacted
  beta/PBX environment, and copied/redacted local PBX config references.
- `environment/docker-<phase>/`: Docker `ps`, `inspect`, and log-tail
  snapshots around PBX up/down/matrix phases.
- `pbx/summary.md`: PBX interop result table.
- `pbx/matrix.tsv`: one row per provider/API/scenario/transport/role command.
- `pbx/<provider>/<api>/<scenario>/<transport>/`: raw command logs,
  per-cell metadata, WAV/media artifacts, analyzer logs, and generated TLS
  listener cert paths where used.
- `pbx/<provider>/<api>/g729_call/<profile>/<transport>/`: G.729A/G.729AB
  profile-specific logs, WAVs, and analyzer evidence.
- `proxy-interop/summary.json`, `summary.md`, and `matrix.tsv`: the
  machine-readable and human-readable 12-row peer/order/transport result.
- `proxy-interop/<peer>/<order>/<transport>/`: aggregate scenario evidence,
  peer version, packet captures, retention convergence, and—on TLS
  rows—`tls-verifier-result.json` plus the three gate-owned boundary logs.
  The TLS proof binds the CA plus rvoip, peer, and SIPp certificate hashes,
  expected DNS identities, both directed native-proxy verification legs, and
  the actual hostname-verifying boundary traffic.
- `proxy-interop/environment.txt`, `source-check.txt`, and
  `runtime-state-{start,end,check}.json`: pinned peer identity,
  clean/unchanged source binding, and proof that pre-existing containers,
  networks, volumes, and test-port listeners were preserved with no added
  leftovers.

## SIPp Matrix

| Scenario | Expected result |
|----------|-----------------|
| INVITE, 200, ACK, BYE | 100% success in smoke; 99.9% at beta load gate. |
| CANCEL before answer | Correct final response and cleanup. |
| REGISTER and unregister | Successful lifecycle and expiry handling. |
| OPTIONS | Correct capability response. |
| re-INVITE hold/resume | Correct SDP direction and dialog state. |
| UPDATE | Correct in-dialog handling; outbound 491 completes the exact UPDATE attempt without entering re-INVITE glare retry. |
| PRACK | Reliable provisional positive and negative behavior. |
| REFER/NOTIFY | Transfer progress and terminal NOTIFY. |
| INFO DTMF | Correct mid-dialog request behavior. |
| Auth success/failure | Digest retry and failure reporting. |
| Malformed request | No panic, correct 4xx or drop behavior. |
| Retransmission/timers | No leaked state or duplicate terminal events. |

## Asterisk Matrix

Run the same functional suite through `Endpoint`, `StreamPeer`, and
`CallbackPeer` where each API surface applies.

| Scenario | Required for beta |
|----------|-------------------|
| UDP registration/unregistration | Yes |
| UDP outbound call | Yes; G.711 PCMU/PCMA baseline with bidirectional tone audio verification |
| UDP inbound call | Yes |
| UDP G.729A/G.729AB call | Yes where PBX has G.729 enabled; both profiles must include bidirectional tone audio verification |
| TLS registration/call | Yes where test cert setup is available |
| Digest auth | Yes |
| CANCEL | Yes |
| BYE cleanup | Yes |
| Hold/resume | Yes |
| Blind transfer | Yes |
| REFER/NOTIFY progress | Yes |
| PRACK/session timers | Yes if peer profile enables them |
| DTMF | Yes |
| SDES-SRTP | Yes where claimed |

## FreeSWITCH Matrix

Mirror the Asterisk matrix where feasible. Any peer-specific difference must
be recorded in `COMPATIBILITY_MATRIX.md` with packet capture or log evidence.
The G.729 row uses the shared `g729_call` scenario. By default the PBX runner
expands it into `PBX_CODEC_PROFILE=g729a` and `PBX_CODEC_PROFILE=g729ab`, so
the beta gate attests G.729A (`annexb=no`) and G.729AB (`annexb=yes`) against
both Asterisk and FreeSWITCH. The G.711 baseline remains the analyzer-enforced
`basic_call` scenario. Override `BETA_PBX_G729_PROFILES` only for targeted
reruns.

## Stateful Proxy Matrix

Run the same packet-level scenarios independently against real, pinned
Kamailio and OpenSIPS processes. The harness follows the local
Asterisk/FreeSWITCH lifecycle pattern: it owns startup, readiness, isolated
configuration, packet capture, logs, exact version/image provenance, teardown,
and restoration of any process that was running before the gate.

| Scenario | Required for `0.3.8` |
|----------|----------------------|
| UDP, TCP, and TLS/SIPS forwarding | Yes |
| Matched and unmatched CANCEL | Yes |
| CANCEL before and after provisional response | Yes |
| Sequential and parallel forks | Yes |
| Multiple and late INVITE 2xx | Yes |
| 2xx and non-2xx ACK routing | Yes |
| Timer C branch handling | Yes |
| Via push/pop and response destination | Yes |
| Route, Record-Route, strict/loose routing, and SIPS | Yes |
| 401/407 challenge aggregation | Yes |
| Transport failure and RFC 3263 failover | Yes |
| True stray-response discard | Yes |
| State and process cleanup after retention drain | Yes |

## Result Artifacts

Each interop run should store:

- peer versions, container/image digests, and allowlisted secret-free Docker
  snapshots (raw `docker inspect` output is never persisted)
- exact command line or compose file
- `rvoip-sip` git revision
- pass/fail summary
- per-provider/API/scenario/transport/role matrix with duration and exit code
- SIPp stats CSV, run TSV, parsed analysis, and screen/error logs where SIPp is used
- relevant `rvoip-sip` logs
- packet capture, raw SIP trace, or Docker log tail for failures

## Release-Gate Policy

- A failure in SIPp, Asterisk, FreeSWITCH, Kamailio, or OpenSIPS blocks the
  `0.3.8` proxy release candidate.
- The proxy matrix must contain exactly both pinned peers, both adjacency
  orders, and UDP/TCP/verified-TLS rows. Its global scenario inventory and
  per-row core scenario set are validated independently while generating the
  release report. Kamailio and OpenSIPS must each independently cover the
  complete scenario inventory through real external-peer traffic across their
  six rows; one peer's evidence cannot fill a coverage gap for the other.
  In-process Rust conformance tests may supplement a row, but do not count as
  observed Kamailio or OpenSIPS interoperability.
- Regressions in previously passing beta scenarios block beta.
