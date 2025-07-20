//! G.729A decoder implementation

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{AudioFrame, DecodedParameters, Q15, CodecError, LSPParameters, LPCoefficients};
use crate::codecs::g729a::spectral::{LSPDecoder, LSPConverter, LSPInterpolator};
use crate::codecs::g729a::excitation::{
    AdaptiveCodebook, AlgebraicCodebook, GainQuantizer, apply_gains
};
use crate::codecs::g729a::synthesis::{SynthesisFilter, AdaptivePostfilter};
use crate::codecs::g729a::codec::bitstream::unpack_frame;

/// G.729A decoder state
pub struct G729ADecoder {
    // Spectral decoding
    lsp_decoder: LSPDecoder,
    lsp_converter: LSPConverter,
    
    // Excitation generation
    adaptive_codebook: AdaptiveCodebook,
    algebraic_codebook: AlgebraicCodebook,
    gain_decoder: GainQuantizer,
    
    // Synthesis
    synthesis_filter: SynthesisFilter,
    postfilter: AdaptivePostfilter,
    
    // State
    prev_lsp: Option<LSPParameters>,
    prev_pitch: f32,
}

impl G729ADecoder {
    /// Create a new G.729A decoder
    pub fn new() -> Self {
        Self {
            lsp_decoder: LSPDecoder::new(),
            lsp_converter: LSPConverter::new(),
            adaptive_codebook: AdaptiveCodebook::new(),
            algebraic_codebook: AlgebraicCodebook::new(),
            gain_decoder: GainQuantizer::new(),
            synthesis_filter: SynthesisFilter::new(),
            postfilter: AdaptivePostfilter::new(),
            prev_lsp: None,
            prev_pitch: 50.0, // Default pitch
        }
    }
    
    /// Decode a frame of audio
    pub fn decode_frame(&mut self, packed: &[u8; 10]) -> Result<AudioFrame, CodecError> {
        // 1. Unpack bitstream
        let params = unpack_frame(packed);
        
        // 2. Decode LSP parameters
        let current_lsp = self.lsp_decoder.decode(&params.lsp_indices);
        let prev_lsp = self.prev_lsp.as_ref().unwrap_or(&current_lsp).clone();
        
        // 3. Synthesize frame
        let mut output = vec![Q15::ZERO; FRAME_SIZE];
        
        for sf_idx in 0..2 {
            // Interpolate LSP for this subframe
            let interpolated_lsp = LSPInterpolator::interpolate(
                &prev_lsp,
                &current_lsp,
                sf_idx,
            );
            
            // Convert to LP coefficients
            let lp_coeffs = self.lsp_converter.lsp_to_lp(&interpolated_lsp);
            
            // Decode subframe
            let sf_output = self.decode_subframe(
                params.pitch_delays[sf_idx],
                params.fixed_codebook_indices[sf_idx],
                &params.gain_indices[sf_idx],
                &lp_coeffs,
            );
            
            // Copy to output
            let sf_start = sf_idx * SUBFRAME_SIZE;
            output[sf_start..sf_start + SUBFRAME_SIZE].copy_from_slice(&sf_output);
        }
        
        // 4. Post-processing
        let postprocessed = self.postfilter.process(
            &output,
            &self.lsp_converter.lsp_to_lp(&current_lsp),
            self.prev_pitch,
        );
        
        // 5. Update state
        self.prev_lsp = Some(current_lsp);
        
        // 6. Create output frame
        let mut frame = AudioFrame {
            samples: [0i16; FRAME_SIZE],
            timestamp: 0,
        };
        for i in 0..FRAME_SIZE {
            frame.samples[i] = postprocessed[i].0;
        }
        
        Ok(frame)
    }
    
    /// Decode a subframe
    fn decode_subframe(
        &mut self,
        pitch_delay: f32,
        fixed_codebook_idx: u32,
        gain_indices: &[u8; 2],
        lp_coeffs: &LPCoefficients,
    ) -> Vec<Q15> {
        // 1. Decode adaptive codebook contribution
        let adaptive_vector = self.adaptive_codebook.decode_vector(pitch_delay);
        
        // 2. Decode algebraic codebook contribution
        let pulses = self.algebraic_codebook.decode_pulses(fixed_codebook_idx);
        let fixed_vector = self.algebraic_codebook.build_vector(&pulses);
        
        // 3. Decode gains
        let gains = self.gain_decoder.decode(gain_indices);
        
        // 4. Compute excitation
        let excitation = apply_gains(
            &adaptive_vector,
            &fixed_vector,
            gains.adaptive_gain,
            gains.fixed_gain,
        );
        
        // 5. Update adaptive codebook
        self.adaptive_codebook.update_excitation(&excitation);
        
        // 6. Synthesize speech
        let synthesized = self.synthesis_filter.synthesize(&excitation, &lp_coeffs.values);
        
        // Update pitch for postfilter
        self.prev_pitch = pitch_delay;
        
        synthesized
    }
    
    /// Reset decoder state
    pub fn reset(&mut self) {
        self.lsp_decoder = LSPDecoder::new();
        self.adaptive_codebook = AdaptiveCodebook::new();
        self.gain_decoder = GainQuantizer::new();
        self.synthesis_filter.reset();
        self.postfilter.reset();
        self.prev_lsp = None;
        self.prev_pitch = 50.0;
    }
    
    /// Decode frame with error concealment
    pub fn decode_frame_with_concealment(
        &mut self,
        packed: Option<&[u8; 10]>,
    ) -> Result<AudioFrame, CodecError> {
        match packed {
            Some(data) => self.decode_frame(data),
            None => self.conceal_frame(),
        }
    }
    
    /// Simple frame erasure concealment
    fn conceal_frame(&mut self) -> Result<AudioFrame, CodecError> {
        // Use previous parameters with attenuated gains
        let mut output = vec![Q15::ZERO; FRAME_SIZE];
        
        if let Some(ref prev_lsp) = self.prev_lsp {
            let lp_coeffs = self.lsp_converter.lsp_to_lp(prev_lsp);
            
            for sf_idx in 0..2 {
                // Use previous pitch with slight variation
                let pitch = self.prev_pitch * (1.0 + 0.05 * sf_idx as f32);
                
                // Generate excitation with attenuated energy
                let adaptive_vector = self.adaptive_codebook.decode_vector(pitch);
                let attenuated_gain = Q15::from_f32(0.5); // 50% attenuation
                
                let mut excitation = vec![Q15::ZERO; SUBFRAME_SIZE];
                for i in 0..SUBFRAME_SIZE {
                    excitation[i] = adaptive_vector[i].saturating_mul(attenuated_gain);
                }
                
                // Update adaptive codebook
                self.adaptive_codebook.update_excitation(&excitation);
                
                // Synthesize
                let synthesized = self.synthesis_filter.synthesize(&excitation, &lp_coeffs.values);
                
                // Copy to output
                let sf_start = sf_idx * SUBFRAME_SIZE;
                output[sf_start..sf_start + SUBFRAME_SIZE].copy_from_slice(&synthesized);
            }
        }
        
        let mut frame = AudioFrame {
            samples: [0i16; FRAME_SIZE],
            timestamp: 0,
        };
        for i in 0..FRAME_SIZE {
            frame.samples[i] = output[i].0;
        }
        
        Ok(frame)
    }
}

impl Default for G729ADecoder {
    fn default() -> Self {
        Self::new()
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_creation() {
        let decoder = G729ADecoder::new();
        assert_eq!(decoder.prev_pitch, 50.0);
    }

    #[test]
    fn test_decode_frame() {
        let mut decoder = G729ADecoder::new();
        
        // Create test packed frame (mostly zeros)
        let packed = [0u8; 10];
        
        let result = decoder.decode_frame(&packed);
        assert!(result.is_ok());
        
        let frame = result.unwrap();
        assert_eq!(frame.samples.len(), FRAME_SIZE);
    }

    #[test]
    fn test_decoder_reset() {
        let mut decoder = G729ADecoder::new();
        
        // Decode a frame to change state
        let packed = [0u8; 10];
        let _ = decoder.decode_frame(&packed);
        
        // Reset
        decoder.reset();
        
        // Check state is reset
        assert!(decoder.prev_lsp.is_none());
        assert_eq!(decoder.prev_pitch, 50.0);
    }

    #[test]
    fn test_frame_concealment() {
        let mut decoder = G729ADecoder::new();
        
        // First decode a normal frame
        let packed = [0u8; 10];
        let _ = decoder.decode_frame(&packed);
        
        // Then test concealment
        let result = decoder.decode_frame_with_concealment(None);
        assert!(result.is_ok());
        
        let frame = result.unwrap();
        assert_eq!(frame.samples.len(), FRAME_SIZE);
    }

    #[test]
    fn test_decode_with_concealment_normal() {
        let mut decoder = G729ADecoder::new();
        
        let packed = [0u8; 10];
        let result = decoder.decode_frame_with_concealment(Some(&packed));
        assert!(result.is_ok());
    }
} 