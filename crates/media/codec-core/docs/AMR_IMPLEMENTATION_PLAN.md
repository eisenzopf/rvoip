# AMR-NB / AMR-WB Implementation Plan

Status: **Draft for review** — branch `feat/amr-codecs`
Owner: TBD
Last updated: 2026-08-08

Goal: add AMR narrowband (AMR-NB) and AMR wideband (AMR-WB / ITU-T G.722.2) to rvoip so
the stack can carry HD voice on ordinary telephony interconnects (VoLTE, IMS, mobile
carrier SBCs), where AMR-WB — not Opus — is the codec that is actually offered.

---

## 1. Executive summary

### 1.1 What "adding AMR" actually means

Three separable pieces of work, in increasing order of cost:

| Layer | What it is | Independent of codec kernel? | Rough size |
|---|---|---|---|
| **L1 — RTP payload format** | RFC 4867 packetization: CMR, ToC, F/FT/Q bits, bandwidth-efficient vs octet-aligned, interleaving, CRC | Yes | ~2–3 weeks |
| **L2 — SDP / signalling** | `mode-set`, `octet-align`, `mode-change-*`, `crc`, `robust-sorting`, dynamic PT allocation, offer/answer intersection, CMR-driven rate adaptation | Yes | ~2–3 weeks |
| **L3 — Codec kernel** | The actual ACELP encoder/decoder for 8 NB modes + 9 WB modes, VAD/DTX/CNG, error concealment | No | months (see §7) |

**L1 + L2 alone are worth shipping on their own.** They let rvoip act as a signalling-aware
SBC/B2BUA that *relays* AMR-WB end-to-end without transcoding — which is the single most
common production requirement, is 100 % clean-room, and carries no IP or licensing risk.
L3 is where the real cost is.

### 1.2 Recommendation

Ship in the order **L1 → L2 → L3-decoder → L3-encoder**, with a non-default FFI escape
hatch available from day one. Rationale in §7.4. Concretely:

1. **Phase 0–2** (payload format + SDP + relay path): pure Rust, no codec kernel, no IP
   exposure. Delivers AMR-WB pass-through calls.
2. **Phase 3** (`amr-ffi`, opt-in, off by default): link Apache-2.0 `opencore-amr` +
   `vo-amrwbenc` so anyone who needs transcoding *now* has it, and — more importantly —
   so we have a **bit-exact oracle** to develop the Rust kernel against.
3. **Phase 4–7**: pure-Rust decoder then encoder, NB first, then WB, validated to
   bit-exactness against the 3GPP test sequences. Retire the FFI feature or keep it as a
   cross-check.

### 1.3 The one thing to get right before writing code

**Patent expiry and source-code copyright are two different things, and only one of them
has expired.** See §2. The published 3GPP reference C (TS 26.073 / 26.173) is *not*
clearly licensed for redistribution — the repo the search turned up
(`pschatzmann/codec-amr`) says so itself: *"The license of the code from 3gpp is
unclear!"*. Our port must be written against the **specification text**, not by
transliterating anyone's C. This is a hard constraint that shapes §6 and §7.

---

## 2. Legal / IP position

> Not legal advice. This section records what the research found so counsel can sign off
> quickly; it is not a substitute for that sign-off.

### 2.1 Patents — consistent with "safe to implement"

- AMR-NB was standardised in 1999. Core essential patents are reported to have lapsed
  around 2019–2020.
- AMR-WB was standardised in 2001–2002 and published as ITU-T G.722.2 in 2003. Wikipedia
  states flatly that *"The patent for AMR expired in 2024"* and describes the technology
  as now royalty-free. Licensing was previously administered by VoiceAge (pool launched
  Feb 2010; members included Nokia, Ericsson, NTT, VoiceAge).
- No public VoiceAge announcement formally winding down the programme was found. Absence
  of an announcement is not evidence of live patents — pools generally go quiet rather
  than issue a press release — but it does mean we are relying on expiry arithmetic
  rather than an affirmative grant.

**Action IP-1 (blocking for Phase 3+, not for Phase 0–2):** have counsel confirm expiry
of the AMR-WB/G.722.2 essential-patent list published at
`voiceage.com/Patent-Portfolio-Essential.html`. Phases 0–2 implement only IETF RFC 4867
framing, which is not covered by the speech-coding patents, so they can proceed in
parallel.

### 2.2 Copyright on reference source — the actual live risk

| Source | License | Can we copy code from it? |
|---|---|---|
| 3GPP TS 26.073 / 26.104 (NB C), TS 26.173 / 26.204 (WB C) | 3GPP copyright, redistribution terms unclear | **No** |
| `pschatzmann/codec-amr` | Wraps the above; author states license "unclear" | **No** |
| FFmpeg `libavcodec/amrnbdec.c`, `amrwbdec.c` (native decoders) | LGPL-2.1-or-later | **No** — LGPL is not on this workspace's `deny.toml` allow-list, and porting from it creates derivative-work exposure |
| Wireshark `epan/dissectors/packet-amr.c` | GPL-2.0 | **No** |
| rtpengine (AMR transcoding + CMR logic) | GPL-3.0 | **No** |
| `opencore-amr` (AMR-NB enc+dec, AMR-WB dec; ex-AOSP/PacketVideo) | **Apache-2.0** | **Yes** — allow-listed in `deny.toml` |
| `mstorsjo/vo-amrwbenc` (AMR-WB encoder, ex-VisualOn/Android) | **Apache-2.0** | **Yes** |
| ITU-T G.722.2 recommendation text, 3GPP TS text | Spec text | **Yes, as the normative source to implement from** |

The workspace publishes under **MIT** (`Cargo.toml:113`) and `deny.toml` allows
MIT / Apache-2.0 / BSD / ISC / Zlib / 0BSD / BSL-1.0 / MPL-2.0 / CDLA-Permissive-2.0 /
CC0-1.0 — **no (L)GPL**. So the GPL/LGPL implementations above are usable as *behavioural
references you read to understand the standard*, but their code cannot enter the tree, and
we should avoid line-by-line study of them entirely to keep the clean-room story simple.

**Working rule for the port:** implement from TS 26.090 / TS 26.190 (and the companion
specs in §3) plus our own reading of the ETSI basic-operator definitions. Where behaviour
is ambiguous, resolve it by *running* the Apache-2.0 `opencore-amr` as a black-box oracle
and comparing output — not by reading anyone's implementation of the ambiguous part.

**Action IP-2:** confirm whether the 3GPP **test sequences** (TS 26.074 for NB, TS 26.174
for WB) may be vendored into the repo. If not, use the opt-in external-fixture pattern in
§8.3. Do not check them in until this is answered.

**Action IP-3:** add AMR entries to `THIRD_PARTY_NOTICES.md` for any Apache-2.0 code
vendored under the `amr-ffi` feature.

---

## 3. Normative specification set

Everything below is what an implementer actually has to read. Get all of them before
starting Phase 4.

### AMR-NB

| Spec | Title | Why we need it |
|---|---|---|
| TS 26.071 | AMR speech codec; General description | Orientation, mode list |
| **TS 26.090** | **AMR speech codec; Transcoding functions** | **The normative encoder/decoder algorithm** |
| TS 26.091 | Error concealment of lost frames | Decoder PLC behaviour |
| TS 26.092 | Comfort noise aspects | CNG |
| TS 26.093 | Source-controlled rate operation | DTX / SID cadence |
| TS 26.094 | Voice Activity Detector (VAD) | VAD option 1 and option 2 |
| TS 26.101 | AMR frame structure | Bit ordering, class A/B/C sorting, IF1/IF2 |
| TS 26.073 | ANSI-C source (fixed point) | **Reference only — do not copy** |
| TS 26.104 | ANSI-C source (floating point) | Reference only |
| TS 26.074 | Test sequences | Conformance vectors |

### AMR-WB

| Spec | Title | Why we need it |
|---|---|---|
| TS 26.171 | AMR-WB speech codec; General description | Orientation |
| **TS 26.190** | **AMR-WB speech codec; Transcoding functions** (= ITU-T G.722.2) | **The normative algorithm** |
| TS 26.191 | Error concealment | Decoder PLC |
| TS 26.192 | Comfort noise aspects | CNG |
| TS 26.193 | Source-controlled rate operation | DTX |
| TS 26.194 | Voice Activity Detector | VAD |
| TS 26.201 | AMR-WB frame structure | Bit ordering |
| TS 26.173 | ANSI-C source (fixed point) | Reference only — do not copy |
| TS 26.204 | ANSI-C source (floating point) | Reference only |
| TS 26.174 | Test sequences | Conformance vectors |

### Transport

| Spec | Title |
|---|---|
| **RFC 4867** | RTP Payload Format and File Storage Format for AMR and AMR-WB |
| RFC 3551 | RTP A/V profile (static PT context; AMR uses dynamic PTs) |
| TS 26.114 | IMS multimedia telephony — media handling (carrier-profile behaviour: mode-set, ptime, CMR policy in real VoLTE deployments) |

TS 26.114 is not strictly required but is what carriers actually test against; read it
before the interop phase.

---

## 4. Codec facts (verified)

These numbers are load-bearing for L1/L2 and are verified against RFC 4867. Per-parameter
bit allocations (how the 244 bits of MR122 split across LSP / lag / pulses / gains) are
**deliberately not reproduced here** — read them from TS 26.090 Table 8 and TS 26.190
Table 7 during Phase 4/6 rather than trusting a secondary source.

### 4.1 AMR-NB

- 8 kHz mono, 20 ms frames = **160 samples**, 4 subframes of 40 samples.
- LPC order 10; LSP representation; ACELP fixed codebook; DTX/VAD/CNG available.
- Mode 7 (12.2 kbit/s) is bit-allocation-identical to GSM-EFR.

| FT | Mode | kbit/s | Bits/frame | Class A bits | Octet-aligned bytes |
|---|---|---|---|---|---|
| 0 | MR475 | 4.75 | 95 | 42 | 12 |
| 1 | MR515 | 5.15 | 103 | 49 | 13 |
| 2 | MR59 | 5.90 | 118 | 55 | 15 |
| 3 | MR67 | 6.70 | 134 | 58 | 17 |
| 4 | MR74 | 7.40 | 148 | 61 | 19 |
| 5 | MR795 | 7.95 | 159 | 75 | 20 |
| 6 | MR102 | 10.2 | 204 | 65 | 26 |
| 7 | MR122 | 12.2 | 244 | 81 | 31 |
| 8 | AMR SID | — | 39 | 39 | 5 |

FT 9–14 are reserved for AMR-NB; **FT 15 = NO_DATA**. Per RFC 4867 a ToC entry with
FT 9–14 (AMR) means the whole packet SHOULD be discarded.

> Note: several secondary sources (including Wikipedia's AMR page) swap the 6.70 and 7.40
> frame sizes. RFC 4867 is authoritative: **6.70 → 134 bits, 7.40 → 148 bits.**

### 4.2 AMR-WB

- 16 kHz mono in/out, **internally resampled to 12.8 kHz**; 20 ms frames = 320 samples at
  16 kHz / 256 at 12.8 kHz, 4 subframes of 64 samples (12.8 kHz).
- Encodes 50–6400 Hz with ACELP; the 6400–7000 Hz high band is synthesised at the decoder.
- LPC order 16; ISP/ISF representation quantised with split-multistage VQ (S-MSVQ).
- 5 ms look-ahead; ~0.9375 ms extra one-way delay from the bandsplitting filter.

| FT | kbit/s | Bits/frame | Octet-aligned bytes |
|---|---|---|---|
| 0 | 6.60 | 132 | 17 |
| 1 | 8.85 | 177 | 23 |
| 2 | 12.65 | 253 | 32 |
| 3 | 14.25 | 285 | 36 |
| 4 | 15.85 | 317 | 40 |
| 5 | 18.25 | 365 | 46 |
| 6 | 19.85 | 397 | 50 |
| 7 | 23.05 | 461 | 58 |
| 8 | 23.85 | 477 | 60 |
| 9 | AMR-WB SID | — (40 bits) | 5 |

FT 10–13 reserved (discard packet); **FT 14 = SPEECH_LOST** (AMR-WB only);
**FT 15 = NO_DATA**. AMR-WB SID carries 40 class A bits.

Mode 8 (23.85) is mode 7 (23.05) plus **16 bits = 4 bits × 4 subframes of transmitted
high-band gain** — 477 − 461 = 16 confirms this. It is the only mode that transmits HB
gain rather than deriving it.

### 4.3 RTP payload format (RFC 4867)

Two framings, and we must support both — carriers differ, and getting this wrong is the
classic AMR interop failure:

- **Bandwidth-efficient** (default when `octet-align` is absent or `0`): 4-bit CMR, then
  6-bit ToC entries (`F | FT[4] | Q`), then speech bits packed with no intermediate
  padding; only the whole payload is octet-aligned.
- **Octet-aligned** (`octet-align=1`): 1–2 octet header (CMR + 4 reserved bits, plus
  ILL/ILP when interleaving), 1-octet ToC entries (`F | FT[4] | Q | PP`), each speech
  frame individually zero-padded to an octet boundary.

Fields: **CMR** (4 bits, requested mode index, `15` = no request), **F** (1 = another
frame follows), **FT** (4 bits), **Q** (0 = frame severely damaged → SPEECH_BAD/SID_BAD).

Octet-aligned-only extras: **frame CRC** (8-bit over class A bits, polynomial
`1 + x² + x³ + x⁴ + x⁸`), **robust sorting**, **interleaving** (ILL = length − 1 in
frame-blocks, ILP = index). `crc=1` and `robust-sorting=1` each imply `octet-align=1`.

Media types: `audio/AMR` @ **8000 Hz** clock (160 samples/frame-block);
`audio/AMR-WB` @ **16000 Hz** clock (320 samples/frame-block). Both use dynamic payload
types and must always be distinct PTs. Storage-format magic: `#!AMR\n`, `#!AMR-WB\n`
(and `#!AMR_MC1.0\n` / `#!AMR-WB_MC1.0\n` multi-channel).

SDP parameters: `octet-align` (0/1, dflt 0), `mode-set` (subset of 0–7 / 0–8, dflt all),
`mode-change-period` (1|2, dflt 1), `mode-change-capability` (1|2, dflt 1),
`mode-change-neighbor` (0/1, dflt 0), `crc` (0/1, dflt 0), `robust-sorting` (0/1, dflt 0),
`interleaving`, `ptime`, `maxptime`.

---

## 5. Reference-implementation survey

| Project | Scope | License | Use for us |
|---|---|---|---|
| `opencore-amr` (SourceForge; AOSP-derived) | AMR-NB enc+dec, AMR-WB **dec only** | Apache-2.0 | **Primary oracle**; optional `amr-ffi` backend |
| `mstorsjo/vo-amrwbenc` | AMR-WB **enc** | Apache-2.0 | Completes the FFI backend; encoder oracle |
| 3GPP TS 26.073 / 26.173 | NB + WB, fixed point, normative | Unclear | Do not use |
| `pschatzmann/codec-amr` (the repo you found) | C++ wrapper over 3GPP C, NB+WB, enc+dec, Arduino-oriented | **Author says license unclear** | Do not use. Useful only as a pointer to the 3GPP sources |
| FFmpeg native `amrnbdec.c` / `amrwbdec.c` | NB+WB decoders written from spec | LGPL-2.1+ | Proof that a clean-room from-spec decoder is achievable at reasonable size. **Do not read while porting** |
| FFmpeg `rtpdec_amr.c` | RFC 4867 depacketizer | LGPL-2.1+ | Do not read |
| Wireshark `packet-amr.c` | RFC 4867 dissector | GPL-2.0 | **Use the built tool** to validate our packets on the wire; don't read the source |
| rtpengine (sipwise) | Production AMR/AMR-WB transcoding, CMR-interval logic, mode-set handling | GPL-3.0 | Read its **documentation** (`docs/transcoding.md`) for behavioural requirements — it documents real-world CMR and octet-align pitfalls. Don't read the source |
| `traud/asterisk-amr` | Asterisk transcoding module over opencore/vo-amrwbenc | Asterisk terms | Interop counterparty for Phase 8 |
| crates.io `amr` | Unrelated ("Applied Mind Radio"), GPL-3.0 | — | Not an audio codec. Name is taken on crates.io |

**Confirmed: there is no pure-Rust AMR-NB/AMR-WB codec in existence.** Searches of
crates.io, docs.rs, and GitHub found nothing. If we complete L3 this would be the first,
which is a meaningful piece of ecosystem value beyond rvoip itself.

Behavioural notes worth stealing from rtpengine's docs (requirements, not code):

- On receiving a CMR, switch encoder bitrate **unconditionally**, even upward past the
  locally preferred rate.
- Its defaults are AMR 6.70 and AMR-WB 14.25 — sensible defaults for us too.
- A `CMR-interval` timer that tracks which FTs actually arrived and requests at most the
  *next-highest* unused allowed mode, at most once per interval, avoids CMR thrash.
- Answer SDP must preserve the offered `octet-align` variant; failing to do so is a
  recurring, widely-reported interop bug. Add an explicit regression test for it.

---

## 6. Architecture decision

### 6.1 Option A — pure-Rust from-spec port (recommended end state)

Mirrors what this repo already did for G.729A (`crates/media/codec-core/src/codecs/g729/`,
~120 files of fixed-point Rust with per-module Q-format documentation). Pros: no C
toolchain, no `build.rs`, `no_std`-able later, cross-compiles everywhere, MIT-clean,
first-in-ecosystem. Cons: by far the largest effort; bit-exactness is unforgiving.

### 6.2 Option B — FFI to `opencore-amr` + `vo-amrwbenc`

Pros: working AMR-NB/WB in ~1–2 weeks; Apache-2.0 is allow-listed. Cons: introduces a C
build dependency and vendored C into a workspace that currently has almost none (the only
precedents are the optional `opus` crate and `env-libvpx-sys` in `rvoip-webrtc`);
complicates cross-compilation and `cargo deny`; Apache-2.0 attribution obligations on an
MIT project.

### 6.3 Option C — payload format only, no codec kernel

Pros: cheap, clean-room, immediately useful for SBC/relay/recording. Cons: no transcoding
to/from G.711 or Opus.

### 6.4 Recommendation

**C, then B (opt-in, off by default), then A.**

The decisive argument for keeping B in the plan is not shipping speed — it is testing.
A bit-exact ACELP port cannot realistically be developed against end-to-end conformance
vectors alone; you need per-stage comparison (autocorrelation → Levinson → LSP quant →
open-loop pitch → closed-loop pitch → codebook search → gain quant) against a known-good
implementation. Having an Apache-2.0 oracle behind a dev-only feature flag turns Phase 4–7
from "debug a 244-bit mismatch" into "diff stage 6". That is the difference between weeks
and months.

The FFI feature stays **off by default** so the shipped default build keeps its
zero-C-dependency posture.

---

## 7. Repo integration map

Every touchpoint below was located in the current tree; this is the concrete surface a PR
has to cover. For comparison, adding G.729 touched ~30 files across five crates.

### 7.1 `crates/media/codec-core` — the codec kernel

```
src/types.rs                      CodecType::{AmrNb, AmrWb}; AmrParameters; CodecConfig
                                  helpers; bitrate_range()/payload_type()/quality_score()/
                                  supported_sample_rates() arms
src/codecs/mod.rs                 CodecFactory::create / create_by_name /
                                  create_by_payload_type; supported_codecs();
                                  CodecCapabilities::get_all()
src/codecs/amr/                   NEW — see §7.5 for the module tree
Cargo.toml                        features: amr-nb, amr-wb, amr = [both],
                                  amr-ffi (opt-in), all-codecs += amr
```

**Blocking API gap.** The `AudioCodec` trait (`src/types.rs:13`) is
`encode(&[i16]) -> Vec<u8>` / `decode(&[u8]) -> Vec<i16>` with a single fixed
`frame_size()`. It has nowhere to express:

- the **mode** a decoded frame arrived in, or the mode to encode the next frame in;
- a **CMR** received from the peer;
- SID / NO_DATA / SPEECH_LOST frame types distinct from "0 bytes";
- the frame-quality (`Q`) bit driving PLC.

G.729 dodged this by encoding frame type in the output length (10 / 2 / 0 bytes). AMR
cannot: several modes share byte lengths across NB and WB, and mode selection is an
*input*, not just an output.

**Decision required:** add a codec-agnostic extension trait rather than widening
`AudioCodec` (which would break `G711Codec` / `G729Codec` / `OpusCodec`):

```rust
/// Codecs whose per-frame bitrate is negotiated and can change frame to frame.
pub trait VariableRateCodec: AudioCodec {
    fn set_mode(&mut self, mode: u8) -> Result<()>;
    fn current_mode(&self) -> u8;
    fn allowed_modes(&self) -> &[u8];
    fn encode_frame(&mut self, samples: &[i16]) -> Result<CodedFrame>;
    fn decode_frame(&mut self, frame: &CodedFrame) -> Result<Vec<i16>>;
}

pub struct CodedFrame {
    pub frame_type: FrameType,   // Speech(mode) | Sid | NoData | SpeechLost
    pub quality_ok: bool,        // maps to RFC 4867 Q bit
    pub bits: Vec<u8>,
}
```

Per the workspace convention on migrations, this is a *new* trait implemented alongside
the existing one — `AudioCodec` keeps working for fixed-rate codecs and AMR implements
both, with the `AudioCodec` impl using the currently-selected mode.

### 7.2 `crates/media/rtp-core` — RFC 4867 payload format

```
src/payload/amr.rs                NEW — AmrPayloadFormat implementing PayloadFormat,
                                  plus the typed packer/depacker
src/payload/mod.rs                module registration + re-exports
src/payload/registry.rs           dynamic PT registration for AMR / AMR-WB
```

The existing `PayloadFormat` trait (`src/payload/traits.rs`) is
`pack(&[u8], u32) -> Bytes` / `unpack(&[u8], u32) -> Bytes` — byte-slices in, byte-slices
out, with no way to carry CMR, ToC, or multiple frames per packet. Implement a **typed
API first** and adapt it to `PayloadFormat` for the trivial single-frame case:

```rust
pub struct AmrPacketizer { variant: AmrVariant, octet_aligned: bool, /* … */ }
pub struct AmrPacket { pub cmr: Option<u8>, pub frames: Vec<AmrToCFrame> }
```

Multi-frame-per-packet (ptime 40/60/80) and interleaving are the reason the typed API is
non-negotiable.

### 7.3 `crates/media/media-core` — pipeline plumbing

```
src/codec/audio/amr.rs            NEW — mirrors codec/audio/g729.rs (423 lines)
src/codec/audio/mod.rs            registration
src/codec/factory.rs              construction
src/codec/mapping.rs              CodecMapper: dynamic PT ↔ "AMR"/"AMR-WB". Note it
                                  currently special-cases Opus via OpusConfig; AMR needs
                                  the same treatment (AmrConfig: variant, octet_align,
                                  mode_set) because one codec name maps to several
                                  negotiated shapes
src/codec/transcoding.rs          AMR ↔ PCMU/PCMA/Opus paths (incl. NB↔WB resampling)
src/rtp_processing/codec/sdp.rs   fmtp emit/parse for AMR parameters
src/rtp_processing/codec/registry.rs
src/rtp_processing/payload/registry.rs
src/relay/controller/codec_fallback.rs, codec_runtime.rs
src/engine/media_engine.rs
src/types/mod.rs, src/api/types.rs
Cargo.toml                        amr-nb/amr-wb/amr features → rvoip-codec-core
```

### 7.4 `crates/sip` — SDP negotiation

```
crates/sip/sip-core/src/sdp/attributes/fmtp.rs     AMR fmtp parameter parsing
crates/sip/sip-core/src/sdp/attributes/rtpmap.rs   AMR/8000, AMR-WB/16000
crates/sip/rvoip-sip/src/adapters/media_adapter.rs offer/answer, PT selection
crates/sip/rvoip-sip/Cargo.toml                    amr features → rvoip-media-core
crates/rvoip/Cargo.toml                            facade features → rvoip-sip
```

**Concrete blocker found at `media_adapter.rs:379`:**

```rust
96..=127 => cfg!(feature = "opus") && mapping_matches("opus", 48_000, &[1, 2]),
```

The entire dynamic payload-type range is currently hardcoded to Opus. `rtpmap_for_pt`
(:254), `fmtp_for_pt_with_g729_annex_b` (:274), `payload_codec_available` (:345) and
`codec_name_for_payload` (:330) are likewise fixed `match` tables over known PTs. AMR
requires PT→codec to become a **negotiated map built from the offer's `a=rtpmap`**, not a
constant table. This refactor is a prerequisite for Phase 2 and should be sized as its own
work item — it also unblocks any future dynamic-PT codec.

AMR fmtp negotiation rules to implement (RFC 4867 §8.3):

- `mode-set` is **not** a preference list — it is a hard restriction. The answer's active
  set is the intersection; if empty, the payload type must be rejected.
- Absent `mode-set` means all modes allowed.
- `octet-align` must match on both sides; the answer must not silently flip it.
- If offering both framings, offer them as **two distinct payload types**.
- `mode-change-period=2` and `mode-change-neighbor=1` constrain the *encoder's* mode
  trajectory — the encoder needs a mode-change policy object, not just a current mode.

### 7.5 Proposed `codecs/amr/` module tree

Follows the existing `codecs/g729/impls/` decomposition (small files, one concern each,
Q-format documented in the module header).

```
src/codecs/amr/
├── mod.rs                    AudioCodec / AudioCodecExt / VariableRateCodec adapter
├── common/                   shared by NB and WB
│   ├── basicop.rs            ETSI basic operators: add, sub, mult, L_mac, L_msu, shr,
│   │                         shl, norm_l, norm_s, div_s, round, saturate  (Q15/Q31)
│   ├── bits.rs               bit pack/unpack, class A/B/C reordering
│   └── dsp/                  autocorrelation, lag windowing, Levinson-Durbin,
│                             A(z)↔LSP/ISP, interpolation, residual + synthesis filters,
│                             weighting filter, correlation helpers
├── nb/
│   ├── tables/               windows, LSP codebooks (MA-predicted split VQ + SMQ for
│   │                         MR122), gain codebooks, grids, bit-ordering tables
│   ├── preproc.rs            HP filter + downscaling
│   ├── lp/                   windowing → autocorr → Levinson → LSP → interpolation
│   ├── lsp_quant/            split VQ w/ MA prediction (MR475..MR102); SMQ (MR122)
│   ├── pitch/                open-loop (per-mode cadence), closed-loop (1/6 and 1/3
│   │                         fractional), lag encode/decode
│   ├── fixed_cb/             per-mode algebraic codebook search + decode
│   │                         (2/3/4/8/10-pulse variants)
│   ├── gain/                 per-mode gain VQ, MA log-energy gain prediction
│   ├── dtx/                  VAD (26.094 opt.1 & opt.2), DTX (26.093), SID/CNG (26.092)
│   ├── conceal.rs            error concealment (26.091)
│   ├── postfilter/           adaptive postfilter (formant, tilt, AGC), HP, upscale
│   ├── bitstream/            TS 26.101 frame structure; IF1 / IF2 / RTP orderings
│   └── codec/                encoder.rs, decoder.rs, mode-switch state machine
└── wb/
    ├── resample.rs           16 kHz ↔ 12.8 kHz decimation/interpolation, pre/de-emphasis
    ├── tables/               ISF codebooks (S-MSVQ), gain tables, bit ordering
    ├── lp/                   order-16 analysis, ISP/ISF conversion + interpolation
    ├── isf_quant/            split + multistage VQ (mode-dependent bit budgets)
    ├── pitch/                open-loop, closed-loop (1/4 and 1/2 resolution)
    ├── fixed_cb/             4 tracks × 16 positions; 1–6 pulses/track by mode
    ├── gain/                 pitch gain + fixed-codebook gain-correction VQ
    ├── highband.rs           6.4–7 kHz synthesis; transmitted HB gain in mode 8 only
    ├── dtx/, conceal.rs, postproc/, bitstream/, codec/   (as NB)
```

`common/basicop.rs` is worth building and testing **first and in isolation** — every
subsequent bit-exactness bug traces back to it, and the G.729 port already proves the
pattern works in this codebase.

---

## 8. Phased plan

Sizing assumes one engineer familiar with the existing G.729 code. Multiply generously if
not; DSP bit-exactness work is notoriously spiky.

### Phase 0 — Foundations (1 week)

- [ ] Acquire and archive all specs from §3.
- [ ] Legal actions **IP-1/IP-2/IP-3** opened; IP-2 answered before any vector is checked in.
- [ ] `CodecType::AmrNb` / `CodecType::AmrWb`, `AmrParameters`, feature flags wired
      end-to-end (`rvoip` → `rvoip-sip` → `media-core` → `codec-core`) with a stub codec
      that returns `feature_not_enabled`.
- [ ] ADR recorded for the `VariableRateCodec` trait (§7.1).

**Exit:** `cargo build --all-features` green; AMR appears in `supported_codecs()` behind
its feature; no behaviour yet.

### Phase 1 — RFC 4867 payload format (2–3 weeks) ← *first shippable value*

- [ ] `AmrPacketizer` / `AmrDepacketizer`: bandwidth-efficient **and** octet-aligned.
- [ ] CMR, ToC chains, F/FT/Q, multi-frame packets, NO_DATA / SPEECH_LOST.
- [ ] Frame CRC (poly `1 + x² + x³ + x⁴ + x⁸` over class A bits); robust sorting; interleaving.
- [ ] AMR file storage format (`#!AMR\n` / `#!AMR-WB\n`) — needed for test fixtures anyway.
- [ ] `PayloadFormat` adapter + dynamic PT registration in `rtp-core`.

**Tests:** exhaustive round-trip over every (variant × mode × framing × frames-per-packet)
combination; malformed-input rejection (reserved FT, truncated payload, F-bit runaway,
CMR out of range); byte-for-byte comparison against captured real-world AMR pcaps
dissected with Wireshark; `cargo fuzz` target in the existing `crates/media/fuzz`.

**Exit:** bit-exact packetization both ways, validated against third-party captures.

### Phase 2 — SDP negotiation + relay path (2–3 weeks) ← *AMR-WB calls work end-to-end*

- [ ] Refactor `media_adapter.rs` dynamic-PT handling off the hardcoded Opus arm (§7.4).
- [ ] fmtp parse/emit for all §4.3 parameters.
- [ ] Offer/answer: `mode-set` intersection, `octet-align` matching, dual-PT offers,
      reject-on-empty-intersection.
- [ ] Mode-change policy object honouring `mode-change-period` / `mode-change-neighbor`.
- [ ] CMR send/receive state machine with a `CMR-interval`-style damper.
- [ ] Pass-through/relay path: AMR in → AMR out with no codec kernel.

**Exit:** rvoip completes an AMR-WB call as a relaying B2BUA against Asterisk and against
Kamailio+rtpengine, in both framings, with a mode switch observed mid-call. **This is the
milestone that delivers HD voice for pass-through deployments.**

### Phase 3 — `amr-ffi` oracle backend (1–2 weeks, opt-in, never default)

- [ ] Vendor or `-sys`-wrap `opencore-amr` (NB enc+dec, WB dec) and `vo-amrwbenc` (WB enc).
- [ ] `build.rs` with `cc`, feature `amr-ffi`, off by default, excluded from default CI matrix.
- [ ] `THIRD_PARTY_NOTICES.md` + `deny.toml` verification.
- [ ] **Vector-generation harness**: drive the C implementations to emit per-stage
      intermediate dumps and full-frame bitstreams; check the *generated data* into
      `tests/vectors/` so the Rust test suite never needs the C library.

**Exit:** transcoding works under `--features amr-ffi`; golden vectors generated and
committed; `cargo deny check` green.

### Phase 4 — AMR-NB decoder, pure Rust (4–6 weeks)

Order: `basicop` → bit unpacking + frame structure → LSP dequant → adaptive codebook →
fixed codebook decode → gain dequant → synthesis filter → postfilter → PLC → CNG.

**Exit:** bit-exact against TS 26.074 decoder test sequences for all 8 modes + SID +
NO_DATA + the erroneous-frame sequences.

### Phase 5 — AMR-NB encoder, pure Rust (6–8 weeks)

Order: preproc → LP analysis → LSP quant → open-loop pitch → closed-loop pitch → fixed
codebook search (per-mode) → gain quant → bitstream → VAD/DTX/SID.

The per-mode algebraic codebook searches are the bulk of the work; MR122's 10-pulse search
is the hardest single item.

**Exit:** bit-exact against TS 26.074 encoder test sequences for all 8 modes, DTX on and off.

### Phase 6 — AMR-WB decoder, pure Rust (5–7 weeks)

Adds over Phase 4: order-16 ISP/ISF dequant (S-MSVQ), 12.8→16 kHz interpolation,
de-emphasis, high-band synthesis, mode-8 transmitted HB gain.

**Exit:** bit-exact against TS 26.174 decoder sequences, all 9 modes + SID.

### Phase 7 — AMR-WB encoder, pure Rust (7–9 weeks)

Adds over Phase 5: 16→12.8 kHz decimation, pre-emphasis, order-16 analysis, ISF S-MSVQ,
4-track/1–6-pulse codebook searches, WB VAD (26.194).

**Exit:** bit-exact against TS 26.174 encoder sequences, all 9 modes, DTX on and off.

### Phase 8 — Interop, performance, hardening (3–4 weeks)

- [ ] Interop matrix: Asterisk (`traud/asterisk-amr`), Kamailio + rtpengine, FreeSWITCH,
      a commercial SBC if available, and at least one real VoLTE/IMS trunk.
- [ ] Transcoding matrix: AMR-NB ↔ PCMU/PCMA, AMR-WB ↔ Opus, AMR-NB ↔ AMR-WB.
- [ ] Benchmarks + profiling per `crates/sip/rvoip-sip/docs/PROFILING.md`.
- [ ] Fuzzing of decoder and depacketizer.
- [ ] Docs, `CHANGELOG.md`, README codec table, release-gate evidence.

**Total: roughly 8–11 months to full bit-exact NB+WB encode/decode**, with useful value
landing at the end of Phase 2 (≈6 weeks) and full transcoding available under `amr-ffi`
at ≈8 weeks.

---

## 9. Test & validation strategy

This repo is deliberately careful about not overclaiming conformance — see the header of
`src/codecs/g711/tests/itu_validation_tests.rs`, which explicitly disclaims evidence from
files not in the tree. Keep that discipline: **a test named `*_conformance` must actually
run normative vectors, or be renamed.**

### 9.1 Layer 0 — basic operators

Every ETSI basic operator gets an exhaustive or boundary-exhaustive unit test:
saturation at `i16`/`i32` limits, rounding direction, `div_s` domain restrictions,
`norm_l`/`norm_s` on zero. `i16`-domain operators can be tested exhaustively over all
2³² input pairs where feasible, otherwise property-tested against a slow-but-obvious
`i64` model. **Do this before anything else** — it is cheap and eliminates a whole class
of downstream bugs.

### 9.2 Layer 1 — per-stage golden vectors (the workhorse)

From Phase 3, dump inputs/outputs of each DSP stage from the Apache-2.0 oracle for a
corpus of speech, and check the dumps in as compact binary fixtures. Each Rust module gets
a test that replays its stage. This is what makes bit-exactness tractable: a failure
localises to one stage instead of one frame.

Keep fixtures small (a few hundred KB total) — sample a diverse subset of frames
(voiced / unvoiced / onset / silence / DTMF / music / clipping) rather than whole files.

### 9.3 Layer 2 — 3GPP conformance sequences (the proof)

TS 26.074 (NB) and TS 26.174 (WB) are the normative vectors and the only acceptable
evidence for a bit-exactness claim. Pending **IP-2**, use the opt-in external-fixture
pattern:

```
AMR_TEST_VECTORS=/path/to/26074 cargo test -p rvoip-codec-core --all-features -- --ignored
```

Tests `#[ignore]` by default and skip loudly with a message when the env var is unset, so
a green local run is never mistaken for conformance evidence. Wire the vectors into the
release gate as a required job once licensing is resolved.

### 9.4 Layer 3 — payload format

Round-trip over the full cross-product, malformed-input rejection, third-party pcap
comparison via Wireshark, and a `cargo fuzz` target. Explicit regression test for the
"answer must preserve the offered `octet-align` variant" bug documented by rtpengine.

### 9.5 Layer 4 — SDP / negotiation

Table-driven offer/answer cases: `mode-set` intersection (including empty → reject),
missing `mode-set` (= all), `octet-align` mismatch, dual-PT offers, `crc`/`robust-sorting`
implying octet-align, `mode-change-period=2` and `mode-change-neighbor=1` constraining the
encoder trajectory.

### 9.6 Layer 5 — end-to-end and interop

Full SIP calls through `rvoip-sip` with AMR-WB, both framings, mid-call mode switching,
packet loss with PLC, DTX gaps, and re-INVITE renegotiation. Interop matrix per Phase 8.

### 9.7 Layer 6 — quality, performance, robustness

- Objective quality (segmental SNR now; PESQ/POLQA if a licensed tool is available) —
  a *sanity* signal, never a substitute for bit-exactness.
- Criterion benchmarks per mode; target real-time factor well under 0.05 per channel per
  core so a single core carries ≥ 20 concurrent AMR-WB legs.
- Fuzzing of decoder and depacketizer; the decoder must never panic on arbitrary input.
- Long-run soak: 24 h continuous encode/decode checking for state drift and leaks.

### 9.8 CI

Per the workspace convention, **all codec validation runs with `--all-features`** —
default `cargo test` silently skips feature-gated targets and gives a false green.

```bash
cargo test -p rvoip-codec-core --all-features
```

Add AMR jobs to `.github/workflows/pr-gate.yml` and `main-ci.yml`; keep `amr-ffi` out of
the default matrix and exercise it in `nightly-interop.yml`.

---

## 10. Risks

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R1 | Patent expiry relied on secondary sources | Legal | **IP-1** before Phase 3; Phases 0–2 are RFC-only and unaffected |
| R2 | 3GPP reference C is not redistributable; accidental contamination | Legal | Clean-room rule (§2.2); oracle used as a black box only; no LGPL/GPL source read during the port |
| R3 | 3GPP test sequences can't be vendored | Weakens conformance claim | **IP-2**; opt-in external fixtures (§9.3) meanwhile |
| R4 | Bit-exactness proves harder than G.729A — 17 modes, two codecs | Schedule | Per-stage oracle vectors (§9.2); NB before WB; decoder before encoder |
| R5 | `AudioCodec` trait can't express variable rate | Blocks everything downstream | `VariableRateCodec` extension trait decided in Phase 0 (§7.1) |
| R6 | Dynamic-PT handling hardcoded to Opus (`media_adapter.rs:379`) | Blocks Phase 2 | Size the refactor as its own item; it unblocks future dynamic-PT codecs too |
| R7 | Bandwidth-efficient vs octet-aligned interop bugs | Field failures | Both framings from Phase 1; explicit regression tests; Wireshark validation |
| R8 | `amr-ffi` C dependency leaks into default builds | Portability / license | Off by default; excluded from default CI matrix; `cargo deny` gate |
| R9 | CMR thrash causing audible mode oscillation | Quality | `CMR-interval` damper (§5); soak test with an adversarial peer |
| R10 | Effort underestimated; project stalls mid-port | Sunk cost | Every phase is independently shippable; Phase 2 alone justifies the branch |

---

## 11. Open questions

1. **Strategy** — confirm C→B→A (§6.4), or go straight to a pure-Rust port and skip the
   FFI oracle entirely?
2. **Scope** — is pass-through/relay (Phase 2) sufficient for the near-term goal, or is
   transcoding to G.711/Opus required from day one? This decides whether Phase 3 is
   optional.
3. **Priority** — AMR-WB is the HD-voice codec; is AMR-NB needed at all, or only as a
   fallback? Doing WB first is possible but NB is the better learning vehicle and shares
   most of `common/`.
4. **IP-2** — can the 3GPP test sequences be vendored? This changes the CI story materially.
5. **Crate placement** — keep AMR inside `rvoip-codec-core` alongside G.711/G.729/Opus, or
   split it into a standalone publishable `amr-codec` crate? A standalone pure-Rust AMR
   crate would be the first in the Rust ecosystem and has value beyond rvoip; note the
   name `amr` is already taken on crates.io by an unrelated GPL-3.0 project.

---

## 12. Sources

- [RFC 4867 — RTP Payload Format and File Storage Format for AMR and AMR-WB](https://www.rfc-editor.org/rfc/rfc4867.txt)
- [3GPP TS 26.071 — AMR Speech Codec; General description](https://www.arib.or.jp/english/html/overview/doc/STD-T63v9_60/5_Appendix/Rel4/26/26071-400.pdf)
- [TS 26.171 — AMR-WB Speech Codec: General Description (tech-invite index)](https://www.tech-invite.com/3m26/tinv-3gpp-26-171.html)
- [3GPP TS 26.190 — AMR-WB Transcoding functions](https://tec.gov.in/pdf/3gpp/TSDSI_Doc_1657/rel18/TS-26.190%20V1.0.0.pdf)
- [Wikipedia — Adaptive Multi-Rate audio codec](https://en.wikipedia.org/wiki/Adaptive_Multi-Rate_audio_codec)
- [Wikipedia — Adaptive Multi-Rate Wideband](https://en.wikipedia.org/wiki/Adaptive_Multi-Rate_Wideband)
- [Patent expiry dates and software for AMR, AMR-WB, and AMR-WB+ (HydrogenAudio)](https://hydrogenaudio.org/index.php/topic,114506.0.html)
- [VoiceAge — AMR-WB essential patent portfolio](https://voiceage.com/Patent-Portfolio-Essential.html)
- [VoiceAge AMR-WB/G.722.2 patent pool launch (2010)](https://www.prnewswire.com/news-releases/voiceage-announces-the-launch-of-the-amr-wbg7222-speech-compression-standards-patent-pool-83263272.html)
- [opencore-amr (SourceForge)](https://sourceforge.net/projects/opencore-amr/)
- [mstorsjo/vo-amrwbenc — VisualOn AMR-WB encoder](https://github.com/mstorsjo/vo-amrwbenc)
- [pschatzmann/codec-amr](https://github.com/pschatzmann/codec-amr)
- [traud/asterisk-amr](https://github.com/traud/asterisk-amr)
- [rtpengine transcoding documentation](https://github.com/sipwise/rtpengine/blob/master/docs/transcoding.md)
- [rtpengine issue #784 — AMR/AMR-WB bandwidth-efficient by default](https://github.com/sipwise/rtpengine/issues/784)
- [FFmpeg general documentation — AMR support and licensing](https://ffmpeg.org/general.html)
- [Wireshark packet-amr.c dissector](https://github.com/wireshark/wireshark/blob/master/epan/dissectors/packet-amr.c)
- [Library of Congress — AMR-WB format description](https://www.loc.gov/preservation/digital/formats/fdd/fdd000255.shtml)
