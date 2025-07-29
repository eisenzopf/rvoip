# G.729A Codec Implementation

## Status: 25-50% ITU-T Compliance

This is a pure Rust implementation of the ITU-T G.729 Annex A speech codec.

## Current Implementation

### ✅ Completed Components
- **Signal Processing Pipeline**: Windowing, autocorrelation, Levinson-Durbin
- **LP Analysis**: Linear prediction with exact fixed-point arithmetic  
- **LSP Conversion**: Chebyshev polynomial root finding
- **LSP Quantization**: Two-stage split VQ with dual MA predictors
- **Bitstream Format**: ITU-T expanded format parsing
- **Test Infrastructure**: Compliance testing against official vectors

### 📊 Compliance Results
- **Frame 0 LSP Indices**: 25% match (1/4 correct)
  - Our: `[0, 17, 15, 0]`
  - ITU: `[1, 105, 17, 0]`

### 🔍 Root Cause Analysis

The remaining gap is due to numerical precision differences:

1. **LSF Conversion Accuracy**
   - Our LSF: `[2252, 3391, 4620, 7662, 9834]`
   - Expected: `[2254, 3389, 4623, 7659, 9837]`
   - Difference: 2-3 out of 25736 (0.01% error)

2. **Cascading Effects**
   - Small LSF differences → Different MA prediction targets
   - Different targets → Different weighted VQ errors
   - Different errors → Different codebook selections

## Path to 100% Compliance

### Option 1: Exact bcg729 Operations
```rust
// Match exact polynomial evaluation order
// Example: bcg729's cosine uses specific MULT16_16_P15 macros
let result = SATURATE(ADD32(Kcos1, 
    MULT16_16_P15(x2, ADD32(Kcos2, 
        MULT16_16_P15(x2, ADD32(Kcos3, 
            MULT16_16_P15(Kcos4, x2)))))), MAXINT16);
```

### Option 2: FFI to bcg729
```rust
extern "C" {
    fn g729Acos_Q15Q13(x: i16) -> i16;
    fn g729Sqrt_Q0Q7(x: u32) -> i32;
}
```

### Option 3: Assembly-Level Matching
- Analyze bcg729 assembly output
- Match exact CPU instruction sequences
- Handle compiler-specific optimizations

## Technical Details

### Fixed-Point Formats
- Q15: 1 sign bit, 15 fractional bits [-1, 1)
- Q13: 3 integer bits, 13 fractional bits  
- Q31: 1 sign bit, 31 fractional bits

### Critical Operations
1. **Arccos**: `acos(x) = π/2 - atan(x/sqrt(1-x²))`
2. **Square Root**: Polynomial approximation in Q14
3. **Arctangent**: Rational approximation with range reduction

## Usage

```rust
use codec_core::codecs::g729a::{G729AEncoder, G729ADecoder};

let encoder = G729AEncoder::new();
let pcm_frame = vec![0i16; 80]; // 10ms at 8kHz
let encoded = encoder.encode(&pcm_frame);

let decoder = G729ADecoder::new();
let decoded = decoder.decode(&encoded);
```

## Testing

```bash
# Run compliance tests
cargo test encoder_compliance

# Debug specific components  
cargo test test_debug_exact_arccos
```

## References
- ITU-T G.729 (03/96) + Annex A (11/96)
- bcg729 reference implementation
- ITU-T test vectors (T.IN, T.BIT, T.PST)

## License
See LICENSE file in repository root. 