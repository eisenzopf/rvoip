# session-core Compatibility Matrix

This matrix separates validated behavior from planned interoperability work.

| Target | Status | API surface | Transport / media | Coverage | Notes |
|--------|--------|-------------|-------------------|----------|-------|
| In-process examples | Supported | `StreamPeer`, `CallbackPeer` | local UDP/RTP | call setup, answer, teardown, DTMF, hold/resume, transfer examples | Useful as smoke tests, not external interop evidence |
| Local multi-process examples | Supported | `StreamPeer`, `CallbackPeer`, `UnifiedCoordinator` | local UDP/RTP | audio, bridge, registration, PRACK, CANCEL, NOTIFY | Run with `examples/run_all.sh` |
| Asterisk UDP/RTP | Validated | `StreamPeer` | UDP/RTP | REGISTER, call, hold/resume, DTMF, CANCEL, blind transfer | See `examples/asterisk` |
| Asterisk TLS/SDES-SRTP | Validated | `StreamPeer` | TLS + SDES-SRTP | registration, call setup, hold/resume, DTMF, blind transfer, audio verification | Requires matching PJSIP TLS/SRTP profile |
| Asterisk registered TLS flow | Validated with profile requirements | `StreamPeer` | TLS + SDES-SRTP | registered-flow reuse, keep-alive evidence, audio verification | Requires Asterisk routing inbound requests over the registration flow |
| Asterisk UDP/RTP | Validated | `CallbackPeer` | UDP/RTP | registration, callback hooks, reject, hold/resume, DTMF, blind transfer | See `examples/asterisk_callback` |
| Asterisk TLS/SDES-SRTP | Validated | `CallbackPeer` | TLS + SDES-SRTP | callback hook coverage plus SRTP tone analysis | Extended tests gated by env var |
| FreeSWITCH/Sofia | Planned | TBD | UDP/TCP/TLS, RTP/SRTP | not validated yet | Next PBX target |
| Kamailio/OpenSIPS + RTPengine | Planned | `UnifiedCoordinator` likely | proxy signaling plus media relay | not validated yet | Intended to flush proxy and topology assumptions |
| Carrier SBC | Partial / not certified | `StreamPeer`, `CallbackPeer` | provider-specific | basic SIP pieces exist, no certification | Needs outbound proxy, Service-Route/Path, SRV/NAPTR, and topology hardening |
| WebRTC edge | Not supported yet | future gateway/orchestrator | ICE + DTLS-SRTP | not implemented | SDES-SRTP support does not imply WebRTC support |

## Release gate

Current external release evidence is the Asterisk StreamPeer and CallbackPeer
suite. FreeSWITCH, proxy/RTPengine, carrier SBC, and WebRTC rows must not be
described as validated until matching automated or documented manual scenarios
exist.
