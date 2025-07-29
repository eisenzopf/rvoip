# G.729A Codec Implementation Design for Rust

## Overview

This document outlines the design for a Rust implementation of the G.729 Annex A (G.729A) speech codec. G.729A is a reduced-complexity version of the G.729 codec that operates at 8 kbit/s using Conjugate-Structure Algebraic-Code-Excited Linear Prediction (CS-ACELP).

## Key Specifications

- **Bit rate**: 8 kbit/s
- **Frame size**: 10 ms (80 samples at 8 kHz)
- **Look-ahead**: 5 ms (40 samples)
- **Algorithm**: CS-ACELP with reduced complexity
- **Fixed-point arithmetic**: Q-format notation for precision

## Architecture Philosophy

Our implementation follows a modular, trait-based design that emphasizes:
1. Clear separation of concerns
2. Type safety through Rust's type system
3. Zero-copy operations where possible
4. Configurable precision (fixed-point with future floating-point support)

## Library Structure

```
g729a/
├── src/
│   ├── lib.rs                    # Main library interface
│   ├── constants.rs              # Codec constants and parameters
│   ├── types.rs                  # Common types and structures
│   ├── math/                     # Mathematical operations
│   │   ├── mod.rs
│   │   ├── fixed_point.rs        # Fixed-point arithmetic
│   │   ├── dsp_operations.rs     # DSP primitives
│   │   └── polynomial.rs         # Polynomial operations
│   ├── signal/                   # Signal processing
│   │   ├── mod.rs
│   │   ├── preprocessor.rs      # High-pass filtering
│   │   ├── windowing.rs         # Window functions
│   │   └── correlation.rs       # Correlation computations
│   ├── spectral/                 # Spectral analysis
│   │   ├── mod.rs
│   │   ├── linear_prediction.rs  # LP analysis
│   │   ├── lsp_converter.rs     # LP to LSP conversion
│   │   ├── quantizer.rs         # LSP quantization
│   │   └── interpolator.rs      # Parameter interpolation
│   ├── perception/               # Perceptual processing
│   │   ├── mod.rs
│   │   ├── weighting_filter.rs  # Perceptual weighting
│   │   └── pitch_tracker.rs     # Pitch detection
│   ├── excitation/               # Excitation generation
│   │   ├── mod.rs
│   │   ├── adaptive_codebook.rs # Adaptive codebook search
│   │   ├── algebraic_codebook.rs # Fixed codebook search
│   │   └── gain_processor.rs    # Gain quantization
│   ├── synthesis/                # Signal synthesis
│   │   ├── mod.rs
│   │   ├── filter_bank.rs       # Synthesis filtering
│   │   └── postprocessor.rs     # Post-filtering
│   ├── codec/                    # Main codec implementation
│   │   ├── mod.rs
│   │   ├── encoder.rs           # Encoder implementation
│   │   ├── decoder.rs           # Decoder implementation
│   │   └── bitstream.rs         # Bitstream handling
│   └── tables/                   # Lookup tables
│       ├── mod.rs
│       ├── lsp_tables.rs        # LSP quantization tables
│       ├── gain_tables.rs       # Gain quantization tables
│       └── codebook_tables.rs   # Codebook tables
├── tests/
│   ├── integration_tests.rs
│   └── compliance_tests.rs
├── benches/
│   └── performance.rs
└── examples/
    ├── encode_decode.rs
    └── streaming.rs
```

## Module Detailed Design

### 1. Core Types (`types.rs`)

```rust
// Frame and sample types
pub struct AudioFrame {
    samples: [i16; FRAME_SIZE],
    timestamp: u64,
}

pub struct SubFrame {
    samples: [i16; SUBFRAME_SIZE],
}

// Fixed-point number representation
#[derive(Copy, Clone, Debug)]
pub struct Q15(i16);  // Q0.15 format
#[derive(Copy, Clone, Debug)]
pub struct Q31(i32);  // Q0.31 format

// Codec parameters
pub struct SpectralParameters {
    lsp_coefficients: [Q15; LP_ORDER],
    quantized_indices: [u8; 4],
}

pub struct ExcitationParameters {
    pitch_delay: f32,
    pitch_gain: Q15,
    codebook_index: u32,
    codebook_gain: Q15,
}

// Bitstream representation
pub struct EncodedFrame {
    bits: [u8; 10], // 80 bits packed
}
```

### 2. Mathematical Operations (`math/`)

#### `fixed_point.rs`
```rust
// Fixed-point operations with overflow protection
pub trait FixedPointOps {
    fn saturating_add(self, other: Self) -> Self;
    fn saturating_mul(self, other: Self) -> Self;
    fn to_q15(self) -> Q15;
    fn to_q31(self) -> Q31;
}

// Core DSP functions
pub fn inverse_sqrt(x: Q31) -> Q15 {
    // Newton-Raphson approximation
    // Returns Q15 result for Q31 input
}

pub fn log2_approximation(x: Q31) -> (i16, Q15) {
    // Returns (exponent, mantissa)
    // Exponent in integer, mantissa in Q15
}

pub fn power2_approximation(exp: i16, mantissa: Q15) -> Q31 {
    // Inverse of log2_approximation
}
```

#### `dsp_operations.rs`
```rust
// Efficient DSP primitives
pub fn autocorrelation(signal: &[Q15], order: usize) -> Vec<Q31> {
    // Compute autocorrelation with lag windowing
    // Returns correlation values in Q31 format
}

pub fn convolution(x: &[Q15], h: &[Q15]) -> Vec<Q15> {
    // Optimized convolution for filter operations
}

pub fn dot_product(a: &[Q15], b: &[Q15]) -> Q31 {
    // Inner product with accumulation in Q31
}
```

### 3. Signal Processing (`signal/`)

#### `preprocessor.rs`
```rust
pub struct SignalPreprocessor {
    hp_filter_state: [Q15; 2],
}

impl SignalPreprocessor {
    pub fn new() -> Self {
        // Initialize high-pass filter
        // Cutoff at 140 Hz
    }
    
    pub fn process_frame(&mut self, input: &AudioFrame) -> AudioFrame {
        // Apply high-pass filtering
        // H(z) = (1 - z^-1) / (1 - 0.93*z^-1)
        // Remove DC offset and low-frequency noise
    }
}
```

#### `windowing.rs`
```rust
pub struct HammingWindow {
    coefficients: [Q15; WINDOW_SIZE],
}

impl HammingWindow {
    pub fn apply(&self, signal: &[Q15]) -> Vec<Q15> {
        // Apply asymmetric Hamming window
        // Different windows for LP analysis
    }
}
```

### 4. Spectral Analysis (`spectral/`)

#### `linear_prediction.rs`
```rust
pub struct LinearPredictor {
    autocorr_window: HammingWindow,
}

impl LinearPredictor {
    pub fn analyze(&self, windowed_signal: &[Q15]) -> LPCoefficients {
        // 1. Compute autocorrelation
        let correlations = autocorrelation(windowed_signal, LP_ORDER + 1);
        
        // 2. Apply lag windowing
        let windowed_corr = self.apply_lag_window(&correlations);
        
        // 3. Levinson-Durbin recursion
        let coefficients = self.levinson_durbin(&windowed_corr);
        
        // 4. Bandwidth expansion
        self.expand_bandwidth(&coefficients)
    }
    
    fn levinson_durbin(&self, r: &[Q31]) -> [Q15; LP_ORDER] {
        // Solve Toeplitz system efficiently
        // Returns LP coefficients
    }
}

pub struct LPCoefficients {
    values: [Q15; LP_ORDER],
    reflection_coeffs: [Q15; LP_ORDER],
}
```

#### `lsp_converter.rs`
```rust
pub struct LSPConverter {
    chebyshev_grid: [Q15; GRID_POINTS],
}

impl LSPConverter {
    pub fn lp_to_lsp(&self, lp_coeffs: &LPCoefficients) -> LSPParameters {
        // 1. Form sum and difference polynomials
        let (f1, f2) = self.form_polynomials(lp_coeffs);
        
        // 2. Find roots using Chebyshev polynomial evaluation
        let roots = self.find_polynomial_roots(&f1, &f2);
        
        // 3. Convert to LSP frequencies
        self.roots_to_lsp(&roots)
    }
    
    pub fn lsp_to_lp(&self, lsp: &LSPParameters) -> LPCoefficients {
        // Inverse conversion for synthesis
    }
    
    fn find_polynomial_roots(&self, f1: &[Q15], f2: &[Q15]) -> Vec<Q15> {
        // Grid search with interval bisection
        // Reduced complexity: 50 evaluation points
    }
}

pub struct LSPParameters {
    frequencies: [Q15; LP_ORDER],
}
```

#### `quantizer.rs`
```rust
pub struct LSPQuantizer {
    codebooks: LSPCodebooks,
    predictor: LSPPredictor,
}

impl LSPQuantizer {
    pub fn quantize(&mut self, lsp: &LSPParameters) -> QuantizedLSP {
        // 1. Compute residual from prediction
        let residual = self.compute_residual(lsp);
        
        // 2. Two-stage vector quantization
        let stage1_idx = self.vq_stage1(&residual[0..5]);
        let stage2_idx = self.vq_stage2(&residual[5..10]);
        
        // 3. Update predictor state
        self.predictor.update(&quantized_lsp);
        
        QuantizedLSP {
            indices: [stage1_idx, stage2_idx],
            reconstructed: quantized_lsp,
        }
    }
    
    fn vq_stage1(&self, residual: &[Q15]) -> u8 {
        // 7-bit codebook search
        // Weighted MSE distortion measure
    }
}

pub struct QuantizedLSP {
    indices: [u8; 4],
    reconstructed: LSPParameters,
}
```

### 5. Perception Module (`perception/`)

#### `weighting_filter.rs`
```rust
pub struct PerceptualWeightingFilter {
    gamma: Q15, // Fixed at 0.75 for G.729A
}

impl PerceptualWeightingFilter {
    pub fn create_filter(&self, lp_coeffs: &LPCoefficients) -> WeightedFilter {
        // W(z) = A(z) / A(z/gamma)
        // Simplified to 1/A(z/gamma) for G.729A
        let denominator = self.apply_bandwidth_expansion(lp_coeffs, self.gamma);
        
        WeightedFilter {
            coefficients: denominator,
        }
    }
    
    pub fn compute_impulse_response(&self, filter: &WeightedFilter) -> [Q15; SUBFRAME_SIZE] {
        // h(n) = impulse response of 1/A(z/gamma)
    }
}
```

#### `pitch_tracker.rs`
```rust
pub struct PitchTracker {
    decimation_filter: LowPassFilter,
}

impl PitchTracker {
    pub fn estimate_open_loop_pitch(&self, weighted_speech: &[Q15]) -> PitchCandidate {
        // 1. Decimate signal for efficiency
        let decimated = self.decimation_filter.process(weighted_speech);
        
        // 2. Compute normalized correlations in 3 ranges
        let candidates = [
            self.search_range(&decimated, 20, 39),   // Short delays
            self.search_range(&decimated, 40, 79),   // Medium delays
            self.search_range(&decimated, 80, 143),  // Long delays
        ];
        
        // 3. Select best with pitch doubling/halving checks
        self.select_best_pitch(&candidates)
    }
    
    fn search_range(&self, signal: &[Q15], min: u16, max: u16) -> PitchCandidate {
        // Correlation-based search
        // Use only even samples for complexity reduction
    }
}

pub struct PitchCandidate {
    delay: u16,
    correlation: Q15,
}
```

### 6. Excitation Module (`excitation/`)

#### `adaptive_codebook.rs`
```rust
pub struct AdaptiveCodebook {
    past_excitation: CircularBuffer<Q15>,
    interpolation_filter: FractionalDelayFilter,
}

impl AdaptiveCodebook {
    pub fn search(&mut self, target: &[Q15], pitch_range: &Range<f32>) -> AdaptiveContribution {
        // 1. Generate candidates with fractional delays
        // Resolution: 1/3 sample for delays < 85
        
        // 2. Simplified search: maximize correlation only
        // Not normalized by energy (G.729A simplification)
        let best_delay = self.find_best_delay(target, pitch_range);
        
        // 3. Compute excitation vector
        let excitation = self.interpolate_excitation(best_delay);
        
        AdaptiveContribution {
            delay: best_delay,
            vector: excitation,
        }
    }
    
    fn find_best_delay(&self, target: &[Q15], range: &Range<f32>) -> f32 {
        // Closed-loop search around open-loop estimate
        // Fractional precision using interpolation
    }
}

pub struct AdaptiveContribution {
    delay: f32,
    vector: [Q15; SUBFRAME_SIZE],
}
```

#### `algebraic_codebook.rs`
```rust
pub struct AlgebraicCodebook {
    structure: PulseConfiguration,
}

impl AlgebraicCodebook {
    pub fn search(&self, target: &[Q15], h: &[Q15]) -> AlgebraicContribution {
        // G.729A: 17-bit algebraic codebook
        // 4 pulses with specific position constraints
        
        // 1. Compute backward filtered target
        let d = self.compute_correlation(target, h);
        
        // 2. Compute correlation matrix (simplified)
        let phi = self.compute_phi_matrix(h);
        
        // 3. Depth-first tree search (not nested loops)
        let pulse_positions = self.tree_search(&d, &phi);
        
        AlgebraicContribution {
            positions: pulse_positions,
            signs: self.determine_signs(&d, &pulse_positions),
        }
    }
    
    fn tree_search(&self, d: &[Q15], phi: &PhiMatrix) -> [u8; 4] {
        // Iterative depth-first search
        // More efficient than full nested loop search
    }
}

pub struct AlgebraicContribution {
    positions: [u8; 4],
    signs: [bool; 4],
}
```

#### `gain_processor.rs`
```rust
pub struct GainProcessor {
    predictor: GainPredictor,
    quantizer: GainQuantizer,
}

impl GainProcessor {
    pub fn process(&mut self, adaptive: &AdaptiveContribution, 
                   algebraic: &AlgebraicContribution, 
                   target: &[Q15]) -> QuantizedGains {
        // 1. Compute optimal gains (2D optimization)
        let (gp_opt, gc_opt) = self.compute_optimal_gains(adaptive, algebraic, target);
        
        // 2. Predict gain based on past frames
        let predicted_gain = self.predictor.predict();
        
        // 3. Vector quantize the gain pair
        let quantized = self.quantizer.quantize_2d(gp_opt, gc_opt, predicted_gain);
        
        // 4. Update predictor state
        self.predictor.update(&quantized);
        
        quantized
    }
}

pub struct QuantizedGains {
    pitch_gain: Q15,
    codebook_gain: Q15,
    indices: [u8; 2],
}
```

### 7. Synthesis Module (`synthesis/`)

#### `filter_bank.rs`
```rust
pub struct SynthesisFilterBank {
    lp_filter: LPFilter,
    memory: FilterMemory,
}

impl SynthesisFilterBank {
    pub fn synthesize(&mut self, excitation: &[Q15], lp_coeffs: &LPCoefficients) -> Vec<Q15> {
        // 1. Update filter coefficients
        self.lp_filter.set_coefficients(lp_coeffs);
        
        // 2. Filter excitation through 1/A(z)
        let synthesized = self.lp_filter.process(excitation, &mut self.memory);
        
        synthesized
    }
}

pub struct FilterMemory {
    state: [Q15; LP_ORDER],
}
```

#### `postprocessor.rs`
```rust
pub struct SignalPostprocessor {
    adaptive_postfilter: AdaptivePostfilter,
    hp_filter: HighPassFilter,
    agc: AutomaticGainControl,
}

impl SignalPostprocessor {
    pub fn process(&mut self, synthesized: &[Q15], parameters: &DecodedParameters) -> Vec<i16> {
        // 1. Long-term postfilter (pitch enhancement)
        let pitch_enhanced = self.adaptive_postfilter.enhance_pitch(
            synthesized, 
            parameters.pitch_delay
        );
        
        // 2. Short-term postfilter (formant enhancement)
        let formant_enhanced = self.adaptive_postfilter.enhance_formants(
            &pitch_enhanced,
            &parameters.lp_coeffs
        );
        
        // 3. Adaptive gain control
        let gain_adjusted = self.agc.process(&formant_enhanced);
        
        // 4. High-pass filtering
        let filtered = self.hp_filter.process(&gain_adjusted);
        
        // 5. Convert to PCM samples
        self.to_pcm(&filtered)
    }
}
```

### 8. Main Codec (`codec/`)

#### `encoder.rs`
```rust
pub struct G729AEncoder {
    preprocessor: SignalPreprocessor,
    lp_analyzer: LinearPredictor,
    lsp_converter: LSPConverter,
    lsp_quantizer: LSPQuantizer,
    pitch_tracker: PitchTracker,
    weighting_filter: PerceptualWeightingFilter,
    adaptive_codebook: AdaptiveCodebook,
    algebraic_codebook: AlgebraicCodebook,
    gain_processor: GainProcessor,
    look_ahead_buffer: LookAheadBuffer,
}

impl G729AEncoder {
    pub fn new() -> Self {
        // Initialize all components
    }
    
    pub fn encode_frame(&mut self, input: &[i16]) -> Result<EncodedFrame, CodecError> {
        // 1. Preprocessing
        let preprocessed = self.preprocessor.process_frame(&AudioFrame::from_pcm(input)?);
        
        // 2. LP analysis (once per frame)
        let lp_coeffs = self.lp_analyzer.analyze(&preprocessed);
        let lsp_params = self.lsp_converter.lp_to_lsp(&lp_coeffs);
        let quantized_lsp = self.lsp_quantizer.quantize(&lsp_params);
        
        // 3. Open-loop pitch estimation
        let pitch_estimate = self.pitch_tracker.estimate_open_loop_pitch(&preprocessed);
        
        // 4. Subframe processing (2 subframes per frame)
        let mut bitstream = BitstreamWriter::new();
        
        for subframe_idx in 0..2 {
            // a. Interpolate LSP parameters
            let interpolated_lsp = self.interpolate_lsp(&quantized_lsp, subframe_idx);
            let subframe_lp = self.lsp_converter.lsp_to_lp(&interpolated_lsp);
            
            // b. Compute weighted speech and target
            let weighted_filter = self.weighting_filter.create_filter(&subframe_lp);
            let target = self.compute_target_signal(&preprocessed, &weighted_filter);
            
            // c. Adaptive codebook search
            let pitch_range = self.get_pitch_search_range(pitch_estimate, subframe_idx);
            let adaptive_contrib = self.adaptive_codebook.search(&target, &pitch_range);
            
            // d. Fixed codebook search
            let impulse_response = self.weighting_filter.compute_impulse_response(&weighted_filter);
            let algebraic_contrib = self.algebraic_codebook.search(&target, &impulse_response);
            
            // e. Gain quantization
            let gains = self.gain_processor.process(&adaptive_contrib, &algebraic_contrib, &target);
            
            // f. Update states
            self.update_excitation_buffer(&adaptive_contrib, &algebraic_contrib, &gains);
            
            // g. Pack bits
            bitstream.write_subframe_data(&adaptive_contrib, &algebraic_contrib, &gains);
        }
        
        // 5. Pack frame data
        bitstream.write_lsp_indices(&quantized_lsp);
        Ok(bitstream.to_frame())
    }
}
```

#### `decoder.rs`
```rust
pub struct G729ADecoder {
    lsp_decoder: LSPDecoder,
    lsp_converter: LSPConverter,
    excitation_generator: ExcitationGenerator,
    synthesis_filter: SynthesisFilterBank,
    postprocessor: SignalPostprocessor,
    error_concealment: ErrorConcealment,
}

impl G729ADecoder {
    pub fn new() -> Self {
        // Initialize decoder components
    }
    
    pub fn decode_frame(&mut self, encoded: &EncodedFrame) -> Result<Vec<i16>, CodecError> {
        // 1. Parse bitstream
        let parameters = BitstreamReader::parse(encoded)?;
        
        // 2. Bad frame handling
        if parameters.bad_frame_indicator {
            return self.error_concealment.conceal_frame();
        }
        
        // 3. Decode LSP parameters
        let lsp_params = self.lsp_decoder.decode(&parameters.lsp_indices);
        
        // 4. Subframe synthesis
        let mut synthesized = Vec::with_capacity(FRAME_SIZE);
        
        for subframe_idx in 0..2 {
            // a. Interpolate parameters
            let interpolated_lsp = self.interpolate_lsp(&lsp_params, subframe_idx);
            let lp_coeffs = self.lsp_converter.lsp_to_lp(&interpolated_lsp);
            
            // b. Generate excitation
            let excitation = self.excitation_generator.generate(
                &parameters.subframes[subframe_idx]
            );
            
            // c. Synthesis filtering
            let subframe_output = self.synthesis_filter.synthesize(&excitation, &lp_coeffs);
            synthesized.extend_from_slice(&subframe_output);
        }
        
        // 5. Postprocessing
        let processed = self.postprocessor.process(&synthesized, &parameters);
        
        Ok(processed)
    }
}
```

#### `bitstream.rs`
```rust
pub struct BitstreamWriter {
    bits: BitVec,
}

impl BitstreamWriter {
    pub fn write_lsp_indices(&mut self, lsp: &QuantizedLSP) {
        // L0: 7 bits (first stage)
        self.write_bits(lsp.indices[0], 7);
        // L1: 5 bits (second stage, lower)
        self.write_bits(lsp.indices[1], 5);
        // L2: 5 bits (second stage, upper)
        self.write_bits(lsp.indices[2], 5);
        // L3: 5 bits (second stage, upper)
        self.write_bits(lsp.indices[3], 5);
    }
    
    pub fn write_subframe_data(&mut self, adaptive: &AdaptiveContribution,
                               algebraic: &AlgebraicContribution,
                               gains: &QuantizedGains) {
        // Write pitch delay (8 or 5 bits depending on subframe)
        // Write algebraic codebook (17 bits)
        // Write gains indices
    }
}

pub struct BitstreamReader;

impl BitstreamReader {
    pub fn parse(frame: &EncodedFrame) -> Result<DecodedParameters, ParseError> {
        // Extract all parameters from 80-bit frame
        // Handle bit unpacking and validation
    }
}
```

## Key Algorithms Pseudo-code

### Levinson-Durbin Algorithm
```rust
// Solve Toeplitz system for LP coefficients
function levinson_durbin(r: autocorrelation) -> lp_coefficients {
    a[0] = 1.0
    k[0] = -r[1] / r[0]
    a[1] = k[0]
    alpha = r[0] + r[1] * k[0]
    
    for m in 2..=LP_ORDER {
        sum = 0
        for j in 1..m {
            sum += a[j] * r[m-j]
        }
        k[m-1] = -(r[m] + sum) / alpha
        
        for j in 1..(m/2 + 1) {
            tmp = a[j] + k[m-1] * a[m-j]
            a[m-j] = a[m-j] + k[m-1] * a[j]
            a[j] = tmp
        }
        
        a[m] = k[m-1]
        alpha *= (1 - k[m-1]^2)
    }
    
    return a[1..=LP_ORDER]
}
```

### Depth-First Algebraic Codebook Search
```rust
// Simplified tree search for G.729A
function algebraic_search(d: correlation, phi: matrix) -> pulse_positions {
    best_criterion = -infinity
    best_positions = [0; 4]
    
    // Track 0: positions 0, 5, 10, 15, 20, 25, 30, 35
    for p0 in track_0_positions {
        C0 = d[p0]^2
        
        // Track 1: positions 1, 6, 11, 16, 21, 26, 31, 36
        for p1 in track_1_positions {
            C1 = C0 + d[p1]^2 + 2*d[p0]*d[p1]*phi[p0][p1]
            
            // Track 2: positions 2, 7, 12, 17, 22, 27, 32, 37
            for p2 in track_2_positions {
                C2 = C1 + d[p2]^2 + 2*(d[p0]*d[p2]*phi[p0][p2] + 
                                       d[p1]*d[p2]*phi[p1][p2])
                
                // Early termination if not promising
                if C2 < threshold * best_criterion {
                    continue
                }
                
                // Track 3: positions 3, 8, 13, 18, 23, 28, 33, 38
                //          4, 9, 14, 19, 24, 29, 34, 39
                for p3 in track_3_positions {
                    C3 = C2 + d[p3]^2 + 2*(d[p0]*d[p3]*phi[p0][p3] +
                                           d[p1]*d[p3]*phi[p1][p3] +
                                           d[p2]*d[p3]*phi[p2][p3])
                    
                    if C3 > best_criterion {
                        best_criterion = C3
                        best_positions = [p0, p1, p2, p3]
                    }
                }
            }
        }
    }
    
    return best_positions
}
```

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("Invalid frame size: expected {expected}, got {actual}")]
    InvalidFrameSize { expected: usize, actual: usize },
    
    #[error("Invalid sample rate: {0} Hz (expected 8000 Hz)")]
    InvalidSampleRate(u32),
    
    #[error("Bitstream corruption detected")]
    BitstreamCorruption,
    
    #[error("Overflow in fixed-point arithmetic")]
    ArithmeticOverflow,
    
    #[error("Invalid codec state")]
    InvalidState,
}
```

## Testing Strategy

1. **Unit Tests**: Test each module in isolation
2. **Integration Tests**: Test complete encode/decode cycle
3. **Compliance Tests**: Verify bit-exact compatibility with reference
4. **Fuzzing**: Test robustness against malformed input
5. **Performance Benchmarks**: Ensure real-time capability

## Performance Considerations

1. **SIMD Optimization**: Use platform-specific SIMD for critical loops
2. **Lookup Tables**: Pre-compute expensive operations
3. **Memory Layout**: Optimize cache usage with data locality
4. **Fixed-Point**: Avoid expensive divisions and square roots
5. **Parallelization**: Process independent subframes concurrently

## Future Enhancements

1. **VAD/DTX Support**: Add Annex B functionality
2. **Floating-Point Mode**: Alternative implementation for non-embedded
3. **WebAssembly Target**: Browser-based codec
4. **Hardware Acceleration**: DSP/FPGA offloading interfaces
5. **Extended Bit Rates**: Support for Annex D (6.4 kbit/s) and E (11.8 kbit/s) 