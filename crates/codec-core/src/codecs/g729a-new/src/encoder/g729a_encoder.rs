use crate::common::basic_operators::*;
use crate::common::oper_32b::*;
use crate::common::tab_ld8a::{L_FRAME, L_SUBFR, M, MP1};
use crate::common::bits::{PRM_SIZE, SERIAL_SIZE};
use crate::common::impulse_response::compute_impulse_response;
use crate::common::lsp_az::int_qlpc;
use crate::common::filter::syn_filt;
use crate::common::adaptive_codebook_common::pred_lt_3;
use crate::encoder::pre_proc::PreProc;
use crate::encoder::lpc::Lpc;
use crate::encoder::lspvq::LspQuantizer;
use crate::encoder::gain_quantizer::GainQuantizer;
use crate::encoder::pitch::Pitch;
use crate::encoder::acelp_codebook::{AcelpCodebook, acelp_code_a};
use crate::encoder::perceptual_weighting::PerceptualWeighting;
use crate::encoder::target::Target;

// Re-export for external use
pub use crate::common::bits::{prm2bits as export_prm2bits, bits2prm as export_bits2prm, PRM_SIZE as EXPORT_PRM_SIZE, SERIAL_SIZE as EXPORT_SERIAL_SIZE};

// G.729A constants
pub const PIT_MAX: usize = 143;      // Maximum pitch delay
pub const L_INTERPOL: usize = 11;    // Length of interpolation filter
pub const L_WINDOW: usize = 240;     // LPC analysis window size
pub const L_NEXT: usize = 40;       // Lookahead size
pub const L_PAST: usize = 120;      // Past speech buffer size

/// G.729A Encoder - Complete encoding pipeline
pub struct G729AEncoder {
    // Modules
    pre_proc: PreProc,
    lpc: Lpc,
    lsp_quantizer: LspQuantizer,
    gain_quantizer: GainQuantizer,
    pitch: Pitch,
    acelp: AcelpCodebook,
    perceptual_weighting: PerceptualWeighting,
    target: Target,
    
    // State buffers
    speech_buffer: [Word16; L_WINDOW],              // [old_speech | current | lookahead]
    old_speech: [Word16; L_FRAME],                  // Previous frame
    old_wsp: [Word16; L_FRAME + PIT_MAX],          // Weighted speech history
    old_exc: [Word16; L_FRAME + PIT_MAX + L_INTERPOL], // Excitation history
    mem_w0: [Word16; M],                            // Memory for W(z) filter
    mem_w: [Word16; M],                             // Memory for W(z) filter
    mem_zero: [Word16; M],                          // Memory for zero filter
    mem_syn: [Word16; M],                           // Synthesis filter memory
    
    // LSP state
    old_lsp: [Word16; M],                           // Previous LSP parameters
    old_lsp_q: [Word16; M],                         // Previous quantized LSP
    
    // Other state
    sharp: Word16,                                  // Sharpening parameter
    
    // Gain history for taming
    past_gain_pit: Word16,                          // Past pitch gain
    gain_pit_buffer: [Word16; 5],                   // History of pitch gains
    gain_buffer_index: usize,                       // Circular buffer index
}

impl G729AEncoder {
    /// Create a new G.729A encoder
    pub fn new() -> Self {
        Self {
            pre_proc: PreProc::new(),
            lpc: Lpc::new(),
            lsp_quantizer: LspQuantizer::new(),
            gain_quantizer: GainQuantizer::new(),
            pitch: Pitch::new(),
            acelp: AcelpCodebook::new(),
            perceptual_weighting: PerceptualWeighting::new(),
            target: Target::new(),
            
            speech_buffer: [0; L_WINDOW],
            old_speech: [0; L_FRAME],
            old_wsp: [0; L_FRAME + PIT_MAX],
            old_exc: [0; L_FRAME + PIT_MAX + L_INTERPOL],
            mem_w0: [0; M],
            mem_w: [0; M],
            mem_zero: [0; M],
            mem_syn: [0; M],
            
            old_lsp: [0; M],
            old_lsp_q: [0; M],
            
            sharp: 0,
            
            past_gain_pit: 0,
            gain_pit_buffer: [0; 5],
            gain_buffer_index: 0,
        }
    }
    
    /// Initialize the encoder (reset all state)
    pub fn init(&mut self) {
        *self = Self::new();
        
        // Initialize LSP values to default
        // These are typical initial LSP values for 8kHz speech
        self.old_lsp[0] = 30000;
        self.old_lsp[1] = 26000;
        self.old_lsp[2] = 21000;
        self.old_lsp[3] = 15000;
        self.old_lsp[4] = 8000;
        self.old_lsp[5] = 0;
        self.old_lsp[6] = -8000;
        self.old_lsp[7] = -15000;
        self.old_lsp[8] = -21000;
        self.old_lsp[9] = -26000;
        
        self.old_lsp_q.copy_from_slice(&self.old_lsp);
        
        // Initialize excitation buffer with small random values to avoid div by zero
        // This simulates background noise
        for i in 0..self.old_exc.len() {
            self.old_exc[i] = ((i as i16) % 3) - 1; // Small values: -1, 0, 1
        }
    }
    
    /// Encode one frame of speech (80 samples)
    /// Returns the analysis parameters
    pub fn encode_frame(&mut self, speech: &[Word16]) -> [Word16; PRM_SIZE] {
        assert_eq!(speech.len(), L_FRAME, "Input frame must be {} samples", L_FRAME);
        
        let mut prm = [0i16; PRM_SIZE];
        let mut speech_proc = speech.to_vec();
        
        // Step 1: Pre-processing (scaling + high-pass filtering)
        self.pre_proc.process(&mut speech_proc);
        
        // Update speech buffer for LPC analysis
        // Buffer layout: [80 old | 80 current | 80 lookahead]
        // Shift buffer left by L_FRAME samples
        for i in 0..160 {
            self.speech_buffer[i] = self.speech_buffer[i + L_FRAME];
        }
        // Add new speech frame at positions 160-239
        self.speech_buffer[160..240].copy_from_slice(&speech_proc);
        
        // Step 2: Linear Prediction Analysis (once per frame)
        let mut a_coeffs = [0i16; MP1];
        let mut r_h = [0i16; MP1];
        let mut r_l = [0i16; MP1];
        
        // Apply Hamming window and compute autocorrelation
        self.lpc.autocorrelation(&self.speech_buffer, M as i16, &mut r_h, &mut r_l);
        
        // Apply lag window
        self.lpc.lag_window(M as i16, &mut r_h, &mut r_l);
        
        // Levinson-Durbin to get LP coefficients
        let mut rc = [0i16; M];
        self.lpc.levinson(&r_h, &r_l, &mut a_coeffs, &mut rc);
        
        // Step 3: Convert LP to LSP and quantize
        let mut lsp = [0i16; M];
        crate::encoder::lsp_quantizer::az_lsp(&a_coeffs, &mut lsp, &self.old_lsp);
        
        // Quantize LSP parameters
        let mut lsp_q = [0i16; M];
        let mut ana = [0i16; 2];  // LSP indices
        self.lsp_quantizer.qua_lsp(&lsp, &mut lsp_q, &mut ana);
        
        prm[0] = ana[0];
        prm[1] = ana[1];
        
        // Step 4: Perceptual weighting filter
        let mut wsp = vec![0i16; L_FRAME];
        self.perceptual_weighting.weight_speech(&speech_proc, &a_coeffs, &mut wsp, &mut self.mem_w);
        
        // Copy current frame's weighted speech into the history buffer
        // old_wsp layout: [PIT_MAX samples from previous frames][L_FRAME current frame]
        self.old_wsp[PIT_MAX..PIT_MAX + L_FRAME].copy_from_slice(&wsp);
        
        // Step 5: Open-loop pitch analysis
        // Pass the entire buffer including history (PIT_MAX + L_FRAME samples total)
        let t_op = self.pitch.open_loop_search(&self.old_wsp);
        
        // Step 6: Interpolate LSP and convert to LP coefficients for both subframes
        let mut az = [0i16; 2 * MP1];  // LP coefficients for both subframes
        int_qlpc(&self.old_lsp_q, &lsp_q, &mut az);
        
        // Process two subframes
        for subframe in 0..2 {
            let sf_start = subframe * L_SUBFR;
            let az_offset = subframe * MP1;
            
            // Get LP coefficients for this subframe
            let mut a_subframe = [0i16; MP1];
            a_subframe.copy_from_slice(&az[az_offset..az_offset + MP1]);
            
            // Step 7: Compute impulse response
            let mut h = [0i16; L_SUBFR];
            compute_impulse_response(&a_subframe, &mut h);
            
            // Step 8: Compute target signal
            let mut target_signal = [0i16; L_SUBFR];
            self.target.compute(&wsp[sf_start..], &a_subframe, &h, &mut target_signal, &mut self.mem_zero);
            
            // Step 9: Adaptive codebook search (closed-loop pitch)
            let (t0, t0_frac) = self.pitch.closed_loop_search(&target_signal, &self.old_exc, t_op, subframe);
            
            // Step 10: Generate adaptive excitation using fractional delay
            let mut exc = [0i16; L_SUBFR];
            let mut y1 = [0i16; L_SUBFR];
            
            // Create a working buffer for pred_lt_3
            // We need to extract the relevant portion of old_exc for pred_lt_3
            let exc_start = PIT_MAX + L_INTERPOL + sf_start;
            let mut exc_work = vec![0i16; L_SUBFR + t0 as usize + 1 + L_INTERPOL as usize];
            
            // Copy the relevant portion of old_exc into working buffer
            let copy_start = exc_start.saturating_sub(t0 as usize + 1 + L_INTERPOL as usize);
            let copy_len = exc_work.len().min(self.old_exc.len() - copy_start);
            exc_work[..copy_len].copy_from_slice(&self.old_exc[copy_start..copy_start + copy_len]);
            
            // The output will be written at the end of exc_work buffer
            let out_offset = exc_work.len() - L_SUBFR;
            
            // Apply fractional delay interpolation
            pred_lt_3(&mut exc_work[out_offset..], t0, t0_frac, L_SUBFR as i16);
            
            // Copy the interpolated excitation to exc
            exc.copy_from_slice(&exc_work[out_offset..]);
            
            // Filter adaptive excitation through h to get y1
            for i in 0..L_SUBFR {
                let mut s = 0i32;
                for j in 0..=i {
                    s = l_mac(s, exc[j], h[i - j]);
                }
                y1[i] = round(l_shl(s, 3)); // Q12
            }
            
            // Ensure y1 has some energy to avoid division by zero
            if y1.iter().all(|&x| x == 0) {
                y1[0] = 1; // Minimal energy
            }
            
            // Step 11: Fixed codebook search
            let mut fixed_code = [0i16; L_SUBFR];
            let mut y2 = [0i16; L_SUBFR];
            let mut fixed_sign = 0i16;
            let fixed_index = acelp_code_a(&target_signal, &h, t0, self.sharp, &mut fixed_code, &mut y2, &mut fixed_sign);
            
            // Extract position and sign from the combined index
            let fixed_position = shr(fixed_index, 4);  // Upper 13 bits are positions
            let fixed_sign_bits = fixed_index & 0x000F;  // Lower 4 bits are signs
            
            // Step 12: Compute correlations for gain quantization
            let (g_coeff, exp_coeff) = self.compute_gain_correlations(&target_signal, &y1, &y2, &target_signal);
            
            // Debug: Check if y1 has any energy
            let mut y1_energy = 0i32;
            for i in 0..L_SUBFR {
                y1_energy = l_mac(y1_energy, y1[i], y1[i]);
            }
            if y1_energy == 0 && subframe == 0 {
                println!("WARNING: y1 has zero energy in first subframe!");
                println!("  exc[0..10]: {:?}", &exc[0..10]);
                println!("  y1[0..10]: {:?}", &y1[0..10]);
                println!("  t0={}, t0_frac={}", t0, t0_frac);
            }
            
            // Step 13: Compute taming flag and quantize gains
            let tameflag = self.compute_taming_flag();
            let (gain_index, gain_pit, gain_cod) = self.gain_quantizer.quantize_gain(
                &fixed_code, &g_coeff, &exp_coeff, L_SUBFR as i16, tameflag
            );
            
            // Update pitch gain history
            self.update_gain_history(gain_pit);
            
            // Store parameters according to G.729A specification
            if subframe == 0 {
                // Subframe 1 parameters
                prm[2] = t0;               // P1: 8-bit pitch delay
                prm[4] = fixed_position;   // C1: 13-bit fixed codebook positions
                prm[5] = fixed_sign_bits;  // S1: 4-bit fixed codebook signs
                prm[6] = gain_index;       // G1: 7-bit gains (GA1 + GB1)
            } else {
                // Subframe 2 parameters
                prm[7] = t0 - prm[2];      // P2: 5-bit relative pitch delay
                prm[8] = fixed_position;   // C2: 13-bit fixed codebook positions  
                prm[9] = fixed_sign_bits;  // S2: 4-bit fixed codebook signs
                prm[10] = gain_index;      // G2: 7-bit gains (GA2 + GB2)
            }
            
            // Step 14: Update excitation buffer with quantized excitation
            // exc = gain_pit * adaptive + gain_cod * fixed
            for i in 0..L_SUBFR {
                // Compute total excitation (Q0)
                let exc_adaptive = mult(exc[i], gain_pit); // Q0 = Q0 * Q14 >> 15
                let exc_fixed = mult(fixed_code[i], gain_cod); // Q0 = Q13 * Q1 >> 15
                let exc_total = add(exc_adaptive, shr(exc_fixed, 1)); // Align Q-formats
                
                // Update excitation buffer at correct position
                self.old_exc[PIT_MAX + L_INTERPOL + sf_start + i] = exc_total;
            }
            
            // Step 15: Update synthesis filter memory
            // Compute synthesis: speech = exc * 1/A(z)
            let mut synth = [0i16; L_SUBFR];
            syn_filt(&a_subframe, &self.old_exc[PIT_MAX + L_INTERPOL + sf_start..], 
                     &mut synth, L_SUBFR as i32, &mut self.mem_syn, true);
        }
        
        // Compute parity bit P0 for pitch delay of first subframe
        // Parity is computed on the 6 MSBs of P1
        let mut parity = 0i16;
        for i in 2..8 {
            parity ^= (prm[2] >> i) & 1;
        }
        prm[3] = parity;
        
        // Update state for next frame
        self.old_lsp.copy_from_slice(&lsp);
        self.old_lsp_q.copy_from_slice(&lsp_q);
        
        // Shift excitation buffer
        for i in 0..(PIT_MAX + L_INTERPOL) {
            self.old_exc[i] = self.old_exc[i + L_FRAME];
        }
        
        // Shift weighted speech buffer (keep last PIT_MAX samples for next frame)
        for i in 0..PIT_MAX {
            self.old_wsp[i] = self.old_wsp[i + L_FRAME];
        }
        
        prm
    }
    
    /// Compute correlation coefficients for gain quantization
    /// Returns (g_coeff, exp_coeff) arrays
    fn compute_gain_correlations(
        &self,
        target: &[Word16],      // Target signal
        y1: &[Word16],          // Filtered adaptive excitation
        y2: &[Word16],          // Filtered fixed excitation
        xn: &[Word16],          // Target for pitch search
    ) -> ([Word16; 5], [Word16; 5]) {
        let mut g_coeff = [0i16; 5];
        let mut exp_coeff = [0i16; 5];
        
        // Compute correlations for gain quantization
        // g_coeff[0] = <y1,y1>
        let mut l_acc = 0i32;
        for i in 0..L_SUBFR {
            l_acc = l_mac(l_acc, y1[i], y1[i]);
        }
        let exp = norm_l(l_acc);
        g_coeff[0] = extract_h(l_shl(l_acc, exp));
        exp_coeff[0] = sub(exp, 1);
        
        // g_coeff[1] = -2*<xn,y1>
        l_acc = 0;
        for i in 0..L_SUBFR {
            l_acc = l_mac(l_acc, xn[i], y1[i]);
        }
        let exp = norm_l(l_acc);
        g_coeff[1] = negate(extract_h(l_shl(l_acc, exp)));
        exp_coeff[1] = sub(exp, 2);
        
        // g_coeff[2] = <y2,y2>
        l_acc = 0;
        for i in 0..L_SUBFR {
            l_acc = l_mac(l_acc, y2[i], y2[i]);
        }
        let exp = norm_l(l_acc);
        g_coeff[2] = extract_h(l_shl(l_acc, exp));
        exp_coeff[2] = sub(exp, 1);
        
        // g_coeff[3] = -2*<xn,y2>
        l_acc = 0;
        for i in 0..L_SUBFR {
            l_acc = l_mac(l_acc, xn[i], y2[i]);
        }
        let exp = norm_l(l_acc);
        g_coeff[3] = negate(extract_h(l_shl(l_acc, exp)));
        exp_coeff[3] = sub(exp, 2);
        
        // g_coeff[4] = 2*<y1,y2>
        l_acc = 0;
        for i in 0..L_SUBFR {
            l_acc = l_mac(l_acc, y1[i], y2[i]);
        }
        let exp = norm_l(l_acc);
        g_coeff[4] = extract_h(l_shl(l_acc, exp));
        exp_coeff[4] = sub(exp, 1);
        
        (g_coeff, exp_coeff)
    }
    
    /// Compute taming flag based on pitch gain history
    /// Returns 1 if taming is needed, 0 otherwise
    fn compute_taming_flag(&self) -> Word16 {
        // Constants for taming decision
        const GPCLIP: Word16 = 15565;  // 0.95 in Q14
        const GPCLIP2: Word16 = 14746; // 0.90 in Q14  
        const GP0999: Word16 = 16383;  // 0.9999 in Q14
        
        // Check if past pitch gain was very high
        if self.past_gain_pit > GPCLIP {
            return 1;
        }
        
        // Check if recent gains have been consistently high
        let mut high_gain_count = 0;
        for i in 0..5 {
            if self.gain_pit_buffer[i] > GPCLIP2 {
                high_gain_count += 1;
            }
        }
        
        // If 3 or more recent gains were high, apply taming
        if high_gain_count >= 3 {
            return 1;
        }
        
        0
    }
    
    /// Update pitch gain history
    fn update_gain_history(&mut self, gain_pit: Word16) {
        self.past_gain_pit = gain_pit;
        self.gain_pit_buffer[self.gain_buffer_index] = gain_pit;
        self.gain_buffer_index = (self.gain_buffer_index + 1) % 5;
    }
}

