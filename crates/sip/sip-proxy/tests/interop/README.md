# Stateful Proxy Interoperability Gate

This directory owns the external-process qualification gate for the bounded
`rvoip-sip-proxy` transaction-stateful profile. Unit tests and raw loopback
peers remain necessary, but they do not replace interoperability with
independent SIP implementations.

## Pinned peers

Release runs use immutable image digests and record the image configuration,
runtime version output, and platform in the evidence bundle. Floating tags such
as `latest` are prohibited.

| Peer | Observed runtime version | Platform | Immutable image |
|---|---|---|---|
| Kamailio | 6.1.3, Debian Bookworm | `linux/amd64` | `ghcr.io/kamailio/kamailio:6.1.3-bookworm@sha256:26b26c61801d679ffbe54ea3597c38964a46c4bfe60fb6537c7eeacc576b0c92` |
| OpenSIPS | 3.6.7 (3.6 LTS line) | `linux/amd64` | `opensips/opensips:3.6@sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979` |

The OpenSIPS registry publishes a moving `3.6` label rather than a patch tag;
the digest above is the release input. The harness must additionally assert
and record `opensips -V` so an image refresh cannot silently change the tested
peer. Both reviewed digests are currently amd64-only. On an arm64 Colima host,
Compose therefore runs them through explicit `linux/amd64` emulation and
records that requested platform alongside Docker's host/runtime architecture.

## Topologies

Each scenario runs in both adjacent-hop orders:

```text
SIPp UAC -> rvoip proxy -> Kamailio or OpenSIPS -> SIPp UAS
SIPp UAC -> Kamailio or OpenSIPS -> rvoip proxy -> SIPp UAS
```

Both orders use real UDP, TCP, and TLS/SIPS sockets. The TLS cases use a
gate-owned test CA, peer-specific certificates, hostname verification, and
mutual trust roots. Disabling certificate verification is not acceptable
release evidence.

SIPp 3.7.7 verifies a CA chain but does not expose RFC 6125 hostname
verification. TLS rows therefore keep SIPp on local TCP behind two
gate-owned boundaries. The UAC boundary verifies the first proxy's exact DNS
SAN before releasing SIP bytes; the UAS boundary requires a verified client
certificate with the last proxy's exact DNS SAN and requires SNI for
`sipp.proxy.test`.

Real Kamailio and OpenSIPS terminate inbound mTLS and bind the verified client
certificate to the configured exact DNS SAN before relaying SIP. For outbound
traffic, each hands SIP over a gate-local TCP connection to a hostname-verifying
mTLS boundary. The boundary establishes CA-verified, exact-DNS-SAN mTLS to the
next hop before releasing the buffered SIP message.

This is an intentionally explicit composed profile. OpenSIPS 3.6 does not
document RFC 6125 hostname matching for its outbound TLS client. Kamailio's
outbound TLS event hook was also exercised against a valid-chain/wrong-name
server during development: the server received SIP application bytes before
the hook could enforce the identity decision. The release gate therefore
claims end-to-end verified mTLS through the boundary, not native outbound
hostname enforcement by either peer.

Every TLS row also runs a dedicated `sips-routing` SIPp exchange. Its OPTIONS
Request-URI, To, Contact, response Contact, and scenario marker use `sips:`.
The scenario-owned capture must observe the same SIPS Request-URI on both
allowlisted plaintext sides of the gate-owned boundaries, both real proxy Via
values at the UAS, TLS records on the external ports, and zero decoded
plaintext SIP on those external TLS ports. The final scenario result combines
that live wire evidence with the independently recomputed positive and negative
mTLS controls; TLS setup alone is not accepted as SIPS-routing evidence.

Ordinary TLS scenarios combine their scenario-bound plaintext SIP captures at
the gate-owned boundaries with encrypted application-data activity on every
external TLS hop. TLS connections are deliberately pooled, so a scenario does
not have to repeat a ClientHello, SNI, or certificate flight. Instead,
`tls-verifier-result.json` strictly aggregates every hashed scenario capture in
the row and requires the complete rvoip, independent-peer, and SIPp SNI and leaf
certificate sets. This preserves exact identity proof without inventing a
per-scenario handshake requirement that a healthy pooled connection cannot
satisfy.

The OpenSIPS TLS modules come from a reviewed derived image. Its exact base
digest, Dockerfile hash, four installed package versions, reviewed Debian
package hashes, installed module hashes, derived image ID, and amd64 platform
are captured from the running container in
`opensips-tls-image-provenance.json`. No image is pushed by this gate.

The lifecycle follows the existing local Asterisk/FreeSWITCH harness:

1. Validate Docker/Colima reachability and available ports.
2. Generate an isolated run directory and test PKI.
3. Start one pinned peer at a time.
4. Poll the peer's native health/version command and SIP OPTIONS readiness.
5. Start the Cargo-produced rvoip proxy executable by exact path.
6. Run the matrix, collecting SIPp statistics and packet captures.
7. Allow the 130-second natural transaction/response-retention drain. Timer
   J/K can precede the independent 64-second response-context retention, so a
   70-second drain is not valid evidence.
8. Assert rvoip retention counters and peer processes converge.
9. Capture inspect data, versions, configuration, logs, and packet summaries.
10. Stop only processes owned by the run and restore any pre-existing peer.

## Required scenarios

- Matched and unmatched CANCEL.
- CANCEL before and after provisional response, including UDP Timer J
  retransmission replay; TCP/TLS require the immediate matched-CANCEL `200`
  without inventing reliable-transport transaction retransmissions.
- Sequential and parallel forks.
- Multiple INVITE 2xx responses with end-to-end ACKs using each Contact and
  reversed Record-Route set.
- A delayed matching 2xx retransmission, with both 2xx responses ACKed, while
  the RFC 6026 client transaction remains in its Timer M Accepted window. This
  does not claim forwarding after transaction termination; that case must be
  discarded under RFC 6026.
- 6xx sibling cancellation.
- End-to-end 2xx ACK and hop-by-hop non-2xx ACK.
- Per-branch Timer C behavior.
- Transport failure and RFC 3263 failover.
- Via push/pop and response destination.
- Route, Record-Route, strict routing, loose routing, and an actual preserved
  SIPS Request-URI over verified external TLS.
- 401/407 challenge aggregation.
- Message bodies and exact Content-Length preservation.
- True stray-response discard.
- Capacity/overload response and complete post-retention cleanup.

## Evidence contract

Every matrix row records:

- source revision, tree fingerprint, and dirty-state fingerprint;
- peer image tag, digest, image ID, platform, and runtime version;
- exact generated configuration and command lines;
- transport, topology order, scenario, start/end timestamps, and exit status;
- SIPp statistics, rvoip logs, peer logs, and a distinct packet capture owned
  by each scenario (a row-wide capture cannot be reused as scenario evidence);
- packet assertions for Via, Route, Record-Route, CSeq/method, CANCEL, ACK and
  BYE dialog routing, response selection, bodies, and per-hop TLS application
  data, plus a strict row-level TLS SNI/certificate aggregate;
- public certificates and raw endpoint/boundary TLS logs, including positive
  hostname checks, wrong-name/wrong-CA controls, untrusted-client rejection,
  and exact verified peer fingerprints;
- pre-run, cooldown, and post-retention rvoip snapshots.

A missing peer, skipped topology order, insecure TLS mode, or unavailable packet
capture is a release-gate failure when external evidence is required.
TLS evidence containing a private key, symlink, or raw Docker inspect output is
also rejected. `tls-verifier-result.json` is derived from raw logs, public
certificates, each scenario's packet-evidence document, and the hashed scenario
captures; the release reporter must recompute it rather than trust a
harness-authored pass flag.

## Commands

Development smoke may select a bounded subset explicitly:

```bash
PROXY_INTEROP_PEERS=opensips \
PROXY_INTEROP_ORDERS=rvoip-first \
PROXY_INTEROP_TRANSPORTS=udp \
tests/interop/scripts/run.sh
```

The stable beta-policy entry point is:

```bash
tests/interop/scripts/beta_gate.sh
```

It requests both pinned peers, both adjacency orders, UDP, TCP, and verified
TLS; requires a clean source tree that remains byte-for-byte unchanged; uses
the 130-second natural drain; and fails fast. Until every requested transport
and protocol scenario is implemented, this command must fail rather than
silently reduce the matrix. Its machine result is `summary.json` using schema
`rvoip-sip-proxy-interop-v1`.
