# session-core Topology Profiles

Use these profiles when describing support or deciding which examples to run.

| Profile | Status | Shape | Required configuration | Limits |
|---------|--------|-------|------------------------|--------|
| `local-lab` | Supported | local examples on one machine | local IP/ports | Good for API and state-machine smoke tests only |
| `lan-pbx` | Supported in common cases | reachable SIP/RTP endpoint on LAN | bind IP, advertised SIP address, media address | Assumes direct reachability |
| `asterisk-udp` | Validated | Asterisk PJSIP over UDP/RTP | Asterisk `.env`, UDP users, advertised IP/media IP | Current plain-RTP PBX baseline |
| `asterisk-tls-srtp-reachable-contact` | Validated | TLS signaling with endpoint TLS listener and SDES-SRTP | TLS cert/key or generated dev cert, TLS CA/insecure dev setting, SRTP required | Asterisk must route to reachable Contact |
| `asterisk-tls-srtp-registered-flow` | Validated with profile requirements | inbound requests reuse registration TLS flow | `ASTERISK_TLS_CONTACT_MODE=registered-flow-symmetric` or RFC 5626 mode | Requires PBX profile support for flow reuse |
| `freeswitch-internal` | Planned | FreeSWITCH/Sofia internal profile | TBD | Not validated yet |
| `proxy-rtpengine` | Planned | Kamailio/OpenSIPS in front of RTPengine | TBD | Not validated yet |
| `carrier-sbc` | Partial / future | public provider/SBC | outbound proxy, DNS, TLS, NAT settings likely | No certification claim |
| `webrtc-edge` | Future | browser/WebRTC edge | ICE, DTLS-SRTP, WebSocket SIP likely | Not supported yet |

## REGISTER lifecycle notes

Supported today:

- REGISTER and unregister.
- Digest auth retry.
- 423 Min-Expires retry.
- Asterisk UDP/TLS registration examples.
- Registered-flow modes in the Asterisk lab.
- Outbound flow failure can trigger a debounced re-register.

Remaining lifecycle work:

- REGISTER through outbound proxy.
- Service-Route and Path behavior.
- Multi-contact and multi-flow registration.
- Refresh timing policy hardening.
- Flow failure recovery under reconnect churn.
- Clean unregister behavior during unstable reconnect loops.
