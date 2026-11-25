# Amazon Connect TURN over UDP

This candidate extends the published 0.5.0 WebRTC wrapper with an actual UDP
TURN data path. The `turn` 0.17.3 client owns authenticated allocation,
permission/channel establishment and refresh. The wrapper exposes the allocated
relay address to the ICE core as a socket and routes ICE, DTLS and RTP through
it. Relay-only gathering emits no host or server-reflexive candidates.

`ConnectConfig.turn` controls the credential source (`auto`, `join_ack`,
`turn_control_url`, or `disabled`), whether relay is required, the HTTPS request
timeout, and an optional exact control-host allowlist. `auto` prefers usable
JOIN_ACK credentials and otherwise requests them from the contact's
`MediaPlacement.TurnControlUrl`. Credentials are scoped to that contact, never
cached between calls. HTTP redirects are disabled and errors omit tokens,
URLs, and response bodies. In permissive mode, automatic provisioning failure
allows direct ICE; relay-only and explicit URL mode fail closed.

`WebRtcConfig.ice_servers = vec![]` disables the default Google STUN entry.
Connect can still append the per-contact TURN servers. Generic peers receive
only operator-configured ICE servers. `gather_timeout_secs` bounds DNS and all
allocation attempts; `turn_attempt_timeout_secs`, `turn_send_timeout_secs` and
`turn_close_timeout_secs` bound each address attempt, TURN send transaction, and
allocation release. Defaults for the latter three are two seconds. The existing
UDP bind address/port range also governs the TURN control socket. Relay-only
uses one socket; policy `all` may allocate one additional socket from that range.

Only `turn:` over UDP is implemented. `turns:` and `transport=tcp` are skipped;
relay-only fails if no usable UDP allocation succeeds. There is no silent TCP
to UDP substitution. This does not qualify deployments that allow only TCP 443.
Changing ICE servers or policy on a running peer is rejected; create a new peer
for new credentials. Allocation and permission refresh use the TURN client's
lifecycle; fetching replacement Chime credentials during a running contact is
not implemented.

The ICE core's default limit remains seven retries (eight transmitted checks
for an unanswered candidate pair). Its checking/failure deadline is separate.
Increasing the limit cannot make a forbidden direct-media destination reachable.
The new relay path supplies a reachable candidate instead.

For a runnable example proving relay allocation, ICE nomination, exact audio
packets in both directions, and cleanup, follow [TURN and ICE checks](TURN_ICE.md).
The example also checks direct ICE with `ice_servers: []`.

## Local validation

Run from this checkout:

```sh
cargo test -p rvoip-webrtc-stack --lib
cargo test -p rvoip-webrtc --test turn_relay_e2e
cargo test -p rvoip-amazon-connect --lib
cargo clippy -p rvoip-webrtc-stack -p rvoip-webrtc -p rvoip-amazon-connect --lib -- -D warnings
```

The relay suite starts an in-process TURN server, requires relay-only SDP,
exchanges media in both directions, checks the nominated candidate type, tests
bad credentials and an unavailable first server, and observes server allocation
removal after explicit close and drop. These are local protocol tests. Live
Amazon Connect and firewall qualification is recorded separately by the
Bridgefu consumer; a local test passing does not establish AWS compatibility.
