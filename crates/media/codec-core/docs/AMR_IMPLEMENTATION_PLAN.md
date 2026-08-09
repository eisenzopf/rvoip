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

**L1 + L2 alone let rvoip act as a signalling-aware SBC/B2BUA that *relays* AMR-WB
end-to-end without transcoding** — useful, and delivered at ~week 6 as a by-product. But
transcoding is a day-one requirement here (§1.2), so L3 is on the critical path and is
where essentially all of the cost lives. L1 also carries the AMR file-storage-format reader
that loads test vectors, which is why it is sequenced first.

### 1.2 Decided approach

Four decisions are now fixed for this branch:

- **Pure Rust in the shipped crate.** No FFI backend, not even an opt-in one. Everything
  that ships is Rust written from the specification text, with no C dependency, no
  `build.rs`, and no change to the workspace's cross-compilation or `cargo deny` posture.
- **Transcoding is required from day one.** Pass-through/relay alone is not sufficient;
  AMR must bridge to G.711 and Opus.
- **A two-tier development-time oracle generates per-stage reference vectors offline.**
  The **3GPP reference C** (TS 26.073 / 26.173) is the authority — in-house use for product
  design is expressly permitted, though it may not be redistributed, so it is fetched and
  never committed (§2.3). `opencore-amr` and `vo-amrwbenc` (Apache-2.0) supply
  redistribution-safe fixtures. Neither tier is linked into the shipped crate or appears in
  the workspace dependency graph. See §1.2.1.
- **AMR-WB is built first.** WB is the deliverable that matters (HD voice); NB follows.

  An earlier revision of this plan put NB first as a de-risking warm-up: simpler
  quantisers, no resampler, no high-band synthesis, so a bit-exactness bug costs days
  instead of weeks. **That argument is largely bought out by the oracle decision above.**
  It assumed hard, unaided debugging — but `opencore-amr` gives per-stage ground truth for
  the WB decoder, which is where most of the new machinery (order-16 ISP, S-MSVQ, 12.8 kHz
  resampling, high-band synthesis) first appears. WB-first does not cost much *total* time
  (the shared `common/` work happens either way, just under a harder first consumer); it
  front-loads the difficulty and delivers the goal roughly 2–3 months sooner.

  The residual exposure is the WB **encoder**: its oracle (`vo-amrwbenc`) is the one that
  may not be bit-exact, and WB-first moves it from last phase to mid-project. This raises
  the stakes on Phase 0 oracle qualification — see the caveat below.

Order of work: **L1 (payload format) → L2 (SDP) → `common/` DSP layer + oracle harness →
WB decoder → WB encoder → NB decoder → NB encoder → transcoding + interop.** L1 comes
first not for its own sake but because the AMR file-storage-format reader it contains is
what loads the 3GPP test sequences and the oracle dumps — it is test infrastructure for
everything after it.

> **Transcoding requires both an encoder and a decoder**, so AMR-WB ↔ Opus — the HD-voice
> goal — lands around month 4–5, with AMR-NB following at month 6–7. Phases 1–2 ship relay
> capability at ~week 6, but relay is not the goal here. See the timeline table in §8.

### 1.2.1 Oracle strategy

Bit-exact ACELP is tractable when you can diff each DSP stage against a known-good
implementation, and intractable when you can only compare final bitstreams. The oracle is
what buys that, turning "debug a 244-bit mismatch" into "diff stage 6".

**Two-tier oracle.** The 3GPP reference is the authority; the Apache-2.0 libraries are the
redistribution-safe convenience layer.

| Tier | Oracle | Covers | Provenance | Vendored? |
|---|---|---|---|---|
| **Primary — authority** | **TS 26.073** (NB fixed-point), **TS 26.173** (WB fixed-point) | NB enc+dec, WB enc+dec | **The normative definition of bit-exactness** — conformance is defined *as* agreement with it | **No — fetched, never committed** (§2.3) |
| Secondary — convenience | `opencore-amr` | NB enc+dec, WB dec | OpenCORE / AOSP, 3GPP-reference-derived | Yes, Apache-2.0 |
| Secondary — convenience | `vo-amrwbenc` | WB enc | VisualOn / Android Stagefright, **independent implementation** | Yes, Apache-2.0 |

Why keep both tiers rather than just the reference:

- The **reference settles correctness**. It is the thing the spec says an implementation
  must agree with, so a mismatch against it is dispositive — no judgement call, no
  "well, this implementation does it differently."
- The **Apache-2.0 libraries produce redistributable fixtures**. Their output can be
  committed without waiting on IP-2 question 2 (§2.3), so CI has trustworthy data from
  day one regardless of how the 3GPP-output question is answered.

Phase 0 qualification therefore becomes cheap and decisive: run `opencore-amr` and
`vo-amrwbenc` against the 3GPP reference over an arbitrary corpus (not just the published
sequences) and record which agree bit-exactly. Where they agree, commit their fixtures and
treat them as authoritative. Where they do not — most likely `vo-amrwbenc`, which is not
reference-derived — fall back to reference-generated dumps held locally, pending IP-2.

**Architecture — out-of-tree, vector-generating.** The oracle lives in a standalone Cargo
project *excluded from the workspace*, at `crates/media/codec-core/tools/amr-oracle/`. It
**vendors** the Apache-2.0 sources and **fetches** the 3GPP reference (a `make fetch` step
that downloads TS 26.073 / 26.173 and unpacks them into a git-ignored directory — nothing
3GPP-copyrighted is ever committed). `build.rs` compiles both with `cc` and applies
instrumentation patches that dump each stage's inputs and outputs. It runs offline, on a
developer machine, and writes compact fixtures into
`crates/media/codec-core/tests/vectors/amr/`. **The committed test suite reads only those
checked-in fixtures** — a normal `cargo test` needs no C toolchain and no 3GPP download,
and the shipped crate's dependency graph is unchanged.

This is why the pure-Rust decision and the oracle decision are not in tension: the oracle
is build-time-only tooling that produces data, not a runtime component.

**What this resolves.** The previous revision's largest open risk was that `vo-amrwbenc`
might not be bit-exact, leaving the AMR-WB encoder — the HD-voice milestone, and now
Phase 5 — without a trustworthy per-stage oracle. **TS 26.173 removes that risk entirely**:
the WB encoder now has a normative oracle by definition. Risk R8 is downgraded accordingly,
and the 2–3 week fallback budget on Phase 5 is no longer expected to be needed.

**Retained regardless of oracle quality** — cheap, independent of any implementation's
reading of the spec, and the fallback if oracle qualification disappoints:

1. **Homing frames (EHF/DHF)** — the spec's own testing mechanism. Special inband frames
   force every bit-exactly-defined function into a predefined home state, and consecutive
   homing frames must produce homing frames at the output. Gives self-synchronising
   checkpoints, so a conformance sequence can be entered mid-stream and a failure
   localised to a frame range instead of poisoning everything downstream.
2. **Brute-force reference searches** in test code — the ACELP codebook searches are fast
   heuristics approximating an optimum that can be computed exhaustively for a single
   subframe. The slow version is the oracle for the fast one. This remains necessary even
   with a good oracle, because it validates the *search* rather than one implementation's
   result.
3. **A reduced float reference model**, scoped to wherever oracle qualification comes back
   weak — expected to be the WB encoder's ISF quantisation and codebook search, and
   nothing else. The full float-model-first approach from the previous revision is
   dropped: with a bit-exact oracle it is redundant, and it cost 2–3 weeks.

### 1.3 The one thing to get right before writing code

**Patent expiry and source-code copyright are two different things, and only one of them
has expired.** See §2. The AMR patents appear to have lapsed; the copyright on the
reference implementations has not.

The useful distinction — and one an earlier revision of this plan got wrong by being too
cautious — is **verification versus distribution**. 3GPP expressly permits "in-house copies
… for product design purposes", so building and instrumenting the reference C (TS 26.073 /
26.173) to validate our implementation is allowed, and it is the best oracle available.
What is prohibited is distributing "verbatim copies of source code (or lightly modified
copies)". So:

- **Verify against the reference freely.** Fetch it, build it, instrument it — locally.
- **Distribute nothing derived from it.** It never enters the repository, and our Rust must
  be structured from the spec's block diagram rather than transliterated from the C, or it
  risks being a "lightly modified copy" of something we may not redistribute.

Full detail and the quoted terms are in §2.3; the consequences shape §1.2.1, §6 and §9.2.

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

**Action IP-1 (blocking for Phase 3+, i.e. before any codec-kernel work; not for Phases
0–2):** have counsel confirm expiry
of the AMR-WB/G.722.2 essential-patent list published at
`voiceage.com/Patent-Portfolio-Essential.html`. Phases 0–2 implement only IETF RFC 4867
framing, which is not covered by the speech-coding patents, so they can proceed in
parallel.

### 2.2 Copyright on reference source — the actual live risk

| Source | License | Can we copy code from it? |
|---|---|---|
| 3GPP TS 26.073 / 26.104 (NB C), TS 26.173 / 26.204 (WB C) | 3GPP copyright. **In-house use for product design is expressly permitted; redistribution is not** — see §2.3 | **Run and instrument locally: yes. Redistribute or transliterate: no** |
| `pschatzmann/codec-amr` | Wraps the above; author states license "unclear" — and it *redistributes* 3GPP source, which §2.3 indicates requires permission | **No** |
| FFmpeg `libavcodec/amrnbdec.c`, `amrwbdec.c` (native decoders) | LGPL-2.1-or-later | **No** — LGPL is not on this workspace's `deny.toml` allow-list, and porting from it creates derivative-work exposure |
| Wireshark `epan/dissectors/packet-amr.c` | GPL-2.0 | **No** |
| rtpengine (AMR transcoding + CMR logic) | GPL-3.0 | **No** |
| `opencore-amr` (AMR-NB enc+dec, AMR-WB dec; ex-AOSP/PacketVideo) | **Apache-2.0** | **Yes** — allow-listed in `deny.toml` |
| `mstorsjo/vo-amrwbenc` (AMR-WB encoder, ex-VisualOn/Android) | **Apache-2.0** | **Yes** |
| ITU-T G.722.2 recommendation text, 3GPP TS text | Spec text | **Yes, as the normative source to implement from** |

The workspace publishes under **MIT** (`Cargo.toml:113`) and `deny.toml` allows
MIT / Apache-2.0 / BSD / ISC / Zlib / 0BSD / BSL-1.0 / MPL-2.0 / CDLA-Permissive-2.0 /
CC0-1.0 — **no (L)GPL**. The copyleft implementations above (FFmpeg, Wireshark, rtpengine)
are therefore the genuinely off-limits ones: their code cannot enter the tree, and we should
avoid studying their source at all. Run their *binaries* freely — `ffmpeg` as a black-box
cross-check and Wireshark as a packet dissector are both fine and useful — but do not read
them.

### 2.3 The 3GPP reference code — usable as an oracle, not redistributable

An earlier revision of this plan treated the 3GPP reference C as entirely off-limits,
following the "license unclear" note on `pschatzmann/codec-amr`. **That was too
conservative.** 3GPP's published terms draw a sharp and useful line:

> "Other than for **in-house copies for the purpose of further development of the 3GPP
> standard or for product design purposes**, you may not reproduce any part of a 3GPP TS
> or TR without seeking permission from 3GPP."

> "You must not provide **verbatim copies of source code (or lightly modified copies)**
> without seeking permission from 3GPP."

So:

- **Downloading TS 26.073 / 26.173, building them, running them, and instrumenting them
  locally to validate our implementation is squarely "product design purposes" — expressly
  permitted.** Verifying an independent implementation is what the reference code is for.
- **Redistributing that source is not.** Nothing 3GPP-copyrighted enters the repository;
  the oracle harness **fetches rather than vendors** (§1.2.1).
- **A line-by-line Rust transliteration of the 3GPP C would plausibly be a "lightly
  modified copy"** and is therefore off the table. This is a sharper and more useful
  constraint than "unclear license": it restricts what we *distribute*, not how we
  *verify*.

This is also why `pschatzmann/codec-amr` stays unusable — not because its terms are unclear
in the abstract, but because it redistributes 3GPP source, which the second clause above
indicates requires permission it does not claim to hold.

**Working rule for the port:** implement from TS 26.090 / TS 26.190 (and the companion
specs in §3). Verify against the 3GPP reference, the conformance sequences, and the
spec-defined homing-frame outputs. Never read or copy the *copyleft* implementations
(FFmpeg, Wireshark, rtpengine) — those are the ones whose licenses genuinely conflict with
this workspace's MIT posture.

Structure the Rust from the **spec's block diagram**, not from any C implementation's file
layout or identifiers. That is the operative discipline now: the risk is not "we looked at
the reference" (permitted) but "what we ship reads as a lightly modified copy of it"
(prohibited). Only the second failure mode matters.

**Action IP-2 (do during Phase 0):** two related questions.

1. May the 3GPP **test sequences** (TS 26.074 / 26.174) be vendored into the repo? If not,
   use the opt-in external-fixture pattern in §9.3.
2. May **per-stage vectors generated by running** the reference code be committed? Program
   output is generally not a derivative work of the program, and 3GPP itself publishes
   reference-codec output as the conformance sequences — but confirm rather than assume.
   Until answered, prefer `opencore-amr`-derived fixtures for committed data (Apache-2.0,
   unambiguous) and keep 3GPP-derived dumps local.

**Action IP-3:** the oracle harness vendors Apache-2.0 C (`opencore-amr`,
`vo-amrwbenc`) under `tools/amr-oracle/`. Even though it is build-time-only tooling that
never ships in the crate, add `THIRD_PARTY_NOTICES.md` entries and retain the upstream
`LICENSE` / `NOTICE` files in the vendored tree — Apache-2.0 §4 attribution applies to
distribution of the source repository, not just to binaries. The 3GPP reference needs no
such entry precisely because it is never redistributed.

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
| **TS 26.073** | **AMR ANSI-C source (fixed point)** | **Primary oracle** — fetch, build, instrument locally; never redistribute (§2.3) |
| TS 26.104 | ANSI-C source (floating point) | Secondary reference; same terms |
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
| **TS 26.173** | **AMR-WB ANSI-C source (fixed point)** | **Primary oracle** — normative for WB encode; same terms (§2.3) |
| TS 26.204 | ANSI-C source (floating point) | Secondary reference; same terms |
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
| `opencore-amr` (SourceForge; AOSP-derived) | AMR-NB enc+dec, AMR-WB **dec only** | Apache-2.0 | **Development-time oracle** for NB encode/decode and WB decode (§1.2.1). Instrumented out-of-tree to dump per-stage vectors; never linked into the shipped crate |
| `mstorsjo/vo-amrwbenc` | AMR-WB **enc** | Apache-2.0 | **Development-time oracle** for WB encode. VisualOn-derived, so bit-exactness with TS 26.173 is **unverified** — qualify in Phase 0 |
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

**Decided: pure-Rust from-spec port in the shipped crate, with Apache-2.0 C
implementations used out-of-tree as a development-time oracle** (§1.2, §1.2.1). The
alternatives are recorded below so the decision is reviewable, not to reopen it.

### 6.1 Chosen — pure-Rust from-spec port, oracle-assisted

Mirrors what this repo already did for G.729A (`crates/media/codec-core/src/codecs/g729/`,
~120 files of fixed-point Rust with per-module Q-format documentation). No C toolchain in
the build, no `build.rs`, `no_std`-able later, cross-compiles everywhere, MIT-clean, and
the first such implementation in the Rust ecosystem.

The key structural insight is that **"pure Rust" is a property of the shipped artefact,
not of the development process.** An oracle that runs offline and emits data files gives
the full per-stage debugging benefit while leaving the dependency graph, the license
posture, and the cross-compilation story untouched. This is strictly better than either
extreme: shipping FFI (§6.2) or developing blind (§6.4).

### 6.2 Rejected — FFI to `opencore-amr` + `vo-amrwbenc` as a shipped backend

Would have given working AMR-NB/WB in ~1–2 weeks, and both are Apache-2.0 and
allow-listed by `deny.toml`. Rejected as a *runtime* component because it introduces a C
build dependency into a workspace that has almost none (only the optional `opus` crate and
`env-libvpx-sys` in `rvoip-webrtc`), complicates cross-compilation and `cargo deny`, and
adds Apache-2.0 attribution obligations to an MIT project. The same libraries are adopted
as build-time tooling instead (§6.1), and remain the recovery option if the port stalls
(risk R10).

### 6.3 Rejected — payload format only, no codec kernel

Cheap and clean-room, but transcoding is a day-one requirement (§1.2), so a kernel-free
build does not meet the goal. Phases 1–2 still deliver this capability as a by-product at
~week 6.

### 6.4 Rejected — float-model-first, no external oracle

The previous revision of this plan front-loaded a full `f64` reference model (~3–4 weeks,
no shippable artefact) to serve as the per-stage oracle. Superseded: a real bit-exact
oracle does the same job better and for less. A **reduced** float model survives, scoped
only to wherever Phase 0 oracle qualification comes back weak — expected to be the WB
encoder's ISF quantisation and codebook search, and nothing else.

### 6.5 Considered and not taken — direct port of `opencore-amr` C to Rust

Worth recording because it is legally available and much faster than a from-spec
implementation. Apache-2.0 permits derivative works, so transliterating the C into Rust
would be a lawful route to a pure-Rust codec, subject to retaining copyright notices,
stating changes, and adding a `THIRD_PARTY_NOTICES.md` entry.

Not taken because it would make parts of `codec-core` Apache-2.0-derived rather than MIT,
inherit C-shaped code structure into a codebase with an established idiom (see the G.729
module layout), and produce something harder to reason about than a spec-derived
implementation. **If schedule pressure becomes acute, this is a more attractive fallback
than the FFI backend** — it preserves the pure-Rust property and costs only license
attribution. Flagged for the Phase 4 checkpoint (risk R10).

Note the asymmetry with the 3GPP reference: the same transliteration applied to TS 26.073 /
26.173 is **not** available, because 3GPP's terms prohibit distributing "verbatim copies of
source code (or lightly modified copies)" (§2.3). The Apache-2.0 route is lawful precisely
because Apache-2.0 grants what 3GPP withholds.

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
tests/vectors/amr/                NEW — checked-in per-stage oracle fixtures (§1.2.1)
tools/amr-oracle/                 NEW — standalone Cargo project, EXCLUDED from the
                                  workspace. Vendors opencore-amr + vo-amrwbenc, builds
                                  them with cc, dumps per-stage vectors. Never a
                                  dependency of the shipped crate
Cargo.toml                        features: amr-nb, amr-wb, amr = [both],
                                  all-codecs += amr. No FFI feature (§6.2).
                                  [workspace] exclude = ["…/tools/amr-oracle"]
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
│   ├── homing.rs             EHF/DHF detection + home-state reset (TS 26.071/26.101)
│   └── dsp/                  autocorrelation, lag windowing, Levinson-Durbin,
│                             A(z)↔LSP/ISP, interpolation, residual + synthesis filters,
│                             weighting filter, correlation helpers
├── reference/                CONDITIONAL, TEST-ONLY float model — only if Phase 0 oracle
│                             qualification comes back weak, and scoped to the affected
│                             stages (expected: WB encoder ISF quant + codebook search).
│                             Never shipped. See §9.2d
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

`reference/` is the one structural departure from the G.729 layout, and it exists because
this port has no external oracle (§6.1, §9.2a). Gate it so it cannot reach a release
build.

---

## 8. Phased plan

Sizing assumes one engineer familiar with the existing G.729 code. Multiply generously if
not; DSP bit-exactness work is notoriously spiky.

### Phase 0 — Foundations (1 week)

- [ ] Acquire and archive all specs from §3.
- [ ] Legal actions **IP-1** and **IP-2** opened; IP-2 answered in this phase.
- [ ] **Oracle qualification.** Fetch and build the 3GPP reference (TS 26.073 / 26.173);
      build `opencore-amr` and `vo-amrwbenc`. Run all three over a shared corpus and record,
      for each of the four paths (NB enc, NB dec, WB enc, WB dec), whether the Apache-2.0
      library is bit-exact with the reference. Determines which fixtures can be committed
      directly and which need the external-fixture pattern (§1.2.1).
- [ ] `CodecType::AmrNb` / `CodecType::AmrWb`, `AmrParameters`, feature flags wired
      end-to-end (`rvoip` → `rvoip-sip` → `media-core` → `codec-core`) with a stub codec
      that returns `feature_not_enabled`.
- [ ] ADR recorded for the `VariableRateCodec` trait (§7.1).

**Exit:** `cargo build --all-features` green; AMR appears in `supported_codecs()` behind
its feature; oracle bit-exactness recorded per path; no codec behaviour yet.

### Phase 1 — RFC 4867 payload format + file storage format (2–3 weeks) ← *test infrastructure*

This comes first because the **AMR file storage format reader is what loads the 3GPP test
sequences and the oracle's bitstream dumps**. Everything from Phase 3 onward depends on it.
The wire-format work is a by-product that happens to also enable relay.


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

### Phase 2 — SDP negotiation + relay path (2–3 weeks) ← *can run in parallel with Phase 3+*

Independent of the codec kernel, so a second engineer can own this concurrently with the
DSP work. If staffed by one person, it can also be deferred until after Phase 5 without
blocking anything — but doing it now means every later phase has a real call path to test
against instead of a synthetic harness.


- [ ] Refactor `media_adapter.rs` dynamic-PT handling off the hardcoded Opus arm (§7.4).
- [ ] fmtp parse/emit for all §4.3 parameters.
- [ ] Offer/answer: `mode-set` intersection, `octet-align` matching, dual-PT offers,
      reject-on-empty-intersection.
- [ ] Mode-change policy object honouring `mode-change-period` / `mode-change-neighbor`.
- [ ] CMR send/receive state machine with a `CMR-interval`-style damper.
- [ ] Pass-through/relay path: AMR in → AMR out with no codec kernel.

**Exit:** rvoip completes an AMR-WB call as a relaying B2BUA against Asterisk and against
Kamailio+rtpengine, in both framings, with a mode switch observed mid-call.

### Phase 3 — `common/` layer + oracle harness (2–3 weeks)

Produces no shippable artefact. It is the foundation everything else is debugged against,
and skipping or rushing it is the single most likely cause of Phases 4–7 overrunning.
Because WB comes first, build `common/` order-16- and ISP-capable from the outset rather
than growing it from an order-10 LSP-only base.

- [ ] `common/basicop.rs` — the full ETSI basic-operator set, with the exhaustive tests in
      §9.1. **Nothing else starts until this is green.**
- [ ] `common/dsp/` — autocorrelation, lag windowing, Levinson-Durbin, A(z)↔LSP **and**
      A(z)↔ISP, interpolation, residual/synthesis/weighting filters, correlation helpers.
- [ ] `common/bits.rs` — bit pack/unpack, class A/B/C reordering.
- [ ] `common/homing.rs` — EHF/DHF per TS 26.071/26.101, plus the checkpoint/resynchronise
      test helper described in §1.2.1.
- [ ] **Oracle harness** at `tools/amr-oracle/` (excluded from the workspace): vendored
      `opencore-amr` + `vo-amrwbenc`; a `make fetch` step that downloads the 3GPP reference
      (TS 26.073 / 26.173) into a **git-ignored** directory — never committed (§2.3);
      `build.rs` with `cc`; instrumentation patches that dump per-stage inputs/outputs; and
      a generator that writes fixtures into `tests/vectors/amr/`.
- [ ] **Stage-diff harness** in the test suite: replays a checked-in fixture through one
      Rust stage and reports the first divergence by frame and stage.

**Exit:** basic operators exhaustively tested; oracle builds and emits per-stage fixtures
for at least one WB decode sequence; stage-diff harness demonstrated on a deliberately
injected bug; `cargo test` green with no C toolchain present.

### Phase 4 — AMR-WB decoder, fixed point (5–7 weeks)

Order: bit unpacking + frame structure (TS 26.201) → ISF dequant (S-MSVQ) → adaptive
codebook → fixed codebook decode → gain dequant → synthesis filter → de-emphasis →
12.8→16 kHz interpolation → high-band synthesis (incl. mode-8 transmitted HB gain) →
post-processing → PLC (26.191) → CNG (26.192).

The resampler and the high-band synthesis are the two places where "close enough" passes
casual listening and fails bit-exactness. Diff them against the oracle aggressively.

One stage at a time, green against the oracle before the next. Do not batch.

Carries the first-codec tax: the `common/` layer gets shaken out here rather than in a
simpler NB pass, which is the cost of WB-first (§1.2).

**Exit:** bit-exact against TS 26.174 decoder sequences, all 9 modes + SID + NO_DATA +
SPEECH_LOST + the erroneous-frame sequences.

### Phase 5 — AMR-WB encoder, fixed point (6–8 weeks) ← *the HD-voice milestone*

Order: preproc (HP filter, 16→12.8 kHz decimation, pre-emphasis) → order-16 LP analysis →
ISF quantisation (S-MSVQ) → open-loop pitch → closed-loop pitch (1/4 and 1/2 resolution) →
algebraic codebook search (4 tracks × 16 positions, 1–6 pulses/track by mode) → gain
quantisation → bitstream → VAD (26.194) / DTX (26.193) / SID.

The codebook searches are the bulk of the work. Validate each against a brute-force
exhaustive search over a single subframe (§9.2) before trusting it on a sequence.

Oracle: **TS 26.173 is normative for WB encode**, so this phase has an authoritative
per-stage reference. If Phase 0 found `vo-amrwbenc` not bit-exact, that only affects which
fixtures can be committed (use reference-generated dumps via the external-fixture pattern),
not whether the phase is debuggable.

**Exit:** bit-exact against TS 26.174 encoder sequences, all 9 modes, DTX on and off.
**AMR-WB ↔ Opus/G.711 transcoding — the HD-voice goal — is reachable here.**

### Phase 6 — AMR-NB decoder, fixed point (2–3 weeks)

Order: bit unpacking + frame structure (TS 26.101) → LSP dequant → adaptive codebook →
fixed codebook decode → gain dequant → synthesis filter → postfilter → PLC (26.091) →
CNG (26.092).

Substantially cheaper than the same phase would have been first: `common/`, the oracle
harness, the stage-diff workflow, and the bitstream machinery are all proven by now, and
NB is the simpler codec (order-10 LSP, no resampler, no high band).

**Exit:** bit-exact against TS 26.074 decoder sequences, all 8 modes + SID + NO_DATA + the
erroneous-frame sequences.

### Phase 7 — AMR-NB encoder, fixed point (3–5 weeks)

Order: preproc → LP analysis (two analyses per frame in MR122) → LSP quant (split VQ with
MA prediction for MR475–MR102; SMQ for MR122) → open-loop pitch → closed-loop pitch (1/6
and 1/3 fractional) → per-mode fixed codebook search → gain quant → bitstream →
VAD (26.094) / DTX (26.093) / SID.

Eight per-mode algebraic codebook searches are the bulk of the work; MR122's 10-pulse
search is the hardest single item. `opencore-amr` is a 3GPP-reference-derived oracle here,
so this phase should be the best-supported of the four.

**Exit:** bit-exact against TS 26.074 encoder sequences, all 8 modes, DTX on and off.
**AMR-NB ↔ G.711/Opus transcoding complete.**

### Phase 8 — Transcoding, interop, performance, hardening (3–4 weeks)

- [ ] Transcoding matrix wired through `media-core/src/codec/transcoding.rs`:
      AMR-NB ↔ PCMU/PCMA, AMR-WB ↔ Opus, AMR-NB ↔ AMR-WB (incl. 8↔16 kHz resampling).
- [ ] Interop matrix: Asterisk (`traud/asterisk-amr`), Kamailio + rtpengine, FreeSWITCH,
      a commercial SBC if available, and at least one real VoLTE/IMS trunk.
- [ ] Benchmarks + profiling per `crates/sip/rvoip-sip/docs/PROFILING.md`.
- [ ] Fuzzing of decoder and depacketizer.
- [ ] Docs, `CHANGELOG.md`, README codec table, release-gate evidence.

### Timeline summary

Single engineer familiar with the existing G.729 code, with Phase 2 parallelised onto a
second person (it is the only phase that parallelises cleanly — Phases 3–7 are one
dependency chain).

| Milestone | Phase | Earliest |
|---|---|---|
| AMR-WB relay / pass-through calls (no transcoding) | 1–2 | ~week 6 |
| `common/` + oracle harness + stage-diff workflow | 3 | ~week 8 |
| AMR-WB decode, bit-exact | 4 | ~month 3–4 |
| **AMR-WB ↔ Opus transcoding — the HD-voice goal** | **5** | **~month 4–5** |
| AMR-NB decode, bit-exact | 6 | ~month 5–6 |
| **AMR-NB ↔ G.711 transcoding** | 7 | ~month 6–7 |
| Interop, transcoding matrix, hardening | 8 | ~month 7–8 |

Add 2–3 weeks to Phase 5 if Phase 0 finds `vo-amrwbenc` is not bit-exact (§1.2.1) — the
largest single schedule uncertainty.

**Effect of the two revisions to this plan.** WB-first moves the HD-voice milestone from
~month 9–11 to ~month 4–5. Roughly half of that comes from reordering (WB no longer waits
behind NB) and roughly half from the oracle decision (per-stage debugging, and dropping
the 3–4 week float-model phase). Total time to *both* codecs is only modestly reduced —
~7–8 months against ~8–11 — because the work is largely the same; what changed is which
codec lands first and how fast each phase debugs.

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

### 9.2 Layer 1 — per-stage differential testing (the workhorse)

This is what makes bit-exactness tractable: a failure localises to one stage instead of one
frame. Three mechanisms, in descending order of how much load they carry.

**(a) Per-stage oracle vectors.** The out-of-tree harness (§1.2.1) runs the instrumented C
implementations over a speech corpus and dumps every stage's inputs and outputs. Those
dumps become compact fixtures; each Rust module gets a test that replays its stage against
them. Because every side is a fixed-point implementation of the same spec, comparison is
**exact equality**, not tolerance-based — which is precisely why this beats the float model
it replaced.

Weight by tier. A mismatch against the **3GPP reference** (TS 26.073 / 26.173) is
dispositive — conformance is *defined* as agreement with it, so there is no judgement call.
A mismatch against `opencore-amr` or `vo-amrwbenc` is dispositive only where Phase 0 found
that library bit-exact with the reference; otherwise it is a lead to investigate.

Fixture provenance follows IP-2 q2 (§2.3): prefer Apache-2.0-derived dumps for committed
data, and hold reference-derived dumps locally (external-fixture pattern, §9.3) until the
question is answered.

Keep fixtures small (a few hundred KB total) — sample a diverse subset of frames
(voiced / unvoiced / onset / silence / DTMF / music / clipping) rather than whole files.

**(b) Homing frames as checkpoints.** TS 26.071/26.101 define encoder and decoder homing
frames that drive every bit-exactly-specified function into a predefined home state, and
consecutive homing frames must produce homing frames at the output. Two uses:

- A standalone conformance test: feed homing frames, assert the exact specified output.
  Cheap, spec-mandated, catches whole classes of state bugs with no test vectors at all,
  and available from Phase 3 before any oracle fixtures are generated.
- Resynchronisation: injecting a homing frame mid-sequence resets state, so a failure at
  frame 900 can be isolated without replaying frames 1–899 and without one early bug
  poisoning every subsequent comparison.

**(c) Brute-force reference searches.** The ACELP codebook searches are fast heuristics
approximating a well-defined optimum. For a single subframe the optimum is computable
exhaustively. Test the fast search against the slow one over a corpus of subframes; the
same technique applies to the LSP/ISF VQ codebook searches. This remains necessary even
with a good oracle, because it validates the *search* rather than one implementation's
result. Useful independently of oracle quality, and cheap.

**(d) Reduced float model — now unlikely to be needed.** It existed as insurance against
having no trustworthy WB-encode oracle. TS 26.173 supplies one, so this drops to a
contingency: build it only if a specific stage resists diagnosis, scoped to that stage. The
previous revisions' float-model-first approach for the whole codec is dropped as redundant
(§6.4).

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

Add AMR jobs to `.github/workflows/pr-gate.yml` and `main-ci.yml`. These must run **with no
C toolchain assumptions** — they consume checked-in fixtures only, which is the property
that keeps the oracle out of the shipped build. Add a `nightly-interop.yml` job that
rebuilds the oracle, regenerates fixtures, and fails on any diff against what is committed;
that is what catches instrumentation drift (risk R11). Interop runs go there too.

---

## 10. Risks

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R1 | Patent expiry relied on secondary sources | Legal | **IP-1** before Phase 3; Phases 0–2 are RFC-only and unaffected |
| R2 | 3GPP reference C or copyleft source is accidentally redistributed | Legal | 3GPP reference is fetched into a git-ignored dir, never committed (§2.3); no copyleft source read at all; see also R13 on structural similarity |
| R3 | 3GPP test sequences can't be vendored | Weakens conformance claim | **IP-2**; opt-in external fixtures (§9.3) meanwhile |
| R4 | Bit-exactness proves harder than G.729A — 17 modes, two codecs | Schedule | Per-stage oracle vectors + homing frames + brute-force searches (§9.2); decoder before encoder within each codec; one stage green before the next |
| R5 | `AudioCodec` trait can't express variable rate | Blocks everything downstream | `VariableRateCodec` extension trait decided in Phase 0 (§7.1) |
| R6 | Dynamic-PT handling hardcoded to Opus (`media_adapter.rs:379`) | Blocks Phase 2 | Size the refactor as its own item; it unblocks future dynamic-PT codecs too |
| R7 | Bandwidth-efficient vs octet-aligned interop bugs | Field failures | Both framings from Phase 1; explicit regression tests; Wireshark validation |
| R8 | `vo-amrwbenc` is not bit-exact with TS 26.173, so its WB-encode stage dumps cannot be trusted | Low — **downgraded from the previous revision.** TS 26.173 is now the primary oracle and is normative by definition, so the WB encoder has an authoritative reference regardless. The only residual cost is that WB-encode fixtures may not be committable until IP-2 q2 is answered | Qualify all four paths against the 3GPP reference in Phase 0. Where a library disagrees, use reference-generated dumps locally and the external-fixture pattern for CI |
| R13 | Our implementation ends up structurally close enough to the reference C to read as a "lightly modified copy" | Legal — this is the constraint 3GPP's terms actually impose (§2.3) | Structure modules from the spec's block diagram, not the C's file layout; do not carry over identifiers; the existing G.729 module layout is the house idiom to follow. Review at each phase exit |
| R9 | CMR thrash causing audible mode oscillation | Quality | `CMR-interval` damper (§5); soak test with an adversarial peer |
| R10 | Effort underestimated; port stalls mid-way with no transcoding delivered | Sunk cost — transcoding is a day-one requirement and does not arrive until ~month 4–5 | Two recovery options, in preference order: (a) direct Rust port of the Apache-2.0 C (§6.5) — keeps the pure-Rust property, costs only license attribution; (b) the FFI backend (§6.2), ~1–2 weeks from zero. Decide at the Phase 4 exit checkpoint |
| R11 | Oracle instrumentation patches drift from upstream, or dumps encode a subtly different stage boundary than the Rust code | Wasted debugging on a phantom mismatch | Pin the vendored upstream revision; keep instrumentation patches minimal and reviewed; when a mismatch resists explanation, check the stage boundary before the arithmetic |
| R12 | WB-first means `common/` is shaken out under the harder codec, inflating Phase 4 | Schedule | Accepted cost of WB-first (§1.2), already priced into Phase 4's 5–7 weeks. Build `common/` order-16/ISP-capable from the start rather than retrofitting |

---

## 11. Decisions taken and questions still open

### Resolved

1. **Strategy** — ✅ **Pure Rust in the shipped crate.** No FFI backend, not even opt-in.
2. **Scope** — ✅ **Transcoding required from day one.** The codec kernel is on the
   critical path; relay is a by-product, not the goal.
3. **Oracle** — ✅ **Two-tier, out-of-tree, vector-generating.** The **3GPP reference C**
   (TS 26.073 / 26.173) is the authority: in-house use for product design is expressly
   permitted, so it is fetched and instrumented locally but never committed (§2.3).
   `opencore-amr` + `vo-amrwbenc` (Apache-2.0) supply redistribution-safe fixtures. The
   shipped dependency graph is unchanged. The float reference model is dropped as
   redundant.
4. **Ordering** — ✅ **WB first, NB second.** WB is the priority and the HD-voice
   deliverable. The earlier NB-first recommendation was premised on WB bugs being expensive
   to debug unaided; the oracle decision largely removes that premise, and WB-first moves
   the goal milestone from ~month 9–11 to ~month 4–5 (§1.2, §8).

### Still open

5. **IP-2** — two parts, both Phase 0 (§2.3): (a) may the 3GPP test sequences be vendored?
   (b) may per-stage vectors *generated by running* the reference code be committed?
   Neither blocks the work — the external-fixture pattern covers both — but the answers
   determine how much of the test suite runs unaided in CI.

6. **Crate placement** — keep AMR inside `rvoip-codec-core` alongside G.711/G.729/Opus, or
   split it into a standalone publishable crate? A standalone pure-Rust AMR crate would be
   the first in the Rust ecosystem and has value beyond rvoip; note the name `amr` is
   already taken on crates.io by an unrelated GPL-3.0 project.

7. **Staffing** — Phase 2 parallelises cleanly onto a second engineer and pulls the
   HD-voice milestone in by ~2–3 weeks. Phases 3–7 are one dependency chain and do not. Is
   a second person available?

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
- [3GPP FAQs — copyright and reproduction terms](https://www.3gpp.org/about-us/3gpp-faqs) ("in-house copies … for product design purposes"; "verbatim copies of source code (or lightly modified copies)")
- [3GPP Terms of Use](https://www.3gpp.org/terms-of-use)
- [3GPP Legal Matters](https://www.3gpp.org/about-us/legal-matters)
- [opencore-amr (SourceForge)](https://sourceforge.net/projects/opencore-amr/)
- [mstorsjo/vo-amrwbenc — VisualOn AMR-WB encoder](https://github.com/mstorsjo/vo-amrwbenc)
- [pschatzmann/codec-amr](https://github.com/pschatzmann/codec-amr)
- [traud/asterisk-amr](https://github.com/traud/asterisk-amr)
- [rtpengine transcoding documentation](https://github.com/sipwise/rtpengine/blob/master/docs/transcoding.md)
- [rtpengine issue #784 — AMR/AMR-WB bandwidth-efficient by default](https://github.com/sipwise/rtpengine/issues/784)
- [FFmpeg general documentation — AMR support and licensing](https://ffmpeg.org/general.html)
- [Wireshark packet-amr.c dissector](https://github.com/wireshark/wireshark/blob/master/epan/dissectors/packet-amr.c)
- [Library of Congress — AMR-WB format description](https://www.loc.gov/preservation/digital/formats/fdd/fdd000255.shtml)
- [Library of Congress — AMR format description](https://www.loc.gov/preservation/digital/formats/fdd/fdd000254.shtml)
- [ETSI TS 126 071 V5.0.0 — AMR general description (homing frames / EHF / DHF)](https://www.etsi.org/deliver/etsi_ts/126000_126099/126071/05.00.00_60/ts_126071v050000p.pdf)
