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
| Opus | 111, or whatever was negotiated | Conventional dynamic PT, overridden by a reported one |
| AMR-NB / AMR-WB | The negotiated PT only | `amr-nb` / `amr-wb` feature; no conventional fallback exists |
| `pcm_s16le` | 120 | Internal only; transports must not advertise or emit it |

A codec's key is `CodecInfo::payload_type` when the transport reported one, and
otherwise its row in [`bridge::codec_to_pt`](../src/bridge/mod.rs). The
reported value wins, which is what admits a dynamic codec at all: its number is
chosen per call, so no table row could describe it.

Admission goes through `media_graph::admit_codec`, which resolves that key
**and** builds the codec to prove it exists, discarding the result. Both halves
are necessary. Before payload types were carried, a resolvable key implied a
buildable codec because keys came from a table of five the graph could all
construct; now a transport can report any number, and a key alone would let a
misspelled codec name through the door to fail somewhere downstream.

## What it does not carry

Any codec that reports no payload type and has no row in `codec_to_pt`, and any
codec whose name does not build. Both fail at every entry point —
`start_media_graph`, `add_sink` / `add_managed_sink`, `update_source_codec`,
`update_sink_codec` — with `RvoipError::UnsupportedCodec`, naming the codec.
`validate_media_graph_codec` exposes the same check standalone so a caller can
ask *before* it acquires a stream's single-consumer receiver to hand over:
`start_media_graph` takes that receiver by value and drops it on the error path.

**AMR without a reported payload type is still refused**, and that is not a
leftover — it is the correct answer. There is no number to label its frames
with, and the key the graph computes is stamped onto what it emits.

### AMR's remaining constraint: 20 ms frames

`ConfiguredTranscoder::transcode` decodes, resamples, and encodes. It never
re-frames. `AmrAdapter::encode` requires exactly one frame — 160 samples
narrowband, 320 wideband — so a 20 ms source works and a 10 ms or 30 ms one
fails on every frame with a codec error rather than a graph refusal, because
the codec built fine and only the frame size is wrong.

This is documented rather than fixed. A re-framing accumulator in the graph
would change buffering and latency for every codec that flows through it, which
is a larger decision than AMR support and should be made on its own terms.

AMR as a *source* can also emit several frames concatenated (redundancy,
bundling), which a target codec with its own fixed frame size will reject. The
same accumulator question governs that case.

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

## How AMR was wired in

This section was a plan. It is now a record of what was done, kept because the
order matters to anyone adding the next dynamic codec.

1. **A negotiated-PT channel into the graph.** `CodecInfo` gained
   `payload_type: Option<u8>` with `#[serde(default)]`, round-tripped through
   the `Codec` wire shape so a value cannot be silently lost on the way back.
   `None` means "not reported", never "no payload type" — a consumer that needs
   one must decide what to do about the absence rather than substitute a guess.

2. **Adapters populating it.** Three producers report one, and they are the
   three that know: the SIP adapter (`codec_descriptor`, where it is already
   the argument the SDP answer settled on), the WebRTC SDP parser (read off the
   m-line, the same number the fmtp beside it is looked up by), and
   `codec_for_payload_type`, whose input is the payload type. Capability
   advertisements, pre-negotiation placeholders and synthesised fallbacks all
   report `None`: an advertisement is not a result.

   UCTP negotiates codecs **by name** and has no payload type to report, which
   is why QUIC and WebTransport still resolve through the name table.

3. **A feature flag.** `amr-nb` / `amr-wb` / `amr` on rvoip-core, forwarding to
   media-core. Opt-in rather than default, unlike `opus`: these are full 3GPP
   codecs and most consumers of the graph never carry one.

4. **Codec construction through `AudioCodecSpec`.** `create_configured_codec`
   keeps its existing arms for payload types 0/8/18/111/`pcm_s16le` and falls
   through to `AudioCodecSpec::build` for everything else — the only
   constructor that takes fmtp, which AMR needs because `octet-align` decides
   the framing.

   The existing arms were deliberately *not* migrated. The trap this document
   recorded is real: the graph's inline Opus arm honours `maxaveragebitrate`
   and `cbr=1`, and `AudioCodecSpec::build`'s Opus arm uses
   `OpusConfig::default()`, so migrating it would silently drop both.
   Consolidating those two is worth doing on its own terms, by teaching
   `AudioCodecSpec` the fmtp handling rather than by discarding it here.

5. **`codec_for_payload_type` still refuses dynamic payload types**, unchanged,
   for the reason in the section above.

6. **Framing** is documented rather than solved — see the 20 ms constraint
   above.

## A related hazard, fixed

The QUIC and WebTransport media pumps both did:

```rust
let default_payload_type = rvoip_core::bridge::codec_to_pt(&codec.name).unwrap_or(111);
```

which stamped **Opus's payload type on the datagrams of any codec the table did
not know** — AMR included — for every frame that did not carry its own
(transcoder output and synthetic frames both leave it `None`). Where the graph
refused an unknown codec and said so, these two accepted it and corrupted it,
producing a well-formed datagram that lies about its contents. A receiver
cannot detect that; there is nothing malformed to notice.

Both now resolve through `bridge::resolve_payload_type`, which prefers the
negotiated value and falls back to the name table. When neither yields a
number the frame is dropped with a counter and a log line naming the codec —
`uctp_datagram_drops_total{reason="unlabelled-payload-type"}` — rather than
sent under a fabricated one. A dropped frame is visible and recoverable; a
mislabelled one is neither.
