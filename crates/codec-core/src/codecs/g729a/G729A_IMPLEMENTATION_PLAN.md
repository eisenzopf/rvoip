# G.729A Codec Implementation Plan

## 📋 Project Overview

This document tracks the implementation progress of the ITU-T G.729A speech codec based on the official ITU reference implementation. G.729A is the reduced-complexity variant of G.729, targeting ~40% complexity reduction while maintaining speech quality.

**Target**: Complete ITU-compliant G.729A encoder/decoder with >90% test vector compliance.

## 🎯 Current Status (Updated: Latest)

### ✅ **COMPLETED COMPONENTS**

| Component | Status | Files | ITU Reference | Tests |
|-----------|--------|-------|---------------|-------|
| **LPC Analysis** | ✅ Complete | `lpc.rs` | LPC.C, LPCFUNC.C | 5/5 passing |
| **LSP Conversion** | ✅ Complete | `lpc.rs` | LPC.C (Az_lsp, Lsp_Az) | ✅ Verified |
| **LSP Interpolation** | ✅ Complete | `lpc.rs` | LPCFUNC.C (Int_qlpc) | ✅ Verified |
| **Synthesis Filtering** | ✅ Complete | `filtering.rs` | FILTER.C (Syn_filt) | 2/2 passing |
| **Basic Operations** | ✅ Complete | `basic_ops.rs` | BASIC_OP.C | ✅ Verified |
| **Framework Structure** | ✅ Complete | `mod.rs`, `types.rs` | LD8A.H | ✅ Verified |
| **Test Infrastructure** | ✅ Complete | `tests/` | N/A | ✅ Working |

**Progress: 40% Complete** 🟩🟩🟩🟩⬜⬜⬜⬜⬜⬜

### 🆕 **TEST INFRASTRUCTURE COMPLETED**

| Test Module | Status | Description |
|-------------|--------|-------------|
| **Unit Tests** | ✅ Complete | Component-level verification tests |
| **Integration Tests** | ✅ Complete | End-to-end and component interaction tests |
| **ITU Compliance Tests** | ✅ Complete | Official test vector validation framework |
| **Performance Tests** | ✅ Complete | Benchmarking and real-time performance verification |
| **Test Utilities** | ✅ Complete | Test data parsing and similarity calculations |

---

## 📅 **IMPLEMENTATION ROADMAP**

### **Phase 1: Core Signal Processing** ⏱️ *3-4 days*

#### 🔄 **Task 1.1: LSP Quantization** `HIGH PRIORITY`
- **Files**: `quantization.rs` (new), `tables.rs` (new)
- **ITU Reference**: `QUA_LSP.C`, `LSPDEC.C`, `TAB_LD8A.C`
- **Functions**:
  - [ ] `qua_lsp()` - LSP vector quantization with MA prediction
  - [ ] `d_lsp()` - LSP dequantization 
  - [ ] LSP codebook tables import
- **Acceptance**: LSP quantization tests pass, <2dB degradation
- **Estimated Effort**: 1.5 days

#### 🎵 **Task 1.2: Pitch Analysis** `HIGH PRIORITY`  
- **Files**: `pitch.rs` (new)
- **ITU Reference**: `PITCH_A.C`, `COR_FUNC.C`
- **Functions**:
  - [ ] `pitch_ol_fast()` - Open-loop pitch estimation (reduced complexity)
  - [ ] `pitch_fr3_fast()` - Closed-loop fractional pitch (1/3 resolution)
  - [ ] `enc_lag3()` - Pitch lag encoding
  - [ ] `dec_lag3()` - Pitch lag decoding
- **Acceptance**: Pitch tracking accuracy >85% vs reference
- **Estimated Effort**: 1.5 days

#### ⚡ **Task 1.3: Perceptual Weighting**
- **Files**: `filtering.rs` (extend)
- **ITU Reference**: `FILTER.C`
- **Functions**:
  - [ ] `weight_az()` - Bandwidth expansion for perceptual weighting
  - [ ] `residu()` - Residual computation
- **Acceptance**: Weighted speech tests pass
- **Estimated Effort**: 0.5 days

---

### **Phase 2: Codebook Search** ⏱️ *2-3 days*

#### 🔍 **Task 2.1: ACELP Search (Reduced Complexity)**
- **Files**: `acelp.rs` (new)
- **ITU Reference**: `ACELP_CA.C` (G.729A variant)
- **Functions**:
  - [ ] `acelp_code_a()` - Algebraic codebook search (simplified)
  - [ ] `decod_acelp()` - ACELP decoding
  - [ ] `cor_xy2()` - Cross-correlation computation
- **Acceptance**: ACELP search provides valid excitation
- **Estimated Effort**: 2 days

#### 📊 **Task 2.2: Gain Quantization**
- **Files**: `gain.rs` (new)
- **ITU Reference**: `QUA_GAIN.C`, `DEC_GAIN.C`, `GAINPRED.C`
- **Functions**:
  - [ ] `qua_gain()` - Gain vector quantization
  - [ ] `dec_gain()` - Gain dequantization  
  - [ ] `gain_predict()` - MA gain prediction
  - [ ] `gain_update()` - Prediction memory update
- **Acceptance**: Gain quantization SNR >25dB
- **Estimated Effort**: 1 day

---

### **Phase 3: Integration & Optimization** ⏱️ *2-3 days*

#### 🔧 **Task 3.1: Encoder Integration**
- **Files**: `encoder.rs` (complete)
- **ITU Reference**: `COD_LD8A.C`
- **Work Items**:
  - [ ] Replace all `todo!()` placeholders
  - [ ] Complete frame processing pipeline
  - [ ] Parameter analysis output (`ana[]`)
  - [ ] Memory state management
- **Acceptance**: Encoder processes full frames without panics
- **Estimated Effort**: 1 day

#### 🔧 **Task 3.2: Decoder Integration**  
- **Files**: `decoder.rs` (complete)
- **ITU Reference**: `DEC_LD8A.C`
- **Work Items**:
  - [ ] Complete bitstream parsing
  - [ ] Synthesis pipeline integration
  - [ ] Post-processing filters
  - [ ] Error concealment (basic)
- **Acceptance**: Decoder reconstructs speech from bitstream
- **Estimated Effort**: 1 day

#### 📦 **Task 3.3: Bitstream Format**
- **Files**: `bitstream.rs` (new)
- **ITU Reference**: `BITS.C`
- **Work Items**:
  - [ ] G.729A frame packing (80 bits)
  - [ ] Parameter serialization
  - [ ] Bit allocation per ITU spec
- **Acceptance**: Bitstream matches ITU format
- **Estimated Effort**: 0.5 days

---

### **Phase 4: Testing & Validation** ⏱️ *1-2 days*

#### 🧪 **Task 4.1: ITU Test Vector Compliance** ✅ **Framework Complete**
- **Files**: `tests/itu_compliance.rs` ✅ Done
- **Test Vectors**: G.729A reference test sequences ✅ Accessible
- **Work Items**:
  - [x] ✅ Test framework implementation
  - [x] ✅ Test vector parsing utilities  
  - [x] ✅ Similarity calculation algorithms
  - [ ] Full compliance validation (depends on implementation)
- **Target**: >90% compliance with ITU test vectors
- **Estimated Effort**: 0.5 days (framework done, validation remains)

#### ⚡ **Task 4.2: Performance Optimization**
- **Files**: All modules
- **Work Items**:
  - [x] ✅ Performance benchmark framework
  - [ ] Complexity profiling vs G.729
  - [ ] Memory usage optimization
  - [ ] SIMD optimization opportunities
- **Target**: 30-40% complexity reduction vs core G.729
- **Estimated Effort**: 1 day

#### 📚 **Task 4.3: Documentation & Examples**
- **Files**: `README.md`, `examples/` (new)
- **Work Items**:
  - [ ] API documentation
  - [ ] Usage examples
  - [ ] Performance benchmarks
- **Estimated Effort**: 0.5 days

---

## 🎯 **MILESTONES**

| Milestone | Target Date | Completion Criteria |
|-----------|-------------|-------------------|
| **M1: Test Infrastructure** | ✅ **COMPLETE** | All test frameworks operational |
| **M2: Signal Processing Complete** | Day 4 | LSP quantization + pitch analysis working |
| **M3: Codebook Search Complete** | Day 7 | ACELP + gain quantization implemented |
| **M4: End-to-End Pipeline** | Day 10 | Complete encoder/decoder chain working |
| **M5: ITU Compliance** | Day 12 | >90% test vector compliance achieved |

---

## 📊 **SUCCESS METRICS**

### **Functional Requirements**
- [ ] **Bit Accuracy**: >95% bitstream match with ITU reference
- [ ] **Audio Quality**: PESQ score >3.8 for clean speech
- [ ] **Complexity Reduction**: 30-40% vs core G.729
- [ ] **Memory Usage**: <64KB peak memory

### **Performance Targets**
- [ ] **Real-time Factor**: <0.1 on modern CPU
- [ ] **Latency**: <20ms algorithmic delay
- [ ] **Robustness**: Handle edge cases gracefully

### **Testing Metrics** ✅ **Frameworks Ready**
- [x] ✅ **Unit Test Coverage**: Component-level verification
- [x] ✅ **Integration Tests**: End-to-end functionality 
- [x] ✅ **ITU Compliance**: Official test vector validation
- [x] ✅ **Performance Benchmarks**: Real-time capability measurement

---

## 🔧 **TECHNICAL DEBT & OPTIMIZATIONS**

### **Current Issues**
- [ ] Error handling needs improvement (reduce panics)
- [ ] Memory allocations in synthesis filter (use pre-allocated buffers)
- [ ] Fixed-point arithmetic validation needed
- [ ] Cross-platform testing required

### **Future Enhancements**
- [ ] G.729B (VAD/DTX) integration preparation  
- [ ] SIMD acceleration for key loops
- [ ] ARM optimization
- [ ] WebAssembly compilation support

---

## 📝 **DEVELOPMENT NOTES**

### **ITU Reference Mapping**
| Rust Module | ITU Files | Key Functions |
|-------------|-----------|---------------|
| `lpc.rs` | LPC.C, LPCFUNC.C | Az_lsp, Lsp_Az, Int_qlpc |
| `filtering.rs` | FILTER.C | Syn_filt, Weight_Az |
| `quantization.rs` | QUA_LSP.C, LSPDEC.C | Qua_lsp, D_lsp |
| `pitch.rs` | PITCH_A.C | Pitch_ol_fast, Pitch_fr3_fast |
| `acelp.rs` | ACELP_CA.C | ACELP_Code_A, Decod_ACELP |
| `gain.rs` | QUA_GAIN.C, DEC_GAIN.C | Qua_gain, Dec_gain |

### **Quality Checkpoints**
1. **After each function**: Unit tests pass
2. **After each module**: Integration tests pass  
3. **After each phase**: ITU compliance tests
4. **Before completion**: Full test suite + benchmarks

### **Test Infrastructure Details** ✅
- **Test Vectors**: 10 official ITU test cases covering all scenarios
- **Test Utilities**: PCM parsing, bitstream analysis, similarity calculation
- **Framework**: Unit, integration, ITU compliance, and performance tests
- **Coverage**: Constants, basic ops, LPC, filtering, encoder/decoder framework

---

**Last Updated**: Current Session  
**Next Priority**: Task 1.1 (LSP Quantization)  
**Status**: Test infrastructure complete, ready for core implementation

---

*This plan serves as the single source of truth for G.729A implementation progress. Update status regularly and adjust estimates based on actual implementation experience.* 