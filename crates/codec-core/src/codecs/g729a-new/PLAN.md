# G.729 Annex A Implementation Plan

This plan outlines the steps to implement the G.729 Annex A specification in Rust.

## Phase 1: Encoder Implementation

- [x] **Preprocessing:** Implement the pre-processing stage of the encoder, including high-pass filtering and scaling.
- [x] **LPC Analysis:** Implement the Linear Prediction (LP) analysis, including windowing, autocorrelation, and Levinson-Durbin recursion.
- [x] **LSP Conversion and Quantization:** Implement the conversion from LP coefficients to Line Spectrum Pairs (LSPs) and their quantization.
- [x] **Perceptual Weighting:** Implement the perceptual weighting filter.
- [x] **Open-Loop Pitch Analysis:** Implement the open-loop pitch analysis to estimate the pitch delay.
- [x] **Impulse Response Computation:** Implement the computation of the impulse response of the weighted synthesis filter.
- [x] **Target Signal Computation:** Implement the computation of the target signal for the adaptive and fixed codebook search.
- [ ] **Adaptive Codebook Search:** Implement the adaptive codebook search to find the optimal pitch lag and gain.
- [ ] **Fixed Codebook Search:** Implement the fixed codebook search to find the optimal innovation vector.
- [ ] **Gain Quantization:** Implement the quantization of the adaptive and fixed codebook gains.
- [ ] **Memory Update:** Implement the memory update of the encoder.

## Phase 2: Decoder Implementation

- [ ] **Parameter Decoding:** Implement the decoding of the transmitted parameters.
- [ ] **Adaptive Codebook Vector Generation:** Implement the generation of the adaptive codebook vector.
- [ ] **Fixed Codebook Vector Generation:** Implement the generation of the fixed codebook vector.
- [ ] **Speech Synthesis:** Implement the synthesis of the speech signal by filtering the excitation through the LP synthesis filter.
- [ ] **Post-processing:** Implement the post-processing stage of the decoder, including the adaptive postfilter, high-pass filtering, and upscaling.

## Phase 3: Testing and Compliance

- [ ] **Test Vector Verification:** Write and run tests to verify the implementation against the provided test vectors.
- [ ] **Compliance Testing:** Ensure that the implementation passes all compliance tests.
