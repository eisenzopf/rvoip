//! G.729A encoder implementation

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{AudioFrame, EncodedFrame, Q15, Q31, CodecError, LSPParameters, LPCoefficients};
use crate::codecs::g729a::signal::{SignalPreprocessor, HammingWindow};
use crate::codecs::g729a::spectral::{
    LinearPredictor, LSPConverter, LSPQuantizer, LSPInterpolator
};
use crate::codecs::g729a::perception::{PerceptualWeightingFilter, PitchTracker};
use crate::codecs::g729a::excitation::{
    AdaptiveCodebook, AlgebraicCodebook, GainQuantizer, apply_gains
};
use crate::codecs::g729a::synthesis::SynthesisFilter;
use crate::codecs::g729a::codec::bitstream::pack_frame;
use crate::codecs::g729a::math::FixedPointOps;

/// G.729A encoder state
pub struct G729AEncoder {
    // Signal processing
    preprocessor: SignalPreprocessor,
    window: HammingWindow,
    
    // Spectral analysis
    lp_analyzer: LinearPredictor,
    lsp_converter: LSPConverter,
    lsp_quantizer: LSPQuantizer,
    
    // Perception
    weighting_filter: PerceptualWeightingFilter,
    pitch_tracker: PitchTracker,
    
    // Excitation
    adaptive_codebook: AdaptiveCodebook,
    algebraic_codebook: AlgebraicCodebook,
    gain_quantizer: GainQuantizer,
    
    // Synthesis
    synthesis_filter: SynthesisFilter,
    
    // State
    prev_lsp: Option<LSPParameters>,
    lookahead_buffer: Vec<Q15>,
    history_buffer: Vec<Q15>,  // 120 samples from previous frame for LP analysis
}

impl G729AEncoder {
    /// Create a new G.729A encoder
    pub fn new() -> Self {
        Self {
            preprocessor: SignalPreprocessor::new(),
            window: HammingWindow::new_asymmetric(),
            lp_analyzer: LinearPredictor::new(),
            lsp_converter: LSPConverter::new(),
            lsp_quantizer: LSPQuantizer::new(),
            weighting_filter: PerceptualWeightingFilter::new(),
            pitch_tracker: PitchTracker::new(),
            adaptive_codebook: AdaptiveCodebook::new(),
            algebraic_codebook: AlgebraicCodebook::new(),
            gain_quantizer: GainQuantizer::new(),
            synthesis_filter: SynthesisFilter::new(),
            prev_lsp: None,
            lookahead_buffer: vec![Q15::ZERO; LOOK_AHEAD],
            history_buffer: vec![Q15::ZERO; 120],
        }
    }
    
    /// Encode a frame of audio with proper look-ahead
    pub fn encode_frame_with_lookahead(&mut self, 
                                      current_frame: &AudioFrame,
                                      next_frame_preview: &[i16]) -> Result<[u8; 10], CodecError> {
        // 1. Build analysis buffer with history, current frame and look-ahead (240 samples total)
        let mut analysis_buffer = vec![Q15::ZERO; 240];
        
        // Copy 120 samples from history buffer
        analysis_buffer[..120].copy_from_slice(&self.history_buffer);
        
        // Preprocess current frame and add to buffer
        let processed_frame = self.preprocessor.process(&current_frame.samples);
        analysis_buffer[120..200].copy_from_slice(&processed_frame);
        
        // Preprocess look-ahead from next frame
        if next_frame_preview.len() >= LOOK_AHEAD {
            // Create AudioFrame for lookahead
            let mut lookahead_samples = [0i16; FRAME_SIZE];
            lookahead_samples[..LOOK_AHEAD].copy_from_slice(&next_frame_preview[..LOOK_AHEAD]);
            let lookahead_frame = AudioFrame {
                samples: lookahead_samples,
                timestamp: 0,
            };
            let lookahead_processed = self.preprocessor.process_frame(&lookahead_frame);
            
            // Extract only the lookahead portion
            let lookahead_q15: Vec<Q15> = lookahead_processed.samples[..LOOK_AHEAD]
                .iter()
                .map(|&s| Q15(s))
                .collect();
            analysis_buffer[200..240].copy_from_slice(&lookahead_q15);
            
            // Store for next frame
            self.lookahead_buffer = lookahead_q15;
        } else {
            // Use zeros if not enough look-ahead available
            analysis_buffer[200..240].fill(Q15::ZERO);
        }
        
        // Update history buffer for next frame: last 120 samples
        self.history_buffer.clear();
        self.history_buffer.extend_from_slice(&analysis_buffer[120..240]);
        
        // 2. Perform LP analysis on windowed signal with look-ahead
        let windowed = self.window.apply(&analysis_buffer);
        let lp_coeffs = self.lp_analyzer.analyze(&windowed);
        
        // 3. Convert to LSP and quantize
        let lsp = self.lsp_converter.lp_to_lsp(&lp_coeffs);
        let quantized_lsp = self.lsp_quantizer.quantize(&lsp);
        
        // 4. Open-loop pitch analysis
        let weighted_speech = self.compute_weighted_speech(&analysis_buffer, &lp_coeffs);
        let open_loop_pitch = self.pitch_tracker.estimate_open_loop_pitch(&weighted_speech);
        
        // 5. Process subframes
        let mut encoded = EncodedFrame {
            lsp_indices: quantized_lsp.indices,
            pitch_delay_int: [0; 2],
            pitch_delay_frac: [0; 2],
            fixed_codebook_idx: [0; 2],
            gain_indices: [[0; 2]; 2],
        };
        
        let prev_lsp = self.prev_lsp.as_ref().unwrap_or(&quantized_lsp.reconstructed).clone();
        
        for sf_idx in 0..2 {
            let sf_start = LOOK_AHEAD + sf_idx * SUBFRAME_SIZE;
            let sf_end = sf_start + SUBFRAME_SIZE;
            let subframe = &analysis_buffer[sf_start..sf_end];
            
            // Interpolate LSP for this subframe
            let interpolated_lsp = LSPInterpolator::interpolate(
                &prev_lsp,
                &quantized_lsp.reconstructed,
                sf_idx,
            );
            
            // Convert back to LP coefficients
            let sf_lp = self.lsp_converter.lsp_to_lp(&interpolated_lsp);
            
            // Encode subframe
            let (pitch, fixed_idx, gains) = self.encode_subframe(
                subframe,
                &sf_lp,
                &weighted_speech[sf_start..sf_end],
                open_loop_pitch.delay,
                sf_idx,
            );
            
            // Store parameters
            encoded.pitch_delay_int[sf_idx] = pitch.floor() as u8;
            encoded.pitch_delay_frac[sf_idx] = ((pitch.fract() * 3.0).round() as u8).min(2);
            encoded.fixed_codebook_idx[sf_idx] = fixed_idx;
            encoded.gain_indices[sf_idx] = gains;
        }
        
        // Update state
        self.prev_lsp = Some(quantized_lsp.reconstructed);
        
        // 6. Pack bitstream
        Ok(pack_frame(&encoded))
    }
    
    /// Compute weighted speech
    fn compute_weighted_speech(
        &self,
        speech: &[Q15],
        lp_coeffs: &LPCoefficients,
    ) -> Vec<Q15> {
        let weighted_filter = self.weighting_filter.create_filter(lp_coeffs);
        self.weighting_filter.filter_signal(speech, &weighted_filter)
    }
    
    /// Encode a subframe
    fn encode_subframe(
        &mut self,
        speech: &[Q15],
        lp_coeffs: &LPCoefficients,
        weighted_speech: &[Q15],
        open_loop_pitch: u16,
        subframe_idx: usize,
    ) -> (f32, u32, [u8; 2]) {
        // 1. Compute impulse response of weighted synthesis filter
        let weighted_filter = self.weighting_filter.create_filter(lp_coeffs);
        let h = self.weighting_filter.compute_impulse_response(&weighted_filter);
        
        // 2. Compute target signal for adaptive codebook search
        // (simplified - using weighted speech as target)
        let target = weighted_speech;
        
        // 3. Adaptive codebook search
        let pitch_range = self.pitch_tracker.get_pitch_search_range(open_loop_pitch, subframe_idx);
        let adaptive_contrib = self.adaptive_codebook.search(target, &pitch_range);
        
        // 4. Update target for fixed codebook search
        let mut fixed_target = vec![Q15::ZERO; SUBFRAME_SIZE];
        for i in 0..SUBFRAME_SIZE {
            // Convolve adaptive codebook with h
            let filtered_adaptive = self.convolve_with_h(&adaptive_contrib.vector, &h);
            fixed_target[i] = target[i].saturating_add(Q15(-filtered_adaptive[i].0));
        }
        
        // 5. Fixed codebook search
        let algebraic_contrib = self.algebraic_codebook.search(
            &fixed_target,
            speech,
            &h,
        );
        
        // 6. Gain quantization
        let gains = self.gain_quantizer.quantize(
            Q15::from_f32(0.8), // Simplified adaptive gain
            Q15::from_f32(0.3), // Simplified fixed gain
            &adaptive_contrib.vector,
            &algebraic_contrib.vector,
            target,
        );
        
        // 7. Update excitation buffer
        let excitation = apply_gains(
            &adaptive_contrib.vector,
            &algebraic_contrib.vector,
            gains.adaptive_gain,
            gains.fixed_gain,
        );
        self.adaptive_codebook.update_excitation(&excitation);
        
        // 8. Update synthesis filter memory
        let synthesized = self.synthesis_filter.synthesize(&excitation, &lp_coeffs.values);
        
        (
            adaptive_contrib.delay,
            algebraic_contrib.codebook_index,
            gains.gain_indices,
        )
    }
    
    /// Simple convolution with impulse response
    fn convolve_with_h(&self, signal: &[Q15], h: &[Q15]) -> Vec<Q15> {
        let mut output = vec![Q15::ZERO; signal.len()];
        
        for i in 0..signal.len() {
            let mut sum = Q31::ZERO;
            let max_k = (i + 1).min(h.len());
            
            for k in 0..max_k {
                let h_k = h[k];
                let x_k = signal[i - k];
                let prod = h_k.to_q31().saturating_mul(x_k.to_q31());
                sum = sum.saturating_add(prod);
            }
            
            output[i] = sum.to_q15();
        }
        
        output
    }
    
    /// Reset encoder state
    pub fn reset(&mut self) {
        self.preprocessor = SignalPreprocessor::new();
        self.lsp_quantizer = LSPQuantizer::new();
        self.pitch_tracker = PitchTracker::new();
        self.adaptive_codebook = AdaptiveCodebook::new();
        self.gain_quantizer = GainQuantizer::new();
        self.synthesis_filter.reset();
        self.prev_lsp = None;
        self.lookahead_buffer.fill(Q15::ZERO);
        self.history_buffer.fill(Q15::ZERO);
    }
}

impl Default for G729AEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export types needed by encoder
use crate::codecs::g729a::{signal, spectral};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = G729AEncoder::new();
        assert_eq!(encoder.lookahead_buffer.len(), LOOK_AHEAD);
    }

    #[test]
    fn test_encode_frame() {
        let mut encoder = G729AEncoder::new();
        
        // Create test frame
        let frame = AudioFrame {
            samples: [(0.1 * 32767.0) as i16; FRAME_SIZE],
            timestamp: 0,
        };
        
        let result = encoder.encode_frame_with_lookahead(&frame, &[(0.1 * 32767.0) as i16; LOOK_AHEAD]);
        assert!(result.is_ok());
        
        let encoded = result.unwrap();
        assert_eq!(encoded.len(), 10); // 80 bits
    }

    #[test]
    fn test_encoder_reset() {
        let mut encoder = G729AEncoder::new();
        
        // Encode a frame to change state
        let frame = AudioFrame {
            samples: [(0.1 * 32767.0) as i16; FRAME_SIZE],
            timestamp: 0,
        };
        let _ = encoder.encode_frame_with_lookahead(&frame, &[(0.1 * 32767.0) as i16; LOOK_AHEAD]);
        
        // Reset
        encoder.reset();
        
        // Check state is reset
        assert!(encoder.prev_lsp.is_none());
        assert_eq!(encoder.lookahead_buffer, vec![Q15::ZERO; LOOK_AHEAD]);
    }
} 