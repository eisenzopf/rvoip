# G.729 Implementation Plan & Progress Tracker

## Overview
This document tracks the implementation progress for G.729 speech codec in Rust, covering all official ITU-T variants:
- **Core G.729**: Full complexity implementation (8 kbit/s)
- **Annex A (G.729A)**: Reduced complexity implementation (8 kbit/s) 
- **Annex B (G.729B)**: VAD/DTX/CNG extensions for bandwidth efficiency
- **Annex BA (G.729BA)**: Reduced complexity + VAD/DTX/CNG (most practical)

Based on ITU-T reference implementations in `/T-REC-G.729-201206/Software/G729_Release3/`.

## 🎯 Target G.729 Variants & Test Data Mapping

### **Core G.729** (Full Complexity)
- **ITU Reference**: `/g729/c_code/`
- **Test Data**: `crates/codec-core/src/codecs/g729/itu_tests/test_data/g729/`
- **Key Test Files**:
  - Input: `SPEECH.IN`, `TAME.IN`, `PITCH.IN`, `LSP.IN`, `FIXED.IN`
  - Bitstreams: `SPEECH.BIT`, `TAME.BIT`, `PITCH.BIT`, `LSP.BIT`
  - Expected Output: `SPEECH.PST`, `TAME.PST`, `PITCH.PST`, `LSP.PST`
- **Feature Flag**: `g729-core`

### **G.729 Annex A** (Reduced Complexity)
- **ITU Reference**: `/g729AnnexA/c_code/`
- **Test Data**: `crates/codec-core/src/codecs/g729/itu_tests/test_data/g729AnnexA/`
- **Key Test Files**:
  - Input: `SPEECH.IN`, `TAME.IN`, `PITCH.IN`, `LSP.IN`, `TEST.IN`
  - Bitstreams: `SPEECH.BIT`, `TAME.BIT`, `PITCH.BIT`, `LSP.BIT`, `TEST.BIT`
  - Expected Output: `SPEECH.PST`, `TAME.PST`, `PITCH.PST`, `LSP.PST`, `TEST.PST`
- **Feature Flag**: `annex-a`

### **G.729 Annex B** (VAD/DTX/CNG)
- **ITU Reference**: `/g729AnnexB/c_codeB/`
- **Test Data**: `crates/codec-core/src/codecs/g729/itu_tests/test_data/g729AnnexB/`
- **Key Test Files**:
  - Test Sequences: `tstseq1.bin`, `tstseq2.bin`, `tstseq3.bin`, `tstseq4.bin`
  - Bitstreams: `tstseq1.bit`, `tstseq2.bit`, `tstseq3.bit`, `tstseq4.bit`, `tstseq5.bit`, `tstseq6.bit`
  - Expected Output: `tstseq1.out`, `tstseq2.out`, `tstseq3.out`, `tstseq4.out`, `tstseq5.out`, `tstseq6.out`
- **Feature Flag**: `annex-b`

### **G.729 Annex BA** (Reduced Complexity + VAD/DTX/CNG)
- **ITU Reference**: `/g729AnnexB/c_codeBA/`
- **Test Data**: `crates/codec-core/src/codecs/g729/itu_tests/test_data/g729AnnexB/` (shared with Annex B)
- **Key Test Files**:
  - Test Sequences: `tstseq1a.bit`, `tstseq2a.bit`, `tstseq3a.bit`, `tstseq4a.bit`
  - Expected Output: `tstseq1a.out`, `tstseq2a.out`, `tstseq3a.out`, `tstseq4a.out`, `tstseq5a.out`, `tstseq6a.out`
- **Feature Flags**: `["annex-a", "annex-b"]`

## 📋 Implementation Task Tracker

### ✅ **Phase 1: Foundation Components** (COMPLETED)
**Duration**: 4-6 weeks | **Status**: ✅ COMPLETED

| Task ID | Component | Status | Test Coverage | ITU Compliance |
|---------|-----------|--------|---------------|-----------------|
| **1.1** | **Basic Infrastructure** | ✅ COMPLETED | 100% | ✅ |
| 1.1.1 | Module structure and type definitions | ✅ | ✅ | ✅ |
| 1.1.2 | G729Variant enum (Core, AnnexA, AnnexB, AnnexBA) | ✅ | ✅ | ✅ |
| **1.2** | **Mathematical Foundation** | ✅ COMPLETED | 100% | ✅ |
| 1.2.1 | Fixed-point arithmetic operations | ✅ | 9/9 tests | ✅ |
| 1.2.2 | DSP utility functions | ✅ | 7/7 tests | ✅ |
| **1.3** | **Linear Predictive Coding** | ✅ COMPLETED | 100% | ✅ |
| 1.3.1 | LPC Analysis implementation | ✅ | 8/8 tests | ✅ |
| 1.3.2 | LPC to LSP conversion | ✅ | 8/8 tests | ✅ |

### 🔄 **Phase 2: Core G.729 Implementation** (IN PROGRESS)
**Duration**: 8-10 weeks | **Status**: 🔄 IN PROGRESS | **Test Data**: `g729/`

| Task ID | Component | Status | Test Files | ITU Compliance |
|---------|-----------|--------|------------|-----------------|
| **2.1** | **Pitch Analysis (Full Complexity)** | 🔶 PARTIAL | `PITCH.IN/BIT/PST` | 66.7% |
| 2.1.1 | Open-loop pitch estimation | 🔶 | `PITCH.IN` → `PITCH.BIT` | 66.7% |
| 2.1.2 | Closed-loop pitch refinement | ❌ | `PITCH.IN` → `PITCH.PST` | PENDING |
| 2.1.3 | Fractional pitch interpolation | ❌ | Multi-frame validation | PENDING |
| **2.2** | **ACELP Analysis (Full Complexity)** | ✅ MAJOR FIX | `FIXED.IN/BIT/PST` | 80.0% |
| 2.2.1 | Fixed codebook search | ✅ | `FIXED.IN` → `FIXED.BIT` | 80.0% |
| 2.2.2 | Adaptive codebook construction | 🔶 | Cross-validation tests | PARTIAL |
| **2.3** | **Quantization and Coding** | 🔶 MIXED | `LSP.IN/BIT/PST` | 99.5% LSP, 0% Gain |
| 2.3.1 | LSP quantization | ✅ | `LSP.IN` → `LSP.BIT` | 99.5% |
| 2.3.2 | Gain quantization | ❌ | `SPEECH.IN` → `SPEECH.BIT` | 0% (CRITICAL) |

### 📅 **Phase 3: Core G.729 Encoder/Decoder** (NEXT)
**Duration**: 4 weeks | **Status**: ❌ PENDING | **Test Data**: `g729/`

| Task ID | Component | Status | Test Files | Target Date |
|---------|-----------|--------|------------|-------------|
| **3.1** | **Encoder Implementation** | ❌ | `SPEECH.IN` → `SPEECH.BIT` | Week 1 |
| 3.1.1 | Main encoder loop integration | ❌ | Full pipeline test | TBD |
| 3.1.2 | Preprocessing and filtering | ❌ | Audio quality validation | TBD |
| **3.2** | **Decoder Implementation** | 🔶 | `SPEECH.BIT` → `SPEECH.PST` | Week 2 |
| 3.2.1 | Main decoder loop | 🔶 | Synthesis working | PARTIAL |
| 3.2.2 | Post-processing | ❌ | Audio enhancement | TBD |

### 📅 **Phase 4: G.729A (Reduced Complexity)** (PLANNED)
**Duration**: 6 weeks | **Status**: ❌ PENDING | **Test Data**: `g729AnnexA/`

| Task ID | Component | Status | Test Files | ITU Reference |
|---------|-----------|--------|------------|---------------|
| **4.1** | **Simplified Pitch Analysis** | ❌ | `PITCH.IN/BIT/PST` | `PITCH_A.C` |
| 4.1.1 | Reduced complexity pitch search | ❌ | `g729AnnexA/PITCH.*` | TBD |
| **4.2** | **Simplified ACELP** | ❌ | `FIXED.IN/BIT/PST` | `ACELP_CA.C` |
| 4.2.1 | Adaptive codebook optimization | ❌ | `g729AnnexA/FIXED.*` | TBD |
| 4.2.2 | Correlation function optimizations | ❌ | `COR_FUNC.C` reference | TBD |
| **4.3** | **G.729A Encoder/Decoder** | ❌ | `TEST.IN/BIT/PST` | `COD_LD8A.C` |
| 4.3.1 | Reduced complexity integration | ❌ | `g729AnnexA/TEST.*` | TBD |

### 📅 **Phase 5: G.729B (VAD/DTX/CNG Extensions)** (PLANNED)
**Duration**: 6 weeks | **Status**: ❌ PENDING | **Test Data**: `g729AnnexB/`

| Task ID | Component | Status | Test Files | ITU Reference |
|---------|-----------|--------|------------|---------------|
| **5.1** | **Voice Activity Detection (VAD)** | ❌ | `tstseq1-6.*` | `VAD.C` |
| 5.1.1 | VAD algorithm implementation | ❌ | Speech/silence detection | TBD |
| 5.1.2 | VAD parameter computation | ❌ | Threshold adaptation | TBD |
| **5.2** | **Discontinuous Transmission (DTX)** | ❌ | `tstseq1-6.*` | `DTX.C` |
| 5.2.1 | DTX control and SID frame generation | ❌ | SID frame validation | TBD |
| 5.2.2 | SID parameter quantization | ❌ | `QSIDGAIN.C/QSIDLSF.C` | TBD |
| **5.3** | **Comfort Noise Generation (CNG)** | ❌ | `tstseq1-6.out` | `DEC_SID.C` |
| 5.3.1 | CNG synthesis | ❌ | Background noise quality | TBD |
| 5.3.2 | Background noise estimation | ❌ | Spectral parameter generation | TBD |

### 📅 **Phase 6: G.729BA (Annex A + B Combined)** (PLANNED)
**Duration**: 3 weeks | **Status**: ❌ PENDING | **Test Data**: `g729AnnexB/`

| Task ID | Component | Status | Test Files | ITU Reference |
|---------|-----------|--------|------------|---------------|
| **6.1** | **G.729BA Integration** | ❌ | `tstseq*a.*` files | `c_codeBA/` |
| 6.1.1 | Combine Annex A + B algorithms | ❌ | `tstseq1a.bit` → `tstseq1a.out` | TBD |
| 6.1.2 | Unified encoder/decoder | ❌ | All AnnexBA test sequences | TBD |
| 6.1.3 | Performance optimization | ❌ | Computational efficiency | TBD |

### 📅 **Phase 7: Integration and Testing** (PLANNED)
**Duration**: 4 weeks | **Status**: ❌ PENDING

| Task ID | Component | Status | Test Coverage | Target |
|---------|-----------|--------|---------------|---------|
| **7.1** | **Multi-variant Integration** | ❌ | All variants | TBD |
| 7.1.1 | Unified API for all G.729 variants | ❌ | API compatibility | TBD |
| 7.1.2 | Runtime variant selection | ❌ | Dynamic switching | TBD |
| 7.1.3 | Feature flag validation | ❌ | Conditional compilation | TBD |
| **7.2** | **ITU Test Vector Validation** | ❌ | All test data | 95%+ |
| 7.2.1 | Core G.729 full compliance | ❌ | `g729/` test vectors | TBD |
| 7.2.2 | G.729A full compliance | ❌ | `g729AnnexA/` test vectors | TBD |
| 7.2.3 | G.729B full compliance | ❌ | `g729AnnexB/` test vectors | TBD |
| 7.2.4 | G.729BA cross-validation | ❌ | Combined functionality | TBD |

## 🚨 **Critical Issues Requiring Immediate Attention**

### **Priority 1: Gain Quantization Fix** 🔥
**Current Status**: 0% compliance - Causing decoder silence
**Task**: 2.3.2 Gain quantization
**Test Files**: `g729/SPEECH.IN` → `g729/SPEECH.BIT` → `g729/SPEECH.PST`
**Issue**: Encoder producing incorrect gain indices (always 0)
**Impact**: Decoder generates silence instead of speech
**Target**: Fix within 1 week

### **Priority 2: Synthesis Filter Energy Preservation** 🔥
**Current Status**: Energy ratio 0.157 (15.7%) vs target 50-200%
**Task**: 3.2.1 Main decoder loop
**Test Files**: All `.PST` output files
**Issue**: 84% energy loss during synthesis
**Impact**: Quiet, distorted audio output
**Target**: Achieve 90%+ energy preservation

### **Priority 3: Complete Core G.729 Pipeline** ⚡
**Current Status**: Partial decoder, no integrated encoder
**Tasks**: 3.1.1, 3.1.2, 3.2.2
**Test Files**: Full `g729/SPEECH.IN` → `g729/SPEECH.BIT` → `g729/SPEECH.PST` pipeline
**Target**: End-to-end Core G.729 functionality

## 📊 **Current Quality Metrics** (December 2024)

### **Overall ITU Compliance: 61.5%** 
| Component | Status | Quality Score | Test Data Used |
|-----------|--------|---------------|----------------|
| **ACELP Search** | ✅ FIXED | **80.0%** | `g729/FIXED.*` |
| **LSP Quantization** | ✅ EXCELLENT | **99.5%** | `g729/LSP.*` |
| **Pitch Analysis** | 🔶 PARTIAL | **66.7%** | `g729/PITCH.*` |
| **Synthesis Filter** | ❌ CRITICAL | **0.0%** | All `.PST` outputs |
| **Gain Quantization** | ❌ BROKEN | **0.0%** | `g729/SPEECH.*` |

### **Energy Preservation Performance**
- **Pipeline Energy Ratio**: 15.7% (Target: 50-200%)
- **Output Energy**: 1,965,960 (vs input 12,497,746)
- **Max Amplitude**: 4,347 (good dynamic range)
- **Non-trivial Samples**: 76/80 (95% meaningful audio)

## 🎯 **Success Criteria by Phase**

### **Phase 2 Completion (Core G.729)**
- [ ] Gain quantization producing correct indices
- [ ] Synthesis filter preserving 90%+ energy  
- [ ] Pitch analysis achieving 80%+ accuracy
- [ ] End-to-end encoder/decoder pipeline working
- [ ] All `g729/` test vectors passing with 90%+ compliance

### **Phase 4 Completion (G.729A)**
- [ ] Reduced complexity algorithms implemented
- [ ] 40% computational improvement vs Core G.729
- [ ] All `g729AnnexA/` test vectors passing
- [ ] Quality maintained relative to full complexity

### **Phase 5 Completion (G.729B)**
- [ ] VAD correctly detecting speech/silence
- [ ] DTX reducing transmission during silence
- [ ] CNG providing natural background noise
- [ ] All `g729AnnexB/tstseq*.bit` → `g729AnnexB/tstseq*.out` passing

### **Phase 6 Completion (G.729BA)**
- [ ] Combined Annex A + B functionality
- [ ] All `g729AnnexB/tstseq*a.bit` → `g729AnnexB/tstseq*a.out` passing
- [ ] Optimal performance/quality balance achieved

### **Final Success Criteria**
- [ ] **95%+ ITU compliance** for all variants
- [ ] **Bit-exact compatibility** with ITU reference implementations
- [ ] **Real-time performance** capability
- [ ] **Memory usage** <100KB per codec instance
- [ ] **Feature flags** working for selective compilation

## 🔧 **Implementation Commands**

### **Run Tests by Variant**
```bash
# Core G.729 tests using g729/ test data
cargo test basic_itu_test --verbose -- --nocapture

# G.729A tests using g729AnnexA/ test data  
cargo test test_g729a_encoder_compliance --verbose -- --nocapture

# G.729B tests using g729AnnexB/ test data
cargo test test_g729b_encoder_compliance --verbose -- --nocapture

# Energy preservation tests (cross-variant)
cargo test energy_preservation --verbose -- --nocapture
```

### **Quality Evaluation Commands**
```bash
# Current quality scores
cargo test quality_evaluation --verbose -- --nocapture

# Individual component testing
cargo test test_acelp_search_quality --verbose -- --nocapture
cargo test test_pitch_analysis_quality --verbose -- --nocapture
cargo test test_synthesis_filter_quality --verbose -- --nocapture
```

## 📅 **Timeline & Milestones**

| Phase | Duration | Start Date | Target Completion | Critical Dependencies |
|-------|----------|------------|-------------------|----------------------|
| **Phase 1** | 6 weeks | ✅ COMPLETED | ✅ COMPLETED | Foundation complete |
| **Phase 2** | 10 weeks | ✅ STARTED | Week 4 (Critical fixes) | Gain quantization fix |
| **Phase 3** | 4 weeks | Week 5 | Week 8 | Phase 2 completion |
| **Phase 4** | 6 weeks | Week 9 | Week 14 | Core G.729 stable |
| **Phase 5** | 6 weeks | Week 15 | Week 20 | G.729A complete |
| **Phase 6** | 3 weeks | Week 21 | Week 23 | G.729B complete |
| **Phase 7** | 4 weeks | Week 24 | Week 27 | All variants stable |

**Total Estimated Duration**: 27 weeks from Phase 1 start
**Current Status**: Week 16 of Phase 2 (Major progress with critical fixes needed)

---
**Last Updated**: December 2024 - Added comprehensive task tracking and test data mapping
**Next Milestone**: Fix gain quantization (Task 2.3.2) using `g729/SPEECH.*` test files
**Priority Focus**: Achieve 90%+ ITU compliance for Core G.729 before advancing to Annex variants

---
**Status Legend**: ✅ Completed | 🔄 In Progress | 🔶 Partial | ❌ Pending | 🔥 Critical 