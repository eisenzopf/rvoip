# rvoip-sip Beta Interop CI Plan

Date: 2026-05-25

The beta release needs repeatable external-peer evidence. This plan defines the
minimum lab matrix. It can run in CI, nightly CI, or a documented release-gate
workflow, but results must be archived before beta release notes are cut.

The release-gate entry point is:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --interop
```

By default, missing external lab dependencies are recorded as `SKIP` artifacts.
For a release candidate, run with `BETA_GATE_REQUIRE_EXTERNAL=1` so missing
SIPp, PBX, or strict-UA evidence fails the gate.

## Required Peers

| Peer | Role | Beta requirement |
|------|------|------------------|
| SIPp | Deterministic UAC/UAS and load generator | Required release gate. |
| Asterisk `res_pjsip` | PBX interop | Required release gate. |
| FreeSWITCH Sofia | PBX/B2BUA interop | Required release gate. |
| PJSIP or baresip | Strict SIP user agent | Required release gate. |
| Kamailio or OpenSIPS | Proxy/registrar | Investigation track unless automated before beta. |

## Current Automation Status

| Gate | Current status | Command |
|------|----------------|---------|
| Local Asterisk/FreeSWITCH matrix | Scripted; manages `~/Developer/asterisk` and `~/Developer/freeswitch` sequentially | `BETA_RUN_LOCAL_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --interop` |
| Already-running PBX matrix | Scripted; requires PBX containers already running | `BETA_RUN_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --interop` |
| SIPp standalone | Scripted; requires SIPp and target host/port | `BETA_RUN_SIPP=1 BETA_SIPP_TARGET_HOST=<host> BETA_SIPP_TARGET_PORT=<port> crates/rvoip-sip/scripts/beta_gate.sh --interop` |
| PJSIP/baresip | Required before beta claims strict-UA evidence | Add runner, then remove the gate skip. |
| Kamailio/OpenSIPS | Investigation only | Do not block beta unless release notes claim this topology. |

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

## SIPp Matrix

| Scenario | Expected result |
|----------|-----------------|
| INVITE, 200, ACK, BYE | 100% success in smoke; 99.9% at beta load gate. |
| CANCEL before answer | Correct final response and cleanup. |
| REGISTER and unregister | Successful lifecycle and expiry handling. |
| OPTIONS | Correct capability response. |
| re-INVITE hold/resume | Correct SDP direction and dialog state. |
| UPDATE | Correct in-dialog handling and glare behavior. |
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
| UDP outbound call | Yes |
| UDP inbound call | Yes |
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

## Result Artifacts

Each interop run should store:

- peer versions, container/image digests, and Docker inspect snapshots
- exact command line or compose file
- `rvoip-sip` git revision
- pass/fail summary
- per-provider/API/scenario/transport/role matrix with duration and exit code
- SIPp stats CSV, run TSV, parsed analysis, and screen/error logs where SIPp is used
- relevant `rvoip-sip` logs
- packet capture, raw SIP trace, or Docker log tail for failures

## Release-Gate Policy

- A failure in SIPp, Asterisk, or FreeSWITCH blocks beta unless documented as a
  non-claim with an explicit exclusion.
- Regressions in previously passing beta scenarios block beta.
- Investigation-track failures do not block beta unless the release notes claim
  the affected topology.
