# Codecs in the rvoip-core media graph

The media graph is the one-source-to-many fan-out in
[`media_graph.rs`](../src/media_graph.rs). It is what UCTP publishing,
recording and MOQT fan-out observe a call through. This document records which
codecs it carries, which it does not, and — for AMR, the one that is fully
implemented everywhere else — the design decision that governs how it would be
added.

## What the graph carries

| Codec | Graph key | Where the key comes from |
|---|---|---|
| PCMU | 0 | RFC 3551 static assignment |
| PCMA | 8 | RFC 3551 static assignment |
| G.729 | 18 | RFC 3551 static assignment |
| Opus | 111 | Conventional dynamic PT |
| `pcm_s16le` | 120 | Internal only; transports must not advertise or emit it |

The table lives in three places that must agree:
[`bridge::codec_to_pt`](../src/bridge/mod.rs) (name → key),
`media_graph::create_configured_codec` (key → constructed codec), and
`media_graph::codec_for_payload_type` (key → codec descriptor, for the
`update_route` compatibility wrapper).

## What it does not carry

**AMR-NB and AMR-WB.** Both variants are implemented in `rvoip-codec-core`,
reachable through media-core's `AudioCodecSpec` and `AmrAdapter`, and carried
end to end on the SIP media path — see
[`AMR_IMPLEMENTATION_STATUS.md`](../../../media/codec-core/docs/AMR_IMPLEMENTATION_STATUS.md).
The graph refuses them.

`codec_to_pt("AMR")` returns `None`, so every graph entry point — the
`start_media_graph` source codec, `add_sink` / `add_managed_sink`,
`update_source_codec` and `update_sink_codec` — fails with
`RvoipError::UnsupportedCodec("AMR")`, naming the codec. `validate_media_graph_codec`
exposes the same check standalone so a caller can ask *before* it acquires a
stream's single-consumer receiver to hand over, which is worth doing:
`start_media_graph` takes that receiver by value and drops it on the error
path. The refusal is deliberate and it is loud; nothing is silently degraded.

The consequence is narrow and worth stating exactly: **a SIP AMR call works,
but it cannot be published over UCTP, recorded through the graph, or fanned out
to MOQT.** "AMR is implemented" and "AMR works everywhere media flows" are
different claims, and only the first is true.

## The design decision

AMR's payload type is negotiated per call — this repo's own SIP integration
test pins 104, 106 and 107, and the interop labs see that range. Opus has the
same property and was given a conventional 111. Does AMR get conventional keys
in the same spirit?

**Decided: no. AMR gets no conventional key. Dynamic codecs must key on the
negotiated payload type.**

Four reasons, in the order that decides the question.

**1. AMR occupies more than one payload type at the same time.** The repo's own
SIP integration test negotiates AMR-NB twice in one session —
`AMR_NB_BE_PT = 106` and `AMR_NB_OA_PT = 107`
([`amr_call_integration.rs:36`](../../../sip/rvoip-sip/tests/amr_call_integration.rs)) —
same codec, two payload types, told apart only by `octet-align`. This is
ordinary SDP practice for AMR, not a corner case. A single conventional key
cannot represent a session that offers both, so the question is not "which
number" but "one number at all", and the answer to that is no.

**2. There is no convention to borrow.** Opus's 111 is a de-facto industry
default that most stacks happen to agree on, which is why keying Opus at 111
mostly works. AMR has no such number. Picking one would invent a convention
rather than follow one, and every peer that negotiated something else would be
mislabelled.

**3. The graph stamps its key onto the wire.** `route_source_frame` sets
`grouped.payload_type = Some(group.target_pt)` on every transcoded frame, and
the QUIC and WebTransport pumps put that value straight into the RTP header of
the outgoing datagram. A fabricated key is not an internal detail; it is a
wrong payload type on a real packet, and the peer cannot decode it.

**4. The graph does not need the payload type to tell AMR sessions apart.**
`CodecGroupKey` carries the codec name, clock rate, channels and the
*normalized fmtp* alongside the payload type, and `make_transcoder` compares
whole keys rather than bare payload types:

```rust
(CodecGroupKey::new(source_codec, source_pt) != CodecGroupKey::new(target_codec, target_pt))
    .then(|| ConfiguredTranscoder::new(...))
```

So `mode-set` and `octet-align` already separate two AMR groups, and two
streams with identical fmtp already share one and pass through — which is
correct. A conventional key would add no information that fmtp is not already
carrying, while making the wire wrong. This is what makes AMR a plumbing
problem rather than a design problem: the graph's internals are already
fmtp-aware, and only its *entry points* insist on deriving a payload type from
a codec name.

`codec_to_pt`'s own doc comment already warns against forwarding an arbitrary
dynamic PT. This decision is that warning applied to AMR.

### Why `codec_for_payload_type` must stay a static-only map

The reverse map returns a descriptor with `fmtp: None`. For AMR that is not a
lossy answer but a wrong one: with no fmtp, `octet-align` falls back to
bandwidth-efficient, and `AmrPayloadFormat::from_negotiated` is explicit that
using defaults instead of the negotiated parameters is "not a degraded mode, it
is a broken one" — the framing is wrong and the peer cannot parse the stream at
all. Any future AMR support must reach the codec through the negotiated fmtp,
never through a payload type alone.

## What wiring AMR in actually requires

Ordered, because the first two are what make this a feature rather than a patch.

1. **A negotiated-PT channel into the graph.** `MediaStream::codec()` returns
   `capability::CodecInfo`, which carries name, clock rate, channels and fmtp —
   and no payload type. No transport reports its negotiated PT to the graph
   today. Either add `payload_type: Option<u8>` to `CodecInfo` (`#[serde(default)]`
   keeps the wire format compatible; roughly 106 struct-literal sites across 11
   crates need updating), or add a payload-type-carrying entry point beside
   `add_managed_sink` / `update_source_codec` / `update_sink_codec`. The graph's
   internal `Command::UpdateRoute` already carries explicit payload types, so
   the actor side needs little change.

2. **Adapters populating it.** Plumbing alone changes nothing. SIP, WebRTC,
   QUIC and WebTransport each have to report the payload type they negotiated,
   or the field is always `None` and AMR still cannot attach.

3. **A feature flag.** rvoip-core pulls media-core as `features = ["opus"]`,
   and `AudioCodecSpec::build`'s AMR arm is behind `amr-nb` / `amr-wb`. Without
   enabling them the arm is not compiled and no amount of routing reaches it.
   AMR is a full 3GPP codec, so this should be opt-in rather than added to
   rvoip-core's defaults.

4. **Codec construction through `AudioCodecSpec`, not `CodecFactory`.**
   `CodecFactory::create_codec(payload_type, sample_rate, channels)` has no
   fmtp parameter and therefore cannot build AMR at all. media-core already
   has the replacement:
   [`AudioCodecSpec`](../../../media/media-core/src/codec/spec.rs) carries name,
   payload type, clock rate, channels and fmtp, and its `build()` handles AMR
   today. Its module doc names this exact gap — Opus "got a special case in the
   transcoder and another in `rvoip-core::media_graph`", and AMR "cannot get one
   at all". `create_configured_codec` should migrate onto it rather than grow a
   third special case.

   One trap when migrating: the graph's inline Opus arm honours
   `maxaveragebitrate` and `cbr=1`, while `AudioCodecSpec::build`'s Opus arm
   uses `OpusConfig::default()`. A naive migration silently drops both.

5. **`codec_for_payload_type` left refusing dynamic payload types**, for the
   reason in the section above.

6. **Framing.** `AmrAdapter::encode` requires exactly one frame — 160 samples
   narrowband, 320 wideband. The graph's transcode path resamples but never
   re-frames, so a 20 ms source works and a 10 ms or 30 ms one fails on every
   frame. AMR as a *source* can also return several frames concatenated
   (redundancy, bundling), which a target codec with its own fixed frame size
   will then reject. Whoever wires AMR in should decide whether the graph grows
   a re-framing accumulator or documents a 20 ms-only constraint.

## A related hazard, unfixed

The QUIC and WebTransport media pumps both do:

```rust
let default_payload_type = rvoip_core::bridge::codec_to_pt(&codec.name).unwrap_or(111);
```

Where the graph refuses a codec it does not know, these two stamp **Opus's
payload type on the datagrams of any codec the table does not know** — AMR
included — for every frame that does not carry its own payload type
(transcoder output and synthetic frames both have `payload_type: None`). The
graph boundary fails loudly; this one does not. It is out of scope for the AMR
decision above, but it is the same missing-negotiated-PT root cause and should
be fixed by item 1 rather than by widening the `codec_to_pt` table.
