# rvoip-sip Beta RFC Compliance Matrix

Date: 2026-05-25

This matrix is intentionally conservative. It records the beta claim level,
not the most optimistic interpretation of parser or module coverage.

| RFC | Area | Beta status | Evidence required before beta | Known gaps |
|-----|------|-------------|-------------------------------|------------|
| RFC 3261 | SIP core | Partial | Method, transaction, dialog, transport, registration, proxy-routing audit | Full audited checklist still missing. |
| RFC 3262 | PRACK/100rel | Partial | UAC/UAS, 420 negative, B2BUA bridge, PBX interop | Broader reliable provisional interop needed. |
| RFC 3263 | Server location | Supported | Resolver unit tests plus DNS interop profile | Failover matrix needs publication. |
| RFC 3264 | SDP offer/answer | Supported | Hold/resume, codec intersection, glare tests | Advanced media changes not beta-scoped. |
| RFC 3325 | Asserted identity | Partial | Header policy tests and trusted-network docs | No carrier certification claim. |
| RFC 3515 | REFER | Supported | Blind transfer and REFER/NOTIFY progress interop | Attended transfer orchestration is primitives only. |
| RFC 3581 | rport | Supported | Dialog/transport tests and PBX interop | NAT matrix incomplete. |
| RFC 4028 | Session timers | Partial | Refresher/uac/uas tests and PBX interop | Full edge-case audit needed. |
| RFC 4475 | SIP torture tests | Supported with exclusions | CI gate and exclusion list | Exclusions must be documented. |
| RFC 5626 | SIP outbound | Partial | REGISTER contact params and flow keepalive tests | Multi-flow behavior not beta. |
| RFC 6086 | INFO | Supported | INFO and DTMF scenario tests | INFO package registry not complete. |
| RFC 7118 | SIP over WebSocket | Partial | WS tests | WSS outbound and browser/WebRTC are post-beta. |
| RFC 8866 | SDP | Supported | Parser/serializer and generated SDP tests | WebRTC attributes are parser-only unless wired higher. |
| RFC 3550 | RTP/RTCP | Supported | RTP loopback, audio round trip, long-run media tests | Full RTCP feedback is not beta. |
| RFC 3711 | SRTP | Partial | SDES-SRTP test matrix and PBX interop | DTLS-SRTP is not beta. |
| RFC 5764 | DTLS-SRTP | Post-beta | None for beta | Do not claim. |
| RFC 8445 | ICE | Post-beta | None for beta | Do not claim. |
| RFC 8489 | STUN | Partial | Startup address discovery tests | Not ICE connectivity checks. |
| RFC 8656 | TURN | Post-beta | None for beta | Do not claim. |

## Required Audit Work

- Add one checklist row per RFC section that affects beta behavior.
- Link each row to a test, SIPp scenario, interop result, or explicit
  de-scope note.
- Fail the beta release if a release-note claim has no matrix evidence.
