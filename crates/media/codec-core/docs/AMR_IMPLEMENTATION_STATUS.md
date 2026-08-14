# AMR implementation status

## Where things stand

| Component | State |
|---|---|
| **AMR-WB decoder** | **Bit-exact against TS 26.173, all nine rates** |
| **AMR-NB decoder** | **Bit-exact against TS 26.073, all eight rates** |
| **AMR-WB encoder** | **Byte-identical bitstream, all nine rates, 50 frames** |
| **AMR-NB encoder** | **Byte-identical bitstream, all eight rates, 50 frames** |
| **Concealment** | **Bit-exact, both variants** — damaged frames and lost frames |
| **All four paths reachable through `AmrCodec`** | **Done, and byte-exact through the public API** |
| **Oracle qualification** | **Measured** — see below |
| **AMR-WB DTX, encoder side** | **Byte-identical SIDs on the reference's own schedule**, 150 frames |
| **AMR-WB DTX, decoder side** | **Sample-exact over a 150-frame DTX stream** |
| **AMR-WB DTX through `AmrCodec`** | **Both directions, byte- and sample-exact** |
| **AMR-NB VAD1** | **Bit-exact** — whole state, 150 frames |
| **AMR-NB DTX, encoder side** | **Byte-identical bitstream, all eight rates, 150 frames** |
| **AMR-NB DTX, decoder side** | **Sample-exact, all eight rates, plus the muting stream** |
| **AMR-NB DTX through `AmrCodec`** | **Both directions, byte- and sample-exact** |
| **AMR-WB homing frames** | **Done** — the encoder emits each mode's pattern |
| **AMR-NB homing frames** | **Done** — the encoder emits each mode's pattern |
| **AMR-WB conformance, speech** | **All nine TS 26.173 vectors, both directions** |
| **AMR-WB conformance, DTX encode** | **`tst_md.cod`, every frame type and payload** |
| **AMR-WB conformance, DTX decode** | **`tst_md`, 200 frames, sample for sample** |
| **AMR-NB conformance, encode** | **`spch_dos`, 425 frames, every bit** |
| **AMR-NB conformance, decode** | **`spch_dos`, 425 frames, sample for sample** |
| **AMR reachable through media-core** | **Both variants resolve, encode and decode end to end** |
| **AMR through `rvoip-core`'s media graph** | **Wired**, behind rvoip-core's `amr-nb`/`amr-wb` — frames cross both directions, tone-verified |
| **Non-20 ms senders** | **Re-framed** — 10 ms and 30 ms sources transcode into AMR |
| **AMR over a QUIC datagram** | **Payload type survives; an unlabellable frame is dropped, not mislabelled** |
| **Every mode in a live call** | **All 8 narrowband and all 9 wideband, walked by CMR** |
| **AMR over SDES-SRTP, in process** | **Both variants, tone-verified** |
| **Transcoding** | **Six AMR pairs, tested by property** |
| **`mode-change-period` / `-neighbor`** | **Honoured** — they were parsed and obeyed by nothing |
| **Performance** | **Measured, with a gate** — see below |
| **SDP to a working codec** | **An AMR-WB offer negotiates and codes** |
| **A real AMR call** | **Three, over loopback, with verified audio** |
| **RFC 4867 wire format** | **136 payloads agree with Wireshark's dissector** |
| **Live PBX interop** | **Both PBXes, both variants, relay AND forced-transcode tiers, UDP ×3 + TLS, quality-gated** |
| **Per-rate PBX interop** | **All 17 modes against Asterisk, each pinned by `mode-set`** — see below |
| **`mode-set` negotiation** | **Offered, echoed in answers, and obeyed by the encoder** |
| **Live proxy interop** | **Kamailio and OpenSIPS with rtpengine relaying all four framings verbatim** |
| **AMR-NB VAD2** | **Bit-exact against TS 26.073** — whole state, 300 half-frames |
| **Interface formats** | **IF1 and IF2, both variants**, from TS 26.101 and TS 26.201 |
| **Redundancy and interleaving** | **`max-red` scheduling with dedup; receive-side deinterleaving** |
| **Soak** | **Opt-in long-run encode/decode, `RVOIP_AMR_SOAK_SECS`** |
| **Fuzz** | **Encoders and decoders both have targets** |

Every claim above is a test, not a note. Both decoders reproduce the reference
decoders sample for sample; both encoders reproduce the reference *bitstream*
byte for byte, magic and table of contents included. Each is asserted twice —
once inside the module and once through `AmrCodec` — because per-stage
exactness cannot see a wiring layer that resets state between frames, and each
comparison is checked for vacuity.

**What "AMR-WB DTX, encoder side" claims, exactly.** The comfort-noise ISF
quantiser matches `Qisf_ns` over 64 vectors; the DTX kernel matches
`dtx_buffer` and `dtx_enc` over 40 frames on all five ISF indices, the energy
index, the dithering flag and the excitation; and with DTX enabled the encoder
reproduces the reference's own speech / SID / `NO_DATA` sequence over all 150
frames of the committed fixture — and every one of the twelve transmitted SID
payloads is byte-identical, STI bit, mode indication and `SID_FIRST` blanking
included. A right kernel fed the wrong residual or the wrong history would
produce a well-formed SID with wrong bits, which the frame-type sequence alone
would not notice.

**What "AMR-WB DTX, decoder side" claims, exactly.** Two things. The kernel —
state machine, backward analysis, interpolation, `DTX_MUTE` fade,
comfort-noise generator — is bit-exact against `rx_dtx_handler`, `dtx_dec` and
`dtx_dec_activity_update` over an 89-frame sequence visiting all three states.
And the assembled decoder reproduces the reference decoder **sample for
sample** over the whole 150-frame DTX stream: speech, comfort noise and gaps,
two transitions into silence and two back.

Two defects stood between those, and neither was in the kernel. The background
energy history is measured on the excitation *brought back out of the frame's
scaling*; skipping that inflated every stored energy by `2^Q_new` and reached
the output as comfort noise hundreds of times too loud while the spectrum was
already exact. And `Reset_decoder(st, 0)` clears more than the excitation —
the ISF predictor, the pitch-lag history, the innovation tilt, the phase
dispersion memory and the noise enhancer's threshold — an omission invisible
until the first speech frame *after* the silence.

**And two more that only the normative stream could reach.** The committed
fixture passed while both were present, which is the whole argument for
fetching TS 26.174 rather than trusting a locally generated one.

The energy history has to be captured from an excitation in *one* exponent.
`rescale_to` rewrites the entire history when a subframe needs a different one,
so by the end of a frame the reference's buffer is uniform and a single
`Scale_sig` undoes it — but snapshotting each subframe as it is built leaves
four exponents mixed together. It corrupts exactly the frames where the scaling
moved, three of eight ring slots on this stream and none on the fixture.

And `CN_dithering` draws its generator **twice** per perturbation and sums the
halves — a triangular variate, not a uniform one — and enforces the 448-unit
ISF spacing *inline* against the coefficient just written rather than in a pass
afterwards. Written from a summary of the algorithm it was wrong in both
respects, and only a stream whose encoder actually sets the dithering bit can
tell. Ours does; the fixture's does not.

**What the narrowband DTX claims, exactly.** With DTX enabled the encoder
reproduces the reference's own speech / SID / `NO_DATA` sequence over all 150
frames at **every one of the eight rates**, and each of the twelve transmitted
SIDs is byte-identical — description, STI bit and mode indication. The decoder
reproduces the reference decoder sample for sample over the same eight streams,
and over the muting stream as well.

Four things cost more than the port itself.

The `_mute` fixture never muted. It dropped every SID from frame 50 on,
including the one that opens the silence, so the decoder never learned DTX had
begun and read all 61 empty frames as lost speech. Its build-time guard passed
because the output does differ from the intact stream — just not for the reason
claimed. With the opening SID kept, `DTX_MUTE` is reached for eleven frames and
the fade is finally covered: deleting the 0.75 dB step moves 374 samples.

The first speech frame out of `DTX_MUTE` must be muted, so `prev_bf` is written
*before* the concealers read it. Snapshotting the frame quality first left them
looking at the value from before the silence.

Narrowband transmits a description on a `SID_FIRST`; wideband blanks one.
Carrying the wideband habit across gives five octets of `00 00 00 00 02` where
the reference has a full description — and since both decoders derive a
`SID_FIRST`'s spectrum by backward analysis and ignore the payload, nothing
sounds wrong. Only a bitstream comparison catches it.

And the speech path has to run the receiver too. Decoding a speech frame
without `rx_dtx_handler` and `dtx_dec_activity_update` enters the next silence
with an empty history and a stale hangover count: 1928 of 24000 samples wrong
at 4.75 kbit/s, every one of them inside the comfort noise.

**And the narrowband verification stream, which the fixtures cannot imitate.**
`spch_dos` is 425 frames driven through `allmodes.txt`, so the rate changes on
424 of them, with DTX on throughout. The encoder reproduces all 36575
transmitted bits — every frame's type, rate word and payload — and the decoder
reproduces all 68000 output samples. Each committed fixture is a single
constant rate; a rate switch carries the LSF predictor, both gain predictors,
the pitch history and the DTX rings across a change in what those numbers mean,
and no fixture reaches that state at all. It contains no homing frame, and the
tests assert that rather than implying otherwise.

## Performance, measured

`cargo bench -p rvoip-codec-core --all-features --bench amr_codec`, then
`python3 tools/check-amr-rtf.py`. A 20 ms frame has a 20 ms budget, so the
real-time factor is `time / 20 ms`.

| Path | Worst rate | RTF |
|---|---|---|
| AMR-NB encode | 12.2 | 0.0095 |
| AMR-NB decode | 7.95 | 0.0010 |
| AMR-WB encode | 23.85 | 0.0229 |
| AMR-WB decode | 23.85 | 0.0056 |
| AMR-NB conceal | — | 0.0007 |
| AMR-WB conceal | — | 0.0031 |

A duplex leg — encode plus decode, since a call does both — costs 0.0105 at
narrowband and 0.0285 at wideband: about 95 and 35 legs per core. The plan's
goal was 20 concurrent legs.

The gate is a script over Criterion's own output rather than an assertion in
`cargo test`, following the policy the G.711 benchmark's header sets out: a
debug build under a loaded scheduler produces numbers that are meaningless as
a pass/fail signal. It fails if it finds *no* AMR results, because a
performance gate that silently checks nothing is the same failure this branch
has hit four times elsewhere.

## Running the conformance sequences

```bash
crates/media/codec-core/tools/run-amr-conformance.sh
```

It fetches and builds both reference trees if they are absent, sets the two
environment variables, and runs the six `#[ignore]`d tests. It fails unless all
six pass — a conformance run that checks five of six is not a pass, and the
count cargo happens to report is not the claim.

**These six cannot run in CI, and that is structural.** The sequences are 3GPP
copyright; only generated output is committed. So the six strongest rows in the
table above are the ones continuous integration protects least, and they are
distinguishable from the fixture-backed rows on exactly that basis. Everything
else — including all four bit-exactness paths against the committed fixtures —
runs on every pull request through the `codec-features` gate.

## The wire format, checked against someone else's dissector

```bash
crates/media/codec-core/tools/verify-amr-rtp-framing.sh
```

136 payloads across eight selections — both variants, both framings, every
speech mode, with and without a mode request, and single- and two-frame
payloads — packed by this crate and read back by **Wireshark's** AMR dissector,
which is an independent implementation of RFC 4867. Frame type and CMR agree on
all of them.

This closes a gap that neither the 3GPP vectors nor an rvoip-to-rvoip call can
reach. The codec *bits* are bit-exact against the reference implementations and
their sorting is checked against reference-produced `.amr` files, but
everything RFC 4867 adds for RTP — the CMR nibble, the table-of-contents chain,
octet-aligned padding — was otherwise verified only by packing and unpacking
with our own code. A round trip cannot catch a symmetric mistake: put the CMR
in the wrong four bits and our depacker reads it out of the wrong four bits,
the audio is perfect, and no peer can read the stream. Two rvoip endpoints
calling each other cannot find that either, for exactly the same reason.

Verified by mutation: rotating the first octet's nibbles makes the dissector
disagree — 47 of the 68 payloads the corpus held when that check was run by
hand. The script has no mutation mode, so that figure is a record of a
one-off run against the smaller corpus, not something the current 136-payload
run reproduces.

## The other two interface formats, IF1 and IF2

```bash
cargo test -p rvoip-codec-core --features amr interface_format
```

RFC 4867 is how AMR crosses an IP network, but it is not how AMR crosses a
radio access network or a 3GPP-defined interface. TS 26.101 (narrowband) and
TS 26.201 (wideband) define IF1 and IF2 for that, and `interface_format.rs`
implements both for both variants.

The two are not one format with a width parameter, and assuming they were is
what made the first attempt wrong. IF2 differs between variants in every field
that matters: narrowband puts the frame type in the **low** nibble and orders
bits **LSB-first** with no frame-quality bit and a 39-bit SID; wideband puts it
in the **high** nibble, orders bits **MSB-first**, carries a 1-bit FQI and a
40-bit SID. IF1 adds the codec CRC, whose generator `G(x) = x⁸+x⁶+x⁵+x⁴+1` is
the bit-reversal of the polynomial RFC 4867 uses for its payload CRC — close
enough to look like a typo and produce wrong bytes silently.

**What this cost, and the lesson.** An earlier wideband IF2 implementation was
committed as "oracle-verified" and was wrong: it omitted the FQI bit, so every
frame was one bit short. The oracle was tshark, which reads the frame-type
nibble and never checks the payload length — so it agreed with a stream no
real IF2 peer could parse. The error surfaced only when the field tables were
read directly. An oracle that does not examine the field you got wrong is not
evidence about that field, however green it prints.

## AMR-NB VAD2

```bash
cargo test -p rvoip-codec-core --features amr-nb vad2
```

TS 26.094 specifies two voice-activity detectors for narrowband and the encoder
selects between them; VAD1 alone is half the specification. VAD2 is a 128-point
FFT over 16 channels feeding an SNR and hangover state machine, bit-exact here
against the reference over 300 half-frames of committed trace covering the
whole state — every counter and all three sixteen-element arrays, not just the
boolean.

Comparing the decision alone would have been close to worthless. It is one bit,
it agrees with a constant most of the time, and VAD2's frame decision is the OR
of two calls per frame — so a wrong half-frame can be masked entirely by its
partner. `tools/nb_vad2_probe.c` dumps the full per-half-frame state from the
reference so a divergence localises to a stage instead of to "somewhere".

## Soak and fuzz

```bash
crates/media/codec-core/tools/run-amr-soak.sh
crates/media/codec-core/tools/run-amr-fuzz.sh
```

The bit-exactness fixtures are 50 frames. Nothing in them can catch state that
degrades over minutes, or an allocation that grows once per thousand frames.
`soak.rs` holds `#[ignore]`d long-run encode/decode tests scaled by
`RVOIP_AMR_SOAK_SECS` so the default `cargo test` stays fast and the long run
is deliberate.

Fuzzing covers both directions. The decoders had a target already — the obvious
one, since decoders eat hostile input by definition. The encoders now have
`fuzz/fuzz_targets/amr_encode.rs` too, because mode changes, DTX transitions and
CMR requests form a state machine that arbitrary drivers can walk into corners
that fixtures never visit.

## Interop, against both PBXes and both proxies

The SDP half is done and tested: an AMR-WB offer with
`octet-align=1; mode-set=0,2,4` negotiates through the SIP layer, reaches
media-core with its name, dynamic payload type, 16 kHz clock rate and fmtp
intact, and a codec built from exactly that negotiation round-trips a frame.
Each of those four was separately broken during this work, so the test asserts
all four together.

The live calls are done, in three tiers that prove different things.

**Tier 1 — `amr_call`, the relay tier.** Both legs offer the same codec, so
both PBXes forward our RTP octets untouched — verified from their own
behaviour: Asterisk logs `bridge_native_rtp.c: Locally RTP bridged` with
`ReadTranscode: No` and `codec_amr.so` use count 0; FreeSWITCH's egress
payloads are byte-identical to its ingress (75/75 narrowband, 38/38 wideband).
The media path is rvoip's encoder to rvoip's own decoder with a forwarder in
between, so this tier proves SDP/fmtp negotiation, per-leg dynamic-PT mapping,
and framing survival — **not** that a foreign codec can read our bitstream. A
bug shared by our encoder and decoder cancels out here, the same
symmetric-mistake blindness `tools/verify-amr-rtp-framing.sh` covers one layer
down. (This tier was briefly claimed as more than it is; the correction is
recorded in the history rather than erased.)

**Tier 2 — `amr_transcode_call`, the tier that closes that hole.** The caller
and callee offer *disjoint* codecs (AMR on one leg, PCMU on the other), so the
PBX physically cannot native-bridge: its own AMR implementation must decode
every frame we send and encode every frame we receive. The 440 Hz the PCMU leg
records exists only because the PBX's AMR decoder read our frames; the 880 Hz
the AMR leg records exists only because the PBX's AMR encoder produced frames
our decoder read. Corroborated from Asterisk's side: the transcode calls stay
on `simple_bridge` with `codec_amr.so` use count 2 mid-call (captured as an
artifact by `PBX_DIAG=1`), where every matched-codec call switches to
`native_rtp` with use count 0.

The guard on tier 2 is a unit test — every pairing's two legs intersect to
telephone-event alone — not the call passing. Measured deliberately: forcing
both legs onto one codec via `ENDPOINT_{user}_CODEC_PROFILE` flips Asterisk
back to `Locally RTP bridged` and the call *still passes*, which is exactly
why a passing call cannot be the guard.

All cells pass the quality gate (below), three consecutive repeats each on
UDP, plus TLS+SRTP — the first AMR-over-SRTP evidence in the repo:

| PBX | Scenario | Cells | UDP ×3 | TLS |
|---|---|---|---|---|
| Asterisk 20.20.1 | relay | amrnb, amrwb | ✔ | ✔ |
| Asterisk 20.20.1 | transcode | amrnb_pcmu, amrwb_pcmu | ✔ | ✔ |
| FreeSWITCH 1.10.12 | relay | amrnb_be, amrwb_be | ✔ | ✔ |
| FreeSWITCH 1.10.12 | transcode | amrnb_be_pcmu, amrwb_be_pcmu | ✔ | ✔ |

The `amrwb_pcmu` cells also exercise the PBX's own 16 kHz ↔ 8 kHz resampler.

**Tier 3 — the proxy tier, Kamailio and OpenSIPS with rtpengine.** A B2BUA
terminates the media; a proxy does not. This tier puts a registrar-proxy in the
signalling path via Record-Route and rtpengine on the media path as a pure
relay, so all four AMR framings cross a middlebox that rewrites addresses and
ports and nothing else. rtpengine's own totals report zero transcoded media,
which is the assertion: our payloads arrive verbatim.

The lab is built to fail closed. A `rtpengine_manage` that does not succeed
returns 503 rather than passing the SDP through, because the alternative is the
endpoints negotiating directly and every media assertion passing vacuously —
which is exactly what an earlier version of this lab did before the guard
existed. Kamailio runs over UDP and TLS with SDES-SRTP; OpenSIPS runs UDP only,
having no TLS image yet.

This tier is lab evidence, not release-gate evidence: it does not run TCP or
both adjacency orders, and it is not bound into the four-peer attestation.

The framing column is not a preference, it is what each PBX can actually carry
end to end:

- **Asterisk** answers whichever framing we offer, on both legs, so the
  octet-aligned offer is honoured in both directions. (Verified separately:
  offering bandwidth-efficient instead yields byte-identical decoded audio, so
  the framing is not what distinguishes these rows — the PBX is.)
- **FreeSWITCH** *relays* AMR between the two legs of a bridged call without
  re-framing the payloads, and its outbound leg always offers `octet-align=0`.
  An octet-aligned inbound leg therefore leaves both endpoints reading the
  framing they did not agree to. The two legs only agree if ours is
  bandwidth-efficient as well, which is what the `amrnb_be` / `amrwb_be`
  profiles offer (PT 106 and 104 rather than 107 and 105).

  `mod_amr`'s `force-oa` does not fix this and is deliberately left at 0:
  setting it to 1 makes FreeSWITCH answer `octet-align=1` to an offer that
  asked for bandwidth-efficient, which RFC 4867 §8.3.1 does not permit, and
  breaks the inbound leg instead of aligning the outbound one.

  On the *transcoding* path, `mod_amr` has a second framing defect, measured
  directly: offered octet-aligned AMR (PT 107, `octet-align=1`), it negotiates
  correctly and then instantiates its **Bandwidth Efficient** decoder anyway —
  `Codec AMR / Bandwidth Efficient decoder error!` on every frame. So the
  bandwidth-efficient pairings are FreeSWITCH's transcode defaults too.

That split is better coverage than either alone: Asterisk exercises the
octet-aligned path and FreeSWITCH the bandwidth-efficient one, whose payload
boundaries do not fall on octets — in both tiers.

**The audio gate measures quality, not just pitch.** The first tone check was
a single Goertzel dominance ratio, and it could not discriminate: measured
against real captures, 1-bit squaring passed at ratio 441, 100× attenuation at
6820, half the frames zeroed at 922 — and a genuinely degraded capture passed
at 237×. The gate now requires, continuously for one second at the leg's own
rate: the far end's tone dominant, per-window fundamental-vs-residual SNR
≥ 15 dB (a true-dB figure — injected noise reads back within 1 dB), and every
20 ms frame above a quarter of the sent RMS. Each clause exists because a
measured failure defeated the others, and each is pinned by a test, including
the wideband-read-as-narrowband case (a clean tone an octave low) that nothing
else notices.

**The one real audio defect found was ours, and not in the codec.** The
harness's tone sender paced with a trailing `sleep(20 ms)`, making the true
send period 20 ms plus work plus scheduler slop — measured 21.5 ms/frame via
the Asterisk relay, 24 ms under load. FreeSWITCH re-clocks media to a true
20 ms; starved by a 24 ms source it stretches, which surfaced as level swings
and a 0.75 Hz-flat tone offset. (The "−12.6 dB" first measured was itself
partly metrology: a whole-file Goertzel at exactly 440.00 Hz cancels a
0.75 Hz-offset tone. Real coherent damage was ~2 dB plus the modulation.)
With the sender on a proper interval the same cells measure +26 to +30 dB.
Every audio measurement made through a PBX before that fix was confounded.

Environment defects found and fixed along the way, none of them AMR's: the
`rvoip_*` FreeSWITCH profiles pinned `inbound-codec-prefs` to `G729,PCMU,PCMA`,
so an accepted AMR call was bridged out as G.729 and refused; the container
advertised its docker-internal address in SDP unless started through
`scripts/up.sh`, so RTP went to an unroutable host; and for the transcode tier,
FreeSWITCH gained `rvoip_udp_xcode`/`rvoip_tls_srtp_xcode` profile twins
(5064/5065) with `disable-transcoding=false`, early codec negotiation (late
negotiation makes its bridge offer the B-leg only the A-leg's codec), and a
dialplan export of the full codec list to the originated leg. The relay
profiles are byte-for-byte untouched, so the tier-1 evidence stands. All of it
lives in `~/Developer/freeswitch/docker-entrypoint.sh`.

Neither reference is committed. `tools/build-amr-reference.sh`,
`build-amrnb-reference.sh` and the two `*-encoder-reference.sh` scripts fetch
and build them; only generated vectors, PCM, bitstreams and traces are checked
in, because they are output rather than source.

## Oracle qualification, measured rather than assumed

Phase 0 of the plan asked which oracles are bit-exact with the normative
references, per path. It went unmeasured for a long time, and risk R8 stayed
open on the strength of `vo-amrwbenc` being VisualOn-derived rather than
reference-derived. Building the encoder ground truth made the measurement free:
both encoders had by then been run over the same deterministic signal, so the
fixtures only had to be compared.

| Path | Apache-2.0 oracle | Agrees with the normative reference? |
|---|---|---|
| AMR-NB encode | `opencore-amr` | **Yes, all eight rates** |
| AMR-WB encode | `vo-amrwbenc` | **Seven of nine.** Not at 12.65 or 14.25 kbit/s |

The narrowband result is the more useful one: an AMR-NB encode fixture derived
from `opencore-amr` carries the same authority as one generated by running the
3GPP reference, without inheriting the redistribution question the plan leaves
open as IP-2b. The wideband result makes R8 real but narrow — a wideband
encoder fixture from that library is ground truth everywhere except those two
adjacent mid-range rates, where it is a lead rather than an answer. The
decoders are unaffected: both were validated against the 3GPP references
directly.

`src/codecs/amr/qualification.rs` pins this as a test, and pins the divergent
set *exactly* rather than bounding it — a rate that starts or stops agreeing
changes what the fixtures are worth and should be read before it is accepted.

## The lessons that cost the most

**1. An oracle that shares your assumption verifies nothing.** This fired three
times. The VAD flag, 23.85's high-band gain index, and AMR-NB's `packed_size`
convention were each wrong in the Rust *and* in the oracle written to check it,
so the test passed. The defence is a conservation law the shared assumption
cannot satisfy — bit counts, permutation checks, cross-table agreement — plus
comparing against something a genuinely independent implementation produced.
The `.amr` fixtures come from opencore-amr and vo-amrwbenc for exactly this
reason.

**2. Diff intermediates, do not reason from output.** A full session spent
reasoning about output PCM produced one speculative lead and no fixes.
Instrumenting the reference and diffing per-stage intermediates found every
remaining bug in a single pass. There are now four instruments, one per codec
path, and each asserts that the instrumented build still reproduces the
committed output byte for byte — so a trace point that changes behaviour rather
than observing it fails loudly:

| Instrument | Covers |
|---|---|
| `tools/trace-amr-reference.sh` | AMR-WB decoder |
| `tools/trace-amrnb-reference.sh` | AMR-NB decoder |
| `tools/trace-amrwb-encoder.sh` | AMR-WB encoder |
| `tools/trace-amrnb-encoder.sh` | AMR-NB encoder |

The Rust side emits the same names. For narrowband:

```sh
tools/trace-amrnb-reference.sh 4
AMR_TRACE_MODE=4 cargo test -p rvoip-codec-core --all-features \
    nb_dump_trace -- --ignored --nocapture
```

**3. A comparison that never happened looks exactly like one that passed.**
This is the same failure as lesson 1 from the other side, and it fired twice in
one session in two different languages. The C instrumentation wrote its trace
to stderr, where the reference's own newline-free progress output merged into
the following line and `grep '^T '` silently dropped it — fifty rows of the
encoder's 16 kHz input, gone. The Rust dump then lost its first row to
`cargo test --nocapture` leaving the test-name line open, which made the very
first pitch lag look wrong when it was correct, and later swallowed the score of
the one rate that was actually failing.

Both are fixed — the C traces have their own file, the Rust dumps start with a
newline — and the trace scripts now report the least frequent trace name, so a
row lost to any future interleaving shows up as an uneven count. The general
form: **a fixture reader that yields nothing must fail, not pass.**
`nb/vectors.rs` panics on a missing or empty section for this reason, and every
bit-exact test asserts how many cases it compared.

**4. Read the reference raw.** Both references interleave instrumentation
counters (`test(); move16();`) on the same line as real assignments, so any
filter that strips those lines strips real code. That removed four lines from
one function, including two shifts, before it was noticed.

**5. The reference declares the same table name twice, more than once.**
`mean_lsf` and `dico1..3_lsf` differ between `q_plsf_3.tab` and `q_plsf_5.tab`;
`inter_6` differs between `pred_lt.c` and `inter_36.tab` — 61 taps for the
adaptive codebook against 25 for the encoder's pitch search, differing from the
first coefficient. The table generator emitted the wrong `inter_6` for a while
under a doc comment describing the other one. It now asserts, for each such
pair, that they still differ; and the module indexing the 61-tap table fails the
*build* rather than a test if a regenerated table changes length.

## AMR-NB: what must not be shared with wideband

The two codecs look like relatives and are not interchangeable anywhere it
matters. Each of these would compile, run, and produce speech-shaped output:

| Thing | Narrowband | Wideband |
|---|---|---|
| Spectral representation | **LSP/LSF** | **ISP/ISF** |
| `F2(z)` factor in the LP conversion | `(1 − z⁻¹)`, subtract `f2[i-1]` | `(1 − z⁻²)`, subtract `f2[i-2]` |
| Final shift in that conversion | 13 | 12 |
| Trailing-coefficient scaling | none | scales by the last ISP |
| Cosine table | 65 entries, top 8 bits | 129 entries, top 9 bits |
| Spacing enforcement | all `M` coefficients | stops one short |
| MA prediction factor | per coefficient | one scalar |
| Interpolation weights | uniform ¼, ½, ¾, 1 | `{0.45, 0.8, 0.96, 1.0}` |
| Adaptive-codebook filter | ⅙ resolution | ¼ resolution |
| `log2` / `pow2` / `sqrt` tables | its own | its own, and G.729 has a third set |
| Output convention | **13-bit** (`& 0xfff8`) | **14-bit** (`& 0xfffC`) |
| Decoder post-filter | **yes** | none |
| `packed_size` in the reference | includes the ToC byte | excludes it |

`nb/lsp.rs` carries a test asserting the two LP conversions **disagree** on the
same input, so a later attempt to unify them fails loudly rather than silently.

## What the narrowband decoder assembly taught

Seven of the eight rates were bit-exact on the first assembly, which is not
what the wideband experience predicted — that one had every stage exact in
isolation and scored 1–3% assembled. The difference is that the three hazards
wideband found the hard way were written into `nb/decoder.rs`'s header before a
line of it ran:

- **Two excitations, and they diverge.** What goes back into the
  adaptive-codebook history is not what the synthesis filter consumes.
- **The previous lag is read on both sides of one write.** Bad-frame graceful
  degradation increments it, and 10.2's sharpening attenuation reads the
  incremented value.
- **The overflow flag is cleared per synthesis call, not per frame.**

The eighth rate, 4.75 kbit/s, failed for its own reason: it transmits one gain
index per *pair* of subframes, on the even one, and the odd subframe re-decodes
that same index against the other half of the table entry. The index has to
survive the two parameters the lag and the codebook consume in between.

## Remaining work, in order

1. ~~**DTX, comfort noise and homing.**~~ Done, both variants, both directions.

   The ground truth landed first, as it did for the encoders:
   `tools/build-amr-dtx-fixtures.sh` produces a 150-frame signal with real
   silence in it, encoded with `-dtx` at all seventeen rates across both
   variants, plus `_mute` variants whose SID updates are dropped so the
   `DTX_MUTE` fade is reachable at all. Five assertions keep it from being
   vacuous, the sharpest being that VAD1 and VAD2 choose different frame types
   on 21 of the 150 frames — established *before* the VAD1 port was written,
   because otherwise nothing would tell the two detectors apart.

   Everything this entry once listed as remaining has landed: the wideband
   decoder side (`rx_dtx_handler`, `dtx_dec` and the `DTX_MUTE` fade), homing
   for both variants, and narrowband — including VAD1, whose long pole was that
   it has no directly observable output at all, and VAD2 alongside it.

   The normative DTX vectors this entry used to describe as unread are now
   read: the wideband `testv/tst_md.cod` and `.out` (80 speech, 1 SID_FIRST, 15
   SID_UPDATE, 104 NO_DATA at 12.65 kbit/s), and the narrowband `spch_dos.inp`,
   `spch_dos.cod` and `spch_dos.out` — 425 frames encoded with `-dtx` across all
   eight rates, which is the reference's own installation check
   (`amr_chk.csh`). Both appear in the table above and in the conformance
   section below. They were the ground truth for the VAD ports, which otherwise
   have *no* directly observable output: the narrowband VAD decision appears
   nowhere in the bitstream, only in which frames become SID or NO_DATA.

2. ~~**Transcoding.**~~ Done — six pairs, tested by property.

3. ~~**Interop and performance.**~~ Done — see the table at the top and the
   interop section. Both PBXes, both variants, tone verified; benchmarks per
   rate with a real-time-factor gate.

4. ~~**The relay path's exit criterion.**~~ Met in full: rvoip as the
   relaying B2BUA (Asterisk OA + FreeSWITCH BE, UDP and TLS+SRTP), the
   mid-call mode switch observed on the wire, and the proxy tier — Kamailio
   and OpenSIPS with rtpengine relaying all four AMR framings verbatim.

5. ~~**Live DTX and CMR.**~~ Both now reach the wire. CMR emission closed with
   `request_peer_codec_mode` (see the exit criterion above). **DTX closed
   2026-08-12**: `Config::amr_dtx` -> `MediaAdapter::set_amr_dtx` ->
   `MediaConfig::with_amr_dtx` on the same commit as the negotiated codec ->
   `resolve_codec` -> the codec's own DTX switch. Sender-side policy only:
   RFC 4867 defines no fmtp for DTX, so nothing is negotiated and the peer
   needs no matching setting.

   Verified on the wire against Asterisk with `PBX_AMR_DTX=1` and a two-second
   silent window in the middle of the call (`amr_call`, octet-aligned, mode 7).
   Speech frames are 33-octet payloads throughout; during the silence the
   stream becomes:

   - `f0 44 2b 07 83 68 0e` — CMR 15, **FT 8 (SID_UPDATE)**, Q=1, five octets
     of comfort-noise parameters. 12 of them.
   - `f0 7c` — CMR 15, **FT 15 (NO_DATA)**, Q=1. 74 of them.

   One SID every eighth frame with NO_DATA between, which is the TS 26.093
   update cadence, and Asterisk relayed them to the far end unchanged. The
   same cell without `PBX_AMR_DTX=1` emits 87-byte frames and nothing else,
   so the switch is non-vacuous in both positions.

   Finding worth keeping: the first wiring set the flag in the harness's
   `session_config`, which returns early for every non-TLS transport — so it
   reached TLS cells only and every UDP run silently behaved as though DTX
   were off, while the diagnostics said it had been requested. What caught it
   was a new `codec generation built` log line naming the flags each codec was
   actually constructed with; that line is now permanent, because every AMR
   field in it has been silently wrong at some point on this branch.

6. ~~**Rates beyond the top of each variant.**~~ Closed, in both senses — see
   [Per-rate attestation](#per-rate-attestation) below.

   Between our own endpoints, `every_amr_narrowband_mode_carries_audio_in_a_live_call`
   and its wideband twin walk the peer down through all 8 and all 9 modes with
   codec mode requests, reading back the mode of frames actually decoded from
   the peer. Against a third party, all 17 modes were run individually against
   Asterisk, each pinned by an RFC 4867 `mode-set` in the offer.

7. ~~**A soak test**, and a fuzz target for the encoders.~~ Both done.
   `soak.rs` holds `#[ignore]`d long-run encode/decode tests scaled by
   `RVOIP_AMR_SOAK_SECS`, driven by `tools/run-amr-soak.sh`; the encoders now
   have `fuzz/fuzz_targets/amr_encode.rs` beside the decoders' target, and both
   run from `tools/run-amr-fuzz.sh`.

8. ~~**AMR through `rvoip-core`'s media graph.**~~ Done. UCTP publishing,
   graph recording and MOQT fan-out can observe an AMR call.

   It was never an AMR gap: the codec built fine from a name and an fmtp all
   along, and what was missing was a negotiated payload type at the graph's
   entry points — which no transport reported, for any codec. `CodecInfo` now
   carries one, the SIP adapter and the WebRTC SDP parser report it, and the
   graph keys on it. The design question it turned on — whether AMR gets a
   conventional payload type the way Opus got 111 — was **settled: it does
   not**, because one AMR session routinely negotiates two payload types at
   once that differ only in `octet-align`.

   Two things came out of it that were not about AMR. The graph's admission
   check now also builds the codec, since a resolvable key stopped implying a
   buildable one. And the QUIC and WebTransport pumps had been stamping
   **Opus's payload type on any codec they could not name** — a well-formed
   datagram that lies about its contents, which a receiver cannot detect.

   The full reasoning is in
   [`MEDIA_GRAPH_CODECS.md`](../../../foundation/rvoip-core/docs/MEDIA_GRAPH_CODECS.md).

   The only item above still open is **6**, and only its third-party half: an
   evidence gap, not a code gap. No rate is unimplemented and no feature is
   blocked.

## Conformance against the normative sequences

**All nine TS 26.173 wideband vectors pass in both directions.** The encoder
reproduces `tst_m0.cod` .. `tst_m8.cod` bit for bit from `tst.inp`, and the
decoder reproduces `tst_m0.out` .. `tst_m8.out` sample for sample from those —
200 frames each, including 23.85 kbit/s, which no amount of ACELP work would
have reached without DTX and homing.

The decoder side needed the driver's own two-state homing protocol, and it
starts *homed*: `reset_flag_old` is initialised to 1, so a sequence opening
with a homing frame is answered with `0x0008` directly rather than decoded.
Starting from the other state emits silence where the vector has 8. `cargo test -- --ignored conformance` runs
them; they panic rather than skip when the sequences are absent, because a
conformance test that quietly passes having found nothing is worse than none.

The sequences are 3GPP copyright and stay out of the tree, on the same rule as
the reference implementations: only generated output is committed. The vectors
are read in the ETSI serial form they ship in rather than converted first — a
converter is one more thing that could be wrong in the same direction as the
code under test.

The normative *DTX* vector, `tst_md.cod` from `dtx.inp`, is covered on the
encode side too: 80 speech frames, a SID_FIRST, 15 SID_UPDATEs and 104 NO_DATA,
every transmit type and every payload matching. `tst.inp` never goes quiet
enough to emit a SID, so without this the DTX path would only ever have been
exercised through its effect on mode 8's gain.

**Everything passes now.** `tst_md.cod` against `tst_md.out` was the last
holdout — one LSB out from sample 16 of frame 80, the first `SID_FIRST` — and
it was two defects rather than one: an excitation snapshot taken per subframe
in four different exponents, and a `CN_dithering` written from a summary
instead of from the C. Both are described above. All six sequences now compare
with zero tolerance.

Mutation-checked: turning DTX off fails mode 8 at frame 0.

## Per-rate attestation

Every AMR mode, run individually against a live third-party PBX.

**Peer:** Asterisk 20.20.1 (aarch64, Linux, built 2026-08-11), `chan_pjsip`
with `codec_amr`. **Run:** 2026-08-13, `amr_call` scenario, `endpoint` API,
UDP, one cell per mode.

Each cell pins one rate the standard way — `Config::amr_mode_set` puts an
RFC 4867 `mode-set` in the INVITE naming exactly that mode, and the set is
bi-directional, so it governs what Asterisk sends as well as what we send.
The lab reaches it through `PBX_AMR_MODE_SET=<mode>`.

| Variant | Modes | Result |
|---|---|---|
| AMR-NB | 0, 1, 2, 3, 4, 5, 6, 7 | all PASS |
| AMR-WB | 0, 1, 2, 3, 4, 5, 6, 7, 8 | all PASS |

**Two independent facts are recorded per cell, because the interesting failure
passes the obvious check.** A cell pinned to one rate and a cell that ignored
the pin both produce clean audio, so "the call passed" attests to nothing about
the rate.

- *The rate was actually in force.* `codec generation built` now logs
  `mode_set=` alongside the codec it constructed, so each cell's log shows the
  mode the codec was built with rather than the environment variable that was
  meant to cause it. Every cell logged the mode it was pinned to: mode 0 logged
  `mode_set="0"`, mode 7 logged `mode_set="7"`, and so on across both variants.
- *The audio survived at that rate.* The analyser's tone check ran per cell —
  the far tone dominating the near one, with a 1 s window above 15 dB SNR. The
  weakest margin across all 17 was 8,508× (AMR-WB mode 0, the 6.6 kbit/s rate,
  where a lower margin is expected); the strongest was 116,348×.

Reproduce with the sweep tool, which is what produced the rows above:

```bash
crates/sip/rvoip-sip/examples/pbx/rate-sweep.sh --profile amrnb --transport UDP
crates/sip/rvoip-sip/examples/pbx/rate-sweep.sh --profile amrwb --transport UDP
```

It fails the run if any cell's `built_mode_set` disagrees with the mode it
pinned, so a sweep cannot quietly attest to a rate it did not test, and it
clears each cell's directory first — a cell that never executes otherwise
leaves the previous run's logs in place, and reading those reports a rate as
attested when nothing ran. This tool did exactly that on its first attempt.

**What this does and does not say.** A third-party implementation decoded our
AMR and encoded audio we decoded, at each of the seventeen rates, one rate at
a time. It is one peer, one version, one topology, and it is not a carrier
certification.

**UDP only.** The same sweep over TLS does not run in this lab, and the reason
is environmental rather than anything about AMR: the Asterisk endpoint's
default TLS port for user `1002` is 5073, which is also what the Kamailio
lab's colima forward holds while that lab is up, so the callee cannot bind its
listener and never registers. `rate-sweep.sh --transport TLS` reports
`no cell ran` rather than a pass. AMR over TLS with SRTP is covered elsewhere —
the `amr_call` TLS cells in the PBX matrix, and the in-process
`amr_*_call_carries_real_audio_over_srtp` tests — but not per-rate.

## Where AMR flows, and the one place it does not

Everything above is about whether AMR *codes* correctly. Whether it reaches
every path media travels in this workspace was a separate question, and for a
while the answer was no. It is now yes, with one boundary worth stating.

**AMR crosses `rvoip-core`'s media graph** — the one-source-to-many fan-out
behind UCTP publishing, recording and MOQT fan-out — behind rvoip-core's own
`amr-nb`/`amr-wb` features. Frames go through in both directions, decoded and
tone-verified rather than counted, and a source whose packet time is not 20 ms
is re-framed rather than refused.

Getting there was not about constructing the codec: `AudioCodecSpec::build`
always built either variant from a name and an fmtp. It was that the graph
derived a payload type from a codec *name*, and AMR has no such number — one
session routinely negotiates the same variant under two payload types
differing only in `octet-align` (`amr_call_integration.rs` uses 106 and 107),
and the key the graph computes is stamped onto the frames it emits. The
decision stands: **AMR gets no conventional payload type; dynamic codecs key on
the negotiated one**, which `CodecInfo` now carries.

**AMR without a reported payload type is still refused**, and that is the
correct answer rather than a leftover — there is no number to label its frames
with. UCTP negotiates codecs by name and reports none, so AMR over QUIC or
WebTransport depends on frames carrying their own label; graph-transcoded
frames do, and one that does not is dropped rather than sent under a
fabricated number.

The decision and its evidence are in
[`crates/foundation/rvoip-core/docs/MEDIA_GRAPH_CODECS.md`](../../../foundation/rvoip-core/docs/MEDIA_GRAPH_CODECS.md).

## What is *not* claimed

Two different claims live in this document and they should not be conflated.

**Bit-exactness** means agreement with the 3GPP reference implementations over
the committed fixtures — 50 frames of one deterministic signal per rate, plus
25 frames of a second for the decoders. Those fixtures are ours; we generated
them.

**Conformance** means the normative sequences that ship with the reference
distributions, which the section above records as passing: all nine TS 26.173
wideband vectors in both directions, the wideband DTX vectors, and the
narrowband `spch_dos` set at 425 frames. Those we did not choose.

What is still *not* claimed is certification. TS 26.074 and TS 26.174 define the
conformance process, and passing the sequences a reference distribution ships is
not the same as being certified against that process by anyone but ourselves. No
3GPP material — neither the reference C nor the specifications — is in this tree;
it is fetched to work against and never redistributed, so every fixture and trace
committed here is generated output. The distinction is worth keeping, because
this repo's own G.711 tests already disclaim evidence from files that are not
present, and the same discipline applies here.

## The normative encoder vectors are DTX-on, and mode 8 needs it

Found while specifying DTX, and it changes what conformance will cost.

TS 26.173 ships nine encoder conformance vectors, and `testv/test_enc.bat`
produces every one of them with `-dtx`. Measured against our own build of the
reference:

| | Reproducible without `-dtx`? |
|---|---|
| `tst_m0.cod` .. `tst_m7.cod` | **Yes**, byte for byte |
| `tst_m8.cod` (23.85 kbit/s) | **No** — 266 of 192 000 bytes differ |

`tst.inp` never goes quiet enough to emit a SID at any rate, so for eight of
the nine the DTX flag changes nothing. Mode 8 is different because it is the
only rate whose high-band correction gain depends on `dtxHangoverCount`: with
DTX off the counter never leaves `DTX_HANG_CONST`, `gain_alpha` is pinned at
32767, and that is exactly what `wb/enc/encoder.rs` hardcodes today. The
differing frames are precisely those where the counter is below 7, and every
differing bit is inside the four 4-bit gain fields.

Two consequences:

1. **Eight of the nine normative vectors are within reach now**, without DTX.
   That is a stronger claim than agreement with our own fixtures and it costs
   only a fetch — the vectors are 3GPP-copyrighted and stay out of the tree, so
   the test is opt-in, `#[ignore]`d, and panics rather than skips when the
   environment variable naming them is unset.
2. **The ninth is the acceptance test for the DTX hangover counter.** Not a
   proxy for it: the same counter, read by the same expression, over 200 frames
   of the spec's own input.

`tst.inp` also opens with two encoder homing frames (all `0x0008`), each of
which drives `Reset_encoder` and re-arms the hangover — so reproducing
`tst_m8.cod` needs homing as well as DTX.

## Where these tests run

Until 2026-08-10, nowhere. `codec-core` defaults to `["g711"]` and the PR
shards build every crate with its default features, so
`cargo test -p rvoip-codec-core` ran **104 tests where `--all-features` runs
740** — every AMR test on this branch, plus G.729 and Opus, compiled out on
every pull request. media-core had the same hole, 290 against 316, and 14 of
its 26 missing tests are the RFC 4867 AMR payload-format ones.

The `codec-features` specialty gate now runs both crates at `--all-features`,
tests and Clippy, on any change under `crates/media/codec-core/**`,
media-core's codec and payload trees, or `Cargo.lock` — and unconditionally on
Main. It is defined in `scripts/ci/run_checks.py` and `scripts/ci/policy.json`,
and `scripts/ci/test_run_checks.py` asserts both that the gate passes
`--all-features` everywhere and that the shards do not, so the gate cannot
quietly become a duplicate of work already done.

## Detailed history


Living tracker for the AMR-NB / AMR-WB work. The plan is in
[`AMR_IMPLEMENTATION_PLAN.md`](AMR_IMPLEMENTATION_PLAN.md); this file records
where we actually are.

**Branch:** `feat/amr-codecs`
**Last updated:** 2026-08-13

---

## At a glance

| Phase | Scope | Status |
|---|---|---|
| 0 | Foundations: types, feature flags, ADR, oracle qualification | 🟢 **Complete** |
| 1 | RFC 4867 payload format + AMR file storage format | 🟢 **Complete** |
| 2 | SDP negotiation + relay path | 🟡 **Negotiation done** — relay path outstanding |
| 3 | DSP layer + oracle harness | 🟢 **Complete**, all four codec paths |
| 4 | AMR-WB decoder, fixed point | 🟢 **Bit-exact, all nine rates**, plus concealment |
| 5 | **AMR-WB encoder — the HD-voice milestone** | 🟢 **Byte-identical, all nine rates** |
| 6 | AMR-NB decoder, fixed point | 🟢 **Bit-exact, all eight rates**, plus concealment |
| 7 | AMR-NB encoder, fixed point | 🟢 **Byte-identical, all eight rates** |
| 8 | Transcoding, interop, performance, hardening | 🟢 **Transcoding, both PBXes, and a real-time-factor gate**; soak and encoder fuzzing not started |

Of the codec proper, nothing is missing: DTX, comfort noise, concealment and
homing are all in and checked against the normative sequences. The relay-path
exit criterion is met in full (rvoip as the relaying B2BUA, the mid-call mode
switch on the wire, and the Kamailio/OpenSIPS+rtpengine proxy tier). What is
left around the codec: a soak test and a fuzz target for the encoders.

---

## Phase 3 — the codec kernel

### The basic operators already existed

Phase 3 was planned to open with writing `common/basicop.rs`, the ETSI basic
operators, exhaustively tested, with nothing else starting until it was green —
budgeted as the foundation everything else is debugged against.

**It was already there.** The G.729A port carries a complete ETSI basic-operator
set, and AMR specifies its arithmetic against the same ITU-T/3GPP library: `add`,
`sub`, `mult`, `mult_r`, `l_mult`, `l_mac`, `l_msu`, `l_add`, `l_sub`, `shl`,
`shr`, `l_shl`, `l_shr`, `l_shr_r`, `round`, `norm_s`, `norm_l`, `div_s`,
`abs_s`, `l_abs`, `negate`, `extract_h/l`, `l_deposit_h/l`, `mac_r`, `msu_r`,
plus the extended-precision `_c` and `_ns` variants. Faithful to the reference,
including the `MIN_16 → MAX_16` quirk in `abs_s`/`negate` and the overflow/carry
flags — and validated by that codec reaching bit-exactness.

So the work was not writing them but **promoting them**: they lived in
`codecs::g729::impls::dsp`, gated behind the `g729` feature. They now live in
`crate::fixed_point`, shared. G.729 reaches them under the old name through a
re-export, so its ~318 call sites were untouched.

Two things were deliberately *not* shared:

- **`pow2`, `log2`, `inv_sqrt`** moved to `codecs::g729::impls::math` instead.
  They are not basic operators — each reads a G.729 Annex A lookup table. AMR
  specifies the same three functions over its own tables (TS 26.073
  `oper_32b.c`), and whether those tables are numerically identical is an open
  question. Sharing the code before answering it would be an assumption dressed
  as reuse.
- **The lint exemption** was carried across rather than widened. The G.729
  subtree has a narrowly enumerated `allow` list with a written rationale —
  deliberate wrapping casts, reference-style names. The same list now sits on
  `fixed_point` with the same reasoning, and `fixed_point` is `pub(crate)`
  because it is an implementation detail, not API this crate wants to commit to.

Verified: G.729's 26 tests still pass, so the move did not disturb a bit-exact
codec; the feature matrix builds in all five combinations, including AMR without
G.729 and vice versa.

### The oracle is running

**It never needed Docker.** The blocker was assumed to be container egress, but
the harness only ever needed source and a C compiler — and macOS has clang. The
sources come from the **Debian archive**, which is reachable, rather than
SourceForge (whose download mirrors are not) or GitHub (also not).

`tools/build-amr-oracle.sh` fetches and statically builds all three
Apache-2.0 libraries, then regenerates the vectors:

| Library | Covers |
|---|---|
| `opencore-amr` 0.1.6 | AMR-NB encode/decode, AMR-WB decode |
| `vo-amrwbenc` 0.1.3 | AMR-WB encode |

**It validates the whole mode table — all 17 modes.** The reference encoders
were run at every mode of both variants, and every frame size is exactly
`octet_aligned_bytes() + 1` (the extra octet being the storage ToC):

| AMR-WB | 18 | 24 | 33 | 37 | 41 | 47 | 51 | 59 | 61 |
|---|---|---|---|---|---|---|---|---|---|
| **AMR-NB** | 13 | 14 | 16 | 18 | 20 | 21 | 27 | 32 | |

Independent confirmation of a table that until now rested on RFC 4867 plus
arithmetic.

**It also settles the 6.70 / 7.40 question raised at the top of the plan.**
Several secondary sources, Wikipedia's AMR page among them, transpose those two
frame sizes. RFC 4867 says 6.70 carries 134 bits (17 octets) and 7.40 carries
148 (19), and the reference encoder's file lengths agree — mode 3 is the smaller
file. Had they been transposed, the sizes would be the other way round. There is
a test asserting exactly that, so the question cannot quietly reopen.

425 reference frames are checked in at `src/codecs/amr/testdata/` (72 KB), with
tests asserting that we read every mode correctly, that our frame sizes predict
the file lengths exactly, that re-writing reproduces the files byte for byte,
and that every frame survives both RFC 4867 framings. The generator also decodes
each file back through `opencore-amrwb` before accepting it, so the fixtures are
self-consistent independently of anything here.

The script reproduces the checked-in fixtures bit-identically from a clean
workdir, so a future regeneration that changes them means something changed for
a reason worth understanding.

**Nothing from the oracle is linked into the shipped crate.** It emits data;
the data is committed; the test suite reads only that. A normal `cargo test`
needs neither the libraries nor a C toolchain — the property that lets the
crate stay pure Rust while still being developed against a real reference.

### The spec was never actually blocked

An earlier revision of this section recorded Phase 3 as blocked on TS 26.190
being unreachable. **That was wrong, and the mistake is worth keeping visible.**
`3gpp.org` and `etsi.org` return 403 to `curl`'s default user agent and 200 to a
browser one — bot filtering, not an egress block. They were lumped in with hosts
that genuinely fail at the connection layer (`arib.or.jp`, `tech-invite.com`,
which return `EADDRNOTAVAIL` exactly like `github.com`) and the whole set was
called blocked.

The distinction is diagnosable in one command: a 403 means the connection
succeeded and the server refused; a connection-layer failure means it never got
that far. Two different problems, two different fixes.

TS 26.190 v19.0.0 is now available from ETSI. Extracting it also needed `pypdf`
rather than raw stream decompression — it uses subset fonts with custom
encodings, unlike TS 26.201, so naive extraction yields binary noise that looks
like a failed download.

### The float modules were removed

`wb/lp/{window,levinson,isp}.rs` are gone. They had **no callers** — nothing
outside their own directory referenced them, `AmrCodec` never reached them, and
`encode`/`decode` still return `FeatureNotEnabled`. Dead code that could be
mistaken for the codec.

Evidence that float has no role here at all:

- **TS 26.173, the normative reference**: `typedef short Word16; typedef long
  Word32;` — pure fixed point.
- **Zero files** across `opencore-amr` and `vo-amrwbenc` mention `float` or
  `double`.
- Every AMR implementation in this project's own lab — FreeSWITCH's `mod_amr`
  and `mod_amrwb`, Asterisk's `codec_amr`, the oracle libraries — is fixed-point,
  because they all link those same libraries.

The float specs (TS 26.104 / 26.204) exist for research; they do not appear in
telephony, which is why they were also dropped from the oracle roster.

What the removed work actually produced is kept: the window formula verified
against `ham_wind.tab` to within 1 LSB, and the noise-floor placement
discrepancy. Both are recorded above. The code itself was not the codec and
could not have become it.

### Superseded: first DSP as floating-point reference models

`codecs/amr/wb/lp/window.rs` implements TS 26.190 §5.2.1 — the asymmetric
analysis window (L1=256, L2=128, 384 samples), autocorrelation, 60 Hz lag
windowing and the 1.0001 white-noise correction. Floating point for now; the
fixed-point form will be checked against it.

**The oracle earned its keep immediately.** The window formula came through a
garbled PDF extraction and had to be reconstructed. Computing it from the
reconstructed formula and comparing against `vo-amrwbenc`'s `ham_wind.tab`:
all 384 values agree to within 1 LSB in Q15, none differing by more than 1. The
reading was right, and now it is *known* to be right rather than assumed —
without copying the table.

It also surfaced a real discrepancy worth recording. The reference's lag window
values differ from ours by a consistent 1e-4, because `lag_wind.tab` folds the
white-noise correction into the lag window (its own comment says "noise floor =
1.0001 = (0.9999 on r[1]..r[16])") while TS 26.190 places it on `r(0)`. The two
differ only by an overall scale on the autocorrelation sequence, which
Levinson-Durbin is invariant to, so the predictor is identical. We follow the
spec's placement; a test pins ours against the reference's documented values
with the factor restored, so the equivalence is asserted rather than assumed.

### Levinson-Durbin

`codecs/amr/wb/lp/levinson.rs` implements §5.2.2 — the order-16 recursion
solving the Yule-Walker system, returning predictor coefficients, reflection
coefficients and residual energy.

The recursion equations came through the PDF extraction scrambled, but unlike
the window this needed no reconstruction: the spec defers the algorithm to a
standard reference, so it is the textbook recursion. What is AMR-specific is the
fixed-point formulation, which will be written against this float version.

Validated by **an independent solve rather than a restatement**: the tests
solve the same system by Gaussian elimination — O(M³), no knowledge of the
Toeplitz structure — and require agreement. Also checked: the normal equations
are satisfied directly, every reflection coefficient lies inside the unit circle
(the stability condition that makes this recursion worth using over a general
solve), residual energy never increases with order, and a tone is orders of
magnitude more predictable than noise.

One test failure was the test's fault, and worth recording: the "white noise"
generator produced values in `0..2²⁴` minus 8192, so it carried a large DC
offset — and a constant is perfectly predictable, which made the noise look
highly structured. Centring it fixed the data, and the assertion was rewritten
to compare prediction gain between a tone and noise rather than test an absolute
threshold that depends on the analysis window's own spectral shape.

### LP to ISP conversion

`codecs/amr/wb/lp/isp.rs` implements §5.2.3: the sum and difference
polynomials, the removal of the roots at `z = ±1`, and the root search that
yields the 15 ISPs plus `a[16]`.

The coefficient recursions were **derived from the polynomial identities rather
than transcribed**, because the spec's recursion block did not survive PDF
extraction. `f'1(z) = A(z) + z⁻¹⁶A(z⁻¹)` gives `f'1[i] = a[i] + a[16-i]`
directly, and `f2 = f'2/(1-z⁻²)` gives `f2[i] = f'2[i] + f2[i-2]`. Both are
then checked numerically against the identities they came from, so the
derivation is verified rather than trusted.

**One deliberate divergence from the reference.** The reference alternates
between `f1` and `f2` while walking the grid, relying on the roots interlacing
so each switch lands in the next bracket. That is efficient but fragile: when
two roots fall inside one grid interval the search steps past one and then
fails to find the rest — which is exactly what happened here first. This model
scans each polynomial over the whole grid independently, so no bracketed root
can be lost, and the interlacing is then *asserted* rather than assumed. The
fixed-point version will need the reference's cheaper approach; having this one
to check against is the point.

### First fixed-point DSP: LP analysis front end

`codecs/amr/wb/lp/` now holds the real thing — TS 26.190 §5.2.1 in the
arithmetic that defines the codec, built on the ETSI operators in
`crate::fixed_point`.

- `tables.rs` — the analysis window (384 Q15 values) and lag window (16
  double-precision pairs), taken from TS 26.173 because they are normative
  constants. **The window is not computed from the §5.2.1 formula**: it is
  close, `round(w · 32767)` reproduces 377 of 384 entries, but seven differ by
  one LSB and one LSB through `mult_r` changes the output bits. A test checks
  the table against the formula to within 1 LSB, documenting the definition and
  catching transcription errors without pretending the formula is authoritative.
- `autocorr.rs` — windowing, the energy-estimate pre-shift, autocorrelation over
  17 lags in double-precision format, and lag windowing.
- `isp_to_lp.rs` — §5.2.4, ISPs back to predictor coefficients. **Decoder-side
  work**, which is why it lands before the encoder-only analysis stages: the
  decoder receives quantised ISFs and never analyses, so this is on the critical
  path to a working decoder while §5.2.2 and §5.2.3 are not.
- `isf.rs` — §5.2.5–§5.2.6, ISP↔ISF conversion and the four-subframe
  interpolation. The codec quantises in the ISF domain (ordered, bounded, local
  error) and computes in the ISP domain (what the polynomial arithmetic wants),
  converting once per frame each way. Interpolating ISPs rather than
  coefficients is what keeps every intermediate filter stable — ISPs stay
  ordered under a convex combination, and ordered ISPs *are* the minimum-phase
  condition.
- `isf_tables.rs` — the 129-entry cosine table and 128 `acos` slopes. **These
  tables are the transform**, not an approximation of it: both directions
  interpolate rather than evaluating a real trigonometric function, so a more
  accurate cosine gives different bits and fails conformance. The round trip is
  correspondingly not the identity, which is why each direction is checked
  against the reference separately rather than only checking that they compose.

- `isf_dequant.rs` + `isf_codebooks.rs` — §5.2.7/§6.1, indices to ISFs at both
  rates, with erasure concealment. Two-stage split VQ: splitting is what makes
  the codebooks tractable at all, since a joint 46-bit codebook over sixteen
  dimensions would need 7 × 10¹³ entries.

The dequantiser is **predictive, and therefore stateful** — a frame's output
depends on the previous frame's residual — which changes what a useful test
looks like. A single-frame vector would miss whether the state update is right,
and that is precisely the bug that makes a decoder drift away from its encoder
over seconds rather than failing outright. So the oracle runs a *sequence* from
the reset state, dumping output and residual after each frame, and the Rust
replays the whole sequence and checks both. The last frame is marked bad, so
concealment and its distinct state update are covered as well.

The interpolation weights are worth naming: `{0.45, 0.8, 0.96, 1.0}`, not
uniform quarters. The weighting is pushed hard toward the new frame because the
analysis window that produced it is itself concentrated at the end of the frame
— the "new" ISPs already describe the region the first subframe sits in.

Two details worth naming, both invisible in a float model:

- **The accumulator for `r[0]` starts at 1, not 0.** That is what stops a silent
  frame producing `r(0) = 0` and making Levinson-Durbin divide by zero. It is
  *not* the -40 dB noise floor, which lives in the lag window — a distinction
  the prose spec blurs.
- **One shared normalisation shift is applied to every lag.** Levinson-Durbin
  only cares about ratios, so a common shift buys precision without changing
  the answer; per-lag normalisation would corrupt the sequence.

Tests cover the properties that catch scaling mistakes: `r(0)` dominates every
other lag, `r(0)` lands in the top bits after normalisation, silence still gives
a usable `r(0)`, the lag window leaves `r[0]` untouched while shrinking the
rest, and doubling the input amplitude leaves the normalised sequence
essentially unchanged.

### Bit-exact against the normative reference

`tools/build-amr-reference.sh` fetches TS 26.173 and drives its functions
directly, dumping every intermediate of the LP analysis chain. The vectors are
checked in at `testdata/lp_stages_wb.txt`; the source is not, since 3GPP permits
in-house use but not redistribution.

**The fixed-point autocorrelation matches the reference exactly** — all 17 lags,
both halves of each double-precision pair, across every case. Two tests: one for
the whole windowing-and-accumulation path, and one feeding the reference's own
pre-lag values through `lag_window` in isolation so a failure there cannot be
blamed on the accumulation.

**§5.2.4 ISP→LP matches exactly too** — all 17 coefficients, every case, on the
first run. So do **§5.2.5 ISP→ISF, §5.2.6 ISF→ISP, and the subframe
interpolation** (4 × 17 coefficients per case), and **§5.2.7 ISF
dequantisation** at both rates across a six-frame sequence including an
erasure, checking the residual state as well as the output.

**TS 26.201 bitstream unpacking** closes the loop: real payload bytes → codec
bits → parameter indices, checked against every mode's first three frames.

That completes the decoder's spectral path: **payload → indices → ISFs → ISPs
→ interpolated → per-subframe LP coefficients, every stage bit-exact.**

The unpacking vectors deserve a note on method. A bit permutation that is
self-consistent but wrong round-trips perfectly against itself, so a synthetic
round trip would prove nothing. Instead the vectors come from feeding the
committed `.amr` fixtures — produced by **opencore-amr and vo-amrwbenc** —
through **TS 26.173's** own unsorter. Three independent implementations
agreeing on real bitstreams is a much stronger claim than any self-check.

Note the sorting tables ship inside TS 26.173 (`mime_io.tab`), so this needed
no source outside the tier-1 authority.

One test was written on a false assumption and removed: the first parameter
field is *not* the leading octet of the payload. `SORT_660` opens `0, 5, 6, 7`,
so the eight bits of the first ISF index arrive at payload positions 0, 31, 38,
32, 10, 1, 2, 3. The bit-exact test caught it — the guessed property failed
while the reference comparison passed.

### The full parameter walk, and what an oracle cannot catch

`wb/params.rs` reads a whole frame: VAD flag, ISF indices, and per subframe the
pitch lag, LTP filter bit, algebraic codebook pulses and gain index, plus the
high-band gains at 23.85. Bit-exact on the first three frames of all nine modes.

**The first version was wrong, and the way it was wrong is the lesson.** The VAD
flag is the first bit of a speech frame — `dec_main.c` reads it before the ISFs,
not after — so every field was shifted by one bit. The commit before this one
claimed "three independent implementations agree". That was overstated:
opencore-amr and vo-amrwbenc produced the *bitstream*, but the field offsets
came only from one reading of `dec_main.c`, copied into both the C oracle and
the Rust. They agreed perfectly because they shared the mistake.

**An oracle only checks what it does not share with you.** The generic defence
is a conservation law — something that must hold regardless of whether the
shared assumption is right. Here it is bit count: a field-offset error shifts
later fields rather than overrunning, so leftover bits are the only signal.
Every mode had exactly one bit left over, which is the signature of a one-bit
shift at the front. Now every mode consumes its frame exactly — zero left for
modes 0–7, sixteen at 23.85 for the high-band gains — and that check is a
permanent test.

`read_isf_indices` was removed from the bitstream module rather than fixed: it
read from the frame start, so any future caller would have walked straight back
into this bug.

### Algebraic codebook

`wb/codebook.rs` implements §6.2 — pulse indices to the 64-sample innovation
vector, all seven widths from 12 bits (two pulses) to 88 (twenty-four).
Bit-exact on all 108 subframes of the fixtures: 9 modes × 3 frames × 4
subframes, every sample.

The interesting part is that the per-track codes are *combinatorial*. Two
pulses fit in `2N+1` bits because the pair is unordered — the state freed by not
distinguishing `(a, b)` from `(b, a)` carries a sign instead. The wider decoders
are built recursively on that, splitting on a few leading bits that say how the
pulses distribute between two half-ranges.

The bite check here was chosen to match the failure mode: shifting one pulse by
a single sample, which is the classic wrong-track bug. The excitation stays
sparse and plausible while the pulses sit in the wrong places, so a
count-the-pulses test would pass and only a sample-by-sample comparison fails.

### Gains, and the AMR-WB math primitives

`wb/gain.rs` implements §6.3 and `wb/math.rs` the primitives it needs — isqrt,
pow2, log2, normalised dot product, median-of-five. Bit-exact on all 108
subframes, replayed from reset because the code gain is predicted from the last
four subframe energies.

Two design points worth recording:

- **The math primitives are AMR-WB's own, deliberately not shared with G.729.**
  Both codecs have tables for the same functions with different values. Sharing
  them would give a codec that sounds nearly right and is not conformant.
- **The code gain is normalised by the innovation's own energy.** Without that,
  a subframe whose pulses happened to land constructively would be louder than
  one that did not, for the same transmitted index. The normalisation makes the
  index mean intended loudness rather than an artefact of pulse placement.

Three mistakes in this stage, all in work from the same sitting:

- **The grep filter that strips the reference's instrumentation counters also
  strips real assignments** that share a line with them. That silently removed
  four lines from `Isqrt_n`, including two shifts. Reading the source raw is the
  only safe way; the filtered view is for orientation, never for transcription.
- The `log2`/`pow2` round trip was 2% off until the missing `& 0x7fff` mask on
  the interpolation fraction was restored — same cause.
- A test divided by `1i32 << 31`, which is `i32::MIN`, so a *correct* `isqrt`
  looked sign-flipped. The function was right; the test's scaling was negative.

### Long-term prediction

`wb/ltp.rs` implements §6.1/§6.4 — the adaptive codebook with quarter-sample
interpolation, the optional LTP low-pass, preemphasis by the excitation's tilt,
and pitch sharpening. Bit-exact over all 24 combinations of six lags and four
fractions.

**The adaptive codebook must write in place, and this is not a detail.** There
is no stored codebook: the vector is the excitation from one pitch period ago.
When the lag is shorter than a subframe — common, since lags start at 34
against a 64-sample subframe — the filter reads samples it wrote *earlier in
the same subframe*. That self-reference is how a short lag makes a waveform
repeat within the subframe.

My first API took an immutable history and wrote to a separate output slice,
which cannot express that. It read off the end of the history and panicked,
which pointed straight at the design error — but only by luck of the buffer
length. One element longer and it would have produced plausible, wrong samples
for every lag below 64, and the bit-exact test would have been the only thing
standing between that and shipping.

### Low-band synthesis

`wb/synthesis.rs` implements §6.6–§6.8 — LP synthesis, de-emphasis, 50 Hz
high-pass. Excitation in, 12.8 kHz speech out, bit-exact over a four-block
sequence replayed from reset.

**Two filters keep their recursive state in double precision, for the same
reason.** `1/A(z)` is marginally stable by construction, and the 50 Hz
high-pass at 12.8 kHz has poles very close to the unit circle. In a recursive
filter, rounding error is fed back rather than discarded, so sixteen bits of
state would let a sharp formant drift. That is why the synthesis filter hands
its output onward as a `(high, low)` pair rather than a single `Word16` — the
split is structural, not a rounding refinement.

The excitation's per-subframe scaling is undone by shifting `a[0]` rather than
the signal, so putting it back costs no precision.

### Excitation assembly and upsampling

`wb/excitation.rs` implements §6.5 and §6.9 — summing the two excitation
contributions under the decoder's adaptive scaling, and resampling 12.8 → 16
kHz. Bit-exact over a replayed four-subframe sequence.

**The adaptive scaling is structural, not an optimisation.** The excitation
buffer is held at a per-subframe shift, and the *whole* history is rescaled
when that shift moves — because the adaptive codebook reads the history, so
history and present must share a scale. Rescaling only the new subframe would
make pitch prediction read at the wrong loudness.

The shift is bounded by the **smallest** headroom across the last four
subframes, not the current one. A fresh decoder therefore records four zeroes
and cannot shift at all until four subframes have run. That looked like a bug
in a test I wrote and is in fact the point: the scaling must not open up on the
strength of one quiet subframe and then clip when the level returns.

The fixture also caught a real bug: the upsampling filter is centred on its
sample, so the read window starts `NB_COEF_UP - 1` taps earlier. I had omitted
the reference's `x = x - nb_coef + 1` back-step, which read past the buffer.

### Correction: the decoder is not as close to done as the per-stage list suggests

Assembling the decoder turned up **five stages I had not accounted for**, all
sitting between gain decoding and synthesis. An earlier summary here claimed
every decoder stage existed and only the wiring remained. That was wrong, and
the way it was wrong is worth recording: I had been working outward from the
data path in TS 26.190's section order, and these are *enhancement* stages that
the prose treats as refinements but `dec_main.c` runs unconditionally. Reading
per-stage left them invisible; only trying to wire the whole thing exposed them.

Still to implement:

| Stage | Where | Note |
|---|---|---|
| `voice_factor` | inline | Voicing measure driving the two enhancers and the next subframe's tilt |
| Phase dispersion | `ph_disp.c` | Spreads pulse energy in time at low rates; three dispersion levels by rate |
| Noise enhancer | inline | Moves the code gain toward a threshold on noisy, stable frames |
| Pitch enhancer | inline | HP-filters the innovation on voiced frames |
| Enhanced excitation | inline | **Synthesis consumes `exc2`, not the `exc` I assemble** — the two diverge |
| `Isf_Extrapolation` | `isfextrp.c` | The 6.60 kbit/s high-band branch |

The last row of that table is the one that would have bitten hardest: the
excitation fed to the synthesis filter is a separately enhanced copy, while the
one written back to the adaptive-codebook history is not. Wiring my current
modules together naively would have used one buffer for both and produced audio
that was recognisably speech and steadily wrong.

**End-to-end ground truth is now in place**, which is what will catch the rest.
`build-amr-reference.sh` builds the reference decoder and decodes every fixture
to 16 kHz PCM (`amrwb_mode*.pcm`, 25 frames each). Per-stage vectors cannot
reach the state coupling *between* stages; this can.

### Milestone: the wideband decoder reached bit-exactness

Every sample of every frame of every fixture, 6.60 through 23.85 kbit/s, is
identical to the TS 26.173 reference decoder. 8000/8000 per mode, worst error
zero. Asserted by `the_decoder_matches_the_reference_sample_for_sample`.

### How it got there, from 1–3%

Six defects, in the order they were found. Every one was found by **diffing
traced intermediates against the instrumented reference**, not by reading code
or reasoning from output PCM — a full turn spent on the latter produced one
speculative lead and no fixes.

| # | Defect | Effect |
|---|---|---|
| 1 | Comparing 16-bit output against 14-bit reference PCM (`decoder.c` masks with `0xfffC`) | The 1–3% figure itself |
| 2 | `Qsubfr`/`Q_old` seeded to 0 instead of `Q_MAX` | Mis-scaled the start of every stream |
| 3 | Phase dispersion given a rescaled gain, not the Q16 high half | Wrong dispersion state |
| 4 | 23.85's high-band gain parsed as a trailing block, not per subframe | 23.85 unusable |
| 5 | 6.60/8.85 pitch-sharpening blend (`agc2`) missing entirely | Both low rates ~14% |
| 6 | `Scale_sig` implemented as a truncating shift | **The entire residual** |

Number 6 is the one worth remembering. `Scale_sig` is not a shift: the reference
widens each sample to 32 bits, shifts, and *rounds* back, so scaling by −3 is
`floor((x+4)/8)`, not `floor(x/8)`. Those differ on half of all inputs. Because
the rescaled excitation feeds the voicing measure, the error reached the next
subframe's spectral tilt, both enhancers, and the low-rate sharpening. Fixing it
took modes from 71–81% to 100% in one step.

Number 4 is the one worth learning from: **my oracle shared the wrong
assumption**, so the parameter test passed while both were wrong. The
bit-conservation check could not catch it either — the bit *count* was
unchanged, only the assignment. This is the second time that trap fired (the
VAD flag was the first).

### The 6.60 high band

The last mode needed its own path: it extrapolates an order-20 predictor from
ISF *spacing* rather than borrowing the low band's order-16 filter. Wiring it
required generalising `isp_to_lp` to any even order — above 16 the reference
runs the same expansion four times smaller and shifts back, because twenty
accumulations overflow Q23 where sixteen do not — and an order-20 shaper over
the full memory at gamma 0.9.

One detail that would have been silently wrong: the high-band ISF interpolation
uses the plain complement `32767 - frac`, **not** the `+1` complement
`interpolate_isp` uses. Reusing that helper would have been off by one LSB per
subframe.

### Superseded: how the accuracy figure was corrected, in two steps

The two entries below are kept because the *reasoning* in them is still useful,
but both of their headline numbers are obsolete — the decoder is now bit-exact.
Read them as a record of how a wrong measurement was diagnosed, not as status.

#### Step 2 — the 1–3% figure was mostly a wrong yardstick

**TS 26.173's `decoder.c` masks its output with `0xfffC` before writing** — AMR-WB's
output is defined as 14-bit linear, so the low two bits are deleted. I was
comparing unmasked 16-bit output against 14-bit reference PCM. A decoder that was
substantially correct scored 1–3%.

With the mask applied and two real bugs fixed:

| Mode | Before | After | Worst \|delta\| |
|---|---|---|---|
| 6.60 | 1.2% | 15.2% | 220 |
| 8.85 | 1.4% | 13.1% | 220 |
| 12.65 | 1.5% | **80.7%** | 16 |
| 14.25 | 1.4% | **80.2%** | 24 |
| 15.85 | 1.3% | **76.5%** | 16 |
| 18.25 | 2.1% | **73.4%** | 12 |
| 19.85 | 2.1% | **78.0%** | 12 |
| 23.05 | 2.7% | **79.0%** | 8 |
| 23.85 | 0.5% | 1.1% | 25 640 |

The two real bugs, both found by tracing rather than reading:

- **`Qsubfr[0..3]` and `Q_old` initialise to `Q_MAX` (8), not zero.** The shift is
  bounded by the *minimum* of the four, so starting at zero pinned it to zero until
  four subframes had run — mis-scaling the start of every stream, and with it the
  synthesis filter's `a0`.
- **Phase dispersion takes the high half of the Q16 code gain** (`L_Extract`), not a
  rounded or rescaled version.

**Method that worked, after one that did not.** A full turn spent reasoning from
output PCM produced one speculative lead and no fixes. Instrumenting the reference
decoder with trace points and diffing intermediates found all three issues in a
single pass. That harness is now committed as `tools/instrument-amr-decoder.py` and
`tools/trace-amr-reference.sh`, and the Rust decoder emits the same names under
`cfg(test)`.

Verified equal to the reference for mode 12.65, frame 0, subframes 0 and 1: every
scalar (`T0`, `T0_frac`, `tilt_code`, `gain_pit`, `L_gain_code`, `Q_new`,
`gain_code`, `voice_fac`) and every vector (`pred`, `code`, `exc_total`,
`exc2_final`, `hfband`). The excitation chain is correct; the residual is
downstream.

**Remaining, in priority order:** 23.85 is badly broken (its transmitted high-band
gain path); 6.60 and 8.85 share a mode-specific bug (they take the
`nb_bits <= NBBITS_9k` branches — forced LTP filter, 6-bit gains, and a `pit_sharp`
post-processing step the Rust omits entirely); modes 12.65–23.05 have a small
residual worth 8–24 LSB.

#### Step 1 — the original measurement: fourteen exact stages, 1–3% right

The decoder is wired and runs on every mode without panicking, producing
speech-shaped output. Against the reference PCM:

| Mode | Exact samples | Worst \|delta\| |
|---|---|---|
| 6.60 | 1.2% | 8 493 |
| 8.85 | 1.4% | 15 713 |
| 12.65 | 1.5% | 6 972 |
| 14.25 | 1.4% | 7 011 |
| 15.85 | 1.3% | 7 899 |
| 18.25 | 2.1% | 6 351 |
| 19.85 | 2.1% | 5 673 |
| 23.05 | 2.7% | 7 336 |
| 23.85 | 0.5% | 28 241 |

**Every stage is bit-exact in isolation and the composition is not.** The
errors are therefore in the wiring: state carried between stages, order of
operations, or an interface I have matched to the wrong thing. This is exactly
the class of defect per-stage vectors cannot reach — which is why the
end-to-end ground truth was built first, and why the earlier claim that "only
the wiring remained" was worth distrusting even after the five missing stages
were found.

The suite now carries a **ratchet** test (fails on regression, floors set below
current values) and an exact-match test marked `#[ignore]` with the figure in
its reason. "Runs without panicking" is far too weak to be the only assertion,
and prose in this document is not a substitute for a failing test.

> *No longer true.* Both the ratchet and the `#[ignore]` were removed once the
> decoder became bit-exact: a floor on match percentage is meaningless at 100%,
> and an exact-match assertion supersedes it. The reasoning above still holds
> for any stage that is not yet exact — which is why the narrowband work uses
> the same approach.

One concrete bug the assembly did surface: the excitation buffer was one sample
too short. The adaptive codebook writes `L_SUBFR + 1` samples because the LTP
low-pass reads one ahead, and no per-stage test ever asked for that length.

**Debugging approach from here.** The per-stage vectors are still the lever:
instrument the assembled decoder to dump the same intermediates the oracle
already emits, run both on the same frame, and find the first stage whose
output diverges. The divergence point localises the wiring error far faster
than comparing PCM.

### High-band synthesis — the wideband half

`wb/highband.rs` implements §6.10, bit-exact over three blocks covering both
VAD weightings.

**Below 23.85 kbit/s not one bit of the payload describes this band.** It is
noise, scaled to the excitation's energy, levelled by the low band's spectral
tilt, shaped by a bandwidth-expanded LP filter, and band-limited. That is not a
shortcut: above about 6 kHz speech is mostly fricative energy with little
perceptually relevant fine structure, so the ear cares that the band is
*present* and at the right level far more than what is in it.

Three details, each of which is the reason for a whole step:

- **The noise is a defined sequence, not randomness.** Encoder and decoder must
  generate identical noise, so "random" means unpredictable to the ear and
  exactly reproducible to the codec.
- **The level follows spectral tilt** because a flat noise level would make
  vowels hiss. Voiced speech has a falling spectrum and gets far less.
- **The shaping filter is bandwidth-expanded**, widening the formants. Sharp
  resonances in a noise band sound like tones rather than like speech.

**Not covered:** the 6.60 kbit/s branch, which extrapolates a separate order-20
ISF set (`Isf_Extrapolation`) rather than reusing the low band's filter. Every
other mode uses the implemented path.

And one corrected premise: a median is **not** unchanged by an outlier —
replacing the smallest of five values with the largest shifts it up one rank.
The honest claim, which the test now makes, is that it moves far less than a
mean.

A bit-exact test can pass vacuously if the fixture reader silently returns
nothing, so the reader is checked too: corrupting one value in the dump by a
single LSB makes the corresponding test fail. Worth redoing whenever the dump
format changes, since a passing suite is otherwise indistinguishable from a
suite that compares nothing.

This is the only evidence that counts. Property tests can show output is
*plausible*; conformance means matching these integers.

The ISP→LP result also settled a question I could not settle by reading. The
reference's `Get_isp_pol` walks pointers rather than indices, and traced by
hand it appears to add the `f[i-2]` term twice — once from the pre-set
`*f = f[-2]` and once from the loop body's `L_add(*f, f[-2])`. I stopped
tracing, implemented it exactly as written, and let the vectors decide; they
came out bit-exact, so the apparent duplication is the algorithm. **With an
oracle in place, checking beats deriving.** The hour lost to hand-tracing this
before the harness existed is the argument for building the harness first.

Two things the harness needed, both worth recording for whoever runs it next:

- **ETSI serves 403 to curl's default user agent.** A browser one gets 200. Bot
  filtering, not an access restriction — the same trap that made me wrongly
  record the spec as unreachable earlier.
- **The reference's `typedefs.h` predates arm64** and stops with
  `#error "can't determine architecture; adapt typedefs.h to your platform"`.
  Its integer widths come from `limits.h` and are already right; only the
  platform/endianness block needs a branch, which is what the error asks for.
  The script adds it automatically.

A first attempt at these vectors was worthless and the reason is worth keeping.
Synthesising "ordered ISP-looking values" and feeding them to `Isp_Az` produced
saturated output — `a[1] = 32246` in Q12 is 7.9, and several coefficients came
out zero. Ordered values are not generally the roots of a minimum-phase filter.
The vectors now come from running the reference's own analysis chain, so the
ISPs are ones the codec could actually produce.

### Course correction: fixed point, and the reference is the definition

**AMR is defined in fixed-point arithmetic.** TS 26.190 §8.1: "The adaptive
multi-rate wideband speech codec is described in a bit-exact arithmetic to allow
easy type approval." The prose spec describes the algorithm; **TS 26.173, the
ANSI-C fixed-point source, defines it.** A floating-point AMR is a different
codec output — it will not pass conformance, and two endpoints running float and
fixed versions produce different bitstreams from identical audio.

The Phase 3 modules written so far are `f64` reference models, not the codec.
The plan (§6.4) had already rejected float-model-first as redundant once a real
oracle existed; building them anyway was a drift that should have been flagged
at the time.

**TS 26.173 is now obtained** (58 C files) and immediately overturned a decision:

> `lag_wind.c` loops `i = 1..M` and never touches `r[0]`. The noise-floor
> reciprocal is folded into the lag table, whose header reads "noise floor =
> 1.0001 = (0.9999 on r[1]..r[16])".

TS 26.190's prose says `r(0)` is multiplied by 1.0001. **The reference uses the
opposite placement**, and `window.rs` had followed the prose. Corrected, with a
test pinning it.

The two are equivalent up to a uniform scale, and Levinson-Durbin is
scale-invariant — but only in exact arithmetic. In fixed point the sequence is
normalised and rounded at each step, so the scale changes intermediate values
and therefore the bits.

Generalising: several "improvements" over the reference are defects. The 60-step
bisection in `isp.rs` converges further than the reference's 4, which for a
bit-exact codec means producing the wrong ISPs. **The reference's imprecision is
part of the specification.**

### Floating-point references removed from the oracle roster

TS 26.104 and TS 26.204 are out. Not being bit-exact, they cannot confirm
bit-exactness, and a disagreement would not identify which side is wrong — a
reference that can only mislead is a liability. FFmpeg stays, scoped to interop
rather than bit-exactness, for the same reason.

### Previously recorded as blocked (resolved)

The next piece of Phase 3 is the LP-analysis chain — order-16 autocorrelation
with lag windowing, Levinson-Durbin, A(z)↔ISP conversion, interpolation. **This
cannot be written bit-exactly without TS 26.190.** The algorithm *structure* is
well known and available from secondary sources, but the parts that decide
bit-exactness are normative data in the spec:

- the LP analysis window (shape, length, exact coefficients),
- the lag window / bandwidth-expansion values,
- the Q-format of each intermediate,
- the per-mode bit allocation.

Writing the structure with plausible-looking tables would produce a codec that
compiles, passes its own tests, sounds approximately right, and is not
bit-exact — the exact failure mode the plan was built to avoid. It is worse
than not writing it, because it looks finished.

**Every spec source is unreachable from this environment.** Checked:

| Host | |
|---|---|
| `arib.or.jp` (mirrored TS 26.201 successfully earlier today) | now unreachable |
| `3gpp.org`, `etsi.org` | 403 |
| `portal.3gpp.org`, `tech-invite.com`, `qtc.jp` | unreachable |

Egress tightened during the session: the ARIB mirror that supplied the TS 26.201
class A table earlier stopped responding.

### To resume, one of these

1. **Supply TS 26.190** (and ideally TS 26.201 for the bit ordering, TS 26.192/3/4
   for CNG/DTX/VAD). Downloadable from `3gpp.org` in a normal browser. This is
   the intended path and keeps the from-spec implementation the plan chose.
2. **Authorise taking the tables and algorithms from `opencore-amr`**, which is
   already built locally and is Apache-2.0. Lawful — plan §6.5 records this as
   the preferred fallback — but it is a **licensing decision, not a technical
   one**: parts of `codec-core` would become Apache-2.0-derived rather than MIT,
   requiring `THIRD_PARTY_NOTICES.md` entries and attribution. Deliberately not
   taken unilaterally.

> **RESOLVED.** Both tier-1 references are fetched and building —
> TS 26.173 by `tools/build-amr-reference.sh` and TS 26.073 by
> `tools/build-amrnb-reference.sh`. The blocker was never egress: ETSI serves
> 403 to curl's default user agent and 200 to a browser one, which is bot
> filtering rather than an access restriction. Nothing was taken from
> `opencore-amr`, so no licensing decision was needed and `codec-core` stays
> MIT; opencore is used only to *produce fixtures*, which is what makes the
> three-way comparison meaningful.
> **Superseded layout.** This planned a shared `common/dsp/`. That is not what
> was built, and deliberately so: the two variants turned out to share far less
> than the plan assumed (see *AMR-NB: what must not be shared with wideband*
> above). What is genuinely common lives in `fixed_point/`; everything
> spectral is per-variant. Actual state of the items listed here:
>
> - Autocorrelation and lag windowing — ✅ `wb/lp/autocorr.rs`
> - ISP→LP — ✅ `wb/lp/isp_to_lp.rs`; LSP→LP — ✅ `nb/lsp.rs` (a *different*
>   algorithm, not the same one at order 10)
> - ISF↔ISP and subframe interpolation — ✅ `wb/lp/isf.rs`; LSF↔LSP — ✅ `nb/lsp.rs`
> - Synthesis filter — ✅ `wb/synthesis.rs`
> - Levinson-Durbin, LP→ISP, residual filter — ❌ encoder-side, not started
> - Bit handling — ✅ `amr/bits.rs`, `wb/bitstream.rs`, `nb/bitstream.rs`
> - Homing (EHF/DHF) — ❌ not started

---

## Phase 2 — SDP negotiation and relay

### Done

| Item | Where |
|---|---|
| RFC 4867 §8.1 fmtp parsing and emission | `src/codecs/amr/sdp.rs` |
| RFC 4867 §8.3.1 offer/answer rules | `src/codecs/amr/sdp.rs` |
| `ModeChangePolicy` — `mode-change-period` / `mode-change-neighbor` | `src/codecs/amr/rate.rs` |
| `CmrDamper` — CMR-interval damping | `src/codecs/amr/rate.rs` |
| Dynamic PT resolution from `a=rtpmap` instead of assumed Opus | `rvoip-sip/src/adapters/media_adapter.rs` |
| AMR payload types, rtpmap and fmtp in offers | `rvoip-sip/src/adapters/media_adapter.rs` |
| Negotiated `a=fmtp` carried to the media layer | `NegotiatedConfig::negotiated_fmtp`, and into `MediaConfig` as of 2026-08-10 |
| `AmrPayloadFormat::from_negotiated` — written, **not yet called** | `media-core/src/rtp_processing/payload/amr.rs` |

### A phase 0 mistake, corrected

`AmrModeSet::intersect` was documented as implementing offer/answer
negotiation. **It does not, and building negotiation on it would have been a
compliance bug.** RFC 4867 §8.3.1:

> "If a mode set was supplied in the offer, the answerer SHALL return the
> mode-set unmodified or reject the payload type."

`mode-set` is **match-or-reject, and bi-directional** — it binds media sent
*and* received by both parties. An answerer that narrows it is non-compliant,
and the peer would keep sending modes we implied were acceptable. `intersect`
survives as a plain set operation with corrected docs; `is_superset_of` is the
test the rule actually needs.

Two further rules that are easy to get wrong, both now implemented and tested:

- `octet-align`, `crc`, `robust-sorting`, `interleaving` and `channels` must be
  **echoed verbatim** by the answerer. Each combination is a distinct bit
  pattern, so changing one yields a stream the offerer cannot parse. Endpoints
  supporting several should offer them as separate payload types.
- `mode-change-period` and `mode-change-capability` are a declarative pair:
  requiring `period=2` of a peer is only permitted if that peer declared
  `capability=2` or `period=2` itself.

### The dynamic payload-type blocker is gone

`media_adapter.rs` hardcoded the entire 96–127 range to Opus, so any AMR offer
on a dynamic PT was rejected. Dynamic payload types now dispatch on the
`a=rtpmap` encoding name. Widening the range did **not** make it a wildcard —
an unknown encoding is still rejected, and there is a test pinning that.

Our own offers use four payload types, one per transport configuration, per the
RFC's "separate payload types" guidance:

| PT | Codec | Framing | fmtp |
|---|---|---|---|
| 104 | AMR-WB | bandwidth-efficient | *(none — it is the default)* |
| 105 | AMR-WB | octet-aligned | `octet-align=1` |
| 106 | AMR-NB | bandwidth-efficient | *(none)* |
| 107 | AMR-NB | octet-aligned | `octet-align=1` |

96–98 are H.264/VP8/VP9 and 111 is Opus in this stack, so these were chosen to
avoid collisions. A *peer's* AMR payload type is identified from its rtpmap, not
from these numbers — they are only our own assignments.

### Signalling reaches the media layer — corrected 2026-08-10

**This section was written as "Signalling now reaches the wire" and overclaimed.**
What it describes was built and tested; what it did not check is whether anything
called it. Two separate consumers of the negotiated fmtp were left unwired:

- `MediaConfig::with_negotiated_fmtp` had **zero callers**.
  `apply_negotiated_media_config` took six arguments, none of them the fmtp, so
  the string died at the SIP/media boundary and the relay's framing guard
  compared `""` against `""` on every call. Fixed, with an end-to-end test that
  negotiates real SDP and asserts the value in media-core's own config — plus
  the half an insert-only builder gets wrong, that a renegotiation carrying no
  fmtp must *clear* the previous one rather than leave it behind.
- `AmrPayloadFormat::from_negotiated` still has **zero production callers**; all
  of them are in its own test module. The payload layer therefore still assumes
  RFC 4867 defaults. Harmless for a transparent relay, which never unpacks a
  frame, and a real gap the moment media-core terminates AMR.

The lesson is the branch's own recurring one, in a new place: a test named
`negotiated_fmtp_is_carried_verbatim_to_the_media_layer` asserted only that the
SDP parser could read an fmtp line. It stopped one call short of the boundary it
named, and reading the test list was enough to believe the path was covered.

What follows is accurate about the mechanism.

`NegotiatedConfig` gained `negotiated_fmtp: Option<String>` — the raw `a=fmtp`
parameters agreed for the negotiated payload type. It is deliberately
**unparsed**: interpreting format parameters is the codec layer's job, and
keeping the string opaque means the signalling layer needs no AMR knowledge.
The field is generic rather than AMR-specific, so other codecs can use it.

`AmrPayloadFormat::from_negotiated(pt, codec_name, fmtp)` closes the loop.
`None` for the fmtp is a positive statement — every RFC 4867 default applies —
not missing data.

Why this mattered enough to do before the relay: **using defaults instead of the
negotiated parameters is not a degraded mode, it is a broken one.**
`octet-align` selects the framing itself, so guessing it wrong yields a stream
the peer cannot parse at all. There is a test asserting the two framings produce
different bytes for identical content and that each rejects the other's.

Which SDP is authoritative differs by role, and both are handled: the UAC reads
the answer (what the peer will send), while the UAS reads the **offer**, since
RFC 4867 §8.3.1 requires the answerer to echo transport parameters unmodified —
reading back our own answer would be circular.

### Interop peers

`g729_call` in `rvoip-sip/examples/pbx` is the template for an AMR scenario; the
runner is `./run.sh --pbx asterisk|freeswitch|both --api all --scenario ...`.
Neither peer image shipped AMR, so both needed changes. **Everything below is
outside this repo** — in `~/Developer/freeswitch` and `~/Developer/asterisk` —
and each edited file has a timestamped `.pre-amr.*` backup, since neither
directory is version-controlled.

**FreeSWITCH — done and verified.** Its image already built FS 1.10.12 from
source, so this was three `-dev` packages in the build stage, three runtime
libraries, and two lines in `freeswitch-modules.conf`. First built as
`rvoip-freeswitch:amr`, a separate tag so the working `:local` image stayed
untouched; the AMR changes are now in `:local` as well, which is the tag
`docker-compose.yml` and `scripts/up.sh` actually run. Verified: `mod_amr.so`
and `mod_amrwb.so` present, linking `libopencore-amrnb`, `libopencore-amrwb` and
`libvo-amrwbenc`, zero unresolved symbols.

**Start it with `scripts/up.sh`, not `docker compose up`.** Only `up.sh` passes
`FS_EXTERNAL_SIP_IP`/`FS_EXTERNAL_RTP_IP` from the colima VM address; without
them the entrypoint falls back to the container's own address, FreeSWITCH
advertises `172.21.0.2` in SDP, and every RTP packet from a macOS-hosted client
fails with `No route to host`. SIP still registers and the call still connects,
so it presents as a codec problem rather than a routing one.

**One trap worth recording:** FreeSWITCH has *two* module lists. `modules.conf`
decides what gets **compiled**; `autoload_configs/modules.conf.xml` decides what
gets **loaded**. Adding the codecs to the first is not enough — and this image's
`docker-entrypoint.sh` overwrites the second with its own explicit list, so the
sample config's AMR entries were being discarded. Only caught by starting the
container and running `show codec`; the build looked perfectly healthy.

With both fixed, FreeSWITCH registers four codecs:

```
AMR / Bandwidth Efficient      mod_amr
AMR / Octet Aligned            mod_amr
AMR-WB / Bandwidth Efficient   mod_amrwb
AMR-WB / Octet Aligned         mod_amrwb
```

That is a real implementation independently arriving at the same shape as our
four offered payload types (104–107): one per transport configuration, because
the framings are not interchangeable.

`mode-set-overwrite=0` mirrors the offered mode-set, which is the RFC 4867
§8.3.1-compliant behaviour our negotiation implements.

`force-oa` reads as the knob that would let both framings be exercised against a
real peer. It is not, and it was measured rather than assumed: with `force-oa=1`
FreeSWITCH answered `octet-align=1` to our PT 106 offer, which asked for
bandwidth-efficient. That is not a legal answer, and it breaks the leg it
touches. It is left at 0 — see the interop section for what does work.

**Asterisk — built, and calls placed through it.** AMR is not a loadable module
for Asterisk; it is a source patch, and the packaged Alpine Asterisk cannot take
it. `Dockerfile.amr` (separate file and tag) builds Asterisk 20 from source and
applies the patches from `traud/asterisk-amr`, with `config-amr/` carrying
`allow = amrwb` / `allow = amr`.

Three things had to be fixed before it built and ran, none of them in the
patches: `make third-party` fails under `-j` with no diagnostic and has to be
serialised to `-j1`; the patch set omits `codecs/ex_amr.h`, which is fetched
from upstream; and `astdatadir` pointed at Alpine's `/usr/share/asterisk`, which
produced only `Stasis initialization failed. ASTERISK EXITING!` until it was
pointed at `/var/lib/asterisk`.

The patches document Asterisk 13/16 support, which looked like a serious
forward-port risk across four major versions. **It is not.** Dry-run against
Asterisk 20.20.1: every hunk applies, two with fuzz 1. They are purely additive
— appending to alphabetically-ordered registration lists — and the APIs they
touch (`ast_codec`, `CODEC_REGISTER_AND_CACHE`, `set_next_mime_type`,
`add_static_payload`) have been stable since 13. Verifying this took two minutes
and converted an open-ended risk into a known quantity; worth doing before a
long build rather than after.

To rebuild:

```sh
cd ~/Developer/asterisk && docker build -f Dockerfile.amr -t rvoip-asterisk:amr .
```

Note the two PBXes both bind 5060 and cannot run at once; `docker compose stop`
one before starting the other.

The Dockerfile fails loudly rather than silently producing an AMR-less image: it
asserts `configure` detected all three libraries, and that `codec_amr.so` and
`res_format_attr_amr.so` both exist, before the runtime stage.

### First evidence against a real implementation

FreeSWITCH 1.10.12 was started with AMR and asked to originate an AMR-WB call.
The `a=fmtp` it emitted:

```
a=rtpmap:102 AMR-WB/16000
a=fmtp:102 octet-align=0; mode-set=8; max-red=0; mode-change-capability=2
```

That line is now a checked-in fixture in `sdp.rs`, with tests asserting we parse
it and answer it compliantly. **All four passed first run, with no changes to
the implementation** — which is the point of testing against something other
than our own reading of the RFC.

Three things it independently confirms:

- **The payload type is 102** — neither of the two we offer for AMR-WB. Dynamic
  payload types genuinely must be resolved from the `a=rtpmap` encoding name,
  which is the refactor in `989d97eb`.
- **`mode-set=8` alone is a real, common configuration** (a gateway pinned to
  23.85 kbit/s). Our answer returns it unmodified rather than widening it to the
  nine modes we support — the §8.3.1 rule that phase 0 had wrong.
- **`mode-change-capability=2`** appears in the wild, so the declarative pair we
  implemented is not a theoretical corner.

### And the framing, against real RTP

A live AMR-WB call was then established with FreeSWITCH and its RTP captured.
50 payloads are checked in at `src/codecs/amr/testdata/`, with tests asserting:

- every payload is **61 octets** — 4-bit CMR + 6-bit ToC + 477 speech bits =
  487 bits, exactly what our mode table predicts, arrived at independently;
- each parses as one AMR-WB mode 8 speech frame with the Q bit set;
- **our packetizer reproduces FreeSWITCH's exact octets** from the parsed form,
  byte for byte — the strongest agreement available short of the 3GPP vectors;
- parsing them as octet-aligned fails for all 50, which is the interop failure
  mode this format is prone to.

**A first attempt at this produced a worthless fixture, and the way it was
caught is worth recording.** Using FreeSWITCH's `&echo` gave 50 payloads that
looked perfect — until a check showed all 50 were byte-identical to what we had
sent. FreeSWITCH was passing them through, not transcoding, so the "captured"
bytes were our own returning. It would have been a test of our packetizer
wearing a disguise. Bridging the call into a conference forces a mix in linear
PCM and therefore a real encode; the second capture has 50 distinct payloads.
`real_payloads_carry_genuine_encoder_output` now guards the fixture against that
mistake recurring.

Note what the first attempt *did* legitimately establish: FreeSWITCH parsed our
hand-built RFC 4867 payload, matched it to AMR-WB mode 8 and relayed it, which
is real validation of the packetizer.

Still open: this exercises framing frame-by-frame, not the full relay path with
rvoip bridging two live legs.

### Remaining for Phase 2

- [x] **Pass-through/relay path — already existed.** `relay/controller/bridge.rs`
      forwards RTP payload bytes without inspecting them, so AMR relays through
      it with no codec kernel. What it lacked was a correctness check: it
      compared payload types only, and two AMR legs can share a payload type
      while disagreeing on framing. That now returns `BridgeError::FormatMismatch`
      rather than relaying unparseable audio.
- [ ] `amr_call` scenario in `rvoip-sip/examples/pbx`, modelled on `g729_call`.
      **It cannot be audio-verifying the way `g729_call` is** — that scenario
      pushes tones through rvoip's own codec, which for AMR does not exist. The
      relay topology is what makes audio verification possible without one:
      peer → rvoip (frames only) → peer, with the PBXes doing the codec work.
- [x] CMR **emission** exists: `request_peer_codec_mode` on the coordinator
      stamps a CMR on the next outgoing payload (once — repeating it is what
      `CmrDamper` will be for), `peer_codec_mode` reads back the mode the
      peer is actually sending, and the adapter-level round trip is pinned by
      `a_requested_mode_change_crosses_the_wire_and_moves_the_peer`.
      `CmrDamper` **now has its caller** (2026-08-12): with
      `Config::amr_auto_cmr` set, the decode path feeds every arriving
      frame-block to the damper, and once per five-second interval the mode it
      names is routed across the encoder/decoder seam and stamped on the next
      payload. Off by default — a badly damped requester oscillates the peer's
      rate, which is worse than never asking — and an explicit
      `request_peer_codec_mode` always outranks it, which is pinned by a test.

      What this is **not**: the damper implements rtpengine's up-shift policy,
      asking for a mode the peer is *not* using. A loss-driven *down*-shift —
      telling a peer to slow down because our receive path is losing packets —
      needs receiver statistics the codec object never sees, and remains
      unimplemented.
- **Exit criterion:**
  - [x] AMR-WB call completed with **rvoip as the relaying B2BUA** against
        Asterisk (octet-aligned) and FreeSWITCH (bandwidth-efficient), UDP and
        TLS+SRTP, via the `b2bua_call` harness scenario. rvoip terminates both
        legs and bridges their payloads; the quality gate confirms the caller
        recovers the target's tone and vice versa, and a forced codec mismatch
        on one leg fails the bridge (the cell is not vacuous). This is rvoip in
        the middle, not an endpoint through the PBX's own bridge.
  - [x] **Mid-call mode switch observed on the wire** (`PBX_AMR_MODE_SWITCH=1`):
        after the quality floor is secured, the caller emits CMR 0 and the far
        endpoint's encoder drops from mode 8 (23.85) to mode 0 (6.60),
        observed by the caller's own decoder — through Asterisk's relay, and
        through the full chain Asterisk → rvoip bridge → Asterisk in
        `b2bua_call`. Non-vacuous both ways: the peer must be *at* the top
        mode before the request and at mode 0 after. On FreeSWITCH the same
        run fails deterministically and correctly: FS answers `mode-set=8`,
        making any CMR unsatisfiable on that leg, and our endpoint declines
        it per RFC 4867 §3.4.1 — the negative path, exercised live.
  - [x] **Kamailio+rtpengine and OpenSIPS+rtpengine** (2026-08-12): committed
        labs under `infra/release-runners/pbx/{kamailio,opensips}` — a
        registrar-proxy in the signaling path (Record-Route) with rtpengine
        relaying media in userspace (`table=-1`) and **no transcode flags**.
        `amr_call` sweeps all four framings (`amrnb amrwb amrnb_be amrwb_be`)
        against each proxy, tone-verified at both ends under the quality
        gate. Passthrough is proven three ways: the cell pcap shows the AMR
        PT and `a=fmtp` (octet-align, mode-set) crossing the relay unchanged
        with only address/port rewritten; the rtpengine log carries zero
        transcoding lines; and both endpoints latch on rtpengine's ports, so
        the media demonstrably traversed it. The configs **fail closed** — a
        dead relay 503s the INVITE instead of relaying SDP untouched, which
        is what made the first (vacuous) green run detectable and is now the
        guard against it recurring.

        Getting here surfaced a real stack bug the B2BUA labs could never
        see: neither our UAC nor our UAS ever **learned the route set** (RFC
        3261 §12.1.1/§12.1.2), so every in-dialog request (BYE, re-INVITE)
        bypassed record-routing proxies and went straight to the peer
        Contact. Fixed in `sip-dialog` (UAC learns from the dialog-forming
        2xx's Record-Route reversed, UAS from the request's in order, both
        preserving URI parameters; `create_response` now echoes
        Record-Route), pinned by unit tests and by the wire: the BYE now
        carries the Route header through the proxy and rtpengine receives
        one `delete` per call teardown.

---

## Phase 1 — RFC 4867 payload format

### Done

| Item | Where |
|---|---|
| MSB-first bit reader/writer (bandwidth-efficient framing is not octet-aligned) | `src/codecs/amr/bits.rs` |
| Bandwidth-efficient **and** octet-aligned framing | `src/codecs/amr/payload.rs` |
| CMR, ToC chains, F/FT/Q bits, multi-frame packets | `src/codecs/amr/payload.rs` |
| `NO_DATA` (FT 15) and `SPEECH_LOST` (FT 14, WB only) | `src/codecs/amr/payload.rs` |
| Reserved-FT rejection (RFC 4867: discard the packet) | `src/codecs/amr/mode.rs` |
| AMR file storage format, whole-file and incremental readers | `src/codecs/amr/storage.rs` |
| Frame CRC, robust sorting, interleaving fields | `src/codecs/amr/payload.rs` |
| AMR-WB class A bit table (TS 26.201 Table 2) | `src/codecs/amr/mode.rs` |
| `PayloadFormat` adapter for the existing pipeline | `media-core/src/rtp_processing/payload/amr.rs` |
| `amr_unpack` fuzz target + seed corpus | `crates/media/fuzz/` |

**Verification:** 77 AMR tests in codec-core (230 total) plus 9 in media-core,
and **20 million fuzz iterations with zero crashes**. Notable coverage:

- Round-trip over **every** (variant × mode × framing) combination, and every
  frame count from 1 to the 32-frame limit.
- Truncation rejected at **every byte prefix** of a valid payload.
- Both framings asserted byte-for-byte against the RFC bit diagrams, and
  asserted *not* to be interoperable — decoding one as the other must fail
  rather than yield plausible garbage. That is the most frequently reported AMR
  interop bug, so it gets an explicit test.
- Cross-variant frames rejected on pack: NB and WB frame sizes collide, so
  packing one into the other's stream would silently corrupt the payload.
- Storage frames feed the payload packer without conversion, which is what will
  let 3GPP conformance vectors drive the payload tests directly.
- The CRC is cross-checked against a second, independently written
  implementation of the RFC's prose description, over every mode and several
  data patterns — the two agree.
- The CRC is shown to detect damage to a class A bit and to *ignore* damage to a
  class B bit, which is the behaviour unequal error protection is for.
- Robust sorting is round-tripped with frames of mixed lengths, including
  zero-length ones; short frames dropping out of later rounds is the part of
  §4.4.3 easiest to get wrong.
- A `cargo fuzz` target (`amr_unpack`) sweeps all framing/extension
  combinations, asserting not just no-panic but that anything which unpacks
  re-packs and re-parses to the same packet.

### The three optional octet-aligned extensions — now implemented

| Extension | State |
|---|---|
| Frame CRC (§4.4.2.1) | Implemented for **both** variants |
| Robust sorting (§4.4.3) | Implemented |
| Interleaving (§4.4.1) | ILL/ILP carried; **receive-side reassembly implemented**, transmit-side interleaving not |
| `max-red` redundancy (§3.5, §4.3) | Implemented — `codecs/amr/redundancy.rs` |
| IF1 + IF2 (TS 26.101 / 26.201) | 🟢 **Both formats, both variants**, against the specs' own tables and worked examples |

**VAD2 is ported and bit-exact** (2026-08-12) — `nb/enc/vad2.rs`, 300
half-frames of the committed DTX input matching TS 26.073's own `vad2()` on
every field.

Worth stating precisely, because the name invites a wrong assumption:
**VAD2 is narrowband-only.** AMR-NB defines two detectors and ships both
(`vad1.c`, `vad2.c`, with `vadname.c` reporting which was compiled); AMR-WB
defines one (`wb_vad.c`), which `wb/enc/vad.rs` already implements bit-exactly.
There is no "VAD option 2" for wideband and inventing one would be inventing
spec.

It shares nothing with VAD1 but its output type. VAD1 works from the encoder's
own analysis — LP residual, open-loop lags, tone flag. VAD2 does its own signal
analysis: pre-emphasis, a 128-point real FFT, energy summed into sixteen
non-uniform channels, and an SNR/hangover state machine over those. It also
consumes 80 samples per call, so a 20 ms frame is two calls and the frame
decision is their OR.

Verification follows the VAD1 pattern for the same reason — the decision
appears nowhere in the bitstream, and here a wrong half-frame can additionally
be masked by its partner. `tools/nb_vad2_probe.c` dumps the reference's whole
state per half-frame (sixteen channel energies, sixteen noise estimates, the
long-term dB array, and every counter) and the test compares all of it;
`tools/trace-amrnb-vad2.sh` regenerates the trace, reproducibly.

The test was checked by mutation rather than assumed: perturbing the
pre-emphasis factor fails at half-frame 0 channel 1, and perturbing one entry
of the hangover table fails at half-frame 244 on the counters — two
independent subsystems. A third mutation (`CEE_SM_FAC` by one LSB) is
genuinely absorbed, because `mult(18022, 16384)` and `mult(18023, 16384)` both
truncate to 9011; that is arithmetic, not a weak test.

**IF2 for both variants** (2026-08-12) — `codecs/amr/interface_format.rs`,
built from TS 26.101 Annex A and TS 26.201 Annex A after those specs were
fetched. They are fetched for design and never redistributed, exactly as the
reference C is.

The two formats agree on almost nothing, which is the whole story here:

| | narrowband (26.101) | wideband (26.201) |
|---|---|---|
| Frame Type | four **LSBs** of octet 1 | four **MSBs** of octet 1 |
| Frame Quality Indicator | **absent** | 1 bit, after the frame type |
| Bit packing | **LSB-first** in each octet | **MSB-first** |
| SID Core Frame | 39 bits | 40 bits |

**The first attempt at this was wrong in both variants, and Wireshark called
it correct.** Narrowband was written with the wideband convention and the
dissector rejected it outright — that much was caught. But the wideband frames
it *accepted* were also wrong: they omitted the FQI bit, so every frame was
one bit short and 6.60 kbit/s was a whole octet short of the spec's 18. The
dissector reads the mode from the header nibble and never checks the length,
so it reported all nine modes happily. An oracle that answers a narrower
question than the one being asked will confirm a broken implementation, and
this is what that looks like.

What settles it now is the specs' own numbers: every frame length is asserted
against Table A.1b on both sides, and the worked 6.70 kbit/s (26.101 A.1a) and
8.85 kbit/s (26.201 A.1a) examples are asserted bit by bit — `d(0)` at bit 5 of
octet 1 for narrowband, `d(0)` at bit 3 after the FQI for wideband. Wireshark
still agrees, now for all eight narrowband and all nine wideband modes, but it
is the corroboration rather than the authority.

**IF1 followed** (same day): the generic frame format of both specs' §4 —
frame type, FQI, mode indication, mode request, an 8-bit codec CRC over the
class A bits, then the Core Frame, MSB-first throughout. The two variants
split the auxiliary octets differently (narrowband packs MI into octet 1 and
MR into octet 2 with five spare bits; wideband spares out octet 1 and gives MI
and MR four bits each in octet 2), and FT 14/15 are header-only — four bits
for narrowband, five for wideband, another place the two disagree.

The codec CRC is `G(x) = x^8+x^6+x^5+x^4+1` — the exact bit-reversal of RFC
4867's payload CRC polynomial, so nothing is shared and each carries its own
hand-worked vectors. A CRC mismatch on unpack is reported, not refused:
26.101 Table 1c maps it to `SPEECH_BAD`, whose bits may still assist
concealment. Coverage is asserted precisely — a flipped class A bit fails the
CRC, a flipped class B bit does not, which is the CRC doing what the spec
sized it for.

One extraction hazard recorded for the next reader: flattening the specs'
docx tables to text interleaves cells, and 26.201's worked example then
*appears* to show frame type 3 for 12.65 kbit/s. The normative tables (1a, 7,
A.1b) and the example's own 253-bit core all say frame type 2, and Wireshark
reads the nibble the same way. Checked against the tables, not the flattened
prose.

**Interleaving** (2026-08-12) gained its receive half:
`codecs/amr/interleave.rs` reassembles a group's frame-blocks from the ILL/ILP
fields the parser has always carried, reporting positions that never arrived as
lost so concealment answers for them. A payload at index `ILP` of a group of
`ILL + 1` carries the blocks at `ILP`, `ILP + (ILL+1)`, … — a receiver that
ignores this decodes 20 ms blocks in shuffled order, which sounds broken but
parses perfectly, so nothing upstream would report it.

The buffer holds exactly one group and is bounded by the field widths (16
packets × 32 frame-blocks). A peer that never completes a group cannot make it
grow: the group flushes when the next one starts, and its gaps are reported
rather than waited for.

**This does not make interleaving usable end to end, and the session is still
refused.** RFC 4867 §8.1 makes fmtp declarative — a peer naming `interleaving`
is asking to *receive* interleaved payloads, which obliges our transmit side,
and we do not interleave on transmit. What closed is the parse-only gap on the
direction we can control; `AmrAdapter::new` still declines the negotiation, now
saying which direction is missing.

**`max-red` redundancy** (2026-08-12) is a scheduler and a dedup filter, both
in `redundancy.rs`. Redundancy in RFC 4867 is not a separate mechanism: it is
the multi-frame payload used deliberately, re-sending recent frames beside the
new one, so the scheduler's whole job is choosing the frame list and the
timestamp. The rule that is easy to invert and expensive to debug — §4.3
orders frames **oldest first** and stamps the payload with the *oldest*
frame's timestamp — is stated in the module header and pinned by a test,
because getting it backwards shifts audio by the redundancy depth rather than
failing to parse.

Depth is bounded by what the peer's `max-red` permits and a too-deep request
is **refused rather than clamped**: a caller that thinks it has three-deep
protection and silently got one is worse off than one told no. The dedup side
is wrapping-aware — a 32-bit timestamp at 8 kHz wraps every six days, and a
naive `>` comparison would drop every frame for an epoch after the wrap.

We advertise `max-red=0`, so nothing turns this on by itself; the receive path
handles a peer's multi-frame payloads either way, since a peer may bundle for
its own reasons.

**The WB CRC blocker is gone.** An earlier revision recorded that RFC 4867
defers the AMR-WB class A counts to TS 26.201 and that we did not have them.
They were extracted from TS 26.201 Table 2 and are now in `mode.rs`:

| Mode | 6.60 | 8.85 | 12.65 | 14.25 | 15.85 | 18.25 | 19.85 | 23.05 | 23.85 |
|---|---|---|---|---|---|---|---|---|---|
| Class A | 54 | 64 | 72 | 72 | 72 | 72 | 72 | 72 | 72 |

Cross-check: class A + B + C from TS 26.201 equals the frame total from RFC
4867 for all nine modes — two independent sources agreeing.

TS 26.201 also settles *which* bits the CRC covers: "when the AMR-WB codec mode
is 6.60, then the Class A bits are d(0)..d(53)". Class A bits are the frame's
**leading** bits in importance order, so the CRC is computed over the first
`class_a_bits()` bits of the payload. `class_a_bits()` consequently returns
`usize` rather than `Option<usize>`.

**Interleaving is carried, not applied.** ILL/ILP are emitted and parsed and
exposed on `AmrPacket`, but reassembling frame-block order means holding frames
from up to `ill + 1` packets and emitting them out of arrival order. That is
jitter-buffer work; doing it in the payload format would duplicate reordering
logic that layer already owns. Documented on `AmrInterleaving`.

### One RFC ambiguity resolved by choice

RFC 4867 §4.4.2.1 says the CRC list follows the table of contents but does not
say whether frames carrying no data get a CRC entry. We follow TS 26.201 —
"When Frame Type Index of table 1a is 14 or 15, the CRC field is not included"
— so `NO_DATA` and `SPEECH_LOST` contribute no CRC octet. **If a peer disagrees,
every CRC after the first no-data frame will misalign.** It is the one place in
Phase 1 where we guessed, and it was measured to be **externally unsettleable
for now**: Wireshark 4.6's AMR dissector has no CRC mode at all (its
`amr.encoding.version` preference offers only octet_aligned / bw_efficient /
IF1 / IF2 — RFC 3267's CRC variant never made it in), FreeSWITCH's mod_amr does
not negotiate CRC, and neither reference tree ships CRC framing code. The CRC
*arithmetic* itself is now triple-checked — the module implementation, an
independently written prose implementation, and hand-worked register traces
(`crc_matches_hand_worked_vectors`) — so what remains open is only the
no-data-octet framing choice, to be confirmed if a CRC-negotiating peer ever
appears.

### Remaining for Phase 1

- [ ] Byte-for-byte comparison against captured real-world AMR pcaps via the
      Wireshark dissector. **Needs sample captures, which we do not have.**
      Deferred to Phase 8 interop, where live traffic is available anyway.

---

## Phase 0 — Foundations

### Done

| Item | Where |
|---|---|
| `AmrVariant`, `AmrMode`, `AmrFrameType`, `AmrModeSet` | `src/codecs/amr/mode.rs` |
| RFC 4867 frame-size / class-A / bit-rate tables, with tests asserting each value | `src/codecs/amr/mode.rs` |
| `mode-set` intersection with offer/answer semantics | `src/codecs/amr/mode.rs` |
| `CodecType::AmrNb` / `AmrWb` and all `match` arms | `src/types.rs`, `src/utils/validation.rs` |
| `AmrParameters` mirroring the RFC 4867 SDP attributes | `src/types.rs` |
| `VariableRateCodec`, `CodedFrame`, `FrameKind` | `src/types.rs` |
| `AmrCodec` stub implementing both traits | `src/codecs/amr/mod.rs` |
| Factory / registry / capabilities wiring | `src/codecs/mod.rs`, `src/lib.rs` |
| Feature chain `rvoip` → `rvoip-sip` → `media-core` → `codec-core` | four `Cargo.toml`s |
| ADR 001 for the trait design | [`ADR_001_VARIABLE_RATE_CODEC.md`](ADR_001_VARIABLE_RATE_CODEC.md) |

**Verification**

- 21 AMR tests pass; 174 codec-core unit tests + 9 doc-tests pass with
  `--all-features`.
- Feature matrix compiles: default (no AMR), `amr-nb` alone, `amr-wb` alone,
  `--no-default-features`, and `--all-features`.
- Downstream crates build with the feature on: `rvoip-media-core`,
  `rvoip-sip`, and the `rvoip` facade, each with `--features amr`.
- Clippy clean for new code under `pedantic` + `nursery`. Two warnings remain,
  both pre-existing const-eval lints on assertions this work did not touch
  (`g711/tests/itu_validation_tests.rs:27`, `lib.rs:262`).

```bash
cargo test -p rvoip-codec-core --all-features
cargo check -p rvoip --features amr
```

### Outstanding — needs someone other than the compiler

| Item | Blocks | Owner |
|---|---|---|
| ~~**IP-1** — AMR-WB/G.722.2 essential patents~~ | ~~Phase 3+~~ | ✅ **CLEARED 2026-08-09** — no blockers |
| ~~**Oracle qualification**~~ | ~~Phase 3~~ | ✅ **DONE** — both tier-1 references build; every stage is verified against them |
| ~~**Spec acquisition**~~ | ~~Phase 3~~ | ✅ **DONE** — TS 26.173 and 26.073 fetched by script |
| ~~**IP-2a** — may 3GPP test sequences (TS 26.074 / 26.174) be vendored?~~ | ~~Conformance testing only~~ | ✅ **DECIDED 2026-08-12 — no** |
| **IP-2b** — may vectors *generated by running* the reference be committed? | Nothing — already done | ⚠️ **assumed yes** |
| **Staffing** — is a second engineer available? | Pulls the encoder milestone in | ❗ unassigned |

**IP-1 is cleared and the oracles are qualified.** Everything that was gating
DSP work is resolved.

**IP-2b needs a decision after the fact, not before.** Generated vectors *are*
committed — `lp_stages_wb.txt`, `stages_nb.txt` and the `.pcm` ground truth are
all output from running the 3GPP references. That was judged to be output
rather than redistribution of the source, which is never committed. If that
reading is wrong, the fixtures must be regenerated on demand in CI instead;
both build scripts already reproduce them bit-identically from a clean fetch,
so the change would be mechanical.

**IP-2a is decided: no 3GPP material enters this repository, ever.** That is a
standing rule, not a judgement about these particular files, and it closes the
question rather than deferring it — vendoring the TS 26.074/26.174 sequences
would be redistribution of 3GPP copyright material, which is exactly what the
whole reference-handling posture exists to avoid.

The conformance suites therefore run **fetch-on-demand only**, and nothing
about that is a compromise:

- `tools/build-amr-reference.sh` and `tools/build-amrnb-encoder-reference.sh`
  fetch and build the references into a scratch directory. Nothing they fetch
  is committed.
- `codecs/amr/conformance.rs` reads the sequences from the directories those
  scripts populate, named by `RVOIP_AMRWB_REFERENCE` / `RVOIP_AMRNB_REFERENCE`.
  When the sequences are absent the tests **panic rather than skip** — a
  conformance test that quietly passes because it found nothing to compare is
  worse than no test at all.
- A fresh machine needs exactly one thing to run them: network access to the
  reference sources, then `tools/run-amr-conformance.sh`, whose six-pass
  contract is unchanged.

What is committed is *generated output* — PCM ground truth, per-stage traces,
bitstreams — under the IP-2b reading below. Independently of all of it, the
committed fixtures come from opencore-amr and vo-amrwbenc, which is a stronger
check than the 3GPP sequences would be anyway: an independent implementation
agreeing is worth more than the reference agreeing with itself.

Note the environment currently has a **selective egress allowlist**:
`raw.githubusercontent.com` is reachable but `github.com` is not, and Docker
containers have no outbound network at all. Oracle qualification needs to fetch
and build `opencore-amr`, `vo-amrwbenc` and the 3GPP reference, so it needs
either that egress restored or the sources staged by hand.

---

## Decisions taken

| # | Decision | Recorded in |
|---|---|---|
| 1 | Pure Rust in the shipped crate; no FFI backend at any point | Plan §6.1 |
| 2 | Transcoding required from day one; relay is a by-product | Plan §1.2 |
| 3 | Five-oracle roster, out-of-tree, vector-generating | Plan §1.2.1 |
| 4 | **WB first**, NB second | Plan §1.2 |
| 5 | `VariableRateCodec` as a new trait, not a widened `AudioCodec` | ADR 001 |
| 6 | `mode_set` as a `u16` bitmask, not `Vec<u8>` | ADR 001 |
| 7 | Stub codec errors loudly rather than returning silence | ADR 001 |
| 8 | **No 3GPP material in the repository** — references and test sequences are fetched on demand, only generated output is committed | IP-2a, 2026-08-12 |

---

## Open questions

**Q1 — Where does the RFC 4867 packetizer live?** *(decided: option (c))*

**Fully resolved, and the follow-up dissolved.** The packetizer lives in
`codec-core` beside the codec (`src/codecs/amr/payload.rs`): RFC 4867 framing is
codec framing rather than transport, and it needs the mode tables intimately.

The worry about a dependency edge turned out to be unfounded — the payload
formats do **not** live in `rtp-core` any more. They were moved to
`media-core/src/rtp_processing/payload/`, and `media-core` already depends on
both `rtp-core` and `codec-core`. So the `PayloadFormat` adapter sits there
alongside `g711.rs` and `opus.rs` with **no new dependency edge and no
duplicated tables**.

Original framing of the question, retained for context:

`rtp-core` does **not** depend on `codec-core`; both are consumed by
`media-core`. But the packetizer needs the frame-size tables that now live in
`codecs::amr::mode`. Options:

- **(a)** Add `codec-core` as a dependency of `rtp-core` — new edge in the
  dependency graph, likely unwanted.
- **(b)** Duplicate the ~20 constants in `rtp-core`. Small, but two sources of
  truth for numbers we just carefully verified.
- **(c)** Put the packetizer in `codec-core` beside the codec, and let
  `media-core` wire the two together. Diverges from the existing convention that
  payload formats live in `rtp-core` (`g711.rs`, `opus.rs`, `vp8.rs`).
- **(d)** Extract shared AMR constants into a small crate both depend on.

Chose **(c)**.

**Q2 — Crate placement.** Keep AMR inside `rvoip-codec-core`, or split into a
standalone publishable crate? A pure-Rust AMR crate would be the first in the
Rust ecosystem. Note `amr` is taken on crates.io by an unrelated GPL-3.0 project.
No urgency; revisit once there is a working kernel.

**Q3 — AMR-WB class A bit counts.** *(resolved)* Extracted from TS 26.201
Table 2 and cross-checked against the RFC 4867 frame totals. `class_a_bits()`
now returns `usize` for both variants and WB CRC works. See the Phase 1 section.

---

## Changelog

### 2026-08-10 — All four codec paths bit-exact

Both encoders now produce a byte-identical bitstream at every rate — wideband
nine, narrowband eight, 50 frames each — and both are reachable through
`AmrCodec` alongside the decoders. With the decoders finished earlier the same
day, every path this codec has is exact against its normative reference.

The encoders were built the same way as the decoders and for the same reason:
ground truth first, stages second, assembly last. Committed input PCM and
reference bitstreams at every rate, an instrumented reference encoder emitting
per-stage traces, then one module per stage group each verified against those
traces, then the frame loop. Both assemblies matched on close to the first run,
which is what the decoders' hazards bought — every composition trap the
wideband decoder found the hard way was written into the assembly brief before
a line of it existed.

Also this session: oracle qualification finally measured, the wideband erasure
path closed, a decoder fuzz target, and a latent `prev_bfi` frame-versus-
subframe divergence found and fixed before it could fire.

**Lesson 3 fired twice more and is now the one to expect.** A trace row that is
absent, or that silently duplicates another, reads exactly like a passing
comparison. Two rows were lost to output interleaving, one row was a
byte-identical copy of its neighbour under a different name, and one vacuity
check passed because a single sample moved by 8 is below what 4.75 kbit/s can
resolve. Every one was caught by a count assertion or a mutation check rather
than by reading the code.

### 2026-08-10 — Both decoders bit-exact, and reachable

The narrowband decoder joined the wideband one: all eight rates, every sample
of every frame identical to TS 26.073's own decoder. Seven were exact on the
first assembly, because the three composition hazards the wideband port found
the hard way were written into the module header before any of it ran. The
eighth, 4.75 kbit/s, failed on its shared two-subframe gain index.

Both are now reachable through `AmrCodec`, and the API path is separately
asserted bit-exact — per-stage exactness cannot see a wiring layer that resets
state between frames.

Also this session:

- **Oracle qualification, finally measured.** `opencore-amr` reproduces
  TS 26.073 exactly at all eight narrowband rates; `vo-amrwbenc` reproduces
  TS 26.173 at seven of nine wideband rates and not at 12.65 or 14.25. Risk R8
  is real but narrow, and it is now a test rather than an open question.
- **Encoder ground truth for both variants**, which did not exist in any form:
  committed input PCM, the reference bitstream at every rate, and per-stage
  traces from instrumented reference encoders.
- **The wrong `inter_6`.** The table generator had been emitting the encoder's
  25-tap pitch-search filter under a doc comment describing the decoder's
  61-tap adaptive-codebook filter. Both are now generated under names that say
  which is which.
- **Two trace harnesses silently dropped rows**, in C and in Rust, for the same
  reason — output merging into a line a filter then discarded. Recorded above
  as lesson 3, because it is the same failure as an oracle sharing your
  assumption, seen from the other side.

### 2026-08-09 — Spec obtained; first DSP written

TS 26.190 was never unreachable. `3gpp.org` and `etsi.org` were rejecting
curl's user agent with 403; the previous entry conflated that with hosts that
fail at the connection layer and declared the whole set blocked.

With the spec in hand, wrote the LP analysis front end (§5.2.1). The oracle
confirmed the reconstructed window formula against `ham_wind.tab` — 384 values,
all within 1 LSB — and surfaced that the reference folds the white-noise
correction into its lag window where the spec puts it on r(0).

### Superseded: 2026-08-09 — Phase 3 blocked on spec access

The LP-analysis chain cannot be written bit-exactly without TS 26.190's
normative windows, lag-window values and Q-formats. Every 3GPP spec mirror is
unreachable or 403 from this environment, including the ARIB one that served
TS 26.201 earlier the same day.

Stopped rather than writing structurally-correct DSP with invented tables — that
produces something that looks finished and is not bit-exact, which is worse than
an obvious gap. Two ways forward are recorded above; one of them is a licensing
decision that is not mine to make.

### 2026-08-09 — The oracle is running, and it confirmed the mode table

Built `opencore-amr` and `vo-amrwbenc` natively with clang. The supposed
blocker was container networking, but the harness only ever needed source and a
compiler; sources come from the Debian archive, which is reachable.

The reference encoder's frame sizes match our mode table at all nine AMR-WB
modes. 225 reference frames checked in, with the build script reproducing them
bit-identically.

### 2026-08-09 — Phase 3 opened; the basic operators were already written

The ETSI basic-operator set Phase 3 was going to start by writing already
existed in the G.729A port, validated by that codec being bit-exact. Promoted
from `codecs::g729::impls::dsp` to a shared `crate::fixed_point`; G.729 reaches
it under the old name via a re-export so none of its ~318 call sites changed.

Table-driven transcendentals were deliberately left behind with G.729 rather
than shared, because AMR supplies its own tables and their equality is unproven.

### 2026-08-09 — Framing validated against real FreeSWITCH RTP

Captured 50 AMR-WB payloads from a live call and checked them in. Our
packetizer reproduces them byte for byte.

The first capture attempt was invalid — `&echo` passes through rather than
transcoding, so the payloads were our own bytes returning. Caught by comparing
them against what we sent. A conference bridge forces a real encode; there is
now a test guarding the fixture against that mistake.

### 2026-08-09 — IP-1 cleared

Counsel confirmed no patent blockers. Phase 3 (the codec kernel) is unblocked
for the first time — every phase to date has been protocol and plumbing
precisely because this gate was open.

### 2026-08-09 — Negotiation validated against FreeSWITCH

Captured a live AMR-WB offer from FreeSWITCH 1.10.12 and pinned it as a test
fixture. Parser and answerer both handled it correctly on the first run.

Also fixed a trap in the FreeSWITCH image: the codecs were compiled but never
loaded, because the entrypoint overwrites the runtime module list. The build
looked entirely healthy — only starting the container and running `show codec`
revealed it.

### 2026-08-09 — Interop peers, and the relay that already existed

FreeSWITCH AMR built and verified. Asterisk prepared; its build is blocked on
container networking, not on anything in the patch — which dry-runs cleanly
against Asterisk 20 despite targeting 13/16.

The relay path turned out to be largely a discovery rather than a build: the
transparent bridge is codec-agnostic. The work was closing a hole in its
compatibility check, where matching payload types were treated as sufficient.

### 2026-08-09 — Negotiated parameters reach the media layer

`NegotiatedConfig::negotiated_fmtp` plus `AmrPayloadFormat::from_negotiated`.
The signalling layer carries the fmtp string without interpreting it; the codec
layer parses it.

**Corrected 2026-08-10:** this claimed to close the gap and did not. Neither
consumer was called — `MediaConfig::with_negotiated_fmtp` had zero callers, and
`AmrPayloadFormat::from_negotiated` still has none outside its own tests. The
first is now wired end to end; the second is not. See "Signalling reaches the
media layer" above.

### 2026-08-08 — Phase 2 negotiation layer

SDP parameters, offer/answer, rate adaptation, and the dynamic payload-type
refactor. The relay path itself is still outstanding — see above.

The significant find was that phase 0's `mode-set` intersection was wrong:
RFC 4867 requires match-or-reject, not narrowing. Caught by reading §8.3.1
directly rather than trusting the summary that led to the original code.

### 2026-08-08 — Phase 1 complete

Closed out the remainder: frame CRC (both variants), robust sorting,
interleaving fields, the `PayloadFormat` adapter, and a fuzz target.

- The AMR-WB CRC blocker was resolved by extracting TS 26.201 Table 2. Class A
  + B + C from that table equals the RFC 4867 frame total for all nine modes,
  which is a genuine two-source cross-check rather than an assumption.
- TS 26.201 also confirmed class A bits are the frame's *leading* bits
  ("d(0)..d(53)" for mode 6.60), which is what makes a CRC over the first N
  bits correct.
- The `PayloadFormat` adapter needed no new dependency edge after all: payload
  formats had already been moved out of `rtp-core` into `media-core`, which
  depends on both crates. Q1's follow-up dissolved.
- A test failure caught a real convention issue: a frame is `mode.bits()` bits,
  not `octet_aligned_bytes()` bytes, so trailing bits of the final octet are
  padding and do not survive a round trip. Now documented on `pack` and pinned
  by a test rather than left to be rediscovered.

### 2026-08-08 — Phase 1 core complete

RFC 4867 payload format (both framings) and the AMR file storage format, with
62 AMR tests. Three optional octet-aligned extensions deferred; see above.

Worth flagging:

- The two framings differ in size for the same content — an AMR-WB mode 0 frame
  is 18 octets bandwidth-efficient and 19 octet-aligned. That saving is the
  point of the framing, and it is exactly why the two are not interoperable.
  There is an explicit test asserting cross-decoding fails.
- `unpack` rejects a whole trailing octet beyond what the ToC accounts for.
  Sub-octet padding is legitimate; a full octet means the sender's frame sizes
  disagree with ours, which is better caught at the boundary than delivered to
  the decoder as misaligned bits.
- The ToC chain is bounded at 32 frames (640 ms). RFC 4867 sets no hard limit —
  it is governed by `maxptime` — but an unbounded F-bit chain in a hostile
  payload would otherwise loop until the buffer ran out.

### 2026-08-08 — Phase 0 code complete

Types, modes, mode-set negotiation, the `VariableRateCodec` trait, the stub
codec, and the feature chain across four crates. ADR 001 recorded.

Two things worth flagging from the implementation:

- `AmrParameters::mode_set` began as `Vec<u8>` and forced
  `CodecConfig::with_parameters` to lose `const fn`. Switched to a `u16`
  bitmask, which is `Copy`, makes duplicates unrepresentable, and turns
  negotiation into a bitwise AND.
- `AmrMode` carries its variant rather than being a bare index. NB mode 0 is
  4.75 kbit/s and WB mode 0 is 6.60 kbit/s, and several frame sizes collide
  across variants — this makes the confusion unrepresentable rather than
  something to catch in review.

### 2026-08-08 — Plan finalised

Four revisions on `feat/amr-codecs` as decisions landed: pure-Rust +
transcoding-first, then the Apache-2.0 oracle + WB-first, then the 3GPP
reference as primary oracle, then the five-oracle roster + interop matrix.
