# rvoip 0.3.8 Release Candidate Notes

Date: 2026-08-14

These notes describe the coordinated 44-crate `0.3.8` release candidate.
Publication requires a fresh `remote-release` qualification bound to the exact
clean release source and to the current gate catalog. Prior `0.3.6` and
`0.3.7` qualification evidence does not qualify this release.

## Headline

`0.3.8` is a codec and interop release. AMR-NB and AMR-WB ship end to end —
both interface formats, DTX, redundancy, interleaving, and a negotiated
mode-set — and every rate is proved in a live call through a record-routing
proxy rather than only in unit vectors. Kamailio and OpenSIPS join the
qualification matrix over TLS with SRTP through rtpengine.

Two SIP correctness repairs ride with it: secure dialogs now answer with a
`sips:` fallback Contact, and a UAC learns its route set from the
dialog-forming 2xx, so in-dialog requests follow the proxy path instead of
bypassing it.

The media-reliability, backpressure, and inbound-auth work from `0.3.7`
remains in force.

## AMR

- Both variants at both interface formats (IF1 and IF2), bit-exact against
  3GPP's own reference material for TS 26.073, TS 26.101 and TS 26.201. No
  3GPP source is vendored into this repository; the oracles fetch it.
- VAD1 and VAD2, DTX reaching the wire, receive-side interleaving reassembly,
  max-red redundancy with dedup, and CMR damping.
- The SDP `mode-set` is negotiated and obeyed, and each negotiated rate is
  attested in the release evidence rather than assumed from the top mode.
- The media graph admits a codec by the payload type a transport reports, so
  a peer's own dynamic numbering is honored — including the two numbers an
  AMR session commonly negotiates at once, which no name-keyed table could
  express. Packet times AMR cannot accept are re-framed (10 ms joined,
  30 ms split with the remainder carried).

## Proxy and PBX interop

- Kamailio and OpenSIPS registrar-proxy labs, TLS to the proxy and SRTP
  through rtpengine, with opt-in AMR-NB transcoding. The AMR-WB transcode
  failure is attributed to rtpengine and recorded as such.
- A per-rate AMR sweep bound to the gate catalog, and proxy-PBX matrix rows
  verified in the release report.
- New gates in the catalog: the AMR per-rate sweep family, the proxy-PBX
  media family, and AMR decode/encode/unpack fuzz targets.

## SIP correctness

- RFC 3261 §12.1.1: a secure fallback Contact is generated for every trigger
  — SIPS Request-URI, SIPS topmost Record-Route, or SIPS Contact when no
  Record-Route is present — at the TLS-advertised address. Explicit Contact
  and plain-SIP behavior are unchanged. This also repairs rvoip-to-rvoip
  SIPS setup, since `Dialog::from_2xx_response` refuses a secure dialog whose
  Contact is not `sips:` (issue #176).
- RFC 3261 §12.1.2: the UAC learns its route set from the dialog-forming
  2xx's Record-Route, reversed, preserving every URI parameter. Without it,
  in-dialog requests bypassed every record-routing proxy in the path.
- The profiled egress registration exposes its coordinator for
  observation-only subscriptions, so an application can install security
  evidence monitors before registration. The composite adapter remains the
  sole signaling and lifecycle owner.

## Architecture and compatibility

- `CodecInfo` carries the payload type it negotiated, and `Config` gains
  `amr_dtx`, `amr_auto_cmr`, and `amr_mode_set`. The 0.3.x line accepts
  additive `Config` fields; construct through the documented constructors
  rather than struct literals.
- An opus↔opus bridge stays passthrough when its two legs numbered opus
  differently. The payload type is a per-leg SDP artifact, not a property of
  the encoded audio, so the bypass compares name, clock rate and channels,
  and passthrough restamps the sink's payload type on egress.
- A barge-in flush empties the re-framing accumulator as well as the sink
  queues, so no pre-interruption audio and no dead-timeline timestamp
  survives into the first frame after the flush.
- The AMR claim is bounded by what was measured: the recorded lab matrix,
  its peer versions, and the rates actually swept. It does not extend to
  untested handsets, carriers, or transcoding gateways.

## Qualification

The candidate must pass a fresh `remote-release` qualification from a clean,
committed `0.3.8` source tree. The aggregate is bound to the exact candidate
commit and to the catalog hash, and this release changes the catalog — it adds
the AMR per-rate sweep, proxy-PBX media, and AMR fuzz families — so no earlier
run's evidence can be reused for any gate.

Historical `0.3.2` exception, `0.3.4` carry-forward, and prior `0.3.6` and
`0.3.7` attestations remain unchanged release history. They are not presented
as current `0.3.8` evidence.
