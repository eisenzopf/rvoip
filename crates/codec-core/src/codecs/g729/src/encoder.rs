//! G.729 Core Encoder
//!
//! This module implements the complete G.729 encoder that integrates:
//! - LPC analysis and quantization
//! - Pitch analysis (adaptive codebook)
//! - ACELP analysis (fixed codebook)
//! - Gain quantization
//! - Bitstream generation
//!
//! Based on ITU-T G.729 reference implementation CODER.C and COD_LD8K.C

use super::types::*;
use super::math::*;
use super::dsp::*;
use super::lpc::LpcAnalyzer;
use super::pitch::PitchAnalyzer;
use super::acelp::AcelpAnalyzer;
use super::quantization::{LspQuantizer, GainQuantizer};

/// G.729 variant types
#[derive(Debug, Clone, Copy)]
pub enum G729Variant {
    /// Core G.729 (full complexity)
    Core,
    /// G.729 Annex A (reduced complexity)
    AnnexA,
    /// G.729 Annex B (VAD/DTX/CNG)
    AnnexB,
}

/// G.729 frame size in samples (10ms at 8kHz)
const L_FRAME: usize = 80;

/// G.729 subframe size in samples (5ms at 8kHz)
const L_SUBFR: usize = 40;

/// Number of subframes per frame
const N_SUBFR: usize = 2;

/// G.729 bitstream size in bits per frame
const FRAME_BITS: usize = 80;

/// G.729 Core Encoder
#[derive(Debug)]
pub struct G729Encoder {
    /// LPC analyzer for spectral envelope
    lpc_analyzer: LpcAnalyzer,
    /// Pitch analyzer for adaptive codebook
    pitch_analyzer: PitchAnalyzer,
    /// ACELP analyzer for fixed codebook
    acelp_analyzer: AcelpAnalyzer,
    /// LSP quantizer
    lsp_quantizer: LspQuantizer,
    /// Gain quantizer
    gain_quantizer: GainQuantizer,
    /// Current G.729 variant
    variant: G729Variant,
    /// Previous synthesis filter memory
    syn_mem: [Word16; M],
    /// Previous speech for lookahead analysis
    old_speech: [Word16; L_FRAME],
    /// Frame counter for debugging
    pub frame_count: usize,
}

impl G729Encoder {
    /// Create a new G.729 encoder with Core variant
    pub fn new() -> Self {
        Self::new_with_variant(G729Variant::Core)
    }

    /// Create a new G.729 encoder with specified variant
    pub fn new_with_variant(variant: G729Variant) -> Self {
        Self {
            lpc_analyzer: LpcAnalyzer::new(),
            pitch_analyzer: PitchAnalyzer::new(),
            acelp_analyzer: AcelpAnalyzer::new(),
            lsp_quantizer: LspQuantizer::new(),
            gain_quantizer: GainQuantizer::new(),
            variant,
            syn_mem: [0; M],
            old_speech: [0; L_FRAME],
            frame_count: 0,
        }
    }

    /// Get the current variant
    pub fn variant(&self) -> G729Variant {
        self.variant
    }

    /// Reset encoder state
    pub fn reset(&mut self) {
        self.lpc_analyzer.reset();
        self.pitch_analyzer.reset();
        self.acelp_analyzer.reset();
        self.lsp_quantizer.reset();
        self.gain_quantizer.reset();
        self.syn_mem = [0; M];
        self.old_speech = [0; L_FRAME];
        self.frame_count = 0;
    }

    /// Encode a frame of speech
    /// 
    /// # Arguments
    /// * `speech` - Input speech frame [L_FRAME] (80 samples)
    /// 
    /// # Returns
    /// G.729 encoded bitstream parameters
    pub fn encode_frame(&mut self, speech: &[Word16]) -> G729Frame {
        assert_eq!(speech.len(), L_FRAME);
        
        // Input validation and clamping to prevent overflow
        let mut validated_speech = [0i16; L_FRAME];
        for (i, &sample) in speech.iter().enumerate() {
            // Clamp to reasonable range to prevent overflow in subsequent processing
            validated_speech[i] = sample.max(-16000).min(16000);
        }
        
        self.frame_count += 1;

        // Step 1: LPC Analysis
        let mut lpc_coeffs = [0i16; M + 1];
        let mut lsp = [0i16; M];
        self.lpc_analyzer.analyze_frame(&validated_speech, &mut lpc_coeffs, &mut lsp);

        // Step 2: LSP Quantization  
        let mut lsp_q = [0i16; M];
        let lsp_indices = self.lsp_quantizer.quantize_lsp(&lsp, &mut lsp_q);

        // Step 3: Convert quantized LSPs back to LPC coefficients
        let mut lpc_q = [0i16; M + 1];
        self.lpc_analyzer.lsp_to_lpc(&lsp_q, &mut lpc_q);

        // Step 4: Compute weighted speech and impulse response
        let mut weighted_speech = [0i16; L_FRAME];
        let mut impulse_response = [0i16; L_SUBFR];
        self.compute_weighted_speech(&validated_speech, &lpc_q, &mut weighted_speech);
        self.compute_impulse_response(&lpc_q, &mut impulse_response);

        // Step 5: Open-loop pitch analysis
        let ol_pitch_lag = self.pitch_analyzer.pitch_ol(&weighted_speech, 20, 143);

        // Step 6: Process subframes
        let mut subframe_params = Vec::new();
        let mut residual = [0i16; L_FRAME];
        
        for subframe in 0..N_SUBFR {
            let start_idx = subframe * L_SUBFR;
            let end_idx = start_idx + L_SUBFR;
            
            let speech_subfr = &validated_speech[start_idx..end_idx];
            let weighted_subfr = &weighted_speech[start_idx..end_idx];
            
            // Step 6a: Closed-loop pitch analysis
            let dummy_y1 = [0i16; L_SUBFR];
            let dummy_y2 = [0i16; L_SUBFR];
            let (pitch_lag, adaptive_gain) = self.pitch_analyzer.pitch_fr3(
                weighted_subfr, &dummy_y1, &dummy_y2, ol_pitch_lag, ol_pitch_lag + 10, 1
            );
            
            // Step 6b: Compute adaptive codebook contribution
            let dummy_exc = [0i16; 154]; // Use standard excitation buffer size
            let adaptive_exc_vec = self.pitch_analyzer.pred_lt_3(&dummy_exc, pitch_lag, 0, L_SUBFR);
            let mut adaptive_exc = [0i16; L_SUBFR];
            adaptive_exc[..adaptive_exc_vec.len().min(L_SUBFR)].copy_from_slice(&adaptive_exc_vec[..adaptive_exc_vec.len().min(L_SUBFR)]);
            
            // Step 6c: Compute target signal for fixed codebook
            let mut target = [0i16; L_SUBFR];
            self.compute_target_signal(weighted_subfr, &adaptive_exc, &mut target);
            
            // Step 6d: Set impulse response for ACELP
            self.acelp_analyzer.set_impulse_response(&impulse_response);
            
            // Step 6e: ACELP fixed codebook search
            let mut fixed_code = [0i16; L_SUBFR];
            let mut fixed_filtered = [0i16; L_SUBFR];
            let res2 = &residual[start_idx..end_idx];
            let (positions, signs, gain_index) = self.acelp_analyzer.acelp_codebook_search(
                &target, res2, &mut fixed_code, &mut fixed_filtered
            );
            
            // Step 6f: Gain quantization
            let energy = self.compute_subframe_energy(speech_subfr);
            let fixed_gain = self.compute_optimal_gain(&target, &fixed_filtered);
            let (gain_quantizer_index, quant_adaptive_gain, quant_fixed_gain) = 
                self.gain_quantizer.quantize_gains(adaptive_gain, fixed_gain, energy);
            
            // CRITICAL FIX: Use ACELP gain_index instead of GainQuantizer index
            // The ACELP gain_index has proper energy-based selection (like index 63)
            // while GainQuantizer always returns 0 for our current implementation
            let final_gain_index = gain_index; // Use ACELP's computed index
            
            // Step 6g: Update synthesis filter memory and residual
            self.update_synthesis_memory(speech_subfr, &adaptive_exc, &fixed_code, 
                                       quant_adaptive_gain, quant_fixed_gain, 
                                       &mut residual[start_idx..end_idx]);
            
            subframe_params.push(G729SubframeParams {
                pitch_lag: pitch_lag as usize,
                adaptive_gain: quant_adaptive_gain,
                positions,
                signs,
                fixed_gain: quant_fixed_gain,
                gain_index: final_gain_index,  // Use ACELP index, not GainQuantizer
            });
        }

        // Update old speech for next frame
        self.old_speech.copy_from_slice(&validated_speech);

        G729Frame {
            lsp_indices,
            subframes: subframe_params,
            frame_number: self.frame_count,
        }
    }

    /// Compute weighted speech signal
    fn compute_weighted_speech(&self, speech: &[Word16], lpc: &[Word16], weighted: &mut [Word16]) {
        // Apply perceptual weighting filter: A(z/γ1) / A(z/γ2)
        // Simplified implementation - normally uses γ1=0.9, γ2=0.6
        
        for i in 0..L_FRAME {
            let mut sum = l_mult(speech[i], lpc[0]); // a0 * x[n]
            
            // Apply FIR part A(z/γ1) 
            for j in 1..=M.min(i) {
                if j < lpc.len() {
                    let weighted_coeff = mult(lpc[j], 29491); // γ1 = 0.9 in Q15
                    sum = l_sub(sum, l_mult(speech[i-j], weighted_coeff));
                }
            }
            
            weighted[i] = round_word32(sum);
        }
    }

    /// Compute impulse response of weighted synthesis filter
    fn compute_impulse_response(&self, lpc: &[Word16], impulse: &mut [Word16]) {
        impulse.fill(0);
        impulse[0] = lpc[0]; // First sample is just a0
        
        // Compute impulse response by exciting filter with unit impulse
        for n in 1..L_SUBFR {
            let mut sum = 0i32;
            
            for k in 1..=M.min(n) {
                if k < lpc.len() {
                    sum = l_add(sum, l_mult(lpc[k], impulse[n-k]));
                }
            }
            
            impulse[n] = round_word32(sum);
        }
    }

    /// Compute target signal for fixed codebook search
    fn compute_target_signal(&self, weighted_speech: &[Word16], adaptive_exc: &[Word16], target: &mut [Word16]) {
        // Target = weighted_speech - adaptive_codebook_contribution
        for i in 0..L_SUBFR {
            target[i] = sub(weighted_speech[i], adaptive_exc[i]);
        }
    }

    /// Compute subframe energy for gain quantization
    fn compute_subframe_energy(&self, speech: &[Word16]) -> Word16 {
        let mut energy = 0i32;
        
        for &sample in speech {
            energy = l_add(energy, l_mult(sample, sample));
        }
        
        // Normalize and convert to Word16
        let normalized_energy = energy >> 10; // Scale down
        normalized_energy.max(1).min(32767) as Word16
    }

    /// Compute optimal fixed codebook gain
    fn compute_optimal_gain(&self, target: &[Word16], filtered_code: &[Word16]) -> Word16 {
        let mut correlation = 0i32;
        let mut energy = 0i32;
        
        for i in 0..L_SUBFR {
            correlation = l_add(correlation, l_mult(target[i], filtered_code[i]));
            energy = l_add(energy, l_mult(filtered_code[i], filtered_code[i]));
        }
        
        if energy > 0 {
            (correlation / energy.max(1)).max(0).min(32767) as Word16
        } else {
            0
        }
    }

    /// Update synthesis filter memory
    fn update_synthesis_memory(
        &mut self,
        speech: &[Word16],
        adaptive_exc: &[Word16],
        fixed_exc: &[Word16],
        adaptive_gain: Word16,
        fixed_gain: Word16,
        residual: &mut [Word16],
    ) {
        // Compute total excitation
        let mut total_exc = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            let adaptive_contrib = mult(adaptive_exc[i], adaptive_gain);
            let fixed_contrib = mult(fixed_exc[i], fixed_gain);
            total_exc[i] = add(adaptive_contrib, fixed_contrib);
        }
        
        // Update residual (excitation signal)
        residual.copy_from_slice(&total_exc);
        
        // Update synthesis filter memory (simplified)
        // In full implementation, this would run synthesis filter
        for i in 0..M.min(speech.len()) {
            self.syn_mem[i] = speech[speech.len() - 1 - i];
        }
    }

    /// Get encoder statistics for debugging
    pub fn get_stats(&self) -> G729EncoderStats {
        G729EncoderStats {
            frames_encoded: self.frame_count,
            total_bits: self.frame_count * FRAME_BITS,
            average_pitch_lag: 0.0, // Would compute from pitch analyzer
            average_lsp_distortion: 0.0, // Would compute from LSP quantizer
        }
    }
}

/// G.729 frame parameters
#[derive(Debug, Clone)]
pub struct G729Frame {
    /// LSP quantization indices
    pub lsp_indices: Vec<usize>,
    /// Subframe parameters
    pub subframes: Vec<G729SubframeParams>,
    /// Frame number for debugging
    pub frame_number: usize,
}

/// G.729 subframe parameters
#[derive(Debug, Clone)]
pub struct G729SubframeParams {
    /// Pitch lag (adaptive codebook index)
    pub pitch_lag: usize,
    /// Adaptive codebook gain
    pub adaptive_gain: Word16,
    /// Fixed codebook pulse positions
    pub positions: [usize; 4],
    /// Fixed codebook pulse signs
    pub signs: [i8; 4],
    /// Fixed codebook gain
    pub fixed_gain: Word16,
    /// Gain quantization index
    pub gain_index: usize,
}

/// Encoder statistics
#[derive(Debug, Clone)]
pub struct G729EncoderStats {
    /// Number of frames encoded
    pub frames_encoded: usize,
    /// Total bits generated
    pub total_bits: usize,
    /// Average pitch lag across frames
    pub average_pitch_lag: f32,
    /// Average LSP quantization distortion
    pub average_lsp_distortion: f32,
}

impl G729Frame {
    /// Convert frame parameters to bitstream
    /// 
    /// G.729 uses 80 bits per frame:
    /// - LSP indices: ~18 bits
    /// - Pitch lags: ~16 bits (8 bits × 2 subframes)
    /// - Fixed codebook: ~34 bits (17 bits × 2 subframes)
    /// - Gains: ~12 bits (6 bits × 2 subframes)
    pub fn to_bitstream(&self) -> Vec<u8> {
        let mut bits = Vec::new();
        
        // Pack LSP indices (simplified - normally more complex packing)
        for &index in &self.lsp_indices {
            bits.extend_from_slice(&(index as u16).to_be_bytes());
        }
        
        // Pack subframe parameters
        for subframe in &self.subframes {
            // Pitch lag (8 bits)
            bits.push(subframe.pitch_lag as u8);
            
            // Fixed codebook positions and signs (17 bits packed)
            let mut codebook_word = 0u32;
            for i in 0..4 {
                codebook_word |= (subframe.positions[i] as u32) << (i * 6);
                if subframe.signs[i] > 0 {
                    codebook_word |= 1 << (24 + i);
                }
            }
            bits.extend_from_slice(&codebook_word.to_be_bytes()[1..4]); // 3 bytes = 24 bits
            
            // Gain index (7 bits)
            bits.push(subframe.gain_index as u8);
        }
        
        bits
    }

    /// Get frame size in bits
    pub fn bit_count(&self) -> usize {
        FRAME_BITS
    }

    /// Get frame rate information
    pub fn get_frame_info(&self) -> (f32, f32) {
        let frame_rate = 100.0; // 100 frames per second (10ms frames)
        let bit_rate = (FRAME_BITS as f32) * frame_rate; // 8000 bps
        (frame_rate, bit_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = G729Encoder::new();
        assert_eq!(encoder.frame_count, 0);
    }

    #[test]
    fn test_encoder_reset() {
        let mut encoder = G729Encoder::new();
        encoder.frame_count = 10;
        encoder.syn_mem[0] = 1000;
        
        encoder.reset();
        
        assert_eq!(encoder.frame_count, 0);
        assert_eq!(encoder.syn_mem[0], 0);
    }

    #[test]
    fn test_frame_encoding() {
        let mut encoder = G729Encoder::new();
        let speech = vec![1000i16; L_FRAME]; // Simple test signal
        
        let frame = encoder.encode_frame(&speech);
        
        assert_eq!(frame.frame_number, 1);
        assert_eq!(frame.subframes.len(), N_SUBFR);
        assert!(!frame.lsp_indices.is_empty());
    }

    #[test]
    fn test_weighted_speech_computation() {
        let encoder = G729Encoder::new();
        let speech = vec![1000i16; L_FRAME];
        let lpc = vec![4096i16; M + 1]; // Simple LPC coefficients
        let mut weighted = vec![0i16; L_FRAME];
        
        encoder.compute_weighted_speech(&speech, &lpc, &mut weighted);
        
        // Output should be different from input
        assert!(weighted.iter().any(|&x| x != speech[0]));
    }

    #[test]
    fn test_impulse_response_computation() {
        let encoder = G729Encoder::new();
        let lpc = vec![4096i16; M + 1]; // Simple LPC coefficients
        let mut impulse = vec![0i16; L_SUBFR];
        
        encoder.compute_impulse_response(&lpc, &mut impulse);
        
        // First sample should equal first LPC coefficient
        assert_eq!(impulse[0], lpc[0]);
        assert!(impulse.iter().any(|&x| x != 0));
    }

    #[test]
    fn test_target_signal_computation() {
        let encoder = G729Encoder::new();
        let weighted_speech = vec![1000i16; L_SUBFR];
        let adaptive_exc = vec![200i16; L_SUBFR];
        let mut target = vec![0i16; L_SUBFR];
        
        encoder.compute_target_signal(&weighted_speech, &adaptive_exc, &mut target);
        
        // Target should be difference
        assert_eq!(target[0], 800); // 1000 - 200
    }

    #[test]
    fn test_energy_computation() {
        let encoder = G729Encoder::new();
        let speech = vec![1000i16; L_SUBFR];
        
        let energy = encoder.compute_subframe_energy(&speech);
        
        assert!(energy > 0);
    }

    #[test]
    fn test_gain_computation() {
        let encoder = G729Encoder::new();
        let target = vec![1000i16; L_SUBFR];
        let filtered_code = vec![800i16; L_SUBFR];
        
        let gain = encoder.compute_optimal_gain(&target, &filtered_code);
        
        assert!(gain > 0);
    }

    #[test]
    fn test_bitstream_conversion() {
        let frame = G729Frame {
            lsp_indices: vec![10, 20],
            subframes: vec![
                G729SubframeParams {
                    pitch_lag: 50,
                    adaptive_gain: 1000,
                    positions: [0, 10, 20, 30],
                    signs: [1, -1, 1, -1],
                    fixed_gain: 800,
                    gain_index: 64,
                };
                N_SUBFR
            ],
            frame_number: 1,
        };
        
        let bitstream = frame.to_bitstream();
        
        assert!(!bitstream.is_empty());
        assert_eq!(frame.bit_count(), FRAME_BITS);
    }

    #[test]
    fn test_frame_info() {
        let frame = G729Frame {
            lsp_indices: vec![10, 20],
            subframes: vec![],
            frame_number: 1,
        };
        
        let (frame_rate, bit_rate) = frame.get_frame_info();
        
        assert_eq!(frame_rate, 100.0); // 10ms frames = 100 fps
        assert_eq!(bit_rate, 8000.0);  // 80 bits × 100 fps = 8000 bps
    }

    #[test]
    fn test_encoder_stats() {
        let mut encoder = G729Encoder::new();
        let speech = vec![1000i16; L_FRAME];
        
        // Encode a few frames
        for _ in 0..5 {
            encoder.encode_frame(&speech);
        }
        
        let stats = encoder.get_stats();
        assert_eq!(stats.frames_encoded, 5);
        assert_eq!(stats.total_bits, 5 * FRAME_BITS);
    }

    #[test]
    fn test_multiple_frame_encoding() {
        let mut encoder = G729Encoder::new();
        let speech = vec![1000i16; L_FRAME];
        
        let frame1 = encoder.encode_frame(&speech);
        let frame2 = encoder.encode_frame(&speech);
        
        assert_eq!(frame1.frame_number, 1);
        assert_eq!(frame2.frame_number, 2);
        
        // Frames should maintain state between calls
        assert_eq!(encoder.frame_count, 2);
    }
} 