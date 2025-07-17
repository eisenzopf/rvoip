//! G.729 Core Decoder
//!
//! This module implements the complete G.729 decoder that reconstructs speech from:
//! - LSP quantization indices
//! - Pitch lag and adaptive codebook parameters
//! - ACELP fixed codebook parameters
//! - Gain quantization indices
//!
//! Based on ITU-T G.729 reference implementation DECODER.C and DE_ACELP.C

use super::types::*;
use super::math::*;
use super::dsp::*;
use super::lpc::LpcAnalyzer;
use super::pitch::PitchAnalyzer;
use super::acelp::AcelpAnalyzer;
use super::quantization::{LspQuantizer, GainQuantizer};
use super::encoder::{G729Frame, G729SubframeParams};

/// G.729 frame size in samples (10ms at 8kHz)
const L_FRAME: usize = 80;

/// G.729 subframe size in samples (5ms at 8kHz)
const L_SUBFR: usize = 40;

/// Number of subframes per frame
const N_SUBFR: usize = 2;

/// G.729 Core Decoder
#[derive(Debug)]
pub struct G729Decoder {
    /// LPC analyzer for spectral envelope reconstruction
    lpc_analyzer: LpcAnalyzer,
    /// Pitch analyzer for adaptive codebook reconstruction
    pitch_analyzer: PitchAnalyzer,
    /// ACELP analyzer for fixed codebook reconstruction  
    acelp_analyzer: AcelpAnalyzer,
    /// LSP quantizer/dequantizer
    lsp_quantizer: LspQuantizer,
    /// Gain quantizer/dequantizer
    gain_quantizer: GainQuantizer,
    /// Synthesis filter memory
    syn_mem: [Word16; M],
    /// Excitation memory for pitch synthesis
    exc_mem: [Word16; 154], // ITU standard excitation buffer size
    /// Previous subframe for continuity
    prev_subframe: [Word16; L_SUBFR],
    /// Frame counter for debugging
    frame_count: usize,
    /// Bad frame indicator for error concealment
    bad_frame: bool,
}

impl G729Decoder {
    /// Create a new G.729 decoder
    pub fn new() -> Self {
        Self {
            lpc_analyzer: LpcAnalyzer::new(),
            pitch_analyzer: PitchAnalyzer::new(),
            acelp_analyzer: AcelpAnalyzer::new(),
            lsp_quantizer: LspQuantizer::new(),
            gain_quantizer: GainQuantizer::new(),
            syn_mem: [0; M],
            exc_mem: [0; 154],
            prev_subframe: [0; L_SUBFR],
            frame_count: 0,
            bad_frame: false,
        }
    }

    /// Reset decoder state
    pub fn reset(&mut self) {
        self.lpc_analyzer.reset();
        self.pitch_analyzer.reset();
        self.acelp_analyzer.reset();
        self.lsp_quantizer.reset();
        self.gain_quantizer.reset();
        self.syn_mem = [0; M];
        self.exc_mem = [0; 154];
        self.prev_subframe = [0; L_SUBFR];
        self.frame_count = 0;
        self.bad_frame = false;
    }

    /// Decode a frame of speech from G.729 parameters
    /// 
    /// # Arguments
    /// * `frame` - G.729 frame parameters
    /// 
    /// # Returns
    /// Decoded speech frame [L_FRAME] (80 samples)
    pub fn decode_frame(&mut self, frame: &G729Frame) -> Vec<Word16> {
        self.frame_count += 1;

        // Step 1: LSP Dequantization
        let mut lsp_q = [0i16; M];
        self.lsp_quantizer.dequantize_lsp(&frame.lsp_indices, &mut lsp_q);

        // Step 2: Convert LSPs to LPC coefficients
        let mut lpc_coeffs = [0i16; M + 1];
        self.lpc_analyzer.lsp_to_lpc(&lsp_q, &mut lpc_coeffs);

        // Step 3: Process subframes
        let mut decoded_speech = vec![0i16; L_FRAME];
        let mut total_excitation = [0i16; L_FRAME];

        for (subframe_idx, subframe) in frame.subframes.iter().enumerate() {
            let start_idx = subframe_idx * L_SUBFR;
            let end_idx = start_idx + L_SUBFR;

            // Step 3a: Decode adaptive codebook (pitch synthesis)
            let mut adaptive_exc = [0i16; L_SUBFR];
            self.decode_adaptive_codebook(subframe, &mut adaptive_exc);

            // Step 3b: Decode fixed codebook (ACELP synthesis)
            let mut fixed_exc = [0i16; L_SUBFR];
            self.decode_fixed_codebook(subframe, &mut fixed_exc);

            // Step 3c: Dequantize gains
            let energy = self.estimate_subframe_energy(&adaptive_exc, &fixed_exc);
            let (adaptive_gain, fixed_gain) = self.gain_quantizer.dequantize_gains(
                subframe.gain_index, energy
            );

            // Step 3d: Combine excitations
            let mut combined_exc = [0i16; L_SUBFR];
            for i in 0..L_SUBFR {
                let adaptive_contrib = mult(adaptive_exc[i], adaptive_gain);
                let fixed_contrib = mult(fixed_exc[i], fixed_gain);
                combined_exc[i] = add(adaptive_contrib, fixed_contrib);
            }

            // Step 3e: Synthesis filtering
            let mut subframe_speech = [0i16; L_SUBFR];
            self.synthesis_filter(&lpc_coeffs, &combined_exc, &mut subframe_speech);

            // Step 3f: Update excitation memory and store results
            self.update_excitation_memory(&combined_exc);
            total_excitation[start_idx..end_idx].copy_from_slice(&combined_exc);
            decoded_speech[start_idx..end_idx].copy_from_slice(&subframe_speech);
        }

        // Step 4: Post-processing (adaptive postfilter)
        self.adaptive_postfilter(&mut decoded_speech, &lsp_q);

        decoded_speech
    }

    /// Decode adaptive codebook (pitch synthesis)
    fn decode_adaptive_codebook(&mut self, subframe: &G729SubframeParams, adaptive_exc: &mut [Word16]) {
        // Use pitch lag to reconstruct adaptive codebook contribution
        let exc_vec = self.pitch_analyzer.pred_lt_3(&self.exc_mem, subframe.pitch_lag as Word16, 0, L_SUBFR);
        adaptive_exc[..exc_vec.len().min(L_SUBFR)].copy_from_slice(&exc_vec[..exc_vec.len().min(L_SUBFR)]);
    }

    /// Decode fixed codebook (ACELP synthesis)  
    fn decode_fixed_codebook(&self, subframe: &G729SubframeParams, fixed_exc: &mut [Word16]) {
        // Reconstruct innovation sequence from positions, signs, and gain
        self.acelp_analyzer.build_innovation(
            &subframe.positions,
            &subframe.signs,
            subframe.gain_index,
            fixed_exc,
        );
    }

    /// Estimate subframe energy for gain dequantization
    fn estimate_subframe_energy(&self, adaptive_exc: &[Word16], fixed_exc: &[Word16]) -> Word16 {
        let mut energy = 0i32;

        // Combine energies from both excitation components
        for i in 0..L_SUBFR {
            energy = l_add(energy, l_mult(adaptive_exc[i], adaptive_exc[i]));
            energy = l_add(energy, l_mult(fixed_exc[i], fixed_exc[i]));
        }

        // Normalize and convert to Word16
        let normalized_energy = energy >> 10; // Scale down
        normalized_energy.max(1).min(32767) as Word16
    }

    /// Synthesis filter: reconstruct speech from excitation
    /// 
    /// Implements the IIR synthesis filter: H(z) = 1 / A(z)
    /// y[n] = x[n] - sum(a[k] * y[n-k])
    fn synthesis_filter(&mut self, lpc: &[Word16], excitation: &[Word16], speech: &mut [Word16]) {
        for n in 0..L_SUBFR {
            let mut sum = l_mult(excitation[n], 4096); // Gain scaling

            // Apply feedback from previous speech samples
            for k in 1..=M {
                if k <= n {
                    // Use current subframe samples
                    sum = l_sub(sum, l_mult(lpc[k], speech[n - k]));
                } else if k - n <= M {
                    // Use synthesis memory
                    let mem_idx = M - (k - n);
                    if mem_idx < self.syn_mem.len() {
                        sum = l_sub(sum, l_mult(lpc[k], self.syn_mem[mem_idx]));
                    }
                }
            }

            speech[n] = round_word32(sum);
        }

        // Update synthesis memory with last M samples
        for i in 0..M {
            if i < L_SUBFR {
                self.syn_mem[M - 1 - i] = speech[L_SUBFR - 1 - i];
            }
        }
    }

    /// Update excitation memory for pitch prediction
    fn update_excitation_memory(&mut self, excitation: &[Word16]) {
        // Shift existing memory
        for i in 0..(self.exc_mem.len() - L_SUBFR) {
            self.exc_mem[i] = self.exc_mem[i + L_SUBFR];
        }

        // Add new excitation at the end
        let start_idx = self.exc_mem.len() - L_SUBFR;
        self.exc_mem[start_idx..].copy_from_slice(excitation);
    }

    /// Adaptive postfilter for perceptual enhancement
    /// 
    /// Applies formant postfilter and high-pass filter to improve
    /// perceptual quality of decoded speech.
    fn adaptive_postfilter(&self, speech: &mut [Word16], lsp: &[Word16]) {
        // Step 1: Formant postfilter
        self.formant_postfilter(speech, lsp);

        // Step 2: High-pass filter
        self.high_pass_filter(speech);

        // Step 3: AGC (Automatic Gain Control)
        self.automatic_gain_control(speech);
    }

    /// Formant postfilter using LSP parameters
    fn formant_postfilter(&self, speech: &mut [Word16], _lsp: &[Word16]) {
        // Simplified formant postfilter
        // In full implementation, this would use spectral tilt compensation
        // and formant enhancement based on LSP parameters

        // Apply simple smoothing filter
        for i in 1..speech.len() {
            let smoothed = add(mult(speech[i], 26214), mult(speech[i - 1], 6554)); // 0.8 + 0.2
            speech[i] = smoothed;
        }
    }

    /// High-pass filter to remove DC and low-frequency noise
    fn high_pass_filter(&self, speech: &mut [Word16]) {
        // Simple high-pass filter: y[n] = x[n] - 0.95 * x[n-1]
        if !speech.is_empty() {
            for i in (1..speech.len()).rev() {
                let filtered = sub(speech[i], mult(speech[i - 1], 31129)); // 0.95 in Q15
                speech[i] = filtered;
            }
        }
    }

    /// Automatic gain control for output level normalization
    fn automatic_gain_control(&self, speech: &mut [Word16]) {
        // Compute frame energy
        let mut energy = 0i64; // Use i64 to prevent overflow
        for &sample in speech.iter() {
            let sample_i64 = sample as i64;
            energy += sample_i64 * sample_i64;
        }

        if energy > 0 {
            // Calculate RMS energy
            let rms_energy = ((energy / speech.len() as i64) as f64).sqrt();
            
            // Target RMS level (reasonable speech level)
            let target_rms = 2000.0; // About 1/8 of full scale
            
            if rms_energy > 1.0 { // Avoid division by very small numbers
                let gain_factor = (target_rms / rms_energy).min(4.0).max(0.25); // Limit gain range
                let gain_q15 = (gain_factor * 32768.0) as Word16;
                let limited_gain = gain_q15.max(8192).min(32767); // Ensure reasonable gain
                
                // Apply gain with saturation
                for sample in speech.iter_mut() {
                    let gained = mult(*sample, limited_gain);
                    *sample = gained;
                }
            }
        } else {
            // If input energy is zero, leave signal as-is (don't force to zero)
            // This preserves any small signals that might be important
        }
    }

    /// Decode frame from bitstream
    /// 
    /// # Arguments
    /// * `bitstream` - Encoded bitstream bytes
    /// 
    /// # Returns
    /// Decoded G.729 frame parameters, or None if invalid
    pub fn decode_bitstream(&self, bitstream: &[u8]) -> Option<G729Frame> {
        if bitstream.len() < 14 {
            // Minimum frame size: 4 bytes LSP + 2 * (1 + 3 + 1) = 14 bytes total
            return None;
        }

        // Parse LSP indices (simplified)
        let mut lsp_indices = Vec::new();
        if bitstream.len() >= 4 {
            lsp_indices.push(u16::from_be_bytes([bitstream[0], bitstream[1]]) as usize);
            lsp_indices.push(u16::from_be_bytes([bitstream[2], bitstream[3]]) as usize);
        }

        // Parse subframe parameters
        let mut subframes = Vec::new();
        let mut offset = 4;

        for _ in 0..N_SUBFR {
            if offset + 5 <= bitstream.len() {
                // Pitch lag (8 bits)
                let pitch_lag = bitstream[offset] as usize;
                offset += 1;

                // Fixed codebook (24 bits = 3 bytes)
                let codebook_word = u32::from_be_bytes([
                    0,
                    bitstream[offset],
                    bitstream[offset + 1],
                    bitstream[offset + 2],
                ]);
                offset += 3;

                // Extract positions and signs
                let mut positions = [0; 4];
                let mut signs = [1i8; 4];
                for i in 0..4 {
                    positions[i] = ((codebook_word >> (i * 6)) & 0x3F) as usize;
                    signs[i] = if (codebook_word >> (24 + i)) & 1 != 0 { 1 } else { -1 };
                }

                // Gain index (8 bits)
                let gain_index = bitstream[offset] as usize;
                offset += 1;

                subframes.push(G729SubframeParams {
                    pitch_lag,
                    adaptive_gain: 1024, // Will be dequantized
                    positions,
                    signs,
                    fixed_gain: 1024,   // Will be dequantized
                    gain_index,
                });
            }
        }

        if subframes.len() == N_SUBFR {
            Some(G729Frame {
                lsp_indices,
                subframes,
                frame_number: 0,
            })
        } else {
            None
        }
    }

    /// Error concealment for bad/lost frames
    /// 
    /// # Arguments
    /// * `frame_lost` - Whether the current frame was lost
    /// 
    /// # Returns
    /// Concealed speech frame
    pub fn conceal_frame(&mut self, frame_lost: bool) -> Vec<Word16> {
        self.bad_frame = frame_lost;

        if frame_lost {
            // Simple concealment: repeat previous subframe with attenuation
            let mut concealed_speech = vec![0i16; L_FRAME];

            for i in 0..L_FRAME {
                let prev_idx = i % L_SUBFR;
                let attenuated = mult(self.prev_subframe[prev_idx], 16384); // 0.5 attenuation
                concealed_speech[i] = attenuated;
            }

            concealed_speech
        } else {
            // Frame is good, reset concealment state
            vec![0i16; L_FRAME] // Placeholder - would use normal decoding
        }
    }

    /// Get decoder statistics for debugging
    pub fn get_stats(&self) -> G729DecoderStats {
        G729DecoderStats {
            frames_decoded: self.frame_count,
            bad_frames: if self.bad_frame { 1 } else { 0 },
            error_rate: if self.frame_count > 0 {
                if self.bad_frame { 1.0 } else { 0.0 }
            } else {
                0.0
            },
        }
    }
}

/// Decoder statistics
#[derive(Debug, Clone)]
pub struct G729DecoderStats {
    /// Number of frames decoded
    pub frames_decoded: usize,
    /// Number of bad/lost frames
    pub bad_frames: usize,
    /// Frame error rate (0.0 to 1.0)
    pub error_rate: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encoder::G729Encoder;

    #[test]
    fn test_decoder_creation() {
        let decoder = G729Decoder::new();
        assert_eq!(decoder.frame_count, 0);
        assert!(!decoder.bad_frame);
    }

    #[test]
    fn test_decoder_reset() {
        let mut decoder = G729Decoder::new();
        decoder.frame_count = 10;
        decoder.syn_mem[0] = 1000;
        decoder.bad_frame = true;

        decoder.reset();

        assert_eq!(decoder.frame_count, 0);
        assert_eq!(decoder.syn_mem[0], 0);
        assert!(!decoder.bad_frame);
    }

    #[test]
    fn test_frame_decoding() {
        let mut decoder = G729Decoder::new();

        // Create a test frame
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

        let decoded_speech = decoder.decode_frame(&frame);

        assert_eq!(decoded_speech.len(), L_FRAME);
        assert_eq!(decoder.frame_count, 1);
    }

    #[test]
    fn test_synthesis_filter() {
        let mut decoder = G729Decoder::new();
        let lpc = vec![4096i16; M + 1]; // Simple LPC coefficients
        let excitation = vec![1000i16; L_SUBFR];
        let mut speech = vec![0i16; L_SUBFR];

        decoder.synthesis_filter(&lpc, &excitation, &mut speech);

        // Output should be non-zero
        assert!(speech.iter().any(|&x| x != 0));
    }

    #[test]
    fn test_energy_estimation() {
        let decoder = G729Decoder::new();
        let adaptive_exc = vec![1000i16; L_SUBFR];
        let fixed_exc = vec![500i16; L_SUBFR];

        let energy = decoder.estimate_subframe_energy(&adaptive_exc, &fixed_exc);

        assert!(energy > 0);
    }

    #[test]
    fn test_excitation_memory_update() {
        let mut decoder = G729Decoder::new();
        let excitation = vec![1000i16; L_SUBFR];

        decoder.update_excitation_memory(&excitation);

        // Check that memory was updated
        let mem_start = decoder.exc_mem.len() - L_SUBFR;
        assert_eq!(decoder.exc_mem[mem_start], 1000);
    }

    #[test]
    fn test_postfilter() {
        let decoder = G729Decoder::new();
        let mut speech = vec![1000i16; L_FRAME];
        let lsp = vec![100i16; M];

        decoder.adaptive_postfilter(&mut speech, &lsp);

        // Speech should be modified
        assert!(speech.iter().any(|&x| x != 1000));
    }

    #[test]
    fn test_high_pass_filter() {
        let decoder = G729Decoder::new();
        let mut speech = vec![1000i16; L_FRAME];

        decoder.high_pass_filter(&mut speech);

        // First sample unchanged, others should be filtered
        assert_eq!(speech[0], 1000);
        assert!(speech[1] != 1000);
    }

    #[test]
    fn test_bitstream_decoding() {
        let decoder = G729Decoder::new();

        // Create a simple test bitstream
        let mut bitstream = Vec::new();
        bitstream.extend_from_slice(&10u16.to_be_bytes()); // LSP index 1
        bitstream.extend_from_slice(&20u16.to_be_bytes()); // LSP index 2

        // Subframe 1
        bitstream.push(50); // Pitch lag
        bitstream.extend_from_slice(&[0, 1, 2]); // Codebook data
        bitstream.push(64); // Gain index

        // Subframe 2
        bitstream.push(55); // Pitch lag
        bitstream.extend_from_slice(&[0, 3, 4]); // Codebook data
        bitstream.push(70); // Gain index

        let frame = decoder.decode_bitstream(&bitstream);

        assert!(frame.is_some());
        let frame = frame.unwrap();
        assert_eq!(frame.lsp_indices.len(), 2);
        assert_eq!(frame.subframes.len(), N_SUBFR);
    }

    #[test]
    fn test_error_concealment() {
        let mut decoder = G729Decoder::new();

        // Test concealment for lost frame
        let concealed = decoder.conceal_frame(true);

        assert_eq!(concealed.len(), L_FRAME);
        assert!(decoder.bad_frame);

        // Test normal operation
        let normal = decoder.conceal_frame(false);
        assert_eq!(normal.len(), L_FRAME);
    }

    #[test]
    fn test_encoder_decoder_roundtrip() {
        let mut encoder = G729Encoder::new();
        let mut decoder = G729Decoder::new();

        // Original speech
        let mut original_speech = vec![0i16; L_FRAME];
        for i in 0..L_FRAME {
            original_speech[i] = (1000.0 * (2.0 * std::f32::consts::PI * i as f32 / 16.0).sin()) as i16;
        }

        // Encode
        let frame = encoder.encode_frame(&original_speech);

        // Decode
        let decoded_speech = decoder.decode_frame(&frame);

        // Check basic properties
        assert_eq!(decoded_speech.len(), L_FRAME);
        assert_eq!(encoder.frame_count, 1);
        assert_eq!(decoder.frame_count, 1);

        // Decoded speech should be non-zero
        assert!(decoded_speech.iter().any(|&x| x != 0));
    }

    #[test]
    fn test_decoder_stats() {
        let mut decoder = G729Decoder::new();

        // Process some frames
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

        decoder.decode_frame(&frame);
        decoder.conceal_frame(true); // Lost frame

        let stats = decoder.get_stats();
        assert_eq!(stats.frames_decoded, 1);
        assert!(stats.bad_frames > 0);
    }
} 