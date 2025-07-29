use crate::common::basic_operators::Word16;
use crate::common::tab_ld8a::{L_FRAME, L_SUBFR, M, MP1};
use crate::common::bits::{prm2bits, bits2prm, PRM_SIZE, SERIAL_SIZE};
use crate::encoder::pre_proc::PreProc;
use crate::encoder::lpc::Lpc;
use crate::encoder::lsp_quantizer::LspQuantizer;
use crate::encoder::gain_quantizer::GainQuantizer;
use crate::encoder::pitch::Pitch;
use crate::encoder::acelp_codebook::AcelpCodebook;
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
        let (lsp_index1, lsp_index2) = self.lsp_quantizer.quantize(&lsp, &mut lsp_q, &self.old_lsp_q);
        
        prm[0] = lsp_index1;
        prm[1] = lsp_index2;
        
        // Step 4: Perceptual weighting filter
        let mut wsp = vec![0i16; L_FRAME];
        self.perceptual_weighting.weight_speech(&speech_proc, &a_coeffs, &mut wsp, &mut self.mem_w);
        
        // Copy current frame's weighted speech into the history buffer
        // old_wsp layout: [PIT_MAX samples from previous frames][L_FRAME current frame]
        self.old_wsp[PIT_MAX..PIT_MAX + L_FRAME].copy_from_slice(&wsp);
        
        // Step 5: Open-loop pitch analysis
        // Pass the entire buffer including history (PIT_MAX + L_FRAME samples total)
        let t_op = self.pitch.open_loop_search(&self.old_wsp);
        
        // Process two subframes
        for subframe in 0..2 {
            let sf_start = subframe * L_SUBFR;
            
            // Step 6: Interpolate LSP for this subframe
            let mut a_subframe = [0i16; MP1];
            self.lsp_quantizer.interpolate_lsp(&self.old_lsp_q, &lsp_q, subframe, &mut a_subframe);
            
            // Step 7: Compute impulse response
            let mut h = [0i16; L_SUBFR];
            self.compute_impulse_response(&a_subframe, &mut h);
            
            // Step 8: Compute target signal
            let mut target_signal = [0i16; L_SUBFR];
            self.target.compute(&wsp[sf_start..], &a_subframe, &h, &mut target_signal, &mut self.mem_zero);
            
            // Step 9: Adaptive codebook search (closed-loop pitch)
            let (t0, t0_frac) = self.pitch.closed_loop_search(&target_signal, &self.old_exc, t_op, subframe);
            
            // Step 10: Fixed codebook search
            let fixed_index = self.acelp.search(&target_signal, &h, t0);
            
            // Step 11: Quantize gains (simplified for now)
            let ga_index = 0i16; // Placeholder - should be from gain quantizer
            let gb_index = 0i16; // Placeholder - should be from gain quantizer
            
            // Store parameters according to G.729A specification
            if subframe == 0 {
                // Subframe 1 parameters
                prm[2] = t0;            // P1: 8-bit pitch delay
                prm[4] = fixed_index;   // C1: 13-bit fixed codebook index + sign
                prm[5] = ga_index;      // GA1: 3-bit adaptive gain
                prm[6] = gb_index;      // GB1: 4-bit fixed gain
            } else {
                // Subframe 2 parameters
                prm[7] = t0 - prm[2];   // P2: 5-bit relative pitch delay
                prm[8] = fixed_index;   // C2: 13-bit fixed codebook index + sign
                prm[9] = ga_index;      // GA2: 3-bit adaptive gain
                prm[10] = gb_index;     // GB2: 4-bit fixed gain
            }
            
            // Update filter memories for next subframe
            // Note: simplified for now - real implementation would update memories
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
    
    /// Compute impulse response H(z) = W(z)/A(z)
    fn compute_impulse_response(&self, _a_coeffs: &[Word16], h: &mut [Word16]) {
        // Simplified version - full implementation would compute W(z)/A(z)
        // For now, just use a simple impulse
        h[0] = 4096; // 1.0 in Q12
        for i in 1..L_SUBFR {
            h[i] = 0;
        }
    }
    
}

