# ADR 001 — `VariableRateCodec`: representing per-frame mode and frame type

Status: **Accepted**, implemented in `src/types.rs`
Date: 2026-08-08
Context: AMR implementation, Phase 0 (see `AMR_IMPLEMENTATION_PLAN.md` §7.1)

## Context

`AudioCodec` is the crate's codec trait:

```rust
fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>>;
fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>>;
fn frame_size(&self) -> usize;
```

It assumes one fixed frame size and one implied bit rate. That holds for G.711,
Opus as used here, and G.729 — but not for AMR, which needs four things the trait
cannot express:

1. **Mode as an encoder input.** AMR has 8 (NB) or 9 (WB) speech modes. The mode
   is chosen per frame, and a peer can demand a change mid-call via a Codec Mode
   Request. `encode()` has nowhere to take it.
2. **Mode as a decoder output.** The receiver must know which mode arrived to
   depacketize correctly and to drive rate adaptation.
3. **Frame type distinct from length.** G.729 encodes frame type in output
   length: 10 bytes speech, 2 SID, 0 untransmitted. AMR cannot. Frame sizes
   collide across modes and variants, and — decisively — an empty payload is
   ambiguous between `NO_DATA` (sender deliberately silent during DTX, so
   *continue comfort noise*) and `SPEECH_LOST` (frame lost, so *run
   concealment*). Those drive opposite decoder behaviour.
4. **Frame quality.** RFC 4867's Q bit marks a frame as severely damaged. The
   decoder must treat it as degraded rather than trusting its bits.

## Decision

Add a **new trait alongside `AudioCodec`**, not an extension of it, plus two
codec-agnostic support types.

```rust
pub enum FrameKind { Speech, ComfortNoise, NoData, Lost }

pub struct CodedFrame {
    pub kind: FrameKind,
    pub mode: u8,        // meaningful when kind == Speech
    pub quality_ok: bool,
    pub data: Vec<u8>,
}

pub trait VariableRateCodec: AudioCodec {
    fn allowed_modes(&self) -> Vec<u8>;
    fn current_mode(&self) -> u8;
    fn set_mode(&mut self, mode: u8) -> Result<()>;
    fn apply_mode_request(&mut self, mode: Option<u8>) -> Result<()>;  // provided
    fn encode_frame(&mut self, samples: &[i16]) -> Result<CodedFrame>;
    fn decode_frame(&mut self, frame: &CodedFrame) -> Result<Vec<i16>>;
}
```

Implementors provide both traits. The `AudioCodec` methods operate at the
currently selected mode, so existing generic code keeps working.

## Rationale

**Why not widen `AudioCodec`?** It would break `G711Codec`, `G729Codec` and
`OpusCodec`, all of which are correct as they are, to serve one codec family.
Fixed-rate codecs would gain methods with no meaningful implementation.

**Why a codec-agnostic `FrameKind` rather than AMR's own type?** `AmrFrameType`
(in `codecs::amr::mode`) is the faithful RFC 4867 representation — it carries FT
indices, knows SID is FT 8 for NB and 9 for WB, and rejects reserved values.
That detail belongs in the AMR module. But `CodecParameters` and the pipeline
types are not feature-gated, so they cannot reference it. `FrameKind` is the
pipeline's view; `AmrFrameType` converts to and from it.

**Why not an associated type (`type FrameType`)?** It would make
`Box<dyn VariableRateCodec>` require naming the associated type, which is
awkward at exactly the boundary — the media pipeline — where dynamic dispatch is
used. The four-variant enum covers every variable-rate speech codec we are
likely to add (AMR-WB+, EVS both fit), so the generality would not pay for
itself.

**Why is `apply_mode_request` a provided method that silently ignores
out-of-set modes?** A CMR arrives from the network, so an invalid value is a
peer's bug or an attack, not a local error. Failing the call would let a remote
party break a session by sending a bad CMR. `set_mode`, which is called by local
policy, *does* return an error — a local caller asking for an unnegotiated mode
is a real bug worth surfacing.

**Why does `mode_set` live in `AmrParameters` as a `u16` bitmask?** It started as
`Vec<u8>`, which forced `CodecConfig::with_parameters` to stop being a `const
fn`. The bitmask keeps the type `Copy`, makes duplicates unrepresentable, and
turns offer/answer intersection into a bitwise AND. Zero means "all modes",
matching an absent SDP `mode-set`.

## Consequences

- `CodecType` gains `AmrNb` and `AmrWb`. Every exhaustive `match` on `CodecType`
  in the workspace needs an arm; `utils::validation` was the only one in-crate.
- `AmrCodec` implements both traits today, with the `AudioCodec` methods
  returning `FeatureNotEnabled` until the DSP kernel lands. **Deliberate: a codec
  that returns silence or garbage is far harder to diagnose than one that
  refuses**, and the error names the phase that will supply it.
- The media-core adapter (`codec/audio/amr.rs`) will need to bridge `CodedFrame`
  to media-core's own `AudioCodec` trait. Deferred to the phase that has a
  working kernel — there is nothing to bridge yet.
- `reset()` deliberately preserves the selected mode. Mode is negotiated state,
  not stream state; a discontinuity must not silently renegotiate the bit rate.

## Alternatives rejected

| Alternative | Why not |
|---|---|
| Widen `AudioCodec` | Breaks three working codecs for one codec family |
| Encode frame type in output length, as G.729 does | Impossible: sizes collide, and empty is ambiguous between `NoData` and `Lost` |
| Associated `FrameType` | Poisons `Box<dyn …>` at the pipeline boundary for generality we do not need |
| Put `VariableRateCodec` in the AMR module | It is not AMR-specific; EVS and AMR-WB+ would want it |
| Side-channel the mode through `CodecConfig` and `reset()` | Mode changes per frame; reconstructing the codec per frame destroys its state |
