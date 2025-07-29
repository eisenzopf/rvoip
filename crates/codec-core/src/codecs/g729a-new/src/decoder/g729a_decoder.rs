use crate::common::basic_operators::*;
use crate::common::tab_ld8a::{L_FRAME, L_SUBFR, M, MP1, PIT_MAX, L_INTERPOL};
use crate::common::bits::{bits2prm, PRM_SIZE, SERIAL_SIZE};
use crate::common::filter::syn_filt;
use crate::common::lsp_az::int_qlpc;
use crate::common::adaptive_codebook_common::pred_lt_3;
use crate::decoder::lsp::LspDecoder;
use crate::decoder::gain::GainDecoder;
use crate::decoder::acelp_codebook::AcelpDecoder;
use crate::decoder::adaptive_codebook::AdaptiveDecoder;
use crate::decoder::post_processing::PostProcessing;

/// G.729A Decoder - Complete decoding pipeline
pub struct G729ADecoder {
    // Decoder modules
    lsp_decoder: LspDecoder,
    gain_decoder: GainDecoder,
    acelp_decoder: AcelpDecoder,
    adaptive_decoder: AdaptiveDecoder,
    post_processing: PostProcessing,
    
    // State buffers
    old_exc: [Word16; L_FRAME + PIT_MAX + L_INTERPOL],  // Excitation history
    mem_syn: [Word16; M],                               // Synthesis filter memory
    old_lsp_q: [Word16; M],                            // Previous quantized LSP
    old_bfi: Word16,                                   // Previous bad frame indicator
    seed: Word16,                                      // Random seed for error concealment
    
    // Previous pitch for relative decoding
    prev_pitch: Word16,
}

impl G729ADecoder {
    /// Create a new G.729A decoder
    pub fn new() -> Self {
        Self {
            lsp_decoder: LspDecoder::new(),
            gain_decoder: GainDecoder::new(),
            acelp_decoder: AcelpDecoder::new(),
            adaptive_decoder: AdaptiveDecoder::new(),
            post_processing: PostProcessing::new(),
            
            old_exc: [0; L_FRAME + PIT_MAX + L_INTERPOL],
            mem_syn: [0; M],
            old_lsp_q: [0; M],
            old_bfi: 0,
            seed: 21845, // Standard seed value
            
            prev_pitch: 40, // Initialize to typical pitch value
        }
    }
    
    /// Initialize the decoder (reset all state)
    pub fn init(&mut self) {
        *self = Self::new();
        
        // Initialize LSP values to G.729A default (same as encoder)
        let lsp_init = [
            2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396,
        ];
        self.old_lsp_q.copy_from_slice(&lsp_init);
        
        // Initialize excitation buffer with small values
        for i in 0..self.old_exc.len() {
            self.old_exc[i] = ((i as i16) % 3) - 1; // Small values: -1, 0, 1
        }
    }
    
    /// Decode one frame from bitstream (82 words: sync + size + 80 bits)
    /// Returns the reconstructed speech (80 samples)
    pub fn decode_frame(&mut self, bitstream: &[Word16]) -> [Word16; L_FRAME] {
        assert_eq!(bitstream.len(), SERIAL_SIZE, "Input bitstream must be {} words", SERIAL_SIZE);
        
        let mut speech = [0i16; L_FRAME];
        
        // Check for sync word and frame size
        let sync_word = bitstream[0];
        let frame_size = bitstream[1];
        
        // Basic frame validation
        let bad_frame = if sync_word != 0x6b21 || frame_size != 80 {
            1 // Bad frame indicator
        } else {
            0 // Good frame
        };
        
        if bad_frame == 1 {
            // Error concealment - use previous frame parameters
            return self.conceal_frame();
        }
        
        // Step 1: Bit unpacking
        let prm = bits2prm(bitstream.try_into().expect("Bitstream must be exactly SERIAL_SIZE"));
        
        // Step 2: Parameter extraction
        let lsp_indices = [prm[0], prm[1]];
        let parity = prm[3];
        
        // Subframe parameters
        let pitch_indices = [prm[2], add(prm[2], prm[7])]; // P1, P1+P2
        let acelp_positions = [prm[4], prm[8]];
        let acelp_signs = [prm[5], prm[9]];
        let gain_indices = [prm[6], prm[10]];
        
        // Step 3: LSP decoding
        let mut lsp_q = [0i16; M];
        self.lsp_decoder.decode_lsp(&lsp_indices, &mut lsp_q);
        
        // Step 4: Interpolate LSP and convert to LP coefficients for both subframes
        let mut az = [0i16; 2 * MP1];  // LP coefficients for both subframes
        int_qlpc(&self.old_lsp_q, &lsp_q, &mut az);
        
        // Process two subframes
        for subframe in 0..2 {
            let sf_start = subframe * L_SUBFR;
            let az_offset = subframe * MP1;
            
            // Get LP coefficients for this subframe
            let mut a_subframe = [0i16; MP1];
            a_subframe.copy_from_slice(&az[az_offset..az_offset + MP1]);
            
            // Step 5: Adaptive codebook decoding
            let mut exc_adaptive = [0i16; L_SUBFR];
            let (t0, t0_frac) = self.adaptive_decoder.decode_adaptive(
                pitch_indices[subframe], 
                parity, 
                subframe, 
                &mut self.old_exc,
                &mut exc_adaptive
            );
            
            // Update previous pitch for next frame
            if subframe == 0 {
                self.prev_pitch = t0;
            }
            
            // Step 6: Fixed codebook decoding
            let mut exc_fixed = [0i16; L_SUBFR];
            self.acelp_decoder.decode_acelp(
                acelp_positions[subframe],
                acelp_signs[subframe],
                &mut exc_fixed
            );
            
            // Step 7: Gain decoding
            let (gain_pit, gain_cod) = self.gain_decoder.decode_gain(
                gain_indices[subframe],
                &exc_fixed
            );
            
            // Step 8: Total excitation computation
            // exc = gain_pit * adaptive + gain_cod * fixed
            let mut exc_total = [0i16; L_SUBFR];
            for i in 0..L_SUBFR {
                let adaptive_contrib = mult(exc_adaptive[i], gain_pit);  // Q0 * Q14 = Q14 >> 15 = Q0
                let fixed_contrib = mult(exc_fixed[i], gain_cod);        // Q13 * Q1 = Q14 >> 15 = Q0
                exc_total[i] = add(adaptive_contrib, shr(fixed_contrib, 1)); // Align Q-formats
            }
            
            // Step 9: Update excitation buffer
            for i in 0..L_SUBFR {
                self.old_exc[PIT_MAX + L_INTERPOL + sf_start + i] = exc_total[i];
            }
            
            // Step 10: Synthesis filtering
            // Compute speech: speech = exc * 1/A(z)
            syn_filt(
                &a_subframe,
                &exc_total,
                &mut speech[sf_start..sf_start + L_SUBFR],
                L_SUBFR as i32,
                &mut self.mem_syn,
                true
            );
        }
        
        // Step 11: Post-processing (high-pass filtering, scaling)
        self.post_processing.process(&mut speech);
        
        // Update state for next frame
        self.old_lsp_q.copy_from_slice(&lsp_q);
        self.old_bfi = bad_frame;
        
        // Shift excitation buffer for next frame
        for i in 0..(PIT_MAX + L_INTERPOL) {
            self.old_exc[i] = self.old_exc[i + L_FRAME];
        }
        
        speech
    }
    
    /// Error concealment for bad frames
    fn conceal_frame(&mut self) -> [Word16; L_FRAME] {
        let mut speech = [0i16; L_FRAME];
        
        // Use previous LSP parameters (already in self.old_lsp_q)
        let mut az = [0i16; 2 * MP1];
        int_qlpc(&self.old_lsp_q, &self.old_lsp_q, &mut az); // Repeat previous LSP
        
        // Generate attenuated excitation for both subframes
        for subframe in 0..2 {
            let sf_start = subframe * L_SUBFR;
            let az_offset = subframe * MP1;
            
            let mut a_subframe = [0i16; MP1];
            a_subframe.copy_from_slice(&az[az_offset..az_offset + MP1]);
            
            // Generate random excitation with decreasing energy
            let mut exc = [0i16; L_SUBFR];
            for i in 0..L_SUBFR {
                // Simple random number generator
                self.seed = mult(self.seed, 31821);
                self.seed = add(self.seed, 13849);
                
                // Attenuate the excitation (reduce energy gradually)
                exc[i] = shr(self.seed, 4); // Reduce amplitude
            }
            
            // Update excitation buffer
            for i in 0..L_SUBFR {
                self.old_exc[PIT_MAX + L_INTERPOL + sf_start + i] = exc[i];
            }
            
            // Synthesis filtering
            syn_filt(
                &a_subframe,
                &exc,
                &mut speech[sf_start..sf_start + L_SUBFR],
                L_SUBFR as i32,
                &mut self.mem_syn,
                true
            );
        }
        
        // Post-processing
        self.post_processing.process(&mut speech);
        
        // Shift excitation buffer
        for i in 0..(PIT_MAX + L_INTERPOL) {
            self.old_exc[i] = self.old_exc[i + L_FRAME];
        }
        
        speech
    }
}

// Re-export for external use
pub use crate::common::bits::{bits2prm as export_bits2prm, prm2bits as export_prm2bits};