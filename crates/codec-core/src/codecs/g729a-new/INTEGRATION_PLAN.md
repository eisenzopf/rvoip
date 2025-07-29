# G.729A Integration Plan

## Overview
This document outlines the tasks needed to complete the G.729A encoder implementation so that the integration test can successfully encode audio and compare outputs with the C reference implementation.

## Current Status (Updated 2025-07-29)

### ✅ Completed Components
- **Pre-processing**: High-pass filter with scaling
- **LPC Analysis**: Autocorrelation, lag windowing, Levinson-Durbin
- **LSP Conversion**: LP to LSP conversion (`az_lsp`)
- **LSP Quantization**: Vector quantization with codebook search (using lspvq module)
- **LSP to LP Conversion**: `lsp_az` and `int_qlpc` functions in common/lsp_az.rs
- **Pitch Analysis**: Open-loop and closed-loop pitch search
- **Fixed Codebook**: ACELP algebraic codebook search with sign extraction
- **Gain Quantization**: Complete implementation with codebook search, integrated with correlation computation
- **Perceptual Weighting**: Full implementation with `weight_az` and filtering using `residu`/`syn_filt`
- **Target Signal**: Complete implementation using `target_signal` function
- **Impulse Response**: `compute_impulse_response` in common/impulse_response.rs
- **High-level Encoder**: G729AEncoder struct created with full encoding pipeline
- **Bit Packing**: `prm2bits` and `bits2prm` with proper G.729A format (0x7f/0x81)
- **Correlation Computation**: Implemented for gain quantization (g_coeff, exp_coeff)
- **Integration Test**: Fixed paths and successfully running encoder on test vectors

### 🚀 Recent Accomplishments (2025-07-29)
1. **Perceptual Weighting Filter**: 
   - Implemented `weight_speech` using existing `weight_az` function
   - Added `residu` function for inverse filtering
   - Properly computes W(z) = A(z/γ1)/A(z/γ2) and applies filtering

2. **Target Signal Computation**:
   - Fixed `compute` method to use existing `target_signal` function
   - Properly computes residual and applies perceptual weighting

3. **Gain Quantizer Integration**:
   - Added `compute_gain_correlations` function
   - Integrated existing `quantize_gain` with proper inputs
   - Fixed division by zero issue

4. **Fixed Codebook Integration**:
   - Properly extracts position (13 bits) and sign (4 bits) from ACELP search
   - Calls `acelp_code_a` directly with all parameters

5. **Integration Test**:
   - Fixed test vector paths
   - Encoder successfully processes SPEECH.IN (3750 frames)
   - Produces properly formatted bitstream with sync word and bit format

### ✅ Recently Completed (2025-07-29 - Session 2)
1. **Fixed Division by Zero in Gain Quantizer**:
   - Added proper excitation buffer initialization
   - Implemented safety checks for zero energy
   
2. **Integrated pred_lt_3 for Adaptive Excitation**:
   - Properly integrated fractional delay interpolation
   - Fixed buffer management for pred_lt_3
   - Adaptive excitation now generates valid output
   
3. **Implemented Taming Flag Computation**:
   - Added pitch gain history tracking
   - Implemented compute_taming_flag() method
   - Updates gain history after each subframe

### ❌ Remaining Tasks
1. **Fix LSP Initialization**: Use proper initial values
2. **Improve Closed-loop Pitch Search**: Currently returns placeholder values
3. **Fine-tune Memory Management**: Ensure bit-exact buffer shifts
4. **Optimize Performance**: Remove unnecessary allocations

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

## Key Implementation Details

### Adaptive Excitation Generation
The adaptive excitation is now properly generated using the `pred_lt_3` function:
```rust
// Create working buffer for pred_lt_3
let mut exc_work = vec![0i16; L_SUBFR + t0 + 1 + L_INTERPOL];
// Copy relevant portion from old_exc
// Apply fractional delay interpolation
pred_lt_3(&mut exc_work[out_offset..], t0, t0_frac, L_SUBFR);
```

### Taming Flag Computation
Pitch gain taming is implemented to prevent instability:
```rust
fn compute_taming_flag(&self) -> Word16 {
    const GPCLIP: Word16 = 15565;  // 0.95 in Q14
    const GPCLIP2: Word16 = 14746; // 0.90 in Q14
    
    // Check past gain and recent history
    if self.past_gain_pit > GPCLIP { return 1; }
    // Check if 3+ recent gains were high
}
```

### Memory Management
- Excitation buffer: `old_exc[PIT_MAX + L_INTERPOL + L_FRAME]`
- Updated after each subframe with quantized excitation
- Shifted by L_FRAME samples between frames
- Synthesis memory maintained in `mem_syn`

## Implementation Order

1. **Phase 1**: Hook up existing components (Tasks 1, 2, 3)
   - These just need integration, no new algorithms
   - Will fix most test failures

2. **Phase 2**: Add missing computations (Task 4)
   - Correlation computation for gain quantization
   - Critical for proper gain values

3. **Phase 3**: Complete state management (Tasks 5, 6)
   - Memory updates for continuity
   - Sign extraction for bit-exact output

## Current Test Results

### ✅ Working
- Encoder builds and runs without crashes
- Processes SPEECH.IN test vector (3750 frames)
- Produces valid bitstream format:
  - Sync word: 0x6b21 ✓
  - Size word: 80 ✓
  - Bit format: 0x7f (0), 0x81 (1) ✓
- All major components integrated and functioning

### ⚠️ Known Issues
- LSP quantization returns mostly zeros (needs proper initialization)
- Closed-loop pitch search returns placeholder values
- Test outputs differ from C reference (bit-exactness not achieved yet)
- Some Q-format alignment issues between components

### 📊 Test Output Example
```
Input samples (first 10): [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
Parameters: [26, 772, 37, 0, 0, 15, 6, 0, 0, 0, 7]
Bits: [0x6b21, 0x0050, 0x007f, 0x007f, ...]
```

## Success Criteria

The integration test should:
1. ✅ Build without errors
2. ✅ Process all test vectors without crashing
3. ⚠️ Produce bit-exact output matching C reference (in progress)
4. ⚠️ Pass all ITU-T test vectors (pending full implementation)

## Notes

- All arithmetic must be bit-exact with C reference
- Use existing basic operators for all math
- Maintain Q-formats as per G.729A specification
- Follow the exact algorithm sequence from the standard
- Current implementation successfully encodes but needs memory management for bit-exact output 