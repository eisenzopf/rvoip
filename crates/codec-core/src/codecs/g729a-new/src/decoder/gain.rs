use crate::common::basic_operators::*;
use crate::common::oper_32b::*;

/// Gain decoder for pitch and fixed codebook gains
/// Based on DEC_GAIN.C from G.729A reference implementation
pub struct GainDecoder {
    past_qua_en: [Word16; 4],  // Past quantized energies Q10
}

// Gain quantization tables (same as encoder)
const NCODE1: usize = 8;    // Number of Codebook-1
const NCODE2: usize = 16;   // Number of Codebook-2
const NCAN1: usize = 4;     // Pre-selection order for Codebook-1  
const NCAN2: usize = 8;     // Pre-selection order for Codebook-2

// Codebook 1: Pitch gain (3 bits)
const GBK1: [[Word16; 2]; NCODE1] = [
    [1, 3624],     // 0.111328 gain, -3.424805 log
    [2, 7627],     // 0.232178 gain, -1.425781 log  
    [3, 12861],    // 0.392334 gain, -0.407227 log
    [4, 17793],    // 0.542969 gain, 0.348877 log
    [5, 22670],    // 0.691895 gain, 1.064941 log
    [6, 27344],    // 0.833496 gain, 1.699219 log
    [7, 31946],    // 0.974365 gain, 2.307129 log
    [8, 32767],    // 1.000000 gain, 2.326172 log
];

// Codebook 2: Fixed gain (4 bits)  
const GBK2: [[Word16; 2]; NCODE2] = [
    [159, -11363],
    [206, -10289],
    [268, -9225],
    [349, -8163],
    [454, -7102],
    [590, -6041],
    [767, -4980],
    [998, -3919],
    [1299, -2857],
    [1691, -1796],
    [2200, -735],
    [2864, 327],
    [3727, 1388],
    [4851, 2449],
    [6312, 3511],
    [8214, 4572],
];

// Map tables for index decoding
const MAP1: [i16; NCODE1] = [0, 1, 2, 3, 4, 5, 6, 7];
const MAP2: [i16; NCODE2] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

// MA prediction coefficients for gain prediction
const PRED: [Word16; 4] = [5571, 4751, 2785, 1556]; // Q13

impl GainDecoder {
    pub fn new() -> Self {
        Self {
            past_qua_en: [-14336; 4], // -14.0 in Q10
        }
    }
    
    /// Decode gains from quantization index
    /// Based on Dec_gain function in DEC_GAIN.C
    /// Returns (gain_pitch Q14, gain_code Q1)
    pub fn decode_gain(&mut self, gain_index: Word16, code: &[Word16]) -> (Word16, Word16) {
        // Decode the combined gain index
        let index1 = (gain_index / (NCODE2 as Word16)) as usize;
        let index2 = (gain_index % (NCODE2 as Word16)) as usize;
        
        // Ensure indices are valid
        let index1 = if index1 >= NCODE1 { 0 } else { index1 };
        let index2 = if index2 >= NCODE2 { 0 } else { index2 };
        
        // Step 1: Decode pitch gain directly from codebook
        let gain_pit = GBK1[index1][0]; // Q14
        
        // Step 2: Decode fixed codebook gain with prediction
        let (gcode0, exp_gcode0) = self.gain_predict(code, 40); // L_SUBFR = 40
        
        // Get quantized gain from codebook  
        let g_code = GBK2[index2][0]; // Q11
        let qua_ener = GBK2[index2][1]; // Q10
        
        // Apply gain prediction
        // gcode = gcode0 * g_code
        let exp_code = add(exp_gcode0, 5); // Q11 -> Q16 normalization
        let gain_cod = extract_h(l_shl(l_mult(gcode0, g_code), exp_code));
        
        // Ensure gain_cod is in Q1 format
        let gain_cod = shr(gain_cod, 4); // Adjust to Q1
        
        // Step 3: Update gain predictor memory
        self.gain_update(qua_ener);
        
        (gain_pit, gain_cod)
    }
    
    /// Gain prediction - predicts the fixed-codebook gain
    /// Similar to encoder version but used for decoding
    fn gain_predict(&self, code: &[Word16], l_subfr: i16) -> (Word16, Word16) {
        let mut l_tmp = 0i32;
        
        // Energy coming from code
        for i in 0..(l_subfr as usize) {
            l_tmp = l_mac(l_tmp, code[i], code[i]);
        }
        
        // Avoid division by zero
        if l_tmp == 0 {
            l_tmp = 1;
        }
        
        // Compute: means_ener - 10*log10(ener_code/L_subfr)  
        let (exp, frac) = log2(l_tmp);
        l_tmp = mpy_32_16(exp, frac, -24660); // -3.0103 in Q13
        
        // means_ener = 36 dB
        l_tmp = l_add(l_tmp, l_deposit_h(4096)); // 36 dB in Q13
        
        // Subtract 10*log10(L_subfr) = 10*log10(40) = 16.02 dB
        l_tmp = l_sub(l_tmp, 16151); // 16.02 dB in Q13
        
        // Compute gcode0
        l_tmp = l_shl(l_tmp, 10); // From Q13 to Q23
        for i in 0..4 {
            l_tmp = l_mac(l_tmp, PRED[i], self.past_qua_en[i]); // Q13*Q10 -> Q24
        }
        
        let gcode0 = extract_h(l_tmp); // From Q24 to Q8
        let exp_gcode0 = 8; // Q-format exponent
        
        (gcode0, exp_gcode0)
    }
    
    /// Update gain predictor memory
    fn gain_update(&mut self, qua_ener: Word16) {
        // Shift past energies
        for i in (1..4).rev() {
            self.past_qua_en[i] = self.past_qua_en[i - 1];
        }
        
        // Store new quantized energy
        self.past_qua_en[0] = qua_ener; // Already in Q10
    }
}

// Helper function for log2 computation (simplified version)
fn log2(l_x: Word32) -> (Word16, Word16) {
    if l_x <= 0 {
        return (0, 0);
    }
    
    let exp = norm_l(l_x);
    let normalized = l_shl(l_x, exp);
    let frac = extract_h(normalized);
    
    (sub(30, exp), frac)
}

// Helper function for 32x16 multiplication
fn mpy_32_16(hi: Word16, lo: Word16, n: Word16) -> Word32 {
    let temp = l_mult(hi, n);
    l_mac(temp, mult(lo, n), 1)
}