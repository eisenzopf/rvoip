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
| 1 | RFC 4867 payload format + AMR file storage format | 🟡 **Core done** — three optional extensions deferred |
| 2 | SDP negotiation + relay path | ⚪ Not started |
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

**Verification:** 62 AMR tests, 215 codec-core tests overall. Notable coverage:

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

### Deferred — the three optional octet-aligned extensions

Frame CRC, robust sorting, and interleaving are **not implemented**. Configuring
any of them is rejected at construction rather than silently ignored, because
misparsing a payload that uses them yields plausible garbage rather than an
obvious failure. All three are optional parameters a receiver may legitimately
decline to negotiate, so this is a valid interoperable subset — but it is a
narrowing of the Phase 1 scope in the plan, recorded here rather than glossed.

Frame CRC has an additional blocker for wideband: RFC 4867 defers the AMR-WB
class A bit counts to TS 26.201 instead of tabulating them, so
`AmrMode::class_a_bits()` returns `None` for WB. NB CRC is implementable today.

### Remaining for Phase 1

- [ ] `PayloadFormat` adapter and dynamic PT registration — blocked on **Q1**
      below (where the packetizer lives relative to `rtp-core`).
- [ ] `cargo fuzz` target in `crates/media/fuzz`. The in-module sweep over short
      arbitrary inputs is a stand-in, not a substitute.
- [ ] Byte-for-byte comparison against captured real-world AMR pcaps via the
      Wireshark dissector. Needs sample captures.
- [ ] The three deferred extensions above, if we decide to negotiate them.

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

**Resolved.** The packetizer lives in `codec-core` beside the codec, at
`src/codecs/amr/payload.rs`. RFC 4867 framing is codec framing rather than
transport, and it needs the mode tables intimately. This diverges from the
existing convention that payload formats live in `rtp-core` (`g711.rs`,
`opus.rs`, `vp8.rs`), so the `PayloadFormat` adapter in `rtp-core` will need
either a dependency edge or a thin re-declaration — still to be settled when
that adapter is written.

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

**Q3 — AMR-WB class A bit counts.** `AmrMode::class_a_bits()` returns `None` for
wideband: RFC 4867 defers the table to TS 26.201 rather than reproducing it. The
octet-aligned payload CRC option is blocked on adding it. Not needed until CRC
support in Phase 1, and only then if we choose to implement CRC at all.

---

## Changelog

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
