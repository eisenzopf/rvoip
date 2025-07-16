# G.729 Codec Implementation Plan

## Overview

This document outlines the comprehensive implementation plan for the ITU-T G.729 codec family, including all annexes and applications. The implementation will be based on the ITU-T G.729 Release 3 reference implementation and documentation located in `crates/codec-core/T-REC-G.729-201206/`.

## Current State

- **Location**: `crates/codec-core/src/codecs/g729/mod.rs`
- **Status**: Basic simulation implementation exists
- **Framework**: Integrated with codec-core architecture

## G.729 Family Overview

G.729 is a low bit-rate speech codec using Conjugate-Structure Algebraic-Code-Excited Linear-Prediction (CS-ACELP). The family includes:

### Base Specifications
- **G.729**: Original 8 kbit/s codec
- **G.729A**: Reduced complexity version

### Annexes
- **Annex A**: Reduced complexity algorithm (same as G.729A)
- **Annex B**: Voice Activity Detection (VAD), Discontinuous Transmission (DTX), Comfort Noise Generation (CNG)
- **Annex C**: Compatibility with older G.729 implementations
- **Annex C+**: Enhanced compatibility
- **Annex D**: Simultaneous use with V.8 bis modem
- **Annex E**: 11.8 kbit/s higher bit rate extension
- **Annex F**: 6.4 kbit/s version with DTX (integration of Annexes B and D)
- **Annex G**: 8 kbit/s and 11.8 kbit/s with DTX (integration of Annexes B and E)
- **Annex H**: Reserved for future use
- **Annex I**: C code for fixed-point implementation

### Applications
- **Application II**: Wideband extension
- **Application III**: Floating-point to fixed-point conversion
- **Application IV**: Enhanced VAD with additional algorithms

## Implementation Strategy

### Phase 1: Core G.729 Implementation (Base + Annex A)

#### 1.1 Basic Infrastructure
- [x] **Task 1.1.1**: Create proper module structure with submodules ✅
  - ✅ `types.rs`: G.729-specific types and constants
  - ✅ `math.rs`: Fixed-point arithmetic operations 
  - ✅ `mod.rs`: Module organization
  - [ ] `encoder.rs`: Core encoding functionality
  - [ ] `decoder.rs`: Core decoding functionality
  - [ ] `lpc.rs`: Linear Predictive Coding functions
  - [ ] `pitch.rs`: Pitch analysis and synthesis
  - [ ] `codebook.rs`: Algebraic codebook search
  - [ ] `tables.rs`: Static lookup tables

#### 1.2 Mathematical Foundation
- [x] **Task 1.2.1**: Implement fixed-point arithmetic operations ✅
  - ✅ Basic operations (add, subtract, multiply, divide)
  - ✅ 32-bit operations for extended precision
  - ✅ Saturation and overflow handling
  - ✅ Based on `basic_op.c` and `oper_32b.c`
  - ✅ All operations tested and passing (9/9 tests)

- [x] **Task 1.2.2**: Implement DSP utility functions ✅
  - ✅ Pow2, Log2, Inv_sqrt with ITU table lookup
  - ✅ Autocorrelation for LPC analysis
  - ✅ Convolution for filtering operations
  - ✅ Window functions and energy computation
  - ✅ Based on `dspfunc.c` and `tab_ld8k.c`
  - ✅ All DSP operations tested and passing (7/7 tests)

#### 1.3 Linear Predictive Coding
- [ ] **Task 1.3.1**: Implement LPC analysis
  - Windowing functions
  - Autocorrelation method
  - Levinson-Durbin algorithm
  - Based on `lpc.c` and `lpcfunc.c`

- [ ] **Task 1.3.2**: Implement LSP (Line Spectral Pairs) processing
  - LPC to LSP conversion
  - LSP quantization
  - LSP interpolation
  - Based on `qua_lsp.c`, `lspdec.c`, `lspgetq.c`

#### 1.4 Pitch Analysis and Synthesis
- [ ] **Task 1.4.1**: Implement pitch analysis
  - Open-loop pitch search
  - Closed-loop pitch refinement
  - Fractional pitch estimation
  - Based on `pitch.c` (base) and `pitch_a.c` (Annex A)

- [ ] **Task 1.4.2**: Implement pitch synthesis
  - Long-term prediction
  - Adaptive codebook
  - Based on `pred_lt3.c`

#### 1.5 Algebraic Codebook
- [ ] **Task 1.5.1**: Implement ACELP search
  - Fixed codebook search
  - Pulse position optimization
  - Sign optimization
  - Based on `acelp_co.c` (base) and `acelp_ca.c` (Annex A)

- [ ] **Task 1.5.2**: Implement gain quantization
  - Pitch and fixed codebook gains
  - Gain prediction
  - Based on `qua_gain.c`, `dec_gain.c`, `gainpred.c`

#### 1.6 Core Encoder
- [ ] **Task 1.6.1**: Implement main encoding loop
  - Frame preprocessing
  - LPC analysis per frame
  - Subframe processing
  - Parameter quantization
  - Based on `cod_ld8k.c` (base) and `cod_ld8a.c` (Annex A)

#### 1.7 Core Decoder
- [ ] **Task 1.7.1**: Implement main decoding loop
  - Parameter dequantization
  - Speech synthesis
  - Error concealment
  - Based on `dec_ld8k.c` (base) and `dec_ld8a.c` (Annex A)

#### 1.8 Pre/Post Processing
- [ ] **Task 1.8.1**: Implement preprocessing
  - High-pass filtering
  - DC removal
  - Based on `pre_proc.c`

- [ ] **Task 1.8.2**: Implement postprocessing
  - Post-filtering
  - Quality enhancement
  - Based on `post_pro.c`, `pst.c`, `postfilt.c`

#### 1.9 Bitstream Handling
- [ ] **Task 1.9.1**: Implement bitstream packing/unpacking
  - Parameter serialization
  - Error detection
  - Frame synchronization
  - Based on `bits.c`

### Phase 2: Enhanced Features (Annexes B, C, C+)

#### 2.1 Voice Activity Detection (Annex B)
- [ ] **Task 2.1.1**: Implement VAD algorithm
  - Energy-based detection
  - Spectral analysis
  - Decision logic
  - Based on `vad.c`

- [ ] **Task 2.1.2**: Implement DTX (Discontinuous Transmission)
  - Silence detection
  - Transmission control
  - Based on `dtx.c`

- [ ] **Task 2.1.3**: Implement CNG (Comfort Noise Generation)
  - Noise parameter estimation
  - Noise synthesis
  - SID (Silence Insertion Descriptor) frames
  - Based on `calcexc.c`, `dec_sid.c`, `qsidgain.c`, `qsidlsf.c`

#### 2.2 Compatibility (Annexes C, C+)
- [ ] **Task 2.2.1**: Implement backward compatibility features
  - Legacy bitstream support
  - Interoperability enhancements

### Phase 3: Advanced Extensions (Annexes D, E, F, G)

#### 3.1 Modem Compatibility (Annex D)
- [ ] **Task 3.1.1**: Implement V.8 bis compatibility
  - Simultaneous operation detection
  - Codec parameter adaptation

#### 3.2 Higher Bit Rate (Annex E)
- [ ] **Task 3.2.1**: Implement 11.8 kbit/s mode
  - Enhanced excitation coding
  - Improved quality parameters
  - Backward/forward LPC structure

#### 3.3 Lower Bit Rate (Annex F)
- [ ] **Task 3.3.1**: Implement 6.4 kbit/s mode
  - Combined DTX functionality
  - Reduced parameter set

#### 3.4 Dual Rate with DTX (Annex G)
- [ ] **Task 3.4.1**: Implement dual-rate operation
  - 8 kbit/s and 11.8 kbit/s modes
  - DTX integration
  - Dynamic rate switching

### Phase 4: Advanced Applications

#### 4.1 Enhanced VAD (Application IV)
- [ ] **Task 4.1.1**: Implement advanced VAD algorithms
  - Multiple detection methods
  - Improved performance in noise
  - Based on `vad_fx.c`, `parameters_fx.c`

- [ ] **Task 4.1.2**: Implement enhanced preprocessing
  - Advanced noise reduction
  - Echo cancellation integration
  - Based on `preproc_fx.c`, `enh40.c`, `enh1632.c`

#### 4.2 Wideband Extensions (Applications II, III)
- [ ] **Task 4.2.1**: Implement wideband support
  - Extended frequency range
  - Enhanced quality

### Phase 5: Testing and Validation

#### 5.1 Unit Tests
- [ ] **Task 5.1.1**: Create comprehensive unit tests
  - Test each major component
  - Validate against reference vectors
  - Performance benchmarking

#### 5.2 Integration Tests
- [ ] **Task 5.2.1**: Test with ITU reference vectors
  - Use test vectors from each annex directory
  - Bit-exact comparison where required
  - Quality assessment

#### 5.3 Interoperability Tests
- [ ] **Task 5.3.1**: Test interoperability
  - Cross-compatibility between variants
  - Real-world scenario testing

### Phase 6: Optimization and Production

#### 6.1 Performance Optimization
- [ ] **Task 6.1.1**: Optimize for speed
  - Algorithm optimization
  - SIMD instructions where applicable
  - Memory layout optimization

#### 6.2 Memory Optimization
- [ ] **Task 6.2.1**: Minimize memory footprint
  - State structure optimization
  - Buffer management
  - Stack usage optimization

#### 6.3 Error Handling
- [ ] **Task 6.3.1**: Robust error handling
  - Graceful degradation
  - Error recovery mechanisms
  - Comprehensive logging

## File Structure

```
crates/codec-core/src/codecs/g729/
├── mod.rs              # Main module interface
├── encoder.rs          # Core encoding functionality
├── decoder.rs          # Core decoding functionality
├── lpc.rs             # Linear Predictive Coding
├── pitch.rs           # Pitch analysis and synthesis
├── codebook.rs        # Algebraic codebook operations
├── tables.rs          # Lookup tables and constants
├── types.rs           # G.729-specific types
├── math.rs            # Fixed-point arithmetic
├── dsp.rs             # DSP utility functions
├── bitstream.rs       # Bitstream handling
├── preprocess.rs      # Preprocessing functions
├── postprocess.rs     # Postprocessing functions
├── annexes/
│   ├── mod.rs         # Annex module exports
│   ├── annex_a.rs     # Reduced complexity
│   ├── annex_b.rs     # VAD/DTX/CNG
│   ├── annex_c.rs     # Compatibility
│   ├── annex_d.rs     # V.8 bis compatibility
│   ├── annex_e.rs     # 11.8 kbit/s extension
│   ├── annex_f.rs     # 6.4 kbit/s with DTX
│   ├── annex_g.rs     # Dual rate with DTX
│   └── annex_i.rs     # Fixed-point implementation
├── applications/
│   ├── mod.rs         # Application module exports
│   ├── app_ii.rs      # Wideband extension
│   ├── app_iii.rs     # Floating to fixed-point
│   └── app_iv.rs      # Enhanced VAD
└── tests/
    ├── mod.rs         # Test module
    ├── unit_tests.rs  # Unit tests
    ├── reference_tests.rs  # ITU reference vector tests
    └── integration_tests.rs  # Integration tests
```

## Dependencies

### Rust Crates
- `tracing`: Logging and instrumentation
- `thiserror`: Error handling
- `byteorder`: Endianness handling
- `num-traits`: Numeric operations
- `criterion`: Benchmarking (dev dependency)

### ITU Reference Implementation
- Location: `crates/codec-core/T-REC-G.729-201206/`
- Use for algorithm reference and test vectors
- C code provides exact implementation details

## Success Criteria

1. **Functional Compliance**: Pass all ITU reference test vectors
2. **Performance**: Real-time encoding/decoding on target hardware
3. **Quality**: Subjective quality matching reference implementation
4. **Compatibility**: Interoperability with existing G.729 implementations
5. **Robustness**: Graceful handling of edge cases and errors

## Risks and Mitigation

### Technical Risks
- **Complexity**: G.729 is algorithmically complex
  - *Mitigation*: Phase-based implementation, extensive testing
- **Fixed-point arithmetic**: Precision and overflow issues
  - *Mitigation*: Use ITU reference implementation as guide
- **Performance**: Real-time requirements
  - *Mitigation*: Profile and optimize critical paths

### Resource Risks
- **Time**: Large scope with many variants
  - *Mitigation*: Prioritize core functionality first
- **Testing**: Extensive validation required
  - *Mitigation*: Automated testing with reference vectors

## Timeline Estimate

- **Phase 1 (Core)**: 8-10 weeks
- **Phase 2 (Basic Annexes)**: 4-6 weeks  
- **Phase 3 (Advanced Annexes)**: 6-8 weeks
- **Phase 4 (Applications)**: 4-6 weeks
- **Phase 5 (Testing)**: 3-4 weeks
- **Phase 6 (Optimization)**: 2-3 weeks

**Total Estimated Duration**: 27-37 weeks

## Annex Completion Audit Framework

### Completion Criteria Matrix

Each annex must meet ALL criteria below to be considered complete:

#### Base G.729 Completion Audit
- [ ] **Functional Requirements**
  - [ ] Encodes 80-sample frames to 80-bit bitstreams
  - [ ] Decodes 80-bit bitstreams to 80-sample frames
  - [ ] Supports 8 kHz, 16-bit, mono audio only
  - [ ] Implements CS-ACELP algorithm correctly
- [ ] **Test Vector Compliance**
  - [ ] SPEECH.IN → SPEECH.BIT (encoder test)
  - [ ] SPEECH.BIT → SPEECH.PST (decoder test)
  - [ ] ALGTHM, PITCH, LSP, FIXED test vectors pass
  - [ ] PARITY, ERASURE, OVERFLOW error handling works
- [ ] **Performance Requirements**
  - [ ] Real-time encoding on target hardware
  - [ ] Memory usage < 50KB for encoder+decoder state
  - [ ] No memory leaks or buffer overflows
- [ ] **Integration Tests**
  - [ ] Integrates with codec-core framework
  - [ ] Proper error handling and logging
  - [ ] Thread-safe operation

#### G.729A (Annex A) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Reduced complexity ACELP search
  - [ ] Maintains G.729 bitstream compatibility
  - [ ] ~40% computational complexity reduction
- [ ] **Test Vector Compliance**
  - [ ] All G.729A test vectors in `test_data/g729AnnexA/` pass
  - [ ] Bit-exact output matching ITU reference
  - [ ] Cross-compatibility with base G.729 decoders
- [ ] **Performance Requirements**
  - [ ] Faster encoding than base G.729
  - [ ] Quality metrics within ITU specifications
- [ ] **Documentation**
  - [ ] Performance comparison with base G.729
  - [ ] Complexity analysis documentation

#### Annex B (VAD/DTX/CNG) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Voice Activity Detection algorithm
  - [ ] Discontinuous Transmission control
  - [ ] Comfort Noise Generation
  - [ ] SID (Silence Insertion Descriptor) frames
- [ ] **Test Vector Compliance**
  - [ ] All 29 test vectors in `test_data/g729AnnexB/` pass
  - [ ] `tstseq1-6` test sequences with DTX enabled/disabled
  - [ ] Proper SID frame generation and decoding
- [ ] **Integration Tests**
  - [ ] VAD correctly detects speech vs silence
  - [ ] DTX reduces bandwidth during silence
  - [ ] CNG provides natural background noise
  - [ ] Backward compatibility with non-DTX decoders
- [ ] **Performance Metrics**
  - [ ] VAD accuracy > 95% on test vectors
  - [ ] Bandwidth reduction during silence periods
  - [ ] Seamless transitions between speech and silence

#### Annex C/C+ (Compatibility) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Backward compatibility with older G.729 implementations
  - [ ] Enhanced interoperability features
- [ ] **Integration Tests**
  - [ ] Decodes bitstreams from legacy encoders
  - [ ] Generated bitstreams decode on legacy decoders
  - [ ] Parameter adaptation for compatibility
- [ ] **Interoperability Tests**
  - [ ] Cross-vendor compatibility validation
  - [ ] Edge case handling for non-standard bitstreams

#### Annex D (V.8 bis) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Simultaneous operation with V.8 bis modems
  - [ ] Codec parameter adaptation for modem compatibility
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AnnexD/` pass
- [ ] **Integration Tests**
  - [ ] Proper detection of modem presence
  - [ ] Automatic parameter adjustment
  - [ ] No interference with modem operation

#### Annex E (11.8 kbit/s) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Higher bit rate encoding at 11.8 kbit/s
  - [ ] Enhanced speech quality
  - [ ] Backward/forward LPC structure
- [ ] **Test Vector Compliance**
  - [ ] All 30 test vectors in `test_data/g729AnnexE/` pass
  - [ ] SPEECHE.118, PITCHE.118, LSPE.118 files
  - [ ] Enhanced quality validation tests
- [ ] **Quality Metrics**
  - [ ] PESQ score improvement over base G.729
  - [ ] Subjective quality improvement validation
  - [ ] Wider frequency response

#### Annex F (6.4 kbit/s + DTX) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Lower bit rate at 6.4 kbit/s
  - [ ] Integrated DTX functionality
  - [ ] Combination of Annexes B and D features
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AnnexF/` pass
  - [ ] DTX operation at reduced bit rate
- [ ] **Performance Requirements**
  - [ ] Acceptable quality at 6.4 kbit/s
  - [ ] Effective DTX operation
  - [ ] V.8 bis compatibility maintained

#### Annex G (Dual Rate + DTX) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Dynamic switching between 8 and 11.8 kbit/s
  - [ ] DTX operation at both rates
  - [ ] Rate adaptation based on network conditions
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AnnexG/` pass
  - [ ] Rate switching scenarios
- [ ] **Integration Tests**
  - [ ] Seamless rate transitions
  - [ ] DTX operation at both rates
  - [ ] Rate control algorithm validation

#### Annex H Completion Audit
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AnnexH/` pass
- [ ] **Integration Tests**
  - [ ] Proper integration with existing annexes

#### Annex I (Fixed-Point) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Bit-exact fixed-point implementation
  - [ ] Overflow and saturation handling
  - [ ] Cross-platform consistency
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AnnexI/` pass
  - [ ] Bit-exact output across platforms
- [ ] **Performance Requirements**
  - [ ] Optimized for embedded platforms
  - [ ] Deterministic execution time

#### Application II (Wideband) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Extended frequency range support
  - [ ] Wideband encoding/decoding
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AppII/` pass
- [ ] **Quality Metrics**
  - [ ] Full bandwidth preservation
  - [ ] Enhanced speech naturalness

#### Application III (Float-to-Fixed) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Floating-point to fixed-point conversion utilities
  - [ ] Precision analysis tools
- [ ] **Test Vector Compliance**
  - [ ] All test vectors in `test_data/g729AppIII/` pass
- [ ] **Validation Tests**
  - [ ] Conversion accuracy verification
  - [ ] Precision loss analysis

#### Application IV (Enhanced VAD) Completion Audit
- [ ] **Functional Requirements**
  - [ ] Advanced VAD algorithms implementation
  - [ ] Enhanced noise robustness
  - [ ] Multiple detection methods
- [ ] **Source Code Compliance**
  - [ ] All 74 source files from ITU package ported
  - [ ] `vad_fx.c`, `parameters_fx.c` algorithms implemented
  - [ ] Enhanced preprocessing (`preproc_fx.c`, `enh40.c`, `enh1632.c`)
- [ ] **Performance Requirements**
  - [ ] Improved VAD accuracy in noisy environments
  - [ ] Better music/speech discrimination
  - [ ] Reduced false alarms

### Audit Process

#### Phase-Gate Reviews
1. **Weekly Progress Reviews**: Track task completion against timeline
2. **Milestone Reviews**: Comprehensive audit at end of each phase
3. **Integration Reviews**: Cross-annex compatibility validation
4. **Final Audit**: Complete system validation before production

#### Automated Testing Requirements
- [ ] **Continuous Integration Pipeline**
  - [ ] All test vectors run automatically on commits
  - [ ] Performance regression detection
  - [ ] Memory leak detection
  - [ ] Cross-platform testing (Linux, macOS, Windows)

#### Documentation Requirements
Each completed annex must include:
- [ ] **API Documentation**: Complete Rust docs for all public interfaces
- [ ] **Implementation Notes**: Algorithm-specific implementation details
- [ ] **Performance Analysis**: Benchmarks and complexity analysis
- [ ] **Test Results**: Complete test vector results and analysis
- [ ] **Integration Guide**: How to use the annex with other components

#### Quality Gates
No annex can be marked complete without:
- [ ] **Code Review**: Peer review by at least 2 developers
- [ ] **Security Review**: Vulnerability assessment for memory safety
- [ ] **Performance Review**: Meets real-time requirements
- [ ] **Documentation Review**: Complete and accurate documentation

### Audit Tools

#### Recommended Audit Scripts
```bash
# Test vector validation
./scripts/validate_annex.sh [annex_name]

# Performance benchmarking  
./scripts/benchmark_annex.sh [annex_name]

# Memory analysis
./scripts/memory_analysis.sh [annex_name]

# Cross-compatibility testing
./scripts/interop_test.sh [annex_a] [annex_b]
```

#### Metrics Dashboard
Track completion metrics:
- Test vector pass rate per annex
- Performance benchmarks vs targets
- Memory usage trends
- Code coverage percentages
- Documentation coverage

This audit framework ensures each annex meets ITU specifications and integrates properly with the overall G.729 codec family implementation.

## Notes

- Implementation should prioritize correctness over performance initially
- Each phase should include comprehensive testing before proceeding
- Consider implementing the most commonly used variants (G.729A + Annex B) first
- Regular validation against ITU test vectors is essential
- Documentation should be maintained throughout the implementation process
- **Audit compliance is mandatory** - no annex ships without passing all audit criteria 