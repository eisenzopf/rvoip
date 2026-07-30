# rvoip 0.3.4 Release Candidate Notes

Date: 2026-07-29

These notes describe the coordinated `0.3.4` candidate. They do not represent
a published release or a passing beta attestation until the exact committed
candidate completes the strict full gate and receives owner approval.

## Headline

All 44 publishable crates move together to `0.3.4`. The release adds an exact
inbound-admission terminal signal, completes the RFC 6026 INVITE transaction
updates used by the SIP stack, and introduces a bounded RFC 3261
transaction-stateful proxy profile updated by RFC 4320 and RFC 6026.

## Added

- `InboundAdmissionTermination` and exact-generation `watch` receivers notify
  applications of cancellation, remote end, or failure without polling or a
  global event subscription.
- Private INVITE client/server Accepted lifecycles retain matching 2xx traffic
  through Timer M and Timer L while preserving the public transaction enum.
- Timer M/L retention now uses compact records in the existing sharded
  lifecycle indexes and one manager-owned deadline scheduler. Active runners,
  timer factories, transports, and 128-slot command queues are released at the
  Accepted handoff; due work is generation-protected and capped at 1,024
  records per batch.
- Accepted M/L records and teardown J/K records use independent lazy bounded
  capacity lanes, so an accepted INVITE cannot consume the only slot required
  to process its own BYE.
- Stateful proxy response contexts support matched and unmatched CANCEL,
  cancellation latching, forked and late 2xx responses, ACK ownership,
  response aggregation, Timer C, strict/loose routing, SIPS, and exact response
  flow handling.
- Kamailio and OpenSIPS are mandatory real-process interoperability peers in
  both adjacency orders over UDP, TCP, and verified TLS.
- The beta report generator records revision-bound Asterisk, FreeSWITCH,
  Kamailio, and OpenSIPS attestations and fails closed on missing or skipped
  required rows.

## Fixed

- Renamed workspace dependencies, including the WebRTC and MOQT package
  aliases, are updated and validated during coordinated release preparation.
- Transactionless ACK transport metadata is bounded and eligible for exact
  cleanup after the protocol retention horizon.
- Transaction timer callback and state transition delivery is atomic and
  schedule-generation protected, eliminating the channel-saturation race that
  could strand an otherwise expired transaction.
- Stateless protocol-retention overload responses now inherit the configured
  server `Retry-After` policy across already-running transaction-manager
  worker clones.
- Server Timer L absorbs retransmitted INVITEs without driving cached 2xx
  replay, while later TU-supplied 2xx responses and matching ACK delivery
  remain available through the compact retained indexes.
- Client Timer M forwards every matching or additional 2xx without owning its
  ACK; proxy routes expire at M and endpoint routes promote in place to the
  existing compact late-2xx compatibility horizon.
- Via `received`/`rport`, packed Via popping, body preservation, route-set
  processing, and RFC 3263 failover paths have dedicated packet-level tests.
- The existing SCIM, vCon, Vapi, WebRTC, MOQT, and extension behavior from
  `0.3.2` and `0.3.3` remains part of the coordinated workspace.

## Proxy Claim Boundary

The candidate may be described only as an “RFC 3261 transaction-stateful
proxy profile, updated by RFC 4320 and RFC 6026,” after every applicable row in
the conformance matrix links to green executable evidence.

- Recursive 3xx Contact processing is not part of the claimed profile.
- Loop detection is disabled unless it can distinguish loops from spirals.
- Asymmetric Record-Route rewriting is outside the claim unless separately
  qualified.
- The claim does not cover every SIP extension, topology, or peer version.

## Compatibility

- Public source compatibility is compared against `v0.3.3`.
- RFC 6026 Accepted state remains private protocol state so exhaustive matches
  over the public `TransactionState` remain compatible.
- The admission terminal APIs are additive.
- Stateful proxy wire behavior changes are externally observable but ship as
  `0.3.4` by owner decision within the pre-1.0 release train.

## Limitations and non-claims

- General-user 10,000 CPS full-media capability is not claimed. The supported
  beta envelope remains bounded by the revision-specific 2,000-CPS real-media
  evidence and its documented runtime profile.
- Browser/WebRTC edge behavior remains outside the SIP beta claim; component
  availability does not imply qualified browser, ICE/TURN, or DTLS-SRTP
  interoperability.
- The proxy claim remains limited to the explicitly tested profile below.

## Required Qualification

Promotion requires the exact clean candidate revision to pass:

- all unit, integration, security, lifecycle, RFC, and public-API checks;
- Asterisk and FreeSWITCH signaling plus real audio matrices;
- Kamailio and OpenSIPS in both orders over UDP, TCP, and TLS;
- the SIPp ladder and strict-UA checks;
- three canonical 2,000-CPS runs with the recorded beta thresholds;
- one-hour monolithic and one-hour split soaks; and
- report verification, package inspection, SBOM, provenance, vulnerability
  policy, and publication dry run.

No prior exception or historical report qualifies `0.3.4`. Publication,
tagging, pushing, or deployment requires separate owner authorization.

Historical `0.3.2` disposition and evidence remain unchanged in
[`BETA_RELEASE_EXCEPTION.md`](BETA_RELEASE_EXCEPTION.md) and its immutable
release archive.
