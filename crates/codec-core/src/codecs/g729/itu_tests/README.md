# G.729 ITU-T Compliance Test Suite

This directory contains a comprehensive test suite for validating the G.729 codec implementation against official ITU-T test vectors. The test suite ensures standards compliance and production readiness.

## Overview

The ITU-T G.729 standard provides official test vectors to validate codec implementations. This test suite implements comprehensive compliance testing covering:

- **Core G.729**: Full complexity algorithm
- **G.729A (Annex A)**: Reduced complexity (~40% computational reduction)
- **G.729B (Annex B)**: VAD/DTX/CNG (~50% bandwidth reduction)

## Test Structure

### Core Test Modules

- **`itu_test_utils.rs`**: Binary file parsing, similarity calculations, compliance reporting framework
- **`itu_encoder_tests.rs`**: Encoder compliance against ITU test vectors  
- **`itu_decoder_tests.rs`**: Decoder compliance against ITU test vectors
- **`itu_annex_a_tests.rs`**: G.729A reduced complexity variant tests
- **`itu_annex_b_tests.rs`**: G.729B VAD/DTX/CNG variant tests
- **`itu_integration_tests.rs`**: Comprehensive integration and production readiness assessment

### Test Data Organization

The test suite expects ITU test data to be organized as follows:

```
src/codecs/g729/tests/test_data/
├── g729/              # Core G.729 test vectors
│   ├── algthm.in      # Algorithm conditional parts - input
│   ├── algthm.bit     # Algorithm conditional parts - bitstream
│   ├── algthm.pst     # Algorithm conditional parts - output
│   ├── fixed.in/bit/pst   # Fixed codebook (ACELP) search
│   ├── lsp.in/bit/pst     # LSP quantization
│   ├── pitch.in/bit/pst   # Pitch search algorithms  
│   ├── speech.in/bit/pst  # Generic speech processing
│   ├── tame.in/bit/pst    # Taming procedure
│   ├── erasure.bit/pst    # Frame erasure recovery
│   ├── overflow.bit/pst   # Overflow detection
│   └── parity.bit/pst     # Parity check
├── g729AnnexA/        # G.729A test vectors (reduced complexity)
│   └── [similar file structure]
└── g729AnnexB/        # G.729B test vectors (VAD/DTX/CNG)
    ├── tstseq1.bin/bit/out  # VAD/DTX test sequence 1
    ├── tstseq2.bin/bit/out  # VAD/DTX test sequence 2
    ├── tstseq3.bin/bit/out  # VAD/DTX test sequence 3
    ├── tstseq4.bin/bit/out  # VAD/DTX test sequence 4
    ├── tstseq5.bit/out      # Decoder-only sequence 5
    └── tstseq6.bit/out      # Decoder-only sequence 6
```

### File Formats

- **`.in/.bin files**: 16-bit PCM input samples (Intel little-endian format)
- **`.bit files**: Encoded bitstream data (raw binary, typically 10 bytes per frame)
- **`.pst/.out files**: Expected decoder output samples (16-bit Intel format)

## Running Tests

### Individual Test Suites

```bash
# Run core encoder compliance tests
cargo test test_g729_core_encoder_compliance -- --nocapture

# Run core decoder compliance tests  
cargo test test_g729_core_decoder_compliance -- --nocapture

# Run G.729A reduced complexity tests
cargo test test_g729a_encoder_compliance -- --nocapture

# Run G.729B VAD/DTX/CNG tests
cargo test test_g729b_encoder_compliance -- --nocapture
```

### Comprehensive Compliance Test

```bash
# Run the full ITU-T compliance test suite
cargo test test_full_g729_itu_compliance_suite -- --nocapture
```

This test provides a complete production readiness assessment with detailed compliance reporting.

### Test Data Availability Check

```bash
# Verify ITU test data is properly installed
cargo test test_itu_test_data_availability -- --nocapture
```

## Compliance Criteria

### Encoder Compliance
- **Bitstream Similarity**: ≥85% for core tests, ≥90% for Annex A
- **Parameter Validity**: All encoded parameters within ITU-specified ranges
- **Frame Structure**: Correct 80-bit frame size and subframe organization
- **Complexity Reduction**: ≥30% for G.729A vs core G.729

### Decoder Compliance  
- **Sample Similarity**: ≥80% for core tests, ≥75% for Annex B (due to CNG)
- **Signal Quality**: SNR ≥15dB, THD ≤20%
- **Error Handling**: ≤10% decode error rate
- **Frame Synchronization**: Robust handling of invalid frames

### G.729A Specific
- **Computational Complexity**: ~40% reduction vs core G.729
- **Backward Compatibility**: Bitstreams decodable by core G.729 decoder
- **Quality Maintenance**: Similar audio quality to core G.729

### G.729B Specific
- **VAD Performance**: ≥70% accuracy on speech/silence detection
- **Bandwidth Reduction**: ≥20% during silence periods
- **CNG Quality**: Perceptually appropriate comfort noise generation
- **DTX Operation**: Proper discontinuous transmission behavior

## Production Readiness Assessment

The comprehensive test suite provides automated production readiness scoring:

- **🏆 95%+ Overall Compliance**: Production ready for commercial VoIP
- **🥈 85-94% Compliance**: Near production ready, minor tuning needed
- **🥉 75-84% Compliance**: Development ready, suitable for testing
- **❌ <75% Compliance**: Not ready, significant issues need addressing

## Implementation Requirements

To use this test suite, your G.729 implementation must provide:

### Encoder Interface
```rust
impl G729Encoder {
    fn new() -> Self;
    fn new_with_variant(variant: G729Variant) -> Self;
    fn encode_frame(&mut self, samples: &[i16]) -> G729Frame;
    fn reset(&mut self);
}
```

### Decoder Interface  
```rust
impl G729Decoder {
    fn new() -> Self;
    fn new_with_variant(variant: G729Variant) -> Self;
    fn decode_bitstream(&mut self, bits: &[u8]) -> Option<G729Frame>;
    fn decode_frame(&mut self, frame: &G729Frame) -> Vec<i16>;
    fn conceal_frame(&mut self, lost: bool) -> Vec<i16>;
    fn reset(&mut self);
}
```

### Frame Structure
```rust
pub struct G729Frame {
    pub frame_type: FrameType,       // Active, DTX, SID
    pub lsp_indices: Vec<usize>,     // LSP quantization indices
    pub subframes: Vec<Subframe>,    // 2 subframes per frame
}

pub struct Subframe {
    pub pitch_lag: usize,            // 20-143 samples
    pub positions: Vec<usize>,       // ACELP pulse positions
    pub signs: Vec<i8>,              // ACELP pulse signs (±1)
    pub gain_index: usize,           // Quantized gain index
}
```

## Obtaining ITU Test Data

ITU-T test vectors are available from:
1. **ITU-T Website**: Official test data download
2. **G.729 Reference Implementation**: Includes test vectors
3. **Telecom Equipment Vendors**: Often provide test suites

**Note**: ITU test data is copyrighted material. Ensure proper licensing for commercial use.

## Interpreting Results

### Test Output Example
```
🎯 G.729 ITU-T COMPREHENSIVE COMPLIANCE TEST SUITE
==================================================

📋 Testing G.729 Core Implementation...
✅ Encoder: Algorithm conditional parts - 89.2% similarity
✅ Decoder: Frame erasure recovery - 82.1% similarity
❌ Encoder: Fixed codebook search - 73.4% similarity (Low similarity)

🎉 OVERALL COMPLIANCE: 87.3%
✅ GOOD - Minor issues may need attention
```

### Common Issues and Solutions

1. **Low Encoder Similarity**: Check LPC analysis, pitch search, or ACELP implementation
2. **Decoder Errors**: Verify bitstream parsing, frame reconstruction, or postfilter
3. **G.729A Complexity**: Ensure simplified algorithms are properly implemented
4. **G.729B VAD Issues**: Check voice activity detection thresholds and logic

## Contributing

When adding new tests:
1. Follow the existing naming conventions
2. Add comprehensive documentation
3. Include both positive and negative test cases
4. Update this README with new test descriptions

## References

- **ITU-T Recommendation G.729**: Coding of speech at 8 kbit/s using conjugate-structure algebraic-code-excited linear prediction (CS-ACELP)
- **ITU-T Recommendation G.729 Annex A**: Reduced complexity 8 kbit/s CS-ACELP speech codec
- **ITU-T Recommendation G.729 Annex B**: A silence compression scheme for G.729 optimized for terminals conforming to Recommendation V.70
- **G.191 Software Tools**: ITU-T Software Tool Library for speech and audio coding standardization 