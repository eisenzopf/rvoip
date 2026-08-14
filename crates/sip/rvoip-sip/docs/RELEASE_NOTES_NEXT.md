# rvoip 0.3.7 Release Candidate Notes

Date: 2026-08-06

These notes describe the coordinated 44-crate `0.3.7` release candidate.
Publication requires a fresh strict full-beta qualification bound to the exact
clean release source. Prior `0.3.4` carry-forward and `0.3.6` qualification
evidence do not qualify this release.

## Headline

`0.3.7` is a media-reliability and edge-interop release. It hardens the Vapi
bridge and WebRTC/Connect paths under backpressure, exposes inbound SIP auth
and dialed-number context on the app facade, and repairs SIP/WebRTC cases that
dropped wildcard contacts, late tracks, or DTMF without MID.

The security, RTP/SRTP correctness, transactional renegotiation, and Tokio-only
WebRTC work from `0.3.5` remains in force. This candidate adds operator-facing
reliability on top of that baseline.

## Vapi and media reliability

- Inbound and outbound audio queues are bounded so bursts and uplink stalls no
  longer terminate the session. The RTP clock keeps advancing across underruns,
  and jitter depth re-converges when renegotiation re-arms warm-up.
- An adaptive jitter target (RFC 3550 arrival jitter, held and clamped) replaces
  a fixed five-frame target. The inbound catch-up valve now has stream capacity
  to release more than one frame, and outbound gets a matching drain valve.
- Barge-in flushes stale playout audio so queued assistant speech does not talk
  over the caller for a full buffer depth.
- WebSocket writes move off the media loop. Control (Ping/Pong/commands) uses
  its own channel so media backpressure cannot look like a heartbeat failure.
- Per-call `VapiMediaHealth` reports depth and high-water in milliseconds,
  drops by reason, underrun/catch-up ticks (including catch-up blocked), and
  media write timeouts. Logs carry `connection_id`.

## WebRTC, Connect, and SIP

- Media continues under driver backpressure; unbind stays responsive. Connect
  startup backpressure no longer evicts media sinks or drops retained routes.
- Primary audio and DTMF work when a peer never negotiates the SDES MID
  extension (Amazon Connect and similar). Late remote audio tracks attach; per-
  peer UDP allocation is bounded; remote codec preference order is preserved.
- Wildcard `Contact` routes use the observed source address.
- `SipConfig` surfaces listener auth and inbound context policy already present
  one layer down: `tenant`, `trusted_trunk(cidr, subject)`, and
  `capture_headers`. Defaults remain fail-closed and behavior-compatible when
  unset. `IpNet` is re-exported from `rvoip-sip` for CIDR literals.

## Architecture and compatibility

- Changes preserve the sharded, exact-key, generation-protected, bounded-
  retention SIP signaling architecture in
  [`SIGNALING_PERFORMANCE_ARCHITECTURE.md`](SIGNALING_PERFORMANCE_ARCHITECTURE.md).
- Crypto non-claims from
  [`CRYPTO_CAPABILITIES.md`](CRYPTO_CAPABILITIES.md) are unchanged: AEAD AES-GCM,
  end-to-end SIP DTLS-SRTP, MIKEY, ZRTP, and G.722 codec negotiation remain
  unsupported through the public SIP surface.
- Public compatibility is compared with the documented `0.3.6` baseline.
  Additive facade configuration and typed media-health APIs do not remove
  existing call sites.
- General-user 10,000 CPS full-media capability is not claimed. The strict SIP
  beta envelope remains bounded by its recorded 2,000-CPS real-media profile,
  exact host configuration, peer matrix, workloads, and soak durations.
- Browser/WebRTC edge qualification remains separate from the SIP beta claim;
  Connect and MID/DTMF repairs do not broaden that claim to untested browsers,
  ICE/TURN deployments, or network topologies.

## Qualification

The release candidate must pass the one-command full beta gate from a clean,
committed `0.3.7` source tree. Required evidence includes three fresh canonical
2,000-CPS runs; workspace, public-API, security, parser, PBX, SIPp, strict-UA,
Kamailio, and OpenSIPS gates; full-media performance and resiliency matrices;
and both one-hour monolithic and split soaks. The generated report package and
its source fingerprint are verified before crates.io publication.

Historical `0.3.2` exception, `0.3.4` carry-forward, and prior `0.3.6`
attestations remain unchanged release history. They are not presented as
current `0.3.7` evidence.
