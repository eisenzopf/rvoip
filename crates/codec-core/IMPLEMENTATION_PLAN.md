# Codec-Core G.722 ITU-T Compliance Implementation Plan

## Current Status: G.722 ITU-T Reference Implementation

### Overview

The primary goal is to achieve **100% ITU-T G.722 compliance** by implementing the exact reference algorithm from the official ITU-T G.722 specification (T-REC-G.722-201209). No shortcuts, no approximations, no mocks - this is about bit-exact compliance with the international standard.

### Problem Statement

Current G.722 implementation has:
- ❌ **93 compilation errors** preventing any testing
- ❌ **API incompatibility** - G722Codec doesn't implement AudioCodec trait  
- ❌ **Broken test framework** - integration tests fail to compile
- ❌ **No ITU-T test vector integration** - not using official compliance tests
- ❌ **Unknown compliance level** - cannot verify against ITU-T reference

### Reference Implementation Sources

1. **Primary Reference**: `T-REC-G.722-201209/Software/G.722-Appendix-IV_v3.0/`
   - **Source**: `funcg722.c` - Complete ITU-T G.722 reference implementation
   - **Headers**: `g722.h`, `funcg722.h` - ITU-T data structures and prototypes
   - **Test Vectors**: `testvectors/TV/` - Official ITU-T compliance test files
   - **Documentation**: `00Readme-G.722-App.IV-v3.00.txt` - Implementation notes

2. **Secondary Reference**: `ezk-media/crates/ezk-g722/`
   - **Proven Implementation**: Based on SpanDSP/libg722 (battle-tested)
   - **Clean Architecture**: Proper encoder/decoder separation
   - **Working API**: Functional integration patterns

### Solution Strategy

**Phase 1: Fix Compilation and API Integration**
- Fix all 93 compilation errors
- Implement proper AudioCodec trait
- Create working test framework
- Establish baseline functionality

**Phase 2: ITU-T Reference Implementation**
- Implement exact ITU-T algorithm from funcg722.c
- Use official ITU-T tables and constants
- Maintain bit-exact compliance
- Comprehensive state management

**Phase 3: ITU-T Test Vector Validation**
- Integrate official test vectors (test10.bst, test20.bst, ovfl.bst)
- Achieve 100% compliance on all test cases
- Debug and fix any precision differences
- Validate across all G.722 modes (1, 2, 3)

**Phase 4: Performance and Extensions**
- Add PLC (Packet Loss Concealment) from ITU-T reference
- Support super-wideband extensions if needed
- Performance optimizations while maintaining compliance

## Detailed G.722 Architecture

### ITU-T G.722 Algorithm Components

Based on the official ITU-T reference implementation:

```
G.722 Encoder:
Input PCM (16kHz) -> QMF Analysis -> Low/High Band Split
                                   -> ADPCM Encode (Low) -> 6-bit quantization
                                   -> ADPCM Encode (High) -> 2-bit quantization
                                   -> Bit Packing -> G.722 Bitstream

G.722 Decoder:  
G.722 Bitstream -> Bit Unpacking -> Low/High Band Codes
                                 -> ADPCM Decode (Low) -> Low Band Signal
                                 -> ADPCM Decode (High) -> High Band Signal
                                 -> QMF Synthesis -> Reconstructed PCM
```

### Core ITU-T Functions (from funcg722.c)

1. **QMF Functions**:
   - `qmf_tx()` - QMF analysis filter (encoder)
   - `qmf_rx()` - QMF synthesis filter (decoder)
   - `qmf_rx_buf()` - Optimized synthesis with buffer management

2. **Low-Band ADPCM**:
   - `lsbcod()` - Low sub-band encoding
   - `lsbdec()` - Low sub-band decoding
   - `quantl()` - Low-band quantization
   - `invqal()` - Inverse quantization (encoder)
   - `invqbl()` - Mode-dependent inverse quantization (decoder)

3. **High-Band ADPCM**:
   - `hsbcod()` - High sub-band encoding  
   - `hsbdec()` - High sub-band decoding
   - `quanth()` - High-band quantization
   - `invqah()` - High-band inverse quantization

4. **Prediction and Adaptation**:
   - `filtep()` - Pole predictor filter
   - `filtez()` - Zero predictor filter
   - `uppol1()` - First-order pole coefficient update
   - `uppol2()` - Second-order pole coefficient update
   - `upzero()` - Zero predictor coefficient update
   - `logscl()` - Low-band scale factor adaptation
   - `logsch()` - High-band scale factor adaptation
   - `scalel()` - Low-band scale factor computation
   - `scaleh()` - High-band scale factor computation

### ITU-T State Structure (from g722.h)

```rust
pub struct G722State {
    // Low-band ADPCM state
    al: [i16; 3],      // Pole predictor coefficients
    bl: [i16; 7],      // Zero predictor coefficients  
    detl: i16,         // Low-band scale factor
    dlt: [i16; 7],     // Low-band difference signal history
    nbl: i16,          // Low-band scale factor (log domain)
    plt: [i16; 3],     // Low-band predictor signals
    rlt: [i16; 3],     // Low-band reconstructed signals
    sl: i16,           // Low-band predictor output
    spl: i16,          // Low-band predictor output (previous)
    szl: i16,          // Low-band zero predictor output
    
    // High-band ADPCM state  
    ah: [i16; 3],      // High-band pole predictor coefficients
    bh: [i16; 7],      // High-band zero predictor coefficients
    deth: i16,         // High-band scale factor
    dh: [i16; 7],      // High-band difference signal history
    ph: [i16; 3],      // High-band predictor signals
    rh: [i16; 3],      // High-band reconstructed signals
    nbh: i16,          // High-band scale factor (log domain)
    sh: i16,           // High-band predictor output
    sph: i16,          // High-band predictor output (previous)
    szh: i16,          // High-band zero predictor output
    
    // QMF delay lines
    qmf_tx_delayx: [i16; 24],  // QMF analysis delay line
    qmf_rx_delayx: [i16; 24],  // QMF synthesis delay line
}
```

### ITU-T Tables and Constants

All tables implemented exactly as specified in funcg722.c:

- **ILA2[353]** - Inverse logarithmic scale factor table
- **MISIL[2][32]** - Low-band quantization mapping  
- **MISIH[2][3]** - High-band quantization mapping
- **Q6[31]** - 6-level quantizer decision levels
- **WLI[8]** - Low-band logarithmic scale factor weights
- **WHI[4]** - High-band logarithmic scale factor weights
- **RIL4/5/6[...]** - Inverse quantization index tables
- **OQ4/5/6[...]** - Quantization output tables
- **COEF_QMF[24]** - QMF filter coefficients

## Implementation Phases

### Phase 1: Fix Compilation (IMMEDIATE)

**Goal**: Get code compiling and tests running

**Tasks**:
1. **Fix AudioCodec Trait Implementation**
   ```rust
   impl AudioCodec for G722Codec {
       fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>, CodecError> {
           // Frame-based encoding (160 samples -> 80 bytes)
       }
       
       fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>, CodecError> {
           // Frame-based decoding (80 bytes -> 160 samples)
       }
       
       fn frame_size(&self) -> usize { 160 } // 10ms at 16kHz
       fn reset(&mut self) { /* Reset ITU-T state */ }
   }
   ```

2. **Fix Constructor Compatibility**
   ```rust
   impl G722Codec {
       pub fn new(config: CodecConfig) -> Result<Self, CodecError> {
           let mode = extract_mode_from_config(config)?;
           Self::new_with_mode(mode)
       }
   }
   ```

3. **Error Handling Integration**
   ```rust
   impl From<&str> for CodecError {
       fn from(s: &str) -> Self {
           CodecError::InvalidInput(s.to_string())
       }
   }
   ```

### Phase 2: ITU-T Algorithm Implementation (PRIORITY)

**Goal**: Exact ITU-T reference implementation

**Tasks**:
1. **Direct C-to-Rust Translation**
   - Translate each ITU-T function exactly from funcg722.c
   - Maintain exact arithmetic precision
   - Use exact ITU-T variable names and logic flow

2. **State Management**
   - Implement exact G722State structure from g722.h
   - Proper initialization as per ITU-T reset behavior
   - State consistency across encode/decode operations

3. **Bit-Exact Operations**
   - Use ITU-T arithmetic functions (add, mult, shr, shl, etc.)
   - Maintain exact overflow and saturation behavior
   - Preserve ITU-T rounding and precision

### Phase 3: ITU-T Test Vector Integration (VALIDATION)

**Goal**: 100% compliance with official ITU-T test vectors

**Tasks**:
1. **Test Vector Parser**
   ```rust
   fn load_g192_bitstream(path: &str) -> Result<Vec<u8>, Error> {
       // Parse G.192 format bitstream files
   }
   
   fn load_reference_output(path: &str) -> Result<Vec<i16>, Error> {
       // Load 16-bit little-endian reference output
   }
   ```

2. **Compliance Test Framework**
   ```rust
   #[test]
   fn test_itu_compliance_test10() {
       let input = load_g192_bitstream("test10.bst").unwrap();
       let expected = load_reference_output("test10.out").unwrap();
       
       let mut codec = G722Codec::new_with_mode(1).unwrap();
       let decoded = codec.decode(&input).unwrap();
       
       assert_eq!(decoded, expected, "Failed ITU-T test10 compliance");
   }
   ```

3. **Debug Framework**
   ```rust
   fn compare_sample_by_sample(our: &[i16], reference: &[i16]) -> ComparisonReport {
       // Detailed sample-by-sample analysis
       // Identify exact point of divergence
       // State debugging and arithmetic tracing
   }
   ```

### Phase 4: Advanced Features (EXTENSIONS)

**Goal**: Complete ITU-T G.722 feature support

**Tasks**:
1. **PLC Implementation** (from g722_plc.c)
2. **Super-wideband Extensions** (Annexes B, C, D)
3. **Performance Optimizations** (while maintaining compliance)

## Success Criteria

### Phase 1 Success
- ✅ Zero compilation errors
- ✅ All tests compile and run
- ✅ Basic encode/decode functionality works

### Phase 2 Success  
- ✅ Bit-exact ITU-T algorithm implementation
- ✅ Proper state management and initialization
- ✅ All ITU-T functions translated correctly

### Phase 3 Success
- ✅ **100% ITU-T compliance** on test10.bst
- ✅ **100% ITU-T compliance** on test20.bst  
- ✅ **100% ITU-T compliance** on ovfl.bst
- ✅ All G.722 modes (1, 2, 3) pass compliance tests

### Phase 4 Success
- ✅ PLC functionality working
- ✅ Extended features as needed
- ✅ Performance meets requirements

## Risk Mitigation

1. **Compilation Failures**: Use ezk-media patterns for API design
2. **Arithmetic Precision**: Use ITU-T exact arithmetic operations
3. **State Management**: Follow ITU-T reference exactly  
4. **Test Vector Failures**: Debug sample-by-sample with reference
5. **Performance Issues**: Optimize after compliance is achieved

## Timeline

- **Week 1**: Fix compilation, implement basic API
- **Week 2**: ITU-T algorithm implementation 
- **Week 3**: Test vector integration and debugging
- **Week 4**: Achieve 100% ITU-T compliance
- **Week 5**: Advanced features and optimizations

## Resources

- **ITU-T Reference**: T-REC-G.722-201209/Software/G.722-Appendix-IV_v3.0/
- **Proven Implementation**: ezk-media/crates/ezk-g722/
- **Test Vectors**: Official ITU-T compliance test files
- **Documentation**: ITU-T G.722 specification and appendices

This plan prioritizes **correctness over speed**, **compliance over convenience**, and **precision over approximation**. The goal is to create the definitive ITU-T G.722 implementation that serves as the gold standard for all G.722 needs in the RVOIP ecosystem. 