# session-core Compatibility Matrix

This matrix separates the local examples, protocol regression fixtures, and
external PBX interop evidence.

| Target | Status | API surface | Transport / media | Coverage | Notes |
| --- | --- | --- | --- | --- | --- |
| Local API examples | Supported | `Endpoint`, `StreamPeer`, `CallbackPeer`, `UnifiedCoordinator` | local UDP/RTP | basic call setup, redirect, call control, audio, registration, transfer, bridge | Run `examples/run_all.sh` |
| Regression fixtures | Supported | mostly `StreamPeer` plus support peers | local UDP/RTP/TLS/SRTP | DTMF round-trip, TLS, SRTP, CANCEL, PRACK, session timers, glare retry, NOTIFY | Run `examples/regression/run_all.sh` |
| Asterisk UDP/RTP | Validated | `Endpoint`, `StreamPeer`, `CallbackPeer::builder` | UDP/RTP | registration, basic call, hold/resume, ring/cancel, DTMF, reject, blind transfer | Source of truth: `examples/pbx` |
| Asterisk TLS/SDES-SRTP | Validated with profile requirements | `Endpoint`, `StreamPeer`, `CallbackPeer::builder` | TLS + SDES-SRTP | registration, hold/resume, ring/cancel, DTMF, reject, blind transfer, audio evidence | Requires matching PJSIP TLS/SRTP and contact-flow configuration |
| FreeSWITCH/Sofia UDP/RTP | Validated by unified PBX runner when configured | `Endpoint`, `StreamPeer`, `CallbackPeer::builder` | UDP/RTP | same scenario matrix as Asterisk | Source of truth: `examples/pbx` |
| FreeSWITCH/Sofia TLS/SDES-SRTP | Validated by unified PBX runner when configured | `Endpoint`, `StreamPeer`, `CallbackPeer::builder` | TLS + SDES-SRTP | same scenario matrix as Asterisk with FreeSWITCH SRTP policy | Source of truth: `examples/pbx` |
| Kamailio/OpenSIPS + RTPengine | Planned | `UnifiedCoordinator` likely | proxy signaling plus media relay | not validated yet | Intended to flush proxy and topology assumptions |
| Carrier SBC | Partial / not certified | `Endpoint`, `StreamPeer`, `CallbackPeer` | provider-specific | basic SIP pieces exist, no certification | Needs provider-specific routing, Service-Route/Path, SRV/NAPTR, and topology hardening |
| WebRTC edge | Not supported yet | future gateway/orchestrator | ICE + DTLS-SRTP | not implemented | SDES-SRTP support does not imply WebRTC support |

## Release Gate

External PBX evidence lives in `crates/session-core/examples/pbx`. The older
provider-specific example directories are no longer maintained; use the unified
PBX runner:

```sh
./crates/session-core/examples/pbx/run.sh --pbx asterisk --api all --scenario all
./crates/session-core/examples/pbx/run.sh --pbx freeswitch --api all --scenario all
```

FreeSWITCH, proxy/RTPengine, carrier SBC, and WebRTC rows must not be described
as validated beyond the scenarios represented in this matrix.
