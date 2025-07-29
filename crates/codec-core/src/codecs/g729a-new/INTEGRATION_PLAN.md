# G.729A Integration Plan

## Overview
This document outlines the tasks needed to complete the G.729A encoder implementation so that the integration test can successfully encode audio and compare outputs with the C reference implementation.

## Current Status

### ✅ Completed Components
- **Pre-processing**: High-pass filter with scaling
- **LPC Analysis**: Autocorrelation, lag windowing, Levinson-Durbin
- **LSP Conversion**: LP to LSP conversion (`az_lsp`)
- **LSP Quantization**: Vector quantization with codebook search
- **Pitch Analysis**: Open-loop and closed-loop pitch search
- **Fixed Codebook**: ACELP algebraic codebook search
- **Gain Quantization**: Adaptive and fixed gain quantization
- **Perceptual Weighting**: Filter computation
- **Target Signal**: Target computation for codebook search

### ❌ Missing Components
1. High-level encoder orchestration
2. Speech buffer management (240-sample window)
3. Parameter bit packing/unpacking
4. Complete frame processing pipeline
5. Synthesis filter memory management

## Implementation Tasks

### Task 1: Fix Speech Buffer Management
**Priority**: Critical  
**Location**: Integration test (`rust_test.rs`)

The current issue is that `autocorrelation()` expects 240 samples but only receives 160.

**Requirements**:
- Maintain a 240-sample buffer: [80 old | 80 current | 80 lookahead]
- Update buffer correctly for each frame
- Pass the full 240 samples to LPC analysis

**Implementation**:
```rust
// In G729AEncoder struct:
speech_buffer: [Word16; 240],  // [old_speech | current | lookahead]
```

### Task 2: Create Complete Encoder Pipeline
**Priority**: Critical  
**Location**: New file `src/encoder/g729a_encoder.rs`

Create a proper `G729AEncoder` struct that orchestrates all encoding steps.

**Structure**:
```rust
pub struct G729AEncoder {
    // Modules
    pre_proc: PreProc,
    lpc: Lpc,
    lsp_quantizer: LspQuantizer,
    gain_quantizer: GainQuantizer,
    
    // State buffers
    speech_buffer: [Word16; 240],
    old_speech: [Word16; L_FRAME],
    old_wsp: [Word16; L_FRAME + PIT_MAX],
    old_exc: [Word16; L_FRAME + PIT_MAX + L_INTERPOL],
    mem_w0: [Word16; M],
    mem_w: [Word16; M],
    mem_zero: [Word16; M],
    
    // LSP state
    old_lsp: [Word16; M],
    old_lsp_q: [Word16; M],
}
```

**Methods**:
- `new()` - Initialize all components
- `encode_frame(&mut self, speech: &[Word16]) -> [Word16; PRM_SIZE]`
- Private helper methods for each encoding step

### Task 3: Implement Parameter Bit Packing
**Priority**: High  
**Location**: New file `src/common/bits.rs`

Implement G.729A-compliant bit packing/unpacking.

**Functions needed**:
```rust
pub fn prm2bits(prm: &[Word16; PRM_SIZE]) -> [Word16; SERIAL_SIZE]
pub fn bits2prm(serial: &[Word16; SERIAL_SIZE]) -> [Word16; PRM_SIZE]
```

**Bit allocation** (per G.729A spec):
- LSP indices: 18 bits (L0=10, L1=8)
- Subframe 1: P1=8, S1=1, C1=13, GA1=3, GB1=4
- Subframe 2: P2=5, S2=1, C2=13, GA2=3, GB2=4
- Total: 80 bits + 2 sync = 82 bits

### Task 4: Implement Synthesis Filter Chain
**Priority**: Medium  
**Location**: `src/encoder/synthesis.rs`

Implement the synthesis filter operations needed for target computation.

**Functions needed**:
- `compute_impulse_response()` - H(z) = W(z)/A(z)
- `update_filter_memories()` - Maintain state between subframes
- `residual_signal()` - Compute residual for target

### Task 5: Complete Frame Processing Pipeline
**Priority**: High  
**Location**: Update `encode_frame()` in `g729a_encoder.rs`

Implement the complete encoding sequence:

1. **Pre-processing**
   - Scale and filter input speech

2. **LPC Analysis** (once per frame)
   - Window speech with Hamming window
   - Compute autocorrelation
   - Apply lag window
   - Levinson-Durbin → A(z) coefficients
   - Convert A(z) → LSP
   - Quantize LSP → indices

3. **Perceptual Weighting**
   - Compute W(z) = A(z/γ1)/A(z/γ2)
   - Filter speech through W(z)

4. **Open-loop Pitch Analysis**
   - Find pitch lag estimate

5. **For each subframe**:
   - Interpolate LSP → A(z)
   - Compute impulse response h(n)
   - Compute target signal
   - Closed-loop pitch search → adaptive codebook
   - Fixed codebook search → innovation
   - Quantize gains → indices
   - Update filter memories

6. **Parameter Assembly**
   - Collect all indices into PRM array

### Task 6: Integration Test Updates
**Priority**: High  
**Location**: `tests/integration_test/rust_test.rs`

Update the integration test to use the new complete encoder:

1. Import the new `G729AEncoder` from the library
2. Remove the incomplete local implementation
3. Update buffer management
4. Use library's bit packing functions

## Testing Strategy

### Unit Tests
Each new component should have unit tests:
- Bit packing/unpacking roundtrip
- Synthesis filter operations
- Frame buffer management

### Integration Tests
1. Compare each encoding step output with C reference
2. Bitstream comparison with C encoder
3. Parameter value validation

### Test Vectors
Use ITU-T test vectors:
- SPEECH.IN/BIT - General speech
- ALGTHM.IN/BIT - Algorithm coverage
- LSP.IN/BIT - LSP quantization
- PITCH.IN/BIT - Pitch search
- FIXED.IN/BIT - Fixed codebook

## Implementation Order

1. **Phase 1**: Fix immediate issues (Tasks 1, 6)
   - Fix speech buffer in integration test
   - Get basic frame processing working

2. **Phase 2**: Core infrastructure (Tasks 2, 3)
   - Implement complete encoder struct
   - Add bit packing functions

3. **Phase 3**: Complete pipeline (Tasks 4, 5)
   - Synthesis filters
   - Full encoding pipeline

## Success Criteria

The integration test should:
1. ✅ Build without errors
2. ✅ Process all test vectors without crashing
3. ✅ Produce bit-exact output matching C reference
4. ✅ Pass all ITU-T test vectors

## Notes

- All arithmetic must be bit-exact with C reference
- Use existing basic operators for all math
- Maintain Q-formats as per G.729A specification
- Follow the exact algorithm sequence from the standard 