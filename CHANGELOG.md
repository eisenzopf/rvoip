# Changelog

## Unreleased

### Release process

- Every release must now have a `CHANGELOG.md` entry. **Prepare release PR**
  refuses to run while `## Unreleased` is empty and moves its entries under
  the new `## X.Y.Z — YYYY-MM-DD` heading. Verification and publication
  reject a release without a non-empty section for its version.

### Breaking changes

- The public configuration structs are now `#[non_exhaustive]`, so later
  releases can add fields without a semver break:
  `rvoip_core::Config`, `rvoip_core::TenantQuotas`, `rvoip_sip::Config`,
  `rvoip_sip_dialog::api::DialogConfig`, `rvoip_websocket::UctpWsConfig`,
  `rvoip_quic::UctpQuicConfig`, and
  `rvoip_uctp::state::UctpCoordinatorCaps`.
- Code outside the defining crate can no longer build these with a struct
  literal, including the `..Default::default()` form. Start from a
  constructor and then set fields or call builder methods:

  ```rust
  // Before
  let config = Config { max_concurrent_setups: 64, ..Config::default() };

  // After
  let mut config = Config::default();
  config.max_concurrent_setups = 64;
  ```

  Use `Config::default()` (core), `Config::local` / `Config::on` or a
  profile constructor (SIP), `DialogConfig::new` or `Default`,
  `UctpWsConfig::new`, `UctpQuicConfig::new`, and
  `UctpCoordinatorCaps::default()`. `TenantQuotas` gains
  `with_max_concurrent_sessions`, `with_max_concurrent_recordings`, and
  `with_max_concurrent_ai_sessions`, so
  `TenantQuotas::default().with_max_concurrent_sessions(1)` replaces the
  literal. Reading fields and assigning to them is unchanged.

### SIP media options documentation

- `rvoip_sip::Config::playout` now documents the inbound jitter buffer in
  full: what it does, the `PlayoutConfig` knobs with defaults and units, its
  latency cost (~40 ms at the default depth), when to enable it (routes over
  the public internet, carrier trunks) and when to leave it off (LAN, lab),
  with compiled examples. `PlayoutConfig` and its fields in
  `rvoip-media-core` are documented to match. No defaults changed.
- `Config::rtcp_mux_required` is now the reference for RTCP behavior:
  periodic SR/RR is sent only when `a=rtcp-mux` is negotiated, and peers that
  decline mux get no RTCP quality statistics and may trip RTCP-based
  dead-media detection on SBCs.
- The rvoip-sip README gains a "Media options" section with a decision table
  naming the profile and settings to start from for each deployment shape.

## 0.3.12

### 0.3.12 release recovery

- The protected 0.3.11 publication stopped after 25 of 46 crates reached
  crates.io. No `v0.3.11` tag or GitHub release was created.
- `rvoip-core` now keeps its forward `rvoip-harness` test dependency path-only,
  so Cargo can package core before harness exists on crates.io. Release
  planning detects cycles involving versioned test dependencies before the
  first upload.

### Release workers move to AWS

- The `remote-release`, `remote-preflight`, and `remote-diagnostic` profiles
  now run their ephemeral workers on EC2 (`m5.large` / `m5.xlarge` /
  `m5.4xlarge`, gp3 root volumes) in a dedicated release VPC instead of
  Google Compute Engine. Resource classes are renamed `gcp-*` to `ec2-*`,
  the planner emits `aws_matrix` / `aws_shard_count`, and evidence, logs,
  and the performance prebuild cache live in S3 (`s3://`) rather than GCS.
- `scripts/release/aws_fanout.py` replaces `gcp_fanout.py`. It keeps the
  `prepare`, `verify`, and `early-failure-decision` contracts (schemas
  `rvoip-ec2-release-fanout-v1` / `rvoip-ec2-release-shard-v1`, treating
  `stopping`, `stopped`, `shutting-down`, and `terminated` as finished) and
  adds `user-data`, which renders the gzip-compressed EC2 user-data that
  writes `/etc/rvoip-release.env`, installs the reviewed startup and
  shutdown scripts, and registers the `rvoip-release-shutdown.service`
  checkpoint.
- The controller authenticates with GitHub OIDC role assumption; no cloud
  key is stored in GitHub. The GCP qualification pilot workflow and its
  startup script are removed.
- The release environment identifier becomes
  `rvoip-release-v6-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5-perf16-soaklong16-interop16`,
  so every environment-sensitive gate runs fresh on the first AWS
  qualification.
- The long-soak worker class, which runs the canonical 2,000-CPS gate, is
  sized at 16 vCPUs (`m5.4xlarge`) and must stay at 16 or more. On 8 vCPUs
  the sweep sits at its CPU knee: about 3 ms of CPU per call needs roughly
  5.65 cores at 2,000 CPS, and p99 setup latency swung 17 to 39 ms run to
  run on identical code. Transparent huge pages were ruled out with zero
  compaction stalls.
- Each long gate now owns a long-soak worker: three instead of two. The two
  hour-long soaks previously ran back to back on one worker for about
  2 h 20 min, which left the three-hour controller job about ten minutes of
  slack. Qualification now finishes about an hour sooner, at a peak of 132
  vCPUs.
- The Jambonz OSS interoperability pins move to the 0.9.11 release line:
  `sbc-inbound` `d0d8ba93b2f3f9d09e3877be6a434744305cac4d` and `sbc-outbound`
  `7099671e69342dac60e2ab3001c56b18820ee302`, with source tarball digests
  re-verified. The release check requires the pinned components to be the
  current upstream heads, and upstream had moved on from 0.9.9.
- The short-performance worker class moves to 16 vCPUs (`m5.4xlarge`). Its
  call-setup sweeps drive 2,000 CPS, the same load that put the long soak on
  its CPU knee: on 8 vCPUs the PBX media-server profile's 2,000-CPS point
  passed at 15.8 ms p99 in one run and fell to a 47.5% answer rate in the
  next, on identical code. Its gates use absolute thresholds, so no baseline
  was re-recorded. Peak fleet demand is 192 vCPUs.
- The shutdown checkpoint unit is ordered after `network-online.target`, as
  its script requires. Without the ordering, the network could stop while a
  cut-off worker was still uploading, and interop and the long soaks left no
  `PARTIAL` result when the early-failure cutoff stopped them.
- The Jambonz lab's MySQL fixture makes its seed files world-readable. EC2
  workers check out the candidate under `umask 077`, `COPY` kept the rvoip
  seed at 0600, and the MySQL entrypoint, which reads seeds as the `mysql`
  user, exited before the lab could start. When `compose up` fails, the lab
  now prints every container's logs so the receipt names the cause.
- The interoperability worker moves to 16 vCPUs (`m5.4xlarge`) and, with the
  proxy-interop workers, launches alongside the performance prebuild instead
  of after it. Interop reads no performance bundle; it is the fleet's longest
  serial chain, and three of its gates compile on the worker. On 4 vCPUs the
  SIPp listener's cold full-LTO release build overran its timeout and the
  libSRTP build took 28 minutes. Cold release builds on EC2 now get a
  45-minute timeout, and four interop estimates carry measured durations. Historical GCP qualification evidence under
  `crates/sip/rvoip-sip/docs/` is unchanged. See
  `docs/AWS_RELEASE_WORKERS.md`.

### Cloudflare Tunnel (Parley demo)

- `deploy/cloudflare/config.yml` and `scripts/run-cloudflare-tunnel.sh` front a
  localhost Parley process through `parley.rudeless.ai` (HTTP `/v1`, widget,
  desk, webhooks) and `parley-uctp.rudeless.ai` (UCTP WebSocket). SIP/UDP is
  not in the ingress. Requires a named tunnel or `CLOUDFLARE_TUNNEL_TOKEN`.

### UCTP conversation dispatch

- The UCTP coordinator dispatches `conversation.create`, `conversation.list`,
  and `conversation.close` to the substrate adapter (oneshot + Orchestrator)
  instead of dropping them. WebSocket, QUIC, and WebTransport adapters fulfill
  those envelopes when configured with an Orchestrator. `conversation.close`
  is a new C→S type; `conversation.opened` / `conversation.closed` remain the
  server replies. `session.invite` with an Open `cid` can attach to that
  Conversation via `start_session`.

### Vapi AI Participant (breaking)

- `VapiAdapter::attach_agent` no longer copies the caller's `participant_id`
  onto the Vapi Connection. It joins a distinct `Ai`/`Agent` Participant and
  originates under that id. `attach_agent_for_participant` accepts an existing
  AI Participant (rejected if that id is already `Human`).
- `VapiAgentCall::ai_participant_id()` returns the AI Participant.

### Orchestrator Participant role verbs

- `Orchestrator::set_participant_role`, `take_over`, and `hand_off` change
  voip-3 roles without moving Connections. `take_over` / `hand_off` leave at
  most one `Agent` in the Session; extra agents become `Observer`.
- `Event::ParticipantRoleChanged` (and `rvoip_core.participant_role_changed`
  on the cross-crate bus) fires when the role actually changes.

### SIP ingress budget and admission observer

- `SipListenerAuthPolicy::with_source_rate_limit` drops requests from any
  source address over its token budget before any other admission check,
  trusted trunks included, and answers nothing: a flood is not told it is
  heard. `SipRequestAuthorization::Dropped` carries that outcome through the
  transaction layer without a response.
- `SipListenerAuthPolicy::with_ingress_observer` reports every admission
  decision (`SipIngressEvent`: source, method, admitted/rejected/dropped) to
  a caller-supplied `SipIngressObserver`, so an edge can count what it
  refuses instead of reading it back out of logs.
- `rvoip::app::SipConfig::source_rate_limit` and `ingress_observer` expose
  both through the facade; they take effect with the trusted-trunk policy.

### SIP NAT traversal on the live RTP socket

- `TransportStunClient` in `rvoip-rtp-core` sends Binding requests through a
  running `RtpTransport` and reads the response from the transport event
  stream. `rvoip-sip` now discovers the public mapping after allocating each
  call's RTP transport and waits for that exact mapping before rendering offer
  or answer SDP, so the advertised port is the port media actually uses. A
  static `media_public_addr` remains authoritative; resolution, timeout, and
  response failures still fall back to the local media address. With STUN
  configured, initial SDP generation can wait up to 1.5 seconds.

### SIP DTLS-SRTP interoperability

- `UDP/TLS/RTP/SAVPF` is accepted as a DTLS-SRTP audio profile alongside
  `SAVP`, so media servers that answer with the WebRTC feedback profile no
  longer fail before the DTLS handshake.
- `Config` gains `DtlsSetupRole` (`actpass` by default, `active` or `passive`
  for endpoint and NAT interop), and `srtp_offered_suites` now drives the DTLS
  `use_srtp` profile list in the configured order. AES-256 SDES suites are
  rejected when DTLS-SRTP is selected because the DTLS stack supports only the
  AES-128 SHA1-80 and SHA1-32 profiles.

### WebTransport origin allowlist

- `UctpWtConfig` accepts an opt-in exact `Origin` allowlist. Missing,
  duplicate, and unlisted browser origins are rejected before the WebTransport
  CONNECT is accepted; non-browser behaviour is unchanged when no policy is
  configured.

### WebSocket media bridge browser candidates

- The WebSocket media bridge answerer binds `0.0.0.0` and passes mDNS `.local`
  candidates through instead of using the loopback WebRTC profile, so Chrome's
  anonymised IPv4 host candidates pair and ICE completes.

### First-frame media latency restored on the receive side

- The bounded-audio-delivery work added a paced decoded-playout queue with
  two delays in front of the first decoded frame of every stream: a 45 ms
  startup reorder hold applied unconditionally, and delivery to the
  application only on a 5 ms playout tick. Together they added up to
  50 ms before an application saw the first frame, where it previously saw
  it within the packet's own arrival. An application that answers a call
  the moment the first early-media frame is delivered, as vapi-central's
  node does, lost that race to its own 200 OK every time.
- A decoded frame that is already due now leaves at packet arrival instead
  of waiting for the tick; later frames still pace on the tick.
- The startup reorder hold now applies only once a sequence gap is
  observed among the pending packets. An unbroken first run carries no
  evidence of reordering and is released at once; from the first observed
  gap onward the 45 ms hold protects ordering exactly as before. The one
  behaviour given up: if the first two packets of a stream arrive
  backwards, the earlier one is now treated as late rather than reordered.
  Tests: `startup_releases_a_contiguous_first_packet_without_holding_it`,
  `startup_holds_only_once_a_gap_is_observed`.

### AMR-WB coverage ported from Thelve

- Thelve's vendored fixes for AMR-WB, an `AmrWb` arm on the SIP media
  stream and bridge admission by negotiated payload type rather than the
  codec-name table, were already present here through the SIP-core codec
  wiring. Their acceptance tests are now carried too: the cross-connection
  bridge admits an AMR-WB leg that reports its negotiated payload type and
  still refuses one that does not, and the SIP stream's descriptor and
  codec follow the SDP clock and fmtp in both RFC 4867 framings while a
  non-16 kHz-mono shape is refused before a codec is built.
- vapi-ref-harness carried a state-table override adding the UAS
  `Answering + DialogCANCEL -> Cancelled` transition for a matched CANCEL
  that wins final-response authorship while accept is entering `Answering`.
  The shipped table already contains that transition byte for byte; a
  state-table test now pins it alongside the `Ringing` and `EarlyMedia`
  cases so it cannot regress silently.

### Receive-side playout overflow no longer silences a dialog

- The paced decoded-playout queue added with bounded audio delivery holds
  at most 64 frames. When a burst overflowed it, the RTP event handler for
  that dialog terminated, so no audio was ever decoded again for the rest
  of the call. A bridge releasing media it buffered until commit produces
  exactly such a burst, which is why a caller receiving forwarded early
  media saw the final answer before any decoded audio. Overflow now drops
  the newest frame and keeps the receive loop alive, the same way a full
  application callback channel is handled; the first drop is logged and
  the rest are counted. Regression test:
  `decoded_playout_overflow_drops_frames_but_keeps_the_receive_loop_alive`.

### Two-phase bridge peer handoff

- `Orchestrator::prepare_transport_fenced_peer_handoff` and
  `prepare_peer_handoff` stage a replacement bridge and return a
  `PreparedPeerHandoff` that owns the staged routes and destination
  reservation. `commit_peer_handoff_with_timeout_and_receipt`,
  `commit_peer_handoff_with_timeout`, `commit_peer_handoff_with_receipt`,
  and `commit_peer_handoff` perform the bounded quiescence and ownership
  commit and return a `PeerHandoffReceipt` (previous and replacement
  bridge ids, retained, source, target, `committed_at`). Dropping or
  awaiting `abandon()` on a prepared handoff rolls back without touching
  the original bridge. The strict variant refuses a bridge without a
  transport delivery fence with `NotImplemented`. Applications that must
  do durable work and re-check authority between preparation and commit
  can now do so; `replace_bridge_destination` and its transport-fenced
  wrapper are unchanged and are implemented as prepare followed by
  commit, so there is one commit path.
- `Event::PeerHandoffCommitted` is now emitted. It was defined in 0.3.10
  but never published. It fires exactly once from the shared commit
  path, after the ownership switch and before the compatibility
  `ConnectionsUnbridged` and `ConnectionsBridged` events, and never on
  a failed or abandoned handoff. `VapiAgentCall::wait_shared` now
  resolves to `HandedOff` when the AI leg is retired by a handoff.
- Ported the downstream acceptance tests for the handoff, the Vapi
  existing-call attachment (credential isolation on a shared
  credential-free adapter, no provider-call creation, no fallback to
  creation on socket failure), and the QUIC transport fence (stale
  generation rejected at dequeue; three-client A–B to A–C cutover where B
  receives no post-handoff frame).

### Bridge cutover fence and glare classification

- The media-graph forwarding gate's cutover mutex was only ever taken by
  `set_enabled`; the sink worker checked the flag and then called `send()`,
  which reserves and commits in one step, so a frame parked waiting for
  target capacity could be published after quiesce. The unticketed legacy
  sink now reserves first, races that reservation against the quiesce
  signal, and re-reads the flag under the cutover mutex before committing.
  A worker either publishes strictly before the cutover or abandons its
  frame. Routes that buffer while disabled keep their send; ticketed peer
  routes already fenced on generation and are unchanged.
- A bridge-destination replacement that loses a race to a concurrent
  contender now reports `BridgeNotFound` at every detection point (retired
  peer ticket, quiescence observing a changed peer, or a commit whose
  generation moved), matching the registry lookup and the reservation
  step. Callers get one classifiable retry signal instead of three. A
  quiescence timeout and a data route that ended mid-replacement remain
  `InvalidState`; a bridge that was never transport fenced remains
  `NotImplemented`.
- `SessionState::inbound_audio_codecs` moved from the hot struct to the
  copy-on-write cold block. It is read once per offer/answer, and keeping
  it hot had regressed the SIP hot-layout tripwire.

### Added

- Added Jambonz OSS 0.9.9 as a mandatory external SIP interoperability peer,
  using the shared Endpoint, StreamPeer, and CallbackPeer PBX runner against
  the real SBC/B2BUA, registrar, Drachtio, Redis, MySQL, and RTPengine topology.
- Added `CallbackPeerBuilder::on_refer_accepted` for successful local
  acceptance of an inbound REFER, distinct from the existing callback for a
  remote peer accepting an application-originated transfer.

### Fixed

- Serialized RFC 3515 implicit-subscription NOTIFY requests for an exact REFER
  lifecycle so `100 Trying`, progress, and terminal status cannot race or
  overtake one another.
- Preserved an optional RFC 3892 `Referred-By` header unchanged into the
  referenced INVITE while retaining `Refer-To` as the only required transfer
  target header.
- Hardened sensitive diagnostics, certificate and path handling, CI cache use,
  release feature-bundle checks, and CodeQL publication policy.
- Pinned the canonical performance client's app-session dispatcher shape so
  exact-candidate results cannot change with the release worker's detected CPU
  count.
- Reconciled current performance results from authoritative per-worker output
  instead of nested provenance copies, staged the required high-density burst
  evidence into the generated evaluation, and made the one-hour 500-call split
  soak an explicit machine-validated release result. A missing authoritative
  result tree now fails through the structured gate path rather than an
  uncaught filesystem exception.
- Preserved intentional prior-release and qualification references when
  preparing candidate-aware documentation, while still advancing live crate
  dependency examples and the current workspace runtime marker.

### Interoperability and release evidence

- The Jambonz UDP/plain-RTP profile covers authenticated registration,
  PCMU/PCMA bidirectional calls, provisional/final signaling, hold/resume,
  RFC 4733 DTMF, CANCEL/487, rejection, blind transfer with ordered NOTIFY,
  replacement INVITE, either-side BYE, and cleanup across all three public SIP
  APIs.
- Jambonz-specific exclusions are G.729, AMR, TLS/SRTP, the
  RVoIP-as-B2BUA scenario, WebRTC, PSTN, application verbs, recording, HA, and
  load. These exclusions do not reduce the separately qualified RVoIP codec or
  transport features.
- Publication now requires synchronized release notes, changelog,
  interoperability, compatibility, and RFC documentation with the protected
  exact-candidate qualification receipt.
- Publication requires a fresh exact-candidate performance evaluation with
  three clean canonical 2,000-CPS passes, the complete performance/resiliency
  matrix, high-density media and one-hour soak evidence, regression comparison,
  and a machine-readable artifact index; July measurements are baseline history
  rather than 0.3.10 qualification evidence.

## 0.3.9 — 2026-09-05

This coordinated 45-crate release makes the carrier media and remote-endpoint
paths reachable through the public facade, adds deployment-oriented feature
bundles, and hardens security, codec negotiation, browser interoperability,
and protected release evidence. It incorporates the previously unpublished
`0.3.8-thelve.1` candidate described below into the stable release.

### Production remote SIP endpoint profile

- The built-in registrar can now require authenticated RFC 5626 outbound
  registrations on exact TLS/WSS flows. It processes `ob`, `+sip.instance`,
  and `reg-id`, rejects incomplete remote endpoints with `439`, and retains
  opaque process-local flow capabilities rather than dialing private Contact
  addresses.
- Registered-AOR origination carries an ordered set of verified exact routes
  through the facade, SIP adapter, dialog manager, authentication retries, and
  transport failover. A failed primary stream can advance to a secondary flow
  without losing flow identity.
- Exact connection close, expiry, unregister, replacement, and restart make a
  route unavailable. Replacement uses prepare/response/commit ordering, so a
  zero-wire response or a staged-flow close cannot discard or falsely promote
  the previous live route.
- `SipConfig::remote_endpoint_profile()` fails startup unless TLS, mandatory
  SRTP, registrar identity, and a reachable media address are configured. The
  process-local AOR-affinity and real-NAT qualification boundaries are
  documented in `docs/sip/REMOTE_ENDPOINT_PROFILE.md`.

### DTLS-SRTP on the SIP media path

- `rvoip-sip` can negotiate `UDP/TLS/RTP/SAVP` with SHA-256 certificate
  fingerprints and RFC 8842 setup roles when its `dtls-srtp` feature is
  enabled. `Config::srtp_keying = SrtpKeyingMode::DtlsSrtp` selects the new
  path; SDES remains the compatible default.
- DTLS 1.2 shares the call's RTP socket through RFC 7983 demultiplexing. The
  media transport is latched secure-only before the asynchronous handshake,
  the certificate fingerprint from DTLS must match authenticated SDP before
  contexts are installed, and stale call generations cannot receive keys.
- Release gates now prove a real two-endpoint SIP call, independent
  `webrtc-srtp` RTP/SRTCP interoperability, shared-socket handshake behavior,
  strict DTLS-enabled Clippy, and facade feature forwarding. The carrier and
  full facade bundles include DTLS-SRTP; the minimal SIP endpoint does not.
- This is a supported dedicated handshake path. The older compatibility
  constructors under `rvoip-rtp-core::api::{client,server,common}` remain
  fail-closed and are not silently redirected to it.
- RTP session, client, and server teardown now send BYE behind a Receiver
  Report as RFC 3550 compound RTCP. RVoIP no longer emits unnegotiated
  reduced-size BYE packets that its peer correctly rejects during teardown.

### Deployment-oriented facade feature bundles

- The `rvoip` facade now offers six additive `bundle-*` starting points for a
  SIP endpoint, carrier SIP, browser gateway, AI conversation gateway, the
  complete pure-Rust facade, and the complete facade with native codecs.
  Existing leaf features and the default `sip` selection remain unchanged.
- The bundle catalog is declared with the facade manifest, rendered into
  `docs/FEATURE_BUNDLES.md`, checked for documentation drift, tested once per
  bundle with default features disabled, and inspected at the resolved
  dependency-graph level.
- Codec features now propagate through both the SIP adapter and the shared
  media graph. G.711 is baseline; G.729 and both AMR variants are in the
  carrier and pure-Rust full bundles; native Opus is explicit in the browser,
  AI, and native-full bundles. A pure-Rust build can no longer acquire Opus
  accidentally through `rvoip-core` feature unification.
- Direct `rvoip-core` consumers that previously relied on its default features
  for Opus transcoding must enable the crate's `opus` feature (or
  `all-codecs`) explicitly. The `full` feature continues to select every
  mainline codec, including Opus.

### Codec feature-matrix hardening

- AMR-NB and AMR-WB capability registration now compiles only when the
  corresponding codec is enabled, keeping codec-free builds warning-clean
  without weakening AMR support.
- Capability inventory tests now cover AMR-only builds explicitly, and the
  release gate exercises codec-free, AMR, and `all-codecs` configurations so
  AMR, Opus, PCMU, and PCMA remain first-class negotiated codecs.

### Committed per-session SIP codec renegotiation

- `SipAdapter::renegotiate_media` now renders a one-shot codec offer for the
  exact call generation, waits for the peer's final re-INVITE result, and
  returns the codec and payload type actually committed from the SDP answer.
  It no longer changes the media graph optimistically after request dispatch.
- Rejected, timed-out, replaced, or unobservable re-INVITEs fail closed and
  retain the stable negotiated media generation. A rejected transaction can
  be retried without changing adapter-global codec policy or another call's
  offer.
- A live `SipMediaStream` keeps its identity and channels while a committed
  codec generation rebuilds both media pumps. Managed cross-transport graphs
  now receive complete codec descriptors, preserving dynamic payload types and
  fmtp during UCTP/SIP and WebRTC/SIP hot swaps.

### Awaitable media readiness

- `ConnectionAdapter::wait_for_stream` now provides a transport-neutral,
  cancellation- and deadline-aware alternative to application polling loops.
  Existing adapters inherit a bounded compatibility implementation and may
  override it with a native registration watch without changing callers.
- `Orchestrator::wait_for_stream` captures and revalidates the exact connection
  lifecycle generation. Missing connections, terminal teardown, replacement,
  adapter loss, cancellation, deadline expiry, and adapter query failure are
  distinct outcomes.
- `StreamSelector` filters by media kind, optional codec and direction, plus
  explicit registered, source-ready, or bidirectional readiness. SIP, WebRTC,
  QUIC, and WebTransport production-path tests now consume this public surface.

### Lossless RTP observation on UCTP transports

- `observe_rtp_datagram` and `ObservedRtpDatagram` expose validated RTP
  sequence, timestamp, SSRC, marker, CSRCs, parsed extensions, padding size,
  and padding-free codec payload without taking ownership of adapter internals.
- `UctpQuicAdapter::new_with_rtp_ingress_observer` and
  `UctpWtAdapter::new_with_rtp_ingress_observer` publish those packets with
  their authenticated core route before conversion to `MediaFrame`. Delivery
  is bounded and best-effort, so an unavailable observer never stalls audio.
- The existing constructors and payload-only media consumers are unchanged.
  Normal outbound `MediaFrame` forwarding deliberately starts a new RTP hop:
  marker is false, CSRC/extensions are empty, and padding is omitted. Exact
  reserialization is available only through `pack_observed_rtp_datagram`.

### ICE (RFC 8445) on the SIP path

- New crate **`rvoip-ice-core`**: a sans-io ICE agent and RFC 8489 STUN
  codec. The agent is handed packets and the clock and polled for
  transmissions, events, and deadlines — no sockets, no runtime — so role
  conflicts, nomination races, loss, restarts, and wrong-password handling
  are scripted deterministic tests (19 of them, over a virtual wire with a
  port-restricted NAT). The codec is verified against the RFC 5769 vectors,
  which are self-validating: FINGERPRINT covers every byte before it and the
  HMAC everything before that.
- `SipConfig::ice(SipIcePolicy::{Disabled, Lite, Full})` on the app builder,
  `Config::ice` on the coordinator. **Disabled is the default and is
  today's behavior byte for byte** — SDP without ICE attributes negotiates
  exactly as before, and a peer that declines ICE retires the runtime and
  proceeds on the SDP path.
- Offers and answers carry `a=ice-ufrag`/`a=ice-pwd`/`a=candidate` (and
  `a=ice-lite` for lite) when enabled; the peer's material is extracted
  from parsed SDP; RFC 8839 `ice-mismatch` (a middlebox rewrote c=/m=
  after the peer built its SDP) is detected and stands ICE down for that
  call rather than fighting the box that owns the path.
- One pump task per media session shuttles STUN between the agent and the
  RTP socket: the transport now forwards demuxed STUN datagrams as
  `RtpEvent::StunPacket` (both plain and SRTP receive paths — ICE sits
  below SRTP, so checks on a secured socket are forwarded, not rejected),
  and `RtpTransport::send_stun_bytes` is the one legitimate plaintext send
  on a secured transport, gated on the payload actually classifying as
  STUN. Nomination retargets media through the same `establish_media_flow`
  the SDP path uses.
- A full peer beside a lite one is controlling regardless of who offered
  (RFC 8445 §6.1.1); lite refuses to build without a reachable address.
- Scope honesty: single component (requires rtcp-mux semantics; component
  ids stay in the model so two-component is an extension), no TURN yet, no
  trickle over SIP, and the post-nomination re-INVITE (RFC 8839 §4.4) is a
  recorded follow-up — the media path itself is already correct from the
  retarget.

### Media quality reaches the application

- **`OperationalEventKind::Quality`** puts per-connection quality on the
  authoritative stream. An application that took the operational receiver has
  stopped reading the observational broadcast, so quality it never sees is
  quality it cannot act on — and a call degrading is worth reacting to while
  it is still up. Scaled to integers (hundredths) because the enum is `Eq`
  and floats are not; a negative or non-finite reading clamps to zero rather
  than wrapping into an enormous unsigned value.
- **MOS survives the SIP boundary.** `Event::MediaQualityChanged` gained a
  `mos` field. media-core computed the estimate all along; the adapter was
  discarding it with a comment saying it would keep doing so "until the
  ApiEvent grows a `mos` field".
- **`MediaStream::has_quality_measurement`** (default `true`) lets a
  transport say it has no measurement. `QualitySnapshot::default()` is all
  zeros, which reads as *flawless* rather than *unknown*, and the type had no
  way to distinguish them. `spawn_media_quality_sampler` now skips
  unmeasured connections instead of averaging a perfect score into the
  report.
- **`SipMediaStream` retains its last quality report**, so `quality_snapshot`
  returns the RTCP-derived measurement the adapter routed to it rather than a
  default. Before this it always returned zeros, which made the sampler
  unusable on the SIP path: polling it published flawless quality for every
  call forever.
- `RvoipAppBuilder::media_quality_interval` starts the sampler. Off by
  default.

### STUN-discovered advertised address

- `SipConfig::discover_advertised_addr(stun_server)` learns the reachable
  address at startup instead of requiring it configured, for a listener
  behind NAT whose public address is not known ahead of time.
- **Not ICE, deliberately.** RFC 8445 negotiates candidate pairs with
  connectivity checks, and a carrier SIP trunk does not offer it — the far
  end expects one reachable media address. What ICE's server-reflexive step
  buys on this path is knowing that address, which is a STUN binding request.
  Browser legs, where ICE genuinely applies, are served by the WebRTC
  transport.
- Fails closed: a STUN server that cannot answer fails the build rather than
  starting a listener that advertises a guess. A call that connects and
  carries no audio is harder to diagnose than a service that refused to
  start. A static `advertised_addr` wins over discovery.

### Carrier-grade media on the SIP path

- **`PlayoutBuffer`** (`media-core::processing::audio::playout`) smooths and
  conceals a decoded audio stream: frames are reordered onto the media clock,
  a short backlog absorbs burst arrival, and a frame that never arrives is
  synthesized rather than left as a gap. Concealment is repeat-with-fade —
  the cheap technique, named as such — which removes the click that dominates
  the artifact budget; a long burst fades to silence rather than repeating
  the same 20 ms indefinitely. RTP timestamp wrap is handled, without which a
  single wrap discards every later frame.
- Playout is driven by a local media-clock deadline, not by packet arrival.
  Excess depth opens a bounded drain valve to reconverge latency, while a
  long-baseline RTP/arrival comparison tracks remote oscillator skew. The
  release matrix holds depth bounded for one simulated hour at both +50 ppm
  and -50 ppm and proves G.711 PLC fires on the missing frame's deadline.
- `Config::playout` on the SIP config, `SipConfig::playout` on the app
  builder. Generic/local configs retain the compatibility default; the
  carrier/SBC profile and app listeners admitting a trusted trunk enable the
  carrier-safe default automatically. Controlled LAN labs may opt out.
- Note for anyone surveying this area: `media-core`'s
  `rtp_processing::jitter::JitterBuffer` is a stub — `get_packet` is
  `pop_first` and `flush_old_packets` clears the whole buffer. It has no
  callers. The real packet-level buffer is `rtp-core`'s
  `AdaptiveJitterBuffer`.

### SRTP reachable from the app builder

- `SipConfig::media_security(SipMediaSecurity::{Disabled, Preferred,
  Required})`. `rvoip-sip::Config` already carried `offer_srtp` and
  `srtp_required`, but no builder surfaced them, so an application had no way
  to ask for encrypted media. `Required` refuses plaintext fallback;
  `Preferred` carries the call in the clear when the peer declines, which is
  the case an operator most needs to know about.

### Trusted private identity and carrier signaling

- A trusted trunk's `P-Asserted-Identity` now reaches the inbound context as
  the distinct, redacted `InboundAssertedIdentity` field, with
  `SipTrustedTrunk` provenance. It is intentionally not generic `X-*`
  metadata, so an application cannot accidentally treat an untrusted value as
  carrier-authenticated caller identity.
- Surfaced **only** when the peer was admitted by trusted-trunk policy. RFC
  3325 makes PAI meaningful only inside a trust domain; from an unverified
  peer it is a forgeable header that looks authoritative, which is worse than
  its absence.
- Trusted trunks can opt in to a bounded private-header allowlist. The first
  supported carrier field is `P-Charging-Vector`; the default remains empty,
  unlisted fields are stripped, and PAI/PPI cannot enter through the raw
  header path.
- `OutboundCallBuilder::with_ppi` adds typed `P-Preferred-Identity` alongside
  typed PAI. Both identities are preflight-validated, redacted from
  diagnostics, emitted on the first INVITE, and retained byte-for-byte across
  401/407 authentication retries.

### N-way conferencing

- `Orchestrator::conference_create/join/leave/end/members` plus a
  `conference` module holding the mixer. Bridging is pairwise and does not
  generalize: a conference has to *sum* audio, and every participant needs a
  different sum with their own voice removed, or they hear themselves
  returned a packet late.
- One task per conference mixes on a 20 ms tick. The sum is computed once in
  `i32` and each member receives it minus their own contribution, so the
  work is linear in members rather than quadratic, and the result saturates
  rather than wraps — a wrapped sum is an audible click.
- Members keep their own negotiated codec in both directions and are
  resampled into and out of the conference rate, so a G.711 carrier leg and
  an Opus browser leg mix together without either renegotiating. A member's
  tap is owned by the member, so leaving tears the route down.
- The internal mix bus is canonical mono. Stereo members are downmixed before
  resampling and expanded to their negotiated interleaved layout before
  encoding; RTP timestamps advance by sample frames rather than scalar
  samples. A real stereo-Opus regression protects the ordinary browser leg.
- A member whose transport has closed is removed from the mix rather than
  retried; one member's undecodable packet is skipped rather than silencing
  the conference.
- `conference_set_contribution` silences a member's voice while leaving them
  hearing the mix — a supervisor monitoring a call. Silencing at the mixer
  rather than at the member's transport keeps the rest of the conference
  unable to tell anyone is listening, which is what monitoring means.

### AMR in the codec factory

- `CodecFactory::create_negotiated_codec(payload_type, encoding_name,
  sample_rate, channels, fmtp)` constructs a codec from its negotiated SDP
  identity. `create_codec` keys off the payload type alone, which is enough
  only for statically assigned codecs; AMR is dynamically assigned and its
  mode set arrives in `fmtp`, neither of which a payload type can express.
  Non-AMR names delegate to `create_codec`, so existing callers are
  unaffected.

### Per-recording sink factories

- `RecordingSinkFactory` opens one `RecordingSink` per recording, and
  `Orchestrator::register_recording_sink_factory` registers one under the
  same namespace as a plain sink, taking precedence over it.
- Why: a registered `RecordingSink` is a single shared instance. Two
  concurrent recordings on one name wrote into the same sink, and the first
  `stop_recording` closed it — so their audio mixed and the artifact was
  attributed to whichever stopped first. That is invisible with one call in
  flight, which is the shape the deterministic harness exercises, and wrong
  for any deployment recording more than one call at a time.
- `start_recording` resolves a factory first and falls back to a registered
  sink, so existing single-sink registrations behave exactly as before. An
  unregistered name still fails closed before any tap or quota work.

### Authoritative application ingress (RVOIP-22)

- `RvoipAppBuilder::authoritative_ingress(AuthoritativeIngressConfig)`
  installs the inbound admission gate and the single-consumer operational
  event stream **before** any adapter is registered — the ordering core
  requires and the convenience `build` could not express, because it
  constructed its Orchestrator, registered adapters, and only then returned.
- `RvoipApp::take_authoritative_ingress` hands both receivers to the owning
  application exactly once; `ingress_health` reports mode, core's stream
  health, and whether the runtime still admits new work; `drain(budget)` is
  a bounded terminal join point that reports honestly whether it finished.
- In authoritative mode the app no longer admits inbound connections on the
  application's behalf: every inbound connection is presented as an
  `InboundAdmission` ticket and the normalized event follows acceptance.
- A lagged observational receiver is recorded as degraded ingress instead of
  a warning that keeps serving — `admits_new_work` goes false so a readiness
  probe can fail. Losing the operational receiver degrades the runtime and
  stops admission, which core already enforced and the app now surfaces.

### Vapi barge-in reaches the media graph

- User-speech-start now flushes adapter-local audio and the downstream
  orchestrator media-graph sink queues in the same barge-in operation. The
  discarded graph frames contribute to `VapiMediaHealth::barge_in_dropped`
  and `rvoip_vapi_barge_in_frames_dropped_total`.
- The mock-transport acceptance test backpressures a real bridged caller sink,
  proves audio is parked in the graph, then verifies the speech event drives
  queue depth to zero and accounts for every graph frame dropped.

### Checked RTP/media boundary

- `rvoip_core::rtp_boundary` converts validated RTP packets to payload-only
  `MediaFrame`s with explicit negotiated codec/PT mappings and bounded payload
  allocation. A packet-preserving handle retains marker, CSRCs, extensions,
  padding, SSRC, and sequence identity when no transformation occurred.
- `RtpPacketizer` owns deterministic SSRC, wrapping sequence, and wrapping
  timestamp state. Mismatched frame kind or payload type fails before state
  advances; `Bytes` payloads remain immutable and shared for fanout.
- The provider-neutral `checked_rtp_boundary` example and concrete SIP,
  WebRTC, and UCTP gateway examples all use the same shared API. A dedicated
  fuzz target covers malformed packet-to-frame conversion under fixed input
  and payload bounds.

### Signature freshness

- `Sig9421Verifier` bounds envelope timestamps from above as well as below.
  A far-future `ts` produced a negative age, passed the `age > ttl` test,
  and stayed valid for as long as the sender chose. `DEFAULT_SIG_CLOCK_SKEW`
  (30 s) is the tolerated drift; `with_ttl_and_skew` makes it explicit.


## 0.3.8 — 2026-08-14

This coordinated 44-crate patch release brings AMR-NB and AMR-WB into the
codec set end to end — negotiation, transport, transcoding, and release
evidence — adds record-routed proxy interop to the qualification matrix, and
repairs SIPS dialog and opus-bridge edge cases found on the way.

### AMR codecs

- Add AMR-NB and AMR-WB with IF1 and IF2 interface formats, VAD1/VAD2 and DTX
  reaching the wire, receive-side interleaving reassembly, max-red redundancy
  scheduling with dedup, and CMR damping — bit-exact against the fetched
  TS 26.073/26.101/26.201 material, with no 3GPP sources in the repository.
- Negotiate and obey the SDP mode-set, prove every mode in a live call, and
  attest each rate in the release evidence; long-run soaks cover both
  variants.
- Admit dynamic codecs into the media graph by their negotiated payload type
  (`CodecInfo` now carries it), re-frame packet times AMR cannot accept
  (10 ms joins and 30 ms splits), and label emitted frames so the UCTP pumps
  stop stamping Opus's number on everything else.
- Prove AMR crosses SRTP in process and a QUIC datagram in both directions.

### SIP, proxies, and interop

- Generate a secure fallback Contact for every RFC 3261 §12.1.1 trigger, so
  secure dialogs answer with `sips:` at the TLS-advertised address while
  explicit Contact and plain-SIP behavior are preserved (issue #176). This
  also repairs rvoip-to-rvoip SIPS setup: `Dialog::from_2xx_response` refuses
  a secure dialog whose Contact is not `sips:`, which the old plain fallback
  tripped.
- Learn the UAC route set from the dialog-forming 2xx's Record-Route
  (reversed per §12.1.2), so in-dialog requests stop bypassing
  record-routing proxies; the UAS side reads it from the request.
- Add Kamailio and OpenSIPS registrar-proxy labs with TLS and SRTP through
  rtpengine, opt-in AMR-NB transcoding (the AMR-WB transcode failure is
  attributed to rtpengine), and a per-rate sweep bound to the gate catalog.
- Expose the profiled egress registration's coordinator for
  observation-only event subscriptions; the composite adapter remains the
  sole signaling and lifecycle owner.
- `Config` gains `with_amr_dtx`, `with_amr_auto_cmr`, and
  `with_amr_mode_set` builders (private fields — `Config`'s constructible
  shape stays frozen). DTX and auto-CMR are local media policy; only the
  RFC 4867 `mode-set` is negotiated.
- `CodecInfo` carries the payload type a transport negotiated
  (`payload_type: Option<u8>`). Code constructing `CodecInfo` literals adds
  one field on upgrade; `None` preserves the name-table behavior.

### Media graph and bridges

- Keep opus↔opus bridges passthrough when the two legs numbered opus
  differently: the payload type is a per-leg SDP artifact, so the bypass
  compares name, rate, and channels, and passthrough restamps the sink's
  payload type on egress.
- Make a barge-in flush empty the re-framing accumulator as well as the sink
  queues, so no pre-interruption audio or dead-timeline timestamp survives
  into the first post-flush frame.
- Reach the opus and all-codecs feature sets from the rvoip facade.

### Qualification

`0.3.8` requires a fresh `remote-release` qualification bound to the updated
gate catalog, which adds the AMR per-rate sweep, the proxy-PBX media family,
and the AMR fuzz targets. Because the aggregate is bound to the catalog hash,
no `0.3.7` evidence qualifies this release.

## 0.3.7 — 2026-08-06

This coordinated 44-crate patch release hardens voice-AI and WebRTC media under
backpressure, exposes inbound SIP auth/context on the app facade, and repairs
SIP/WebRTC edge cases that dropped audio, DTMF, or late tracks.

### Vapi and media reliability

- Bound inbound/outbound audio queues so bursts and uplink stalls no longer kill
  the session; keep the RTP clock advancing across underruns and re-converge
  jitter depth on renegotiation.
- Adaptive jitter target, working inbound catch-up, and a symmetric outbound
  drain valve; flush stale playout audio on barge-in.
- Move WebSocket writes off the media loop, isolate control from media
  backpressure, and attribute media logs and health telemetry per call
  (`VapiMediaHealth`, current depth vs high-water, catch-up blocked ticks).

### WebRTC, Connect, and SIP

- Preserve media and unbind under driver backpressure; tolerate WebRTC startup
  backpressure without evicting Connect media routes or sinks.
- Allow primary audio and DTMF when a peer never negotiates MID; attach late
  remote audio tracks; bound per-peer UDP allocation; preserve remote codec
  preference order.
- Route wildcard contacts via the observed source address.
- Surface listener auth and inbound context policy on `SipConfig`
  (`tenant`, `trusted_trunk`, `capture_headers`) so facade apps can do
  DID-based routing and trunk admission.

### Release and workspace

- Publish the exact qualified candidate, keep qualification checkout on `main`,
  and allow signed ancestor release publication when attestation requires it.
- Inherit remaining third-party and `rtc` dependency pins from the workspace so
  version bumps stay single-source.

### Qualification

`0.3.7` requires a fresh, source-bound strict full-beta PASS. Historical
`0.3.2` exception, `0.3.4` carry-forward, and prior `0.3.6` qualification
evidence do not qualify it.

## 0.3.6 — 2026-08-02

This coordinated 44-crate patch release moves full release qualification onto
ephemeral GCP workers, repairs remote gate false failures, and lands SIP/core
correctness fixes needed for reliable attestation.

### Release qualification

- Run complete release qualification on ephemeral GCP workers with parallel
  performance, soak, proxy-interop, and diagnostic profiles.
- Cache exact performance build bundles, stream large artifacts from disk, and
  reuse selective evidence only when digests match.
- Reject failed candidates before deferred gates finish; accelerate long soaks;
  harden burst RSS and FreeSWITCH/PBX readiness checks.
- Automate active release metadata updates in `README.md`,
  `BETA_RELEASE_CHECKLIST.md`, and `RELEASE_NOTES_NEXT.md`.

### SIP, core, and security dependencies

- Publish the established event only after ACK; consume non-2xx ACK at the write
  boundary; tolerate legal final-response retransmission in soak evidence.
- Preserve cross-crate event semantics and make filtered message pagination
  deterministic.
- Send browser DTMF on the negotiated audio source.
- Upgrade jsonwebtoken, OpenTelemetry, and SIP terminal UI dependencies; remove
  the legacy AWS rustls adapter.

### Qualification

`0.3.6` requires a fresh, source-bound strict full-beta PASS. Historical
`0.3.2` exception and `0.3.4` carry-forward evidence do not qualify it.

## 0.3.5 — 2026-07-30

This coordinated 44-crate patch release hardens security and media state,
completes transactional SIP renegotiation, exposes symmetric-RTP NAT policy on
the high-level APIs, and makes Tokio the sole WebRTC runtime.

### Security and media

- Fail closed for placeholder AES-GCM profiles and unsupported DTLS
  construction; incomplete profiles cannot be advertised or negotiated.
- Correct RTP padding and RFC 8285 extensions, RTCP LSR/compound parsing, and
  loss/jitter accounting across rollover, gaps, duplicates, and reordering.
- Separate inbound/outbound and per-SSRC SRTP/SRTCP state, authenticate before
  committing replay state, and cover the result with RFC vectors and pinned
  libSRTP interoperability.
- Generate directional SDES answer keys and accept safely unpadded AES-256 key
  material in compatible mode with secret-safe diagnostics (issue #46).
- Preserve the 0.3.x `Config`, `Event`, `SessionError`, state-table, and
  negotiated-media shapes while exposing new auth, SDES, and renegotiation
  details through bounded additive diagnostic/runtime APIs.

### SIP, codecs, and WebRTC

- Make hold/resume, re-INVITE, UPDATE, delayed offers, authentication retries,
  retransmissions, rollback, and media application transactional and
  exact-generation owned.
- Expose `SipNatConfig` and `SymmetricRtpPolicy` through `EndpointBuilder` and
  `StreamPeerBuilder` (issue #50).
- Use the real Opus backend, make codec names ASCII case-insensitive, and stop
  advertising unavailable Opus or G.722 implementations.
- Remove Smol/async-std runtime support from WebRTC and correct the confirmed
  Chromium audio/SSRC/simulcast SDP regression.
- Preserve bounded, sharded, keyed/no-scan SIP lifecycle paths and make the
  registrar and coordinated workspace pass strict release linting.

### Qualification

`0.3.5` requires a fresh, source-bound strict full-beta PASS. Historical
`0.3.2` exception and `0.3.4` carry-forward evidence do not qualify it.

## 0.3.4 — 2026-07-29

This coordinated 44-crate patch release adds exact inbound-admission terminal
notification, completes RFC 6026 INVITE Accepted lifecycles, and introduces a
bounded RFC 3261 transaction-stateful proxy profile updated by RFC 4320 and
RFC 6026.

### Added

- Exact-generation inbound admission termination notification for cancelled,
  remotely ended, and failed source legs without polling.
- Compact, sharded Timer M/L retention driven by the existing manager-owned
  deadline queues rather than per-transaction runners and sleeper tasks.
- Matched/unmatched proxy CANCEL handling, fork response contexts, multiple
  and late 2xx forwarding, ACK ownership, Timer C, response aggregation, and
  strict/loose routing coverage within the documented bounded profile.
- Fail-closed Kamailio/OpenSIPS interoperability and four-peer beta-report
  attestation tooling for future strict full-beta runs.
- `rvoip-release-carry-forward-attestation-v1` release verification.

### Fixed

- Atomic, generation-protected transaction timer firing removes a saturated
  command-channel race that could strand expired transactions.
- Accepted transactions shed active runners, transports, command queues, and
  active-only locks while preserving RFC retention and exact cleanup.
- INVITE retransmission, late response, Via/route, RFC 3263 failover, and
  stateful-proxy response behavior have focused regression coverage.

### Qualification

The owner approved `0.3.4` with a transparent carry-forward disposition. The
full `0.3.4` beta, four-peer interoperability matrix, and long soaks were not
rerun. Current evidence is the complete workspace release verification and one
clean revision-bound canonical 2,000-CPS/65,000-call real-media PASS. The
immutable `0.3.2` owner-approved exception remains historical background with
strict status `NON-RC` and is not relabeled as a current beta PASS.

## 0.3.3 — 2026-07-29

This unified patch release corrects the vCon wire model, Session-finalization
path, signatures, content hashes, stores, and documentation against
`draft-ietf-vcon-vcon-core` commit
`2342aba64bdb71d9e80ab6e274a3921e2b1c769e`.

### Fixed

- End-of-Session emission now converts snapshots into the canonical
  `rvoip-vcon` model, validates them, serializes with serde, and suppresses
  persistence/`VconReady` on conversion, validation, or serialization failure.
  Inline dialog, analysis, and attachment bodies are preserved as Base64Url.
- The container now uses vCon `0.4.0`, durations in seconds, required analysis
  vendor/encoding data, attachment placement and purpose fields, URL/hash
  dependencies, complete dialog/party fields, and mutually exclusive
  redacted/amended lineage.
- vCon signatures now use JWS General JSON Serialization with appendable
  signatures and certificate references. Compact JWT serialization is no
  longer emitted as a signed vCon.
- New store handles use the specified
  `sha512-<unpadded-base64url-digest>` content hash in memory and PostgreSQL.
  The typed `VconStore` contract exposes the stored content hash, and identical
  canonical documents hash identically across memory and PostgreSQL. Existing
  persisted legacy hashes are not rewritten.
- Federation documentation no longer assigns semantics to reserved core
  `group`; sibling-vCon grouping is deferred to a future named extension
  declared in `extensions[]`.
- Documentation now states the shipped security boundary: core emission is
  unsigned, JWS signing is explicit, JWE is absent, and lineage types do not
  perform redaction.

### Added

- Canonical draft example/schema conformance coverage and a dedicated vCon CI
  job, including live PostgreSQL store tests and affected integration
  boundaries.
- Hash- and commit-bound `--targeted-delta-attestation` release verification.
  It retains unified manifest, workspace compile, and package checks while
  honestly recording that broad beta/workspace test/doc suites were not
  rerun. The approved targeted matrix is rerun and live PostgreSQL evidence is
  machine-verified.

### Breaking vCon changes

- `sign_jws` now accepts a certificate reference and returns `SignedVcon`;
  `append_signature` adds another signer, and verification accepts the General
  JSON form. HMAC algorithms are rejected for this certificate-bound API.
- Dialog `duration_ms` becomes `duration` in seconds. The model adds the
  standard party, dialog, analysis, attachment, extension, and critical
  fields. The undeclared Party `role` field is removed; the core `type` field
  classifies parties (for example, `person`, `bot`, or `organization`), while
  role semantics require a declared extension.
- `redacted: Vec<RedactionRecord>` becomes one optional `Redacted` object and
  gains the mutually exclusive optional `Amended` object.
- Core vCon analysis vendors and attachment placement become required;
  attachment `note` becomes `purpose`; party `did_or_stir` splits into `did`
  and `stir`; and snapshot encoding is fallible. The core byte-store `put`
  contract now also receives `ConversationId` and exposes
  `list_for_conversation` so sibling vCons are linked in index metadata rather
  than through the reserved `group` parameter.

## 0.3.2 — 2026-07-29

This unified release advances the reusable Bridgefu 1.0 foundation across all
44 publishable workspace crates.

### Added

- Hash-bound release-exception reporting for the owner-approved 0.3.2
  candidate, with the strict 106/108 gate result and NON-RC qualification
  preserved rather than rewritten.
- Complete authenticated-principal propagation and ownership checks across
  SIP, WebRTC, UCTP, routes, and operational events.
- Transport-neutral `DataMessage`, arbitrary WebRTC DataChannels, SIP MESSAGE,
  typed initial SIP headers, DTMF, and correlated transfer outcomes.
- Single-consumer `MediaGraph` with directional routes, codec-group
  transcoding, bounded fanout, snapshots, drops/evictions, and metrics.
- Dormant prepare/bind/activate lifecycles for SIP, WebRTC, and Amazon Connect,
  including owned cancellation, terminal events, and bounded drain.
- SIP outbound activation receipts now linearize after the exact session is
  active. Established teardown waits for the peer's successful final BYE
  response while still reclaiming local state on timeout or rejection.
- UCTP 0.2 complete-RTP routing, authenticated raw QUIC/WebTransport sessions,
  virtual publishers, direct-listener limits, and exact cleanup.
- `rvoip-moq` draft-19/MSF-01/LOC-03 publisher, subscriber, origin, relay,
  authorization, compatibility, reconnect, health, and drain abstractions.
- Configurable symmetric RTP, advertised SIP/RTP addresses, RFC 3581 `rport`,
  WebRTC ICE server/NAT policy, and per-exchange WHIP/WHEP versus WS gathering.
- Developer-preview `rvoip-vapi` bidirectional WebSocket agent adapter, exposed
  by the facade's opt-in `vapi` feature and included in `full`.
- High-level `rvoip::app` voice-only admission for SIP or WebRTC customers,
  including transport-neutral accepted-call events, startup-safe event
  retention, explicit SIP/RTP advertisement, and example 14's shared Vapi
  agent server.

### Fixed

- SCIM provisioning now generates policy-safe bootstrap passwords without
  random failures from repeated/sequential characters or username overlap.

### Breaking protocol changes

- UCTP media datagrams now carry a complete RTP packet after the UCTP header.
- Wire-incompatible MOQT draft changes are semver-breaking at the
  `rvoip-moq` compatibility boundary.
- `rvoip_core_traits::connection::Transport` adds the `Vapi` variant; downstream
  exhaustive matches over this public enum must add a corresponding arm.
- `rvoip::app::AppEvent` adds the `InboundCallAccepted` variant; downstream
  exhaustive matches over this public enum must add a corresponding arm.

The private WebRTC/RTC TURN candidate and the dynamic moq-rs publisher-lease
candidate remain outside the consumed dependency graph until project-owner
review. No upstream submission is authorized by this changelog.
