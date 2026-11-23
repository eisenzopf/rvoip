# rvoip 0.5.0 Release Notes

These notes describe the coordinated 48-crate `0.5.0` release. The latest
published release is `0.4.0`. Protected publication requires qualification
of the exact source commit; the signed qualification artifact and the GitHub
release identify that commit, the workflow run, the complete gate inventory,
the measured performance and the publication result.

## Headline

`0.5.0` is a breaking minor release about carrier-scale throughput and
overload behaviour, and it moves the whole dependency set forward.

- **Media executor v2.** A new opt-in executor runs media on dedicated,
  CPU-pinned lanes sized by a host plan. Installing
  `MediaExecutorConfig::v2()` turns it on; `v1()` stays the default, so
  nothing changes until an application opts in.
- **SIP overload hardening.** TLS flows apply back-pressure instead of
  dropping responses. New calls can be shed with a stateless 503 before any
  state exists, and overload rejections carry `Retry-After`. Hangup races
  end calls cleanly.
- **An RFC 7501/7502 benchmark gates every release.** From `0.5.0`, release
  qualification confirms rvoip's recorded Session Establishment Rates and
  RFC 6076 latencies on a pinned testbed.
- **Webex BYOVA and Amazon Connect A2A.** Two contact-centre adapters connect
  Webex Contact Center virtual agents and Amazon Connect external agents to
  rvoip.
- **Rust 1.95 and current dependencies.** The minimum supported Rust version
  is 1.95, and the upgrades held back in `0.4` land, including nom 8 and
  axum 0.8.

Applications upgrading from `0.4.0` should read the Breaking changes section
of the [changelog](../../CHANGELOG.md) before bumping. Each entry there ends
with a one-line migration.

## Media executor v2

`MediaExecutorConfig::v2()` turns on every switch of the new executor. The
per-call task graph, `v1()`, remains the default.

- Media runs on independent lanes, each with its own scheduler, timer and
  I/O driver, pinned to CPUs by a host plan.
- Receive loops and paced sends run inside one poller per lane. Inbound SRTP
  is authenticated, decoded and delivered inline on the socket owner, and RTP
  is sent synchronously.
- Lanes wake on a 2 ms tick and keep their deadlines in a timing wheel.
  SRTP, RTCP and receive statistics stay with the socket owner, and per-call
  hot state is packed into a few cache lines.
- Per-call sends and receives fire from a per-lane call table. The SIP
  inbound pump runs fused on the socket owner with zero-copy playout.
- Media graphs that bridge a SIP leg to an in-process AI run on that leg's
  lane, so caller audio reaches the AI session without a task hop.

The changelog gives each switch's measured CPU-per-call effect. The
benchmark DUT runs executor v2, so the release benchmark measures it under
load.

## SIP overload and call lifecycle

- **TLS back-pressure.** A slow TLS peer slows the flow instead of losing
  responses or having its connection closed.
- **Transaction layer.** Deadlocks between the transaction layer and the
  transaction user are fixed.
- **Stateless admission.** New dialog-forming INVITEs can be shed with a
  stateless `503` before any transaction or dialog exists. Overload
  rejections carry `503 Service Unavailable` with `Retry-After`.
- **Hangup.** Hangup and crossed-BYE races end calls cleanly. Ownerless
  sessions are swept, and established calls hang up from the hold, mute and
  bridged states.
- **Gateways.** Outbound peer leases tie an outbound call to a live peer
  connection. A trusted proxy's accepted TLS flow is reused for in-dialog
  requests. Inbound contexts carry the observed peer address and caller
  number.
- **Protocol coverage.** UAS capabilities are configurable and advertised in
  OPTIONS. Message bodies follow RFC 5621. A single-address-family endpoint
  can run on IPv6.
- **Hot paths.** Signaling and media hot paths copy, lock and scan less per
  call.

`Require: replaces` is now answered with `420 Bad Extension`, since rvoip
does not perform RFC 3891 replacement itself. The changelog explains how to
re-enable it for an application that does.

## RFC 7501/7502 benchmark

`0.5.0` is the first release gated on an RFC 7501/7502 SIP benchmark. The
device under test is `rvoip-bench-uas`, a terminating UAS on `CallbackPeer`
that echoes media through executor v2's inline path. It is not published.

- **What it measures.** The Session Establishment Rate R is the highest
  attempt rate rvoip sustains with zero failed attempts. RFC 6076 latencies
  are measured too, over TLS and UDP, signaling-only and with G.711 media.
- **Recorded rates.** The R values come from a full `minor` search on an
  earlier 0.5 candidate (qualification run 45065590604). It was re-scored
  offline after harness fixes: a device under test whose median setup delay
  collapses past 500 ms now scores as a DUT failure instead of an invalid
  trial, and the R4 recovery trial is scored on its steady window. The device
  under test is a c8i.4xlarge (16 vCPU), driven by three c8in.24xlarge callers
  in us-west-2a.

  | Row | Transport | Media | R (sessions/s) | Confirmations (of 3) |
  |---|---|---|---|---|
  | R1 | TLS | signaling only | 8,348 | 1 |
  | R5 | UDP | signaling only | 8,256 | 1 |
  | R2 | TLS | SDES-SRTP G.711, 10 s hold | 1,124 | 4 |
  | R6 | UDP | RTP G.711, 10 s hold | 1,204 | 2 |

  That search's RFC 7502 report is in the evidence of the same run. Every
  release's confirmation trials re-test R1, R5 and R2 at 0.95 R.
- **Every release.** Patch and minor releases alike confirm the recorded R
  of R1, R5 and R2 on their exact candidate. That means three trials at
  0.95 R and one at 1.05 R per row, plus RFC 6076 latency at 0.9 R. A row
  with no recorded R gets a light search instead. The full search runs on
  demand.
- **Where it runs.** The testbed is pinned by environment id, which covers
  the instance types, image and caller build. Its design and contract are in
  [RFC7502_BENCHMARK.md](../../crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md).

## Webex BYOVA and Amazon Connect A2A

- **Webex BYOVA.** The experimental `rvoip-webex-byova` adapter connects
  Webex Contact Center BYOVA (Bring Your Own Virtual Agent) over Cisco's
  WebSocket protocol, and over gRPC with the `grpc` feature.
  - Every connection is authenticated before it is read, and published with
    its principal.
  - Calls expose caller inputs, interruption, handoff and exact
    reply-boundary marks.
  - rvoip does not redistribute Cisco's protobuf files. The `grpc` feature
    fetches them at build time and checks them against pinned SHA-256 hashes.
- **Amazon Connect A2A.** The developer-preview `rvoip-amazon-a2a` adapter
  connects Amazon Connect external agents (A2A), with native media bridging,
  turn traces, interruption and hand-back. Enable it with the facade's
  `amazon-a2a` feature.
- **Vapi.** Vapi calls can opt into ordered, lossless media delivery. This
  replaces the removed `raw_session` bypass.

Neither contact-centre adapter has been qualified against live Cisco or
Amazon traffic yet.

## Toolchain and dependencies

The minimum supported Rust version is **1.95** (was 1.91).

The dependency set moves forward:

- nom 8 in the SIP parser;
- axum 0.8 and axum-server 0.9;
- the RustCrypto wave with rand 0.10;
- prost and tonic 0.14;
- jsonwebtoken 12;
- reqwest 0.13 on ring;
- sqlx 0.9;
- redis 1;
- web-transport 0.13;
- cpal 0.18;
- the smaller upgrades from the grouped dependabot update.

Nothing on `0.4`'s held-back list is still held back in a published crate.
Where a dependency's types are in rvoip's public API, the changelog's
Breaking changes entry names the affected items and the migration.

Three changes need attention even in code that never names those crates.

### metrics 0.24 and OpenTelemetry 0.33

rvoip emits into whatever metrics recorder and OpenTelemetry global
provider the application installs. Both are keyed to their crate's semver
line. Install a metrics 0.24 recorder (metrics-exporter-prometheus 0.18)
and an OpenTelemetry 0.33 provider (tracing-opentelemetry 0.34). An
application still on metrics 0.23 or OpenTelemetry 0.32 builds and runs,
but rvoip's metrics and spans silently disappear.

### WebRTC on `rtc` 0.21.1

`rvoip-rtc` is rebased onto upstream `rtc` 0.21.1. Two behaviours change for
every WebRTC user, including code that only uses `rvoip-webrtc`:

- **Create data channels before the offer.** SCTP is no longer started for
  a media-only session, so a data channel opens only on a session whose offer
  carried it. A channel added to an established media-only session needs a
  new offer and answer before it opens.
- **Relay-only ICE needs a TURN server.** A peer connection with
  `IceTransportPolicy::Relay` and no TURN server is rejected when it is
  built.

Code that uses `rvoip-rtc` or `rvoip-webrtc-stack` directly follows
upstream's 0.21 API. The changelog lists the migration.

### Explicit rustls crypto provider

Every rustls client and server rvoip builds now names its provider (ring)
instead of using rustls's process-level default. Applications whose
dependency graph compiles both ring and aws-lc-rs, for example through an
AWS SDK, no longer panic with "Could not automatically determine the
process-level CryptoProvider".

Three dependencies can only use the process-level default: DTLS-SRTP,
redis TLS and sqlx Postgres TLS. For those, rvoip installs ring only if no
provider is installed yet. Install your own at startup to choose.

## Interoperability

### Asterisk and FreeSWITCH

The PBX release gates check RTCP and RFC 4028 session-timer behaviour
against real Asterisk and FreeSWITCH, beyond the protocol matrices they
already ran:

- RTCP over negotiated mux;
- RTCP on a separate port;
- no RTCP toward a peer that declines mux;
- session timers, refreshed in both directions.

Each cell also records the PBX's own verdict.

### Jambonz OSS

Jambonz OSS remains a mandatory external SIP interoperability peer.

- **Roles.** It is tested as an independent SBC/B2BUA, registrar and
  RTPengine media anchor. The tests use the same `Endpoint`, `StreamPeer`
  and `CallbackPeer` APIs as Asterisk and FreeSWITCH.
- **Pinning.** The release profile pins the latest reviewed open-source
  component line, currently Jambonz OSS `0.9.15`. Source archives and containers are digest-verified, and
  qualification fails if the pins are no longer the selected upstream heads.
- **Matrix.** The mandatory UDP/plain-RTP matrix covers:
  - authenticated registration;
  - PCMU and PCMA calls with bidirectional audio;
  - provisional and final call signaling;
  - hold and resume;
  - RFC 4733 DTMF;
  - CANCEL and 487;
  - rejection;
  - REFER/NOTIFY blind transfer;
  - replacement INVITE;
  - BYE from either side;
  - resource cleanup.

  `Refer-To` remains mandatory for REFER. RFC 3892 `Referred-By` remains
  optional, and is preserved unchanged when a peer supplies it.
- **Exclusions.** These are outside the Jambonz profile: G.729, AMR,
  TLS/SRTP, the RVoIP-as-B2BUA scenario, WebRTC, PSTN, application verbs,
  recording, high availability and load. Codec and transport support
  elsewhere in rvoip is not reduced by these peer-specific exclusions.

The qualification record below names the Jambonz OSS release line this
candidate was tested against.

## Performance evaluation

The protected `0.5.0` qualification is a `release-minor` qualification:
the full remote release profile plus the RFC 7502 confirmation lane. It
requires:

- the canonical 2,000-CPS passes;
- the full performance and resiliency matrix;
- the high-density full-media burst;
- the monolithic soak and the two-hour split soak;
- the teardown and churn tests;
- regression comparison against the reviewed performance baselines;
- the RFC 7502 confirmation of R1, R5 and R2 against the recorded rates,
  with RFC 6076 latency.

It produces structured JSON and Markdown metrics and a SHA-256 index of every
performance artifact of the run. Results from earlier releases are
historical baselines and cannot qualify this release.

General-user 10,000 CPS full-media capability is not claimed. The supported
envelope remains bounded by the 2,000-CPS real-media evidence, the exact host
configuration, workloads and soak durations recorded for this release, and
the benchmark rates recorded and confirmed on the pinned testbed.

The signed qualification artifact is the authority for the measured
results:

- the host shape, source commit and workload configuration;
- ASR and error counts, and setup-latency percentiles;
- CPU and RSS behaviour, media delivery and cleanup;
- the confirmed Session Establishment Rates;
- the artifact hashes.

The GitHub release links that artifact and its workflow run.

<!-- generated:changes:start -->
## Changes in this release

### Breaking changes summary

- **The minimum supported Rust version is 1.95** (was 1.91). The toolchain
  file, every CI and release workflow, and the AWS release workers move to
  Rust 1.95.0. Migration: build with Rust 1.95 or newer. (#415)
- **parking_lot 0.12 → 0.13 in public fields.** `rvoip-media-core`'s
  relay-controller `vad`/`agc`/`aec` handles, `rvoip-websocket`'s adapter
  `bridge` and `rvoip-webrtc`'s media-source `last_payload` are
  `parking_lot` 0.13 locks. Migration: depend on parking_lot 0.13 where you
  name those types. (#415)
- **`rvoip-amazon-connect`'s `aws-control` feature moves to aws-config
  1.13 / aws-sdk-connect 1.226 / aws-smithy-types 1.9** (still exact pins).
  `AwsConnectStarter::new` takes an aws-config 1.13 `SdkConfig`.
  Migration: load your `SdkConfig` with aws-config 1.13. (#415)
- **`rvoip-users-core` and `rvoip-vcon-postgres` move to sqlx 0.9.**
  `SqliteUserStore::pool`/`from_pool`, `PostgresUserStore::pool`,
  `AuthenticationService::set_pool`, `PostgresVconStore::new`/`pool` and
  `users_core::Error::Database` now carry sqlx 0.9 types. Migration: move
  your own sqlx dependency to 0.9 when you pass pools in or match on the
  database error. (#415)
- **jsonwebtoken 10 → 12** (security). jsonwebtoken types are part of these
  APIs: `rvoip-vcon`'s `TrustedKey`, `JwsProtectedHeader::alg`, `sign_jws`
  and `verify_jws`; `rvoip-auth-core`'s `JwtValidator::from_decoding_key` /
  `with_algorithm`, `JwksJwtValidator::new_with_validation` /
  `with_algorithms` and `DpopValidator::with_algorithms`; `rvoip-users-core`'s
  `JwtIssuer::algorithm` / `decoding_key`; and `rvoip-webex-byova`'s
  `WebexJwsConfig::algorithms`. `jsonwebtoken::Algorithm` is now
  `#[non_exhaustive]`. Migration: move your own jsonwebtoken dependency to 12
  and add a wildcard arm to `match`es on `Algorithm`. (#420)
- **`rvoip_auth_core::JwtValidator::from_hmac_secret` returns
  `Result<Self, BearerAuthError>`.** jsonwebtoken 12 refuses empty HMAC
  keys, so an empty secret is now `BearerAuthError::Unavailable` at
  construction instead of a validator that accepts tokens signed with an
  empty key. `rvoip-users-core`'s `JwtIssuer::new` likewise returns
  `Error::Config` for an empty HS256 `signing_key`. Migration: add `?` (or
  handle the error) after `from_hmac_secret(...)`. (#420)
- **prost 0.13 → 0.14 and tonic 0.12 → 0.14.** The generated types in
  `rvoip_webex_byova::grpc::wire` (feature `grpc`) and
  `rvoip_amazon_connect::signaling::proto` implement prost 0.14's `Message`,
  and `WebexByovaGrpcService::into_server` returns a tonic 0.14
  `VoiceVirtualAgentServer`. Migration: move your own prost / tonic
  dependencies to 0.14 (tonic 0.14 needs `tonic-prost` next to it for
  generated clients); `tonic::transport::server::TcpIncoming::from_listener`
  is now `TcpIncoming::from(listener)`. (#421)
- **axum 0.7 → 0.8 and axum-server 0.8 → 0.9.** `rvoip-users-core`'s
  `api::create_router` / `create_router_with_state` /
  `into_peer_aware_make_service` / `ApiMakeService` and `rvoip-scim`'s
  `ScimService::router` return axum 0.8 types. Routes now use axum 0.8's
  `{id}` path syntax; the old `/:id` form panics when a router is built.
  Migration: move your own axum dependency to 0.8 (and axum-server to 0.9)
  before merging or nesting these routers, and write any routes you add as
  `/{param}`. (#422)
- **cpal 0.15 → 0.18.** `rvoip-webrtc`'s `CpalAudioSource::new_with_device`
  and `CpalSpeakerSink::new_with_device` (feature `client-cpal`) take a
  cpal 0.18 `Device`. Migration: depend on cpal 0.18; `Device::name()` is
  now `Device::description()?.name()` and `SampleRate` is a plain `u32`. (#426)
- **metrics 0.23 → 0.24 and OpenTelemetry 0.32 → 0.33.** rvoip emits into
  whatever metrics recorder and OpenTelemetry global provider the
  application installs, and those are keyed to the crate's semver line.
  Migration: install a metrics 0.24 recorder (metrics-exporter-prometheus
  0.18) and an OpenTelemetry 0.33 provider (tracing-opentelemetry 0.34), or
  rvoip's metrics and spans silently disappear. (#426)
- **ordered-float 4 → 5 in `rvoip-sip-core`.** `Contact::q`,
  `Address::q`, `AcceptLanguage`'s `q`, and `Timestamp::new` / `value` /
  `delay` / `set_value` / `set_delay` use ordered-float 5's `NotNan`.
  Migration: depend on ordered-float 5 where you name `NotNan`. (#426)
- **quick-xml 0.41 → 0.42 in `rvoip-sip-registrar`.** `RegistrarError`
  implements `From<quick_xml::Error>` for quick-xml 0.42. Migration: move
  your own quick-xml dependency to 0.42 if you rely on that conversion. (#426)
- **redis 0.27 → 1.x in `rvoip-redis`.** `RedisAuthError::Redis` wraps a
  redis 1.x `RedisError`. Migration: depend on redis 1 when you match on
  the wrapped error's kind (`ErrorKind::IoError` is now `ErrorKind::Io`). (#426)
- **`rvoip_core_traits::Transport` is `#[non_exhaustive]`**, so later
  transports are no longer breaking changes. Migration: add a wildcard arm
  to `match`es on `Transport` outside `rvoip-core-traits`. (#426)
- **reqwest 0.12 → 0.13.** `rvoip_oidc::OidcProvider::from_metadata` takes a
  reqwest 0.13 `Client`. rvoip builds reqwest with `rustls-no-provider` on
  the workspace's ring rustls provider: every rvoip client constructor
  installs ring as the process-default provider when none is installed yet,
  and reqwest 0.13 verifies server certificates with the platform verifier
  (the OS trust store) instead of the bundled webpki roots. Migration: build
  the client you pass to `from_metadata` with reqwest 0.13; an application
  that wants another rustls provider installs it before creating rvoip
  clients. (#427)
- **nom 7 → 8 in rvoip-sip-core.** The public parser functions (about 220
  returning `nom::IResult` / `ParseResult`) keep their signatures, but the
  `nom::IResult`, `nom::Err` and `nom::error::Error` types in them are now
  nom 8's. So are the `rvoip_sip_core::parser::{NomError, ErrorKind,
  ParseError}` re-exports, the `ParseResult` aliases, and the `From` impls on
  `rvoip_sip_core::Error` for `nom::Err<…>` and `NomError<&[u8]>`.
  `comma_separated_list0`, `comma_separated_list1` and
  `semicolon_separated_params0` still return `impl FnMut`. Migration: callers
  that compose sip-core parsers with their own nom combinators must use nom 8
  and call `.parse(input)` (with `nom::Parser` in scope) instead of
  `combinator(…)(input)`; callers that only call sip-core parsers need no
  change. (#428)
- **rand 0.8 → 0.10.** `rvoip_rtp_core::RtcpIntervalParams::randomized`
  takes any rand 0.10 `Rng` (the rand_core trait formerly called
  `RngCore`). Migration: pass a rand 0.10 generator, such as `rand::rng()`. (#429)
- **validator 0.20 → 0.21.** `rvoip_users_core::validation::ValidatedCreateUserRequest`
  implements validator 0.21's `Validate`. Migration: depend on validator 0.21
  if you call `.validate()` through your own validator import. (#429)
- **web-transport 0.13 in the MoQ crates; `NegotiatedTransport` carries the
  raw-QUIC URL.** web-transport 0.13 raw-QUIC sessions have no CONNECT
  request, so `rvoip_moq_transport::session::NegotiatedTransport` gains
  `raw_quic_url: Option<Url>` (set by `NegotiatedTransport::raw_quic`) and is
  `Clone` instead of `Copy`; `rvoip-moq-native` fills it with the dialed
  target or the accepted server name. `Writer::stopped` returns the widened
  `Option<u32>` stream error code, and `SessionConnection` and the
  `Session::connect*` / `accept*` functions take web-transport 0.13 sessions.
  Migration: move to web-transport 0.13, build raw-QUIC values with
  `NegotiatedTransport::raw_quic(protocol, url)` and `.clone()` a
  `NegotiatedTransport` where it used to be copied. (#433)
- `rvoip-rtc` (library name `rtc`) is rebased from upstream `rtc` 0.20.0-alpha.1
  onto `rtc` 0.21.1, and its `rtc-*` dependencies move to the exact 0.21.1
  releases. The rvoip patches it still needs are kept and the ones upstream now
  ships are dropped (see the crate's `RVOIP_PATCHES.md`). Its public API is now
  upstream 0.21's, which is not source compatible with 0.20: `RTCPeerConnection`
  is no longer generic over its interceptor chain, `RTCPeerConnectionBuilder::build`
  and the calls that act at a point in time (`set_local_description`,
  `set_remote_description`, `RTCRtpSender::write_rtp`, `write_rtcp`, data-channel
  `send`/`send_text`, `get_stats`) take an `Instant`, `poll_read` yields a
  `TaggedRTCMessage`, `RTCDataChannelId` is a connection-local `usize` handle
  rather than the SCTP stream id, settings are built with `SettingEngineBuilder`,
  and cryptography goes through a per-connection `RTCCryptoProvider`. The
  `openssl`, `vendored-openssl` and `pem` features are removed; `crypto-ring`
  (the default) and `crypto-aws-lc-rs` replace them. `rvoip-webrtc-stack`'s
  `PeerConnectionBuilder` loses its interceptor type parameter, and its
  `DataChannel::id` is now that per-connection handle; the new
  `DataChannel::stream_id` returns the SCTP stream id both peers share.

  Two behaviours change for every user, including code that only uses
  `rvoip-webrtc`:

  - **Create data channels before the offer.** SCTP is no longer started for a
    media-only session, so a data channel opens only on a session whose offer
    carried it. Migration: call `create_data_channel` before
    `create_offer`; a channel added to an established media-only session needs
    a renegotiation (a new offer/answer) before it opens.
  - **Relay-only ICE needs a TURN server.** A peer connection with
    `IceTransportPolicy::Relay` and no TURN server is rejected when it is
    built. Migration: configure a TURN server with the relay policy, or use the
    default `All` policy.

  To migrate code that uses `rvoip-rtc` directly, follow upstream's
  [crypto-provider migration guide](https://github.com/webrtc-rs/rtc/blob/master/docs/crypto-provider-migration.md):
  drop the interceptor type parameter, pass `Instant::now()` where a call now
  asks for the time, destructure `TaggedRTCMessage { message, .. }` from
  `poll_read`, replace `SettingEngine::set_*` calls with
  `SettingEngineBuilder::new().with_*(..).build()`, store data-channel ids as
  `RTCDataChannelId`, and enable `crypto-aws-lc-rs` instead of the removed
  `openssl` features if you need a non-`ring` backend. For
  `rvoip-webrtc-stack`, write `PeerConnectionBuilder<A>` instead of
  `PeerConnectionBuilder<A, I>` (`with_interceptor_registry` now takes a
  `Registry`), and compare `DataChannel::stream_id` rather than `id` across
  peers. Configure a TURN server with `IceTransportPolicy::Relay`. (#434)
- SIP UDP transports and RTP/RTCP media sockets no longer set `SO_REUSEADDR`,
  on any port. On Linux the flag let any other local socket that also set it,
  in any process and under any user, bind a live SIP or media port and receive
  every datagram sent to it, so signalling or call audio could be silently
  taken. UDP has no TIME_WAIT, so a port can be bound again as soon as its
  socket closes, including when its process is killed: restarts are not slowed.

  On Windows, where a later `SO_REUSEADDR` socket could take a held port
  whatever the holder set, SIP and media UDP sockets now set
  `SO_EXCLUSIVEADDRUSE` before they bind. The Windows build is compile-checked
  (`x86_64-pc-windows-gnu`) but not tested, since rvoip has no Windows CI. The
  macOS/BSD behaviour follows the BSD socket rules and is not CI-tested either.

  Migration: a second socket can no longer bind a UDP port that an rvoip
  transport still holds. If you paired a wildcard bind with a specific address
  on the same port (for example `0.0.0.0:5060` and `10.0.0.1:5060`), bind each
  interface address explicitly instead. If you started a new process on the
  port before the old one exited, or rebound a port before closing the
  transport that held it, close or stop the old one first. (#456)
- SCIM now enforces a request body limit inside the SCIM router (default 2 MiB,
  set with `ScimConfig::with_max_request_body_bytes`). The router wraps each
  request body in the limit, so it stacks with any limit the application sets
  where it mounts the router and the smallest wins. An outer
  `DefaultBodyLimit::disable()` or a larger outer limit no longer widens the
  limit on SCIM routes, and a smaller outer `DefaultBodyLimit` keeps working.
  axum's own 2 MiB extractor limit still applies too, so to accept SCIM bodies
  over 2 MiB, raise the SCIM limit and also raise or disable `DefaultBodyLimit`
  where the router is mounted. A larger body gets `413 Payload Too Large`.

  SCIM also caps the group members one request may carry (default 10,000, set
  with `ScimConfig::with_max_group_members_per_request`) when it creates or
  replaces a Group, or when a PATCH replaces `members`. Over the cap, the router
  returns a `400` SCIM error with `scimType` `invalidValue`, and the service
  methods return the new `ScimError::InvalidValue`.

  Breaking changes and migration:

  - If you mounted the SCIM router with `DefaultBodyLimit::disable()` (or a
    limit above 2 MiB) and accepted SCIM bodies over 2 MiB, they now get `413`.
    Raise `ScimConfig::with_max_request_body_bytes` to the size you accept; your
    mount already lifts axum's 2 MiB extractor limit.
  - If you provision groups with more than 10,000 members in one request, raise
    `ScimConfig::with_max_group_members_per_request`. The adapter's PATCH
    supports only `replace` of `members`, which carries the whole membership, so
    splitting a group across PATCH requests does not get past the cap.
  - An exhaustive `match` on `ScimError` needs an arm for `InvalidValue`. (#466)

### Changed

- Unlocked by the Rust 1.95 minimum: **sqlx** 0.8 → 0.9 (runtime SQL built from
  fixed fragments is now marked with `AssertSqlSafe`; vcon-postgres keeps the
  ring rustls provider through `tls-rustls-ring-webpki`), **serial_test** 3 → 4
  (dev only), **parking_lot** 0.12 → 0.13, **sysinfo** 0.31 → 0.40 (perf
  harness only), and the AWS SDK pins above, which take `aws-config` off the
  `crypto-bigint` 0.5 path (RUSTSEC-2026-0387). New Rust 1.95 Clippy lints are
  fixed in `rvoip-codec-core` and the perf harness. (#415)
- **jsonwebtoken** 10.4 → 12.0 in auth-core, users-core, vcon, stir-shaken,
  webex-byova, moq and uctp (11 is skipped). 12.0 fixes the remaining
  `sub`/`iss`/`aud` type confusion, rejects unknown `crit` header extensions and
  refuses empty HMAC keys. (#420)
- **prost** / **prost-build** / **prost-types** 0.13 → 0.14 and **tonic** 0.12 →
  0.14 move together. tonic 0.14 splits prost support out: `rvoip-webex-byova`'s
  `grpc` feature adds `tonic-prost`, and its build script uses
  `tonic-prost-build` in place of `tonic-build`. (#421)
- **axum** 0.7 → 0.8 and **axum-server** 0.8 → 0.9 across users-core, scim,
  rvoip-webrtc (WHIP/WHEP signalling), the `rvoip` facade and vapi's test mock.
  The ten users-core routes, the two SCIM resource routes, the WHIP/WHEP
  resource routes and the browser-demo file route move from `/:id` to `/{id}`;
  new tests send every parameterised users-core and SCIM route a request and
  fail if any is unmatched. users-core's extractors drop `#[async_trait]`, and
  its rate-limit middleware reads `ConnectInfo` from the request extensions
  (axum 0.8 no longer extracts `Option<ConnectInfo<_>>`). The workspace
  axum-server uses its new `tls-rustls-ring` feature, so it no longer turns on
  rustls's aws-lc-rs provider next to the workspace's ring provider. (#422)
- **ordered-float** 4 → 5, **quick-xml** 0.41 → 0.42, **redis** 0.27 → 1.x,
  **cpal** 0.15 → 0.18, **metrics** 0.23 → 0.24 with
  **metrics-exporter-prometheus** 0.16 → 0.18 (moq-relay already installed a
  0.24 recorder, so rvoip's 0.23 metrics never reached it), and the
  **opentelemetry** / **opentelemetry_sdk** / **opentelemetry-otlp** 0.32 → 0.33
  group with **tracing-opentelemetry** 0.33 → 0.34 (`rvoip-infra-common`'s
  optional `otel` exporter). quick-xml 0.42 events are `str`, so the
  registrar's PIDF parser matches names as `str` and reads text with
  `xml10_content()`. With redis 1.x the single-node connection manager takes
  its retry floor as `set_min_delay` and its timeouts as `Option`s, and rvoip
  still sets every timeout explicitly; `cluster_routing::get_slot` is no
  longer public, so the live-cluster test computes key slots itself and
  checks them against Redis reference values. cpal 0.18 removes
  `Device::name()`, so `rvoip-audio-device` and the `sip_client` example name
  devices by `description()`. web-transport 0.13 is not in this group: its
  raw-QUIC sessions no longer carry a URL, which moq-transport uses as the
  session target, so it lands with its own change. (#426)
- **reqwest** 0.12 → 0.13, **tokio-tungstenite** 0.29 → 0.30, **opus** 0.3 →
  0.4 (its `opusic-sys` binding builds the bundled libopus and replaces the
  unmaintained `audiopus_sys`), and `dtls` 0.17.2 → 0.17.3 in the lockfile.
  reqwest 0.13's `rustls` feature turns on aws-lc-rs, which conflicts with the
  workspace's ring-only rustls, so every rvoip crate uses `rustls-no-provider`
  (plus `form` where it posts forms) and installs ring before building a
  client; the tests that build their own clients do the same. The unused
  `tower` 0.4, `tower-http` 0.5 and `mio` 0.8 workspace dependency entries are
  removed. (#427)
- **rand** 0.8 → 0.10, **validator** 0.20 → 0.21 and the RustCrypto wave
  together, since the new RustCrypto releases share rand_core 0.10 and
  hybrid-array: **sha1** / **sha2** / **md-5** 0.10 → 0.11, **hmac** 0.12 → 0.13,
  **aes** 0.8 → 0.9, **ctr** 0.9 → 0.10, **aes-gcm** 0.10 → 0.11, **argon2** 0.5
  → 0.6 with **password-hash** 0.6, and **rsa** 0.9 → 0.10.

  rand 0.10 renames: `thread_rng()` → `rng()`, `gen*` → `random*`,
  `distributions` → `distr`, the extension trait `Rng` → `RngExt`, `RngCore`
  → `Rng`, and `OsRng` → `SysRng` (fallible; key material is drawn with
  `UnwrapErr(SysRng)`, which panics on an OS RNG failure exactly as
  `OsRng` did).

  Digests and cipher blocks are `hybrid_array::Array`: `GenericArray`
  conversions become `TryFrom` (each length is checked first, so none of
  them can fail), `BlockEncrypt`/`BlockDecrypt` become
  `BlockCipherEncrypt`/`BlockCipherDecrypt`, `Hmac::new_from_slice` comes
  from `KeyInit`, and SRTP's SHA-1 fast path calls
  `sha1::block_api::compress`. A digest no longer formats with `{:x}`, so
  hex strings come from `hex::encode` (`rvoip-sip` adds `hex`).

  argon2 0.6 draws the 16-byte salt itself (`hash_password(password)`); the
  PHC strings are unchanged and a new test verifies a hash stored by argon2
  0.5.

  rsa 0.10: `PrivateKey` replaces `RsaPrivateKey`, PEM support is in the
  default `encoding` feature, and the JWK `n`/`e` come from
  `to_be_bytes_trimmed_vartime()` so they stay minimal octets (a test pins
  `e = AQAB` and a 256-byte `n`). (#429)
- **The release split soak runs two hours.** `perf.soak-candidate` (500 active
  calls) soaks for 7,200 s instead of 3,600 s. Its RSS limit (15 MB/h) and window
  (the last 20 minutes of the active phase) are unchanged.

  In 0.5 the receiver's heap warms up for about an hour before it plateaus, so a
  one-hour soak measured the warm-up. A two-hour soak of `main` plateaus at about
  168 MB with −0.52 MB/h over its last 20 minutes; 0.4.0 plateaued at about
  182 MB. The plateau is lower because of #354's smaller per-session maps and
  fixed loss-window ring. The warm-up is longer because of a gradual increase
  across the 0.5 merge trains.

  The release controller job's timeout grows to 270 minutes, and a qualification
  takes about an hour longer. The monolithic soak stays at one hour. (#465)

### Fixed

- `rvoip-amazon-connect`'s `server` feature now enables Opus in rvoip-core's
  media graph (`rvoip-core/opus`). Before, building only with `server` (as
  `examples/13-sip-to-amazon-connect` does) let the media-core transcoder handle
  Opus, but rvoip-core refused the Opus stream, so creating the SIP ⟷ Amazon
  Connect media bridge failed with `ConnectError { class: Mapping }`. (#444)
- A SIP UDP transport bound to an ephemeral port (port 0) no longer sets
  `SO_REUSEADDR`. On Linux that let another reuse-flagged UDP socket, in the
  same process or another, be given the same port and take every datagram sent
  to it, so a call could silently lose its SIP responses and time out. (#454)

### Security

- **Explicit rustls crypto provider.** Every rustls client and server rvoip
  builds now names its provider (ring) instead of using rustls's process-level
  default. Applications whose dependencies compile both rustls providers
  (`ring` and `aws-lc-rs`, for example through an AWS SDK) no longer panic with
  "Could not automatically determine the process-level CryptoProvider". This
  covers:

  - SIP TLS and WSS, client and server, including client-certificate
    verification;
  - the WebRTC WHIP/WHEP HTTPS and WSS listeners, and outbound WHIP and
    WebSocket signaling;
  - the users-core HTTPS listener;
  - the HTTP clients in auth-core (JWKS, token introspection), OIDC, Keycloak,
    audit, IMS-AKA, STIR/SHAKEN and Vapi;
  - the Vapi, Amazon Connect (Chime) and UCTP WebSocket clients, and the UCTP
    QUIC client;
  - LDAP.

  Three dependencies can only use rustls's process-level default:

  - the `dtls` crate behind DTLS-SRTP (`rvoip-rtp-core` `dtls-webrtc`, used by
    SIP DTLS-SRTP calls);
  - redis TLS (`rediss://` in `rvoip-redis`);
  - sqlx Postgres TLS (`rvoip-vcon-postgres`).

  If no process-level rustls CryptoProvider is installed when one of those
  paths first runs, rvoip installs ring; install your own at startup to choose.
  rvoip never replaces a provider that is already installed, and installs one
  nowhere else. (#441)
- **Dependency advisories.**

  - `libsqlite3-sys` moves from 0.30 to 0.40, bundling SQLite 3.54, through
    sqlx 0.9.1. This fixes RUSTSEC-2026-0459 (CVE-2026-119375): opening a
    crafted database together with a `-wal` file of a different page size could
    return uninitialized memory. It affects `rvoip-users-core` and the crates
    built on it (SAML, SCIM, WebAuthn).
  - DTLS-SRTP (`rvoip-rtp-core`'s `dtls-webrtc` feature) moves to `dtls` 0.17.3.
    Its P-256/P-384 0.14 stack uses the fixed `crypto-bigint` 0.7, which removes
    the DTLS path of RUSTSEC-2026-0387 (non-constant-time arithmetic).
  - `cargo deny` and the release audit no longer ignore RUSTSEC-2026-0104,
    -0098 and -0099 (`rustls-webpki`) or RUSTSEC-2026-0185 (`quinn-proto`). The
    locked versions already carry the fixes.

  Two advisories remain accepted. Both come only through `rvoip-webauthn`'s
  `crypto-glue` 0.1, and neither is reachable as a WebAuthn relying party uses
  it, because it only verifies signatures: RUSTSEC-2023-0071 (`rsa` 0.9) and
  RUSTSEC-2026-0387 (`crypto-bigint` 0.5). They go once `crypto-glue` supports
  `rsa` 0.10.0 final.

  Applications with their own lockfile can take the fixes with
  `cargo update -p sqlx -p libsqlite3-sys -p dtls`. (#449)
- The `url` dependency moves to 2.5.9, fixing RUSTSEC-2026-0592: in 2.5.8 and
  earlier, `Url::origin()` recursed once per nested `blob:` prefix with no limit,
  so one crafted URL could overflow the stack and abort the process. rvoip's own
  calls to `Url::origin()` are not exposed: the SIP analytics `url` parameter is
  length-limited and must be `http`/`https` first, and WebAuthn origin matching
  rejects a scheme mismatch before it computes origins. Applications with their
  own lockfile can take the fix with `cargo update -p url`. (#469)
<!-- generated:changes:end -->

## Compatibility

`0.5.0` is not source-compatible with `0.4.0` for:

- applications that match exhaustively on the enums that gained variants;
- applications that build the structs that gained fields with struct
  literals;
- applications that name the dependency types now on newer major lines
  (nom, axum, jsonwebtoken, prost and tonic, sqlx, reqwest, rand, redis,
  ordered-float, quick-xml, validator, cpal, parking_lot and web-transport);
- WebRTC code that uses `rvoip-rtc` or `rvoip-webrtc-stack` directly.

Each case has a one-line migration in the changelog's Breaking changes
section.

Rust 1.95 is required.

Runtime behaviour changes for every application in these cases:

- the metrics and OpenTelemetry coupling above;
- the two WebRTC behaviours above;
- `Require: replaces` answered with 420;
- a call whose media a peer BYE stopped first now ends normally instead of
  as a media failure.

Executor v2 changes nothing unless installed.

The `0.4.0` tag, crates and qualification evidence remain immutable release
history. Qualification reuses earlier evidence only where a gate's exact
inputs are unchanged, through the candidate's receipt chain.

<!-- generated:qualification:start -->
## Qualification record

The release candidate `81452a5fb8a5730d4e085dac0e110ca6f9e3af02` passed the protected `remote-release` qualification ([workflow run](https://github.com/eisenzopf/rvoip/actions/runs/45260396686)): 220 of 220 gates (18 run fresh, 202 reused by exact inputs), including the complete crate matrix, feature bundles, CodeQL policy, Asterisk, FreeSWITCH, Jambonz OSS, Kamailio, OpenSIPS, SIPp, strict-UA, security, performance, resiliency, soak, regression and cleanup gates.

Jambonz OSS was tested at the pinned `0.9.15` release line.

`0.5.0` is that candidate plus the version bump. Its requalification reuses these receipts through version-normalized input digests and reruns only the bookkeeping gates; the protected Release Publish workflow accepts only that requalification, bound to the exact release commit, and the `v0.5.0` tag identifies it. The run's Performance evaluation report is attached to the GitHub release.
<!-- generated:qualification:end -->
