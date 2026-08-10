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
| 3 | `common/` DSP layer + oracle harness | 🟡 **In progress** — operators, oracle, fixed-point LP front end |
| 4 | AMR-WB decoder, fixed point | ⚪ Not started |
| 5 | **AMR-WB encoder — the HD-voice milestone** | ⚪ Not started |
| 6 | AMR-NB decoder, fixed point | ⚪ Not started |
| 7 | AMR-NB encoder, fixed point | ⚪ Not started |
| 8 | Transcoding, interop, performance, hardening | ⚪ Not started |

**There is no working AMR encoder or decoder yet.** `AmrCodec` constructs and
negotiates, but `encode`/`decode` return `FeatureNotEnabled` naming the phase
that will supply them.

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

The tier-1 3GPP reference *code* (TS 26.073 / 26.173) is blocked by the same
egress restriction, so tier-1 oracle qualification is also waiting on this.
- [ ] `common/dsp/` — autocorrelation, Levinson-Durbin, A(z)↔LSP/ISP,
      interpolation, residual/synthesis filters. Order-16 and ISP-capable from
      the outset, since WB comes first.
- [ ] `common/bits.rs` and `common/homing.rs` (EHF/DHF).

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
| ~~**IP-1** — AMR-WB/G.722.2 essential patents~~ | ~~Phase 3+~~ | ✅ **CLEARED 2026-08-09** — no blockers |
| **IP-2a** — may 3GPP test sequences (TS 26.074 / 26.174) be vendored? | Nothing hard; decides CI shape | ❗ unassigned |
| **IP-2b** — may vectors *generated by running* the reference be committed? | Nothing hard; decides fixture provenance | ❗ unassigned |
| **Oracle qualification** — build the five oracles, record which are bit-exact per path | Phase 3 | ❗ unassigned |
| **Spec acquisition** — TS 26.090, 26.190, 26.101, 26.201, 26.091–094, 26.191–194 | Phase 3 | ❗ unassigned |
| **Staffing** — is a second engineer available for Phase 2? | Pulls HD-voice milestone in ~2–3 weeks | ❗ unassigned |

**IP-1 is cleared, so Phase 3 — the codec kernel — is unblocked.** That was the
gate on all DSP work; everything shipped so far is protocol and plumbing.

Oracle qualification is now the highest-value outstanding item and the last
thing standing between here and starting Phase 3 properly. It is cheap, and it
settles whether Phase 5 needs its 2–3 week fallback budget (risk R8).

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
