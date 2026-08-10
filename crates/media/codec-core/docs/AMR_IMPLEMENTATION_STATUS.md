# AMR Implementation Status

Living tracker for the AMR-NB / AMR-WB work. The plan is in
[`AMR_IMPLEMENTATION_PLAN.md`](AMR_IMPLEMENTATION_PLAN.md); this file records
where we actually are.

**Branch:** `feat/amr-codecs`
**Last updated:** 2026-08-08

---

## At a glance

| Phase | Scope | Status |
|---|---|---|
| 0 | Foundations: types, feature flags, ADR, oracle qualification | 🟡 **In progress** — code landed, external items outstanding |
| 1 | RFC 4867 payload format + AMR file storage format | 🟢 **Complete** |
| 2 | SDP negotiation + relay path | 🟡 **Negotiation done** — relay path outstanding |
| 3 | `common/` DSP layer + oracle harness | ⚪ Not started |
| 4 | AMR-WB decoder, fixed point | ⚪ Not started |
| 5 | **AMR-WB encoder — the HD-voice milestone** | ⚪ Not started |
| 6 | AMR-NB decoder, fixed point | ⚪ Not started |
| 7 | AMR-NB encoder, fixed point | ⚪ Not started |
| 8 | Transcoding, interop, performance, hardening | ⚪ Not started |

**There is no working AMR encoder or decoder yet.** `AmrCodec` constructs and
negotiates, but `encode`/`decode` return `FeatureNotEnabled` naming the phase
that will supply them.

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
| Negotiated `a=fmtp` carried to the media layer | `NegotiatedConfig::negotiated_fmtp` |
| `AmrPayloadFormat::from_negotiated` — signalling to wire | `media-core/src/rtp_processing/payload/amr.rs` |

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

### Signalling now reaches the wire

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
libraries, and two lines in `freeswitch-modules.conf`. Built as
`rvoip-freeswitch:amr`, a **separate tag** so the working `:local` image is
untouched. Verified: `mod_amr.so` and `mod_amrwb.so` present, linking
`libopencore-amrnb`, `libopencore-amrwb` and `libvo-amrwbenc`, zero unresolved
symbols.

Two of its config knobs are directly useful for testing:
`force-oa` originates octet-aligned (so both framings can be exercised against
a real peer), and `mode-set-overwrite=0` mirrors the offered mode-set — which is
the RFC 4867 §8.3.1-compliant behaviour our negotiation implements.

**Asterisk — prepared, build blocked.** AMR is not a loadable module for
Asterisk; it is a source patch, and the packaged Alpine Asterisk cannot take it.
`Dockerfile.amr` (separate file and tag) builds Asterisk 20 from source and
applies the patches from `traud/asterisk-amr`, with `config-amr/` carrying
`allow = amrwb` / `allow = amr`.

The patches document Asterisk 13/16 support, which looked like a serious
forward-port risk across four major versions. **It is not.** Dry-run against
Asterisk 20.20.1: every hunk applies, two with fuzz 1. They are purely additive
— appending to alphabetically-ordered registration lists — and the APIs they
touch (`ast_codec`, `CODEC_REGISTER_AND_CACHE`, `set_next_mime_type`,
`add_static_payload`) have been stable since 13. Verifying this took two minutes
and converted an open-ended risk into a known quantity; worth doing before a
long build rather than after.

The build is blocked only by the environment: **Docker containers currently have
no outbound network** (HTTP, HTTPS and even Alpine's own repositories all
refused) while the host does. The FreeSWITCH image built successfully earlier
under the same Dockerfile pattern, so this is a recent change rather than
anything wrong with the setup. Re-run when container networking returns:

```sh
cd ~/Developer/asterisk && docker build -f Dockerfile.amr -t rvoip-asterisk:amr .
```

The Dockerfile fails loudly rather than silently producing an AMR-less image: it
asserts `configure` detected all three libraries, and that `codec_amr.so` and
`res_format_attr_amr.so` both exist, before the runtime stage.

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
- [ ] Mode-change policy and CMR damper driven from the live stream.
- [ ] **Exit criterion not yet met:** an AMR-WB call completed as a relaying
      B2BUA against Asterisk and Kamailio+rtpengine, in both framings, with a
      mid-call mode switch observed.

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
| Interleaving (§4.4.1) | ILL/ILP **carried**, reordering deliberately not performed |

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
every CRC after the first no-data frame will misalign.** Worth confirming
against a real implementation during Phase 8 interop; it is the one place in
Phase 1 where we guessed.

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
| **IP-1** — counsel confirms AMR-WB/G.722.2 essential patents expired | Phase 3+ (codec kernel). Phases 1–2 are RFC-only and unaffected | ❗ unassigned |
| **IP-2a** — may 3GPP test sequences (TS 26.074 / 26.174) be vendored? | Nothing hard; decides CI shape | ❗ unassigned |
| **IP-2b** — may vectors *generated by running* the reference be committed? | Nothing hard; decides fixture provenance | ❗ unassigned |
| **Oracle qualification** — build the five oracles, record which are bit-exact per path | Phase 3 | ❗ unassigned |
| **Spec acquisition** — TS 26.090, 26.190, 26.101, 26.201, 26.091–094, 26.191–194 | Phase 3 | ❗ unassigned |
| **Staffing** — is a second engineer available for Phase 2? | Pulls HD-voice milestone in ~2–3 weeks | ❗ unassigned |

Oracle qualification is the highest-value outstanding item: it is cheap, and it
settles whether Phase 5 needs its 2–3 week fallback budget (risk R8).

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
layer parses it. Closes the gap flagged in the previous entry, where a relay
would have framed packets with defaults rather than what was negotiated.

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
