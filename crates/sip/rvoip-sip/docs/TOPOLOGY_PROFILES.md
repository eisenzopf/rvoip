# rvoip-sip Beta Topology Profiles

Date: 2026-05-26

This document defines which deployment shapes beta is allowed to claim and
which shapes remain post-beta or advanced tuning work.

Final reference report: `crates/sip/rvoip-sip/beta-report/20260526T221457Z`,
generated from clean git revision `865430d4`.

## Beta-Supported Profiles

| Profile | Status | Required validation |
|---------|--------|---------------------|
| Local loopback app | Supported | In-process examples and integration tests. |
| Basic SIP client | Supported | `Endpoint` and `StreamPeer` outbound call, registration, DTMF, hold/resume. |
| Basic SIP server | Supported | `CallbackPeer` inbound call, reject/accept, DTMF, BYE cleanup. |
| Asterisk PBX | Interop tested | UDP/TLS registration and calls, digest auth, SDES-SRTP where claimed. |
| FreeSWITCH PBX | Interop tested | Mirrors the Asterisk matrix where feasible. |
| SIPp UAC/UAS | Release gate | Standalone load matrix at 30, 100, 300, 1,000, and 2,000 CPS. |
| baresip strict-UA | Interop tested | Strict-UA INVITE, 200 OK, ACK, established call, BYE, and rvoip accept checks. |
| Signaling-only B2BUA/gateway | Supported with limits | Multi-leg signaling tests and clear media relay caveats. |
| Full-media beta perf | Beta target | Media enabled, PCMU/PCMA/DTMF, up to 2,000 CPS in the final clean report. |

## Advanced or Post-Beta Profiles

| Profile | Status | Reason |
|---------|--------|--------|
| Tuned high-CPS above 2,000 CPS | Advanced | Requires explicit tuning, hardware notes, and topology caveats. |
| Kamailio/OpenSIPS plus RTPengine | Investigation | Proxy de-scope audit passed; this is not a supported beta deployment shape. |
| Carrier SBC certification | Post-beta | Requires carrier-specific certification and security audit. |
| Browser/WebRTC edge | Post-beta | DTLS-SRTP, ICE, TURN, and browser interop are outside beta. |
| ICE/TURN NAT traversal | Post-beta | Current STUN support is limited address discovery, not ICE. |
| Recording/announcement/IVR media server | Post-beta unless completed | Media-core feature plan still lists gaps. |

## General Full-Media Beta Profile

The default beta performance claim is:

- Media mode: `MediaMode::Enabled`
- Codecs: PCMU (`0`), PCMA (`8`), telephone-event (`101`)
- Optional: comfort noise (`13`) only with `comfort_noise_enabled=true`
- Security: plaintext RTP or tested SDES-SRTP profile
- Target: stepped SIPp/media runs at 30, 100, 300, 1,000, and 2,000 CPS
- Success: at least 99.9% completed calls at the declared target, no stuck
  sessions, no unbounded memory growth, and published p50/p95/p99 setup
  latency
- Soak: 24-hour soak is waived for beta; the final 30-minute soak is accepted
  as the beta bar

Results above 2,000 CPS must be labeled as tuned or experimental unless they
use the same general profile and pass the same evidence bar.
