# rvoip 0.4.0 Candidate Release Notes

These notes describe the coordinated 46-crate `0.4.0` release candidate.
Protected publication requires qualification of its exact source commit. The
signed qualification artifact and GitHub release identify that commit,
workflow run, complete gate inventory, measured performance, and publication
result. The sections marked pending below are filled from that run before
publication. The latest published release is `0.3.12`.

## Headline

`0.4.0` is a breaking minor release. RTCP is fully wired for the first time:
rvoip offers `a=rtcp-mux` by default, sends RFC 3550-correct reports, keeps
other calls' RTCP out, and can carry RTCP on a separate port for peers that
decline multiplexing. Per-call media quality, including the peer's view of
the stream rvoip sends, now reaches applications. `rvoip_sip::Config` gains
deployment profiles for the common server, trunk, NAT and mutual-TLS
shapes, plus SIP OPTIONS keep-alive. RFC 4028 session timers and in-dialog
request routing are corrected, so calls on the new profiles survive refresh
rejections and transfers reach the peer's Contact. `rvoip-core` gains bounded
lifecycle retention for long-running workers, supervised playback and
periodic workers, resource snapshots and codec-aware TTS. UCTP gains
authenticated application profiles over WebSocket and QUIC.

Public configuration structs are now `#[non_exhaustive]`, and a small set of
types and defaults changed. Applications upgrading from `0.3.12` should read
the [0.4 migration guide](../../../../docs/MIGRATING_0.4.md) before bumping.

## RTCP and media quality

- **Multiplexed RTCP by default.** Every SDP offer (initial INVITE,
  re-INVITE, UPDATE, hold/resume and the late-SDP offer in a 200 OK) carries
  `a=rtcp-mux` (RFC 5761). Periodic reports flow whenever the peer answers
  with `a=rtcp-mux`; before this release, calls rvoip originated never sent
  RTCP. When the answer declines mux, the call sends no RTCP at all, as
  RFC 5761 §5.1.1 requires. `Config::offer_rtcp_mux = false` restores the
  0.3.x offer for a peer that mishandles the attribute, and
  `Config::rtcp_mux_required` is the strict mode (`a=rtcp-mux-only`).
- **Separate RTCP port for non-mux peers.** `Config::rtcp_non_mux` reserves
  an even RTP port and RTP + 1 per call (RFC 3550 §11), advertises
  `a=rtcp:` (RFC 3605), and sends reports and the close-time BYE to the
  peer's RTCP address, never its RTP port. SRTCP covers it under SDES-SRTP.
  This is how PBXes such as Asterisk with default pjsip settings, and
  carriers without mux, get RTCP. A call that keeps its RTCP port uses two
  ports. It is not used with ICE, DTLS-SRTP keying, strict mux or
  signalling-only media.
- **RFC 3550-correct reports.** Sender Reports carry the media clock's RTP
  timestamp at the report's NTP time; a session that stopped sending reports
  with an RR; the interval follows RFC 3550 Appendix A.7 (five-second
  minimum, randomised, halved before the first report) instead of one
  second; every report and the BYE carry an SDES CNAME. The CNAME is a random
  per-session value (RFC 7022) instead of `user@host`. RFC 3611 XR is off
  unless `Config::rtcp_xr_voip_metrics` is set and the peer advertises it,
  and `Config::rtcp_reduced_minimum_interval` opts into the §6.2 reduced
  minimum.
- **Inbound RTCP filtering and liveness.** RTCP is accepted only from the
  call's signalled or latched peer, or from an SSRC already sending RTP, so a
  neighbouring call's RTCP can no longer feed this call's statistics or BYE
  handling. `Config::active_call_rtcp_counts_as_media` lets RTCP keep a held
  or silent call alive under the dead-media watchdog.
- **Media quality API.** `UnifiedCoordinator::media_quality(&SessionId)` and
  `SessionHandle::media_quality()` return `MediaQualityStats`: local packet
  counts, loss, jitter and MOS estimate, plus RTT and the peer-reported loss
  and jitter from its RTCP. `Config::media_quality_interval` publishes
  `Event::MediaQualityChanged` periodically, and through `rvoip-core` the SIP
  adapter now produces `Event::MediaQuality` with the same peer data. See
  [MEDIA_QUALITY.md](MEDIA_QUALITY.md).

## Deployment profiles

Each profile is a constructor whose rustdoc lists every field it sets; all of
them stay public fields you can change afterwards.

- `Config::carrier_trunk_udp`: plain UDP trunk authenticated by source IP,
  trunk SBC as outbound proxy, playout buffer, 1800 s session timers.
- `Config::public_server`: public or 1:1-NAT server with advertised
  addresses, ICE Lite, playout buffer and session timers; chain
  `tls_reachable_contact` for TLS, which now advertises the public IP.
- `Config::behind_nat`: TLS RFC 5626 registered flow with CRLF keep-alive,
  STUN, ICE Full and `rtcp_mux_required`.
- `Config::tls_direct_routing`: mutual-TLS SBC peering with an FQDN Contact,
  SDES-SRTP required, ICE Lite, OPTIONS keep-alive and session timers. It is
  modelled on Microsoft Teams Direct Routing's SBC requirements; it is not
  certified or tested against Teams.
- `Config::carrier_sbc` and `Config::proxy_rtpengine` now enable RFC 4028
  session timers (1800 s, Min-SE 90). `Config::freeswitch_internal` is
  deprecated in favour of `Config::lan_pbx`.
- **OPTIONS keep-alive.** `Config::options_keepalive_targets` and
  `options_keepalive_interval_secs` (default 60 s) ping each target with an
  out-of-dialog OPTIONS and publish `Event::PeerReachabilityChanged` on the
  first outcome and on every change.

## Session timers and in-dialog routing

- RFC 4028 conformance as UAS and UAC: no `Require: timer` toward callers
  that did not advertise it, the non-refresher's BYE at
  `interval - min(32, interval / 3)`, refreshes that always say
  `refresher=uac` and carry the learned Min-SE, a refresh re-INVITE fallback
  that actually reaches the wire with the current SDP, and session-timer
  headers only on 2xx to INVITE and UPDATE.
- A rejected refresh no longer ends the call. Only a timeout, a transport
  failure, 408 or 481 ends it with `Reason: SIP;cause=408`; 491 is retried
  after the RFC 3261 §14.1 backoff, 422 is retried with the peer's Min-SE,
  and other rejections keep the call with one more attempt before the
  deadline. Refresh answers renegotiate the interval and refresher, a peer
  whose `Allow` omits UPDATE is refreshed with re-INVITE, and held calls can
  be refreshed.
- `Config::validate` enforces the RFC 4028 90-second floor for the interval
  and Min-SE.
- Every in-dialog request (UPDATE, re-INVITE, REFER, MESSAGE, INFO, NOTIFY,
  OPTIONS) uses the peer's Contact as its Request-URI (RFC 3261 §12.2.1.1),
  so transfers and refreshes reach the right host; PRACK targets the reliable
  18x Contact.
- INVITE authentication retries keep the Min-SE learned from a 422, stay on
  the RFC 5626 registered flow, and re-sign retained Digest credentials with
  the next `nc` and a fresh `cnonce` instead of replaying `nc=00000001`.
- An outbound INVITE whose TLS connection is closed by the server, such as a
  TLS 1.3 `certificate_required` rejection, no longer hangs `send()`.

## Core, playback and TTS

- **Bounded lifecycle retention.** `configure_bounded_connection_lifecycles`
  bounds retained connection, session and conversation rows instead of
  cumulative calls, and `release_closed_conversation` releases finished
  history. Published `0.3.12` kept every retired connection ID and failed
  closed after 262,144 admissions. `connection_id_budget_usage()` and
  `resource_snapshot()` expose usage for monitoring and worker rotation.
- **Supervised workers.** Periodic workers and playback workers are owned,
  bounded and drained at shutdown; playback can be cancelled while it waits
  for transport capacity.
- **Codec-aware TTS and PCM playback.** TTS requests carry the destination
  codec, providers declare PCM or encoded output, and rvoip encodes, frames
  and paces it at real time. `Orchestrator::play_pcm` plays caller-fed PCM
  through the same path.
- **Media fixes.** G.711 has one implementation for every caller: the
  utility helpers and the tone generator produced non-standard bytes before.
  Stereo playout timestamps, replacement-SSRC timelines and variable-length
  Opus packets in the SIP media pump are also fixed.
- Tenant quota updates are reconciled against configured capacity, and
  outbound SIP DNS resolution no longer holds a dialog lock.

## UCTP

- Experimental authenticated application profiles: a host installs one
  `ApplicationHandler`, selected by `payload.profile`, behind the
  coordinator's authentication, scope and replay checks, with a bounded
  per-call timeout. Available over WebSocket and QUIC, with a runnable
  example.
- `UctpWsAdapter` owns its listener: `begin_drain()`, `shutdown(budget)`, and
  cancellation when the adapter is dropped.
- `sdk/uctp-js` is an experimental browser and Node 22+ client source package
  (not published to npm).

## Breaking changes summary

The full list, with a one-line migration for each, is in the
[changelog](../../../../CHANGELOG.md); before/after code is in
[docs/MIGRATING_0.4.md](../../../../docs/MIGRATING_0.4.md).

- `ConnectionId` / `ConversationId` are no longer tuple structs; use
  `from_string` and `as_str`.
- Public configuration structs are `#[non_exhaustive]`; start from a
  constructor and assign fields.
- `TtsPlayback::audio_format` is required and `TtsRequest` gains
  `destination_codec`.
- `Event::MediaQualityChanged` gains `quality`, `Event` gains
  `PeerReachabilityChanged`, and the quality and RTP statistics structs gain
  fields. media-core's `MediaSessionInfo` gains `rtcp_port`.
- Low-level INVITE authentication retries take `InviteAuthRetryOptions`
  (dialog-core `DialogManager`) and a learned Min-SE
  (`rvoip_sip::internals::DialogAdapter`).
- RTCP defaults: `a=rtcp-mux` offered, RFC 3550 interval, XR off, random
  CNAME, no RTCP toward peers that decline mux unless `rtcp_non_mux` is on.
- Session timers below 90 s are rejected; `carrier_sbc` and
  `proxy_rtpengine` enable session timers.
- Dropping a `UctpWsAdapter` stops its listener; shrinking a busy tenant
  quota is rejected; `Config::freeswitch_internal` is deprecated.

## Interoperability

### Asterisk and FreeSWITCH lab

A local PBX lab run against Asterisk 20.20.1 and FreeSWITCH 1.10.12 passed
the new RTCP and session-timer scenarios: RTCP over negotiated mux, RTCP on a
separate port toward peers that decline mux, SRTCP on the separate port under
SDES-SRTP, media quality with peer-reported statistics, RFC 4028 session
timers, RFC 3550 report timing, and a random per-session CNAME. That run is
diagnostic evidence for these features; publication independently requires
the protected qualification below.

### Jambonz OSS

Jambonz remains a mandatory external SIP interoperability peer for this
release.

- Jambonz is tested as an independent SBC/B2BUA, registrar, and RTPengine
  media anchor using the same PBX runner and the same `Endpoint`, `StreamPeer`,
  and `CallbackPeer` APIs used for Asterisk and FreeSWITCH.
- The release profile pins the latest reviewed open-source component line,
  currently Jambonz OSS `0.9.11`. Source archives and every container are
  digest-verified, and qualification fails if the component pins are no
  longer the selected upstream heads.
- The mandatory UDP/plain-RTP matrix covers authenticated registration,
  separate PCMU and PCMA calls with bidirectional audio, provisional and final
  call signaling, hold/resume, RFC 4733 DTMF, CANCEL/487, rejection,
  REFER/NOTIFY blind transfer, replacement INVITE, BYE from either side, and
  resource cleanup. `Refer-To` remains mandatory for REFER; RFC 3892
  `Referred-By` remains optional and is preserved unchanged when a peer
  supplies it.
- G.729, AMR, TLS/SRTP, the RVoIP-as-B2BUA scenario, WebRTC, PSTN,
  application verbs, recording, high availability, and load are explicit
  exclusions from this Jambonz profile. Codec and transport support elsewhere
  in RVoIP is not reduced by those peer-specific exclusions.
- **Pending.** The protected run must record the complete Jambonz matrix as
  PASS for the exact `0.4.0` source commit.

## Performance evaluation

**Pending.** The protected `0.4.0` qualification runs the canonical 2,000-CPS
passes, the full performance and resiliency matrix, the high-density
full-media burst, the monolithic and split soaks, teardown/churn tests, and
regression comparison against the `0.3.12` baselines. The default RTCP
changes add periodic report traffic on calls whose peers accept mux; the
results and any baseline movement are recorded here from that run. Earlier
results remain historical baselines and cannot qualify this release.

General-user 10,000 CPS full-media capability is not claimed.

## Compatibility

`0.4.0` is not source-compatible with `0.3.12` for applications that build
the configuration structs with literals, construct or mutate
`ConnectionId` / `ConversationId` directly, implement `TtsPlayback`, or match
exhaustively on `rvoip_sip::Event` or the changed structs. Each case has a
mechanical migration in [docs/MIGRATING_0.4.md](../../../../docs/MIGRATING_0.4.md).
Wire behaviour changes are limited to the RTCP defaults, session-timer
conformance, in-dialog Request-URIs and the profile defaults listed above;
`Config::offer_rtcp_mux = false` and explicit field overrides restore the
previous behaviour where a peer depends on it.

Known limitations: a TLS or TCP connection that closes after an INVITE was
written still fails the call only at Timer B (#311); `rtcp_non_mux` is not
supported with DTLS-SRTP; the grouped dependency update (#297) is deferred.

The `0.3.12` tag, crates, and qualification evidence remain immutable release
history; none of that evidence is reused to qualify `0.4.0`.

## Qualification record

**Pending.** The protected Release Qualification workflow runs from a clean
`main` commit and must record every selected gate as PASS, including the full
46-crate matrix, feature bundles, CodeQL policy, Asterisk, FreeSWITCH, Jambonz,
Kamailio, OpenSIPS, SIPp, strict-UA, security, performance, resiliency, soak,
regression, source-fence, and cleanup gates. It produces a signed aggregate
bound to the exact source SHA and immutable artifact hashes.

The protected Release Publish workflow accepts that aggregate only after
verifying that its source SHA is on `main`, its version is exactly `0.4.0`, its
evidence is fresh and complete, and the dry-run and live publication inputs
resolve to the same commit. The resulting GitHub release links the exact
qualification run and measured reports; the protected `v0.4.0` tag identifies
the source.
