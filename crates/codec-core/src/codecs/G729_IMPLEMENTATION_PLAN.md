# G.729 Implementation Plan

## Overview
This document outlines the implementation plan for G.729 speech codec in Rust, focusing specifically on:
- **Core G.729**: Full complexity implementation (8 kbit/s)
- **Annex A (G.729A)**: Reduced complexity implementation (8 kbit/s) 
- **Annex B (G.729B)**: VAD/DTX/CNG extensions for bandwidth efficiency

Based on ITU-T reference implementations in `/T-REC-G.729-201206/Software/G729_Release3/`.

## Feature Flags and Build Configuration

The G.729 implementation uses Cargo feature flags to allow selective compilation of variants, reducing binary size and complexity when not all features are needed.

### Available Features

#### Core Features
```toml
# Default - Core G.729 only (full complexity reference implementation)
rvoip-codec-core = "0.1"

# Core G.729 explicitly
rvoip-codec-core = { version = "0.1", features = ["g729-core"] }
```

#### Annex Features
```toml
# G.729A - Reduced complexity (~40% faster)
rvoip-codec-core = { version = "0.1", features = ["annex-a"] }

# G.729B - VAD/DTX/CNG (~50% bandwidth savings in silence)
rvoip-codec-core = { version = "0.1", features = ["annex-b"] }

# G.729A + G.729B combined (most practical for production)
rvoip-codec-core = { version = "0.1", features = ["annex-a", "annex-b"] }
```

#### Convenience Features
```toml
# All G.729 variants (G.729, G.729A, G.729B, G.729BA)
rvoip-codec-core = { version = "0.1", features = ["all-annexes"] }

# Development build with all features and testing utilities
rvoip-codec-core = { version = "0.1", features = ["dev-all"] }
```

### Feature Dependencies
- **`g729-core`**: Base implementation, required by all other features
- **`annex-a`**: Depends on `g729-core`, adds reduced complexity algorithms
- **`annex-b`**: Depends on `g729-core`, adds VAD/DTX/CNG extensions
- **`all-annexes`**: Enables both `annex-a` and `annex-b`

### Recommended Configurations

| Use Case | Features | Description |
|----------|----------|-------------|
| **Production VoIP** | `["annex-a", "annex-b"]` | G.729BA - optimal balance |
| **Low-power devices** | `["annex-a"]` | G.729A - reduced CPU usage |
| **Bandwidth-critical** | `["annex-b"]` | G.729B - silence suppression |
| **Reference/Testing** | `["g729-core"]` | Full complexity validation |
| **Development** | `["dev-all"]` | All variants + test utilities |

### Conditional Compilation

Implementation modules use conditional compilation based on feature flags:

```rust
// Core G.729 - always available
#[cfg(feature = "g729-core")]
pub mod encoder;

// G.729A reduced complexity variants
#[cfg(feature = "annex-a")]
pub mod encoder_a;
#[cfg(feature = "annex-a")]
pub mod pitch_a;
#[cfg(feature = "annex-a")]
pub mod acelp_a;

// G.729B VAD/DTX/CNG extensions
#[cfg(feature = "annex-b")]
pub mod vad;
#[cfg(feature = "annex-b")]
pub mod dtx;
#[cfg(feature = "annex-b")]
pub mod cng;

// Combined G.729BA functionality
#[cfg(all(feature = "annex-a", feature = "annex-b"))]
pub mod encoder_ba;
```

## Target Implementations

### Core G.729 (Full Complexity)
**ITU Reference**: `/g729/c_code/`
**Feature Flag**: `g729-core` (included in default)
- **Encoder**: `COD_LD8K.C` (679 lines)
- **Decoder**: `DEC_LD8K.C` (283 lines) 
- **ACELP**: `ACELP_CO.C` (869 lines) - Fixed codebook search
- **Pitch**: `PITCH.C` (623 lines) - Full pitch analysis
- **Post-processing**: `PST.C` (1031 lines) - Advanced postfilter

### G.729A (Reduced Complexity) 
**ITU Reference**: `/g729AnnexA/c_code/`
**Feature Flag**: `annex-a`
- **Encoder**: `COD_LD8A.C` (451 lines) - Simplified encoder
- **Decoder**: `DEC_LD8A.C` (279 lines) - Simplified decoder
- **ACELP**: `ACELP_CA.C` (931 lines) - Adaptive codebook search
- **Pitch**: `PITCH_A.C` (555 lines) - Simplified pitch analysis
- **Post-processing**: `POSTFILT.C` (459 lines) - Simplified postfilter
- **Correlation**: `COR_FUNC.C` (141 lines) - Optimized correlations

### G.729B (VAD/DTX/CNG Extensions)
**ITU Reference**: `/g729AnnexB/c_codeB/` and `/g729AnnexB/c_codeBA/`
**Feature Flag**: `annex-b`

#### G.729B (Full + VAD/DTX/CNG)
- **VAD**: `VAD.C` (513 lines) - Voice Activity Detection
- **DTX**: `DTX.C` (444 lines) - Discontinuous Transmission
- **CNG**: `DEC_SID.C` (198 lines) - Comfort Noise Generation
- **SID Quantization**: `QSIDGAIN.C` (137 lines), `QSIDLSF.C` (305 lines)

#### G.729BA (Reduced + VAD/DTX/CNG) 
**Feature Flags**: `["annex-a", "annex-b"]`
- Combines G.729A reduced complexity with G.729B extensions
- Most practical implementation for real-world deployment

## Implementation Phases

### Phase 1: Foundation Components ✅
**Duration**: 4-6 weeks (COMPLETED)

#### Task 1.1: Basic Infrastructure ✅ 
**Duration**: 1 week (COMPLETED)
- **Task 1.1.1**: Module structure and type definitions ✅
  - Status: COMPLETED
  - Files: `types.rs`, `mod.rs`
  - ITU Basis: LD8K.H definitions
  - Test Status: ✅ All tests passing

#### Task 1.2: Mathematical Foundation ✅
**Duration**: 2 weeks (COMPLETED)
- **Task 1.2.1**: Fixed-point arithmetic operations ✅
  - Status: COMPLETED  
  - Files: `math.rs`
  - ITU Basis: BASIC_OP.C
  - Test Status: ✅ 9/9 tests passing
- **Task 1.2.2**: DSP utility functions ✅
  - Status: COMPLETED
  - Files: `dsp.rs` 
  - ITU Basis: DSPFUNC.C, TAB_LD8K.C
  - Test Status: ✅ 7/7 tests passing

#### Task 1.3: Linear Predictive Coding ✅
**Duration**: 2-3 weeks (COMPLETED)
- **Task 1.3.1**: Implement LPC Analysis ✅
  - Status: COMPLETED
  - Files: `lpc.rs`
  - ITU Basis: LPC.C (Autocorr, Lag_window, Levinson functions)
  - Functions: autocorr(), lag_window(), levinson()
  - Test Status: ✅ 8/8 tests passing
- **Task 1.3.2**: LPC to LSP conversion ✅
  - Status: COMPLETED
  - Files: `lpc.rs`
  - ITU Basis: LPC.C (Az_lsp), LPCFUNC.C (Lsp_Az)
  - Functions: az_lsp(), lsp_az(), chebps_11(), chebps_10()
  - Test Status: ✅ 8/8 tests passing

**Phase 1 Summary**: ✅ COMPLETED
- All mathematical foundation components implemented
- LPC analysis with ITU-compliant autocorrelation, Levinson-Durbin algorithm
- LSP conversion with Chebyshev polynomial root finding
- Comprehensive test coverage with 100% pass rate
- Ready for Phase 2 implementation

### Phase 2: Core G.729 Implementation 🔄
**Duration**: 8-10 weeks (NEXT)
**Feature Flag**: `g729-core`

#### Task 2.1: Pitch Analysis (Full Complexity)
**Duration**: 3 weeks
- **Task 2.1.1**: Open-loop pitch estimation
  - Files: `pitch.rs` 
  - ITU Basis: `PITCH.C` (623 lines)
  - Functions: `Pitch_ol()`, `Lag_max()`, `Cor_max()`
- **Task 2.1.2**: Closed-loop pitch refinement  
  - ITU Basis: `PITCH.C`, `PRED_LT3.C`
  - Functions: `Pitch_fr3()`, `Pred_lt_3()`
- **Task 2.1.3**: Fractional pitch interpolation
  - ITU Basis: Interpolation filters in TAB_LD8K.C
  - Functions: `Interpol_3()`, `Interpol_6()`

#### Task 2.2: ACELP Analysis (Full Complexity)
**Duration**: 4 weeks  
- **Task 2.2.1**: Fixed codebook search
  - Files: `acelp.rs`
  - ITU Basis: `ACELP_CO.C` (869 lines)
  - Functions: `ACELP_Codebook()`, `cor_h_x()`, `D4i40_17()`
- **Task 2.2.2**: Adaptive codebook construction
  - ITU Basis: `COD_LD8K.C` integration
  - Functions: Adaptive codebook filtering and gain calculation

#### Task 2.3: Quantization and Coding
**Duration**: 3 weeks
- **Task 2.3.1**: LSP quantization
  - Files: `quantization.rs`
  - ITU Basis: `QUA_LSP.C` (345 lines), `LSPGETQ.C` (229 lines)
  - Functions: `Qua_lsp()`, `Lsp_get_quant()`
- **Task 2.3.2**: Gain quantization
  - ITU Basis: `QUA_GAIN.C` (430 lines), `GAINPRED.C` (155 lines)
  - Functions: `Qua_gain()`, `Gain_predict()`

### Phase 3: Core G.729 Encoder/Decoder 
**Duration**: 4 weeks
**Feature Flag**: `g729-core`

#### Task 3.1: Encoder Implementation
**Duration**: 2 weeks
- **Task 3.1.1**: Main encoder loop
  - Files: `encoder.rs`
  - ITU Basis: `COD_LD8K.C` (679 lines)
  - Functions: `Coder_ld8k()`, frame processing pipeline
- **Task 3.1.2**: Preprocessing and filtering
  - ITU Basis: `PRE_PROC.C` (85 lines), `FILTER.C` (126 lines)
  - Functions: `Pre_Process()`, `Syn_filt()`, `Residu()`

#### Task 3.2: Decoder Implementation  
**Duration**: 2 weeks
- **Task 3.2.1**: Main decoder loop
  - Files: `decoder.rs`
  - ITU Basis: `DEC_LD8K.C` (283 lines)
  - Functions: `Decoder_ld8k()`, parameter reconstruction
- **Task 3.2.2**: Post-processing
  - ITU Basis: `PST.C` (1031 lines), `POST_PRO.C` (84 lines)
  - Functions: `Post_Filter()`, `Post_Process()`

### Phase 4: G.729A (Reduced Complexity) 
**Duration**: 6 weeks
**Feature Flag**: `annex-a`

#### Task 4.1: Simplified Pitch Analysis
**Duration**: 2 weeks
- **Task 4.1.1**: Reduced complexity pitch search
  - Files: `pitch_a.rs`
  - ITU Basis: `PITCH_A.C` (555 lines)
  - Functions: `Pitch_ol_fast()`, simplified correlation search

#### Task 4.2: Simplified ACELP 
**Duration**: 3 weeks
- **Task 4.2.1**: Adaptive codebook search optimization
  - Files: `acelp_a.rs`
  - ITU Basis: `ACELP_CA.C` (931 lines)
  - Functions: `ACELP_Code_A()`, optimized search procedures
- **Task 4.2.2**: Correlation function optimizations
  - ITU Basis: `COR_FUNC.C` (141 lines)
  - Functions: `Cor_h()`, `Cor_h_x()`

#### Task 4.3: G.729A Encoder/Decoder
**Duration**: 1 week
- **Task 4.3.1**: Reduced complexity integration
  - Files: `encoder_a.rs`, `decoder_a.rs`
  - ITU Basis: `COD_LD8A.C` (451 lines), `DEC_LD8A.C` (279 lines)
  - Functions: `Coder_ld8a()`, `Decoder_ld8a()`

### Phase 5: G.729B (VAD/DTX/CNG Extensions)
**Duration**: 6 weeks
**Feature Flag**: `annex-b`

#### Task 5.1: Voice Activity Detection (VAD)
**Duration**: 2 weeks
- **Task 5.1.1**: VAD algorithm implementation
  - Files: `vad.rs`
  - ITU Basis: `VAD.C` (513 lines)
  - Functions: `vad()`, energy and spectral analysis
- **Task 5.1.2**: VAD parameter computation
  - ITU Basis: VAD decision logic and thresholds
  - Functions: Frame classification and adaptation

#### Task 5.2: Discontinuous Transmission (DTX)
**Duration**: 2 weeks
- **Task 5.2.1**: DTX control and SID frame generation
  - Files: `dtx.rs`
  - ITU Basis: `DTX.C` (444 lines)
  - Functions: `dtx()`, `sid_frame_generation()`
- **Task 5.2.2**: SID parameter quantization
  - ITU Basis: `QSIDGAIN.C` (137 lines), `QSIDLSF.C` (305 lines)
  - Functions: `Qua_Sid_Cng()`, `sid_lsfq_noise()`

#### Task 5.3: Comfort Noise Generation (CNG)
**Duration**: 2 weeks
- **Task 5.3.1**: CNG synthesis
  - Files: `cng.rs`
  - ITU Basis: `DEC_SID.C` (198 lines), `CALCEXC.C` (328 lines)
  - Functions: `Dec_sid()`, `Calc_exc_rand()`
- **Task 5.3.2**: Background noise estimation
  - ITU Basis: Noise parameter estimation and synthesis
  - Functions: Spectral and energy parameter generation

### Phase 6: Integration and Testing
**Duration**: 4 weeks
**Feature Flags**: All variants

#### Task 6.1: Multi-variant Integration
**Duration**: 2 weeks
- **Task 6.1.1**: Unified API for G.729/G.729A/G.729B/G.729BA
- **Task 6.1.2**: Runtime variant selection and configuration
- **Task 6.1.3**: Feature flag validation and testing

#### Task 6.2: ITU Test Vector Validation
**Duration**: 2 weeks
- **Task 6.2.1**: Core G.729 test vector compliance
- **Task 6.2.2**: G.729A test vector compliance  
- **Task 6.2.3**: G.729B test vector compliance (VAD/DTX/CNG)
- **Task 6.2.4**: Cross-variant compatibility testing

## Success Criteria

### Phase 1 Completion Criteria ✅
- [x] All basic math operations implemented with ITU compliance
- [x] LPC analysis producing stable coefficients
- [x] LSP conversion working correctly
- [x] Comprehensive test coverage (>95%)
- [x] No memory safety issues

### Phase 2 Completion Criteria (Core G.729)
- [ ] Pitch analysis producing reasonable lag values (20-143 samples)
- [ ] ACELP fixed codebook search functional
- [ ] LSP and gain quantization working
- [ ] Full encoder/decoder integration complete
- [ ] Core G.729 test vectors passing

### Phase 4 Completion Criteria (G.729A)
- [ ] Reduced complexity algorithms implemented
- [ ] Computational efficiency improved vs Core G.729
- [ ] G.729A test vectors passing
- [ ] Quality maintained relative to full complexity

### Phase 5 Completion Criteria (G.729B)
- [ ] VAD correctly detecting speech/silence
- [ ] DTX reducing transmission during silence
- [ ] CNG providing natural background noise
- [ ] G.729B test vectors passing
- [ ] Bandwidth efficiency demonstrated

### Overall Success Criteria
- [ ] Bit-exact compatibility with ITU reference for all test vectors
- [ ] Real-time performance capability for all variants
- [ ] Memory usage within acceptable bounds (<100KB per instance)
- [ ] Full G.729, G.729A, and G.729B compliance
- [ ] Feature flags working correctly for selective compilation
- [ ] Binary size optimization when features are disabled

## Implementation Priorities

### Primary Target: G.729BA (Annex A + B Combined) 🎯
**Feature Flags**: `["annex-a", "annex-b"]`
The most practical combination for real-world deployment:
- **G.729A**: Reduced computational complexity
- **G.729B**: Bandwidth efficiency with VAD/DTX/CNG
- **Combined**: Optimal balance of quality, complexity, and efficiency

### Secondary Targets
1. **Core G.729**: Reference implementation for validation
   **Feature Flag**: `["g729-core"]`
2. **G.729B (full)**: Full complexity + VAD/DTX/CNG
   **Feature Flag**: `["g729-core", "annex-b"]`

## Dependencies
- ITU-T G.729 reference implementations (attached)
- Test vectors from ITU-T for all variants
- Rust audio processing ecosystem integration
- Cargo feature flag system for conditional compilation

## Risk Mitigation
- Incremental implementation with continuous ITU reference comparison
- Separate modules for each variant to enable independent testing
- Comprehensive test coverage at each phase
- Performance monitoring throughout development
- Feature flag testing to ensure correct conditional compilation

---
**Last Updated**: Current progress through Phase 1.3 completion + Feature flags added
**Next Milestone**: Phase 2.1 - Core G.729 Pitch Analysis implementation
**Focus**: Core G.729 → G.729A → G.729B → G.729BA integration 