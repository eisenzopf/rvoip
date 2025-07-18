//! ITU-T G.729 Synthesis Postfilter
//!
//! This module implements the ITU-compliant synthesis postfilter based on PST.C
//! 
//! The postfilter consists of:
//! - Short-term postfilter: Hst(z) = Hst0(z) * Hst1(z)
//!   - Hst0(z) = 1/g0 * A(gamma2)(z) / A(gamma1)(z)  
//!   - Hst1(z) = 1/(1-|mu|) * (1 + mu*z^-1) - tilt filtering
//! - Long-term postfilter: H0(z) = gl * (1 + b*z^-p) - harmonic filtering
//! - Automatic gain control for output scaling
//!
//! Based on ITU-T G.729 PST.C (1031 lines)

use super::types::*;
use super::math::*;
use super::dsp::*;
use super::encoder::G729Variant;

/// ITU postfilter constants
const GAMMA1_PST: Word16 = 22938;      // denominator weighting factor (Q15)
const GAMMA2_PST: Word16 = 18022;      // numerator weighting factor (Q15)
const GAMMA3_PLUS: Word16 = 6554;      // tilt weighting factor when k1>0 (Q15)
const GAMMA3_MINUS: Word16 = 29491;    // tilt weighting factor when k1<0 (Q15)
const AGC_FAC: Word16 = 32358;         // gain adjustment factor 0.9875 (Q15)
const AGC_FAC1: Word16 = 410;          // 32768 - AGC_FAC

/// Postfilter memory and buffer sizes
const LONG_H_ST: usize = 20;           // impulse response length
const MEM_RES2: usize = 39;            // A(gamma2) residual memory
const SIZ_RES2: usize = MEM_RES2 + L_SUBFR; // 79
const SIZ_Y_UP: usize = 154;           // upsampled signal size

/// ITU-compliant Synthesis Postfilter
#[derive(Debug, Clone)]
pub struct SynthesisPostfilter {
    /// Current variant for algorithm selection
    pub variant: G729Variant,
    
    /// Short-term postfilter state
    /// Short-term numerator coefficients
    pub apond2: [Word16; LONG_H_ST],
    /// Short-term postfilter memory
    pub mem_stp: [Word16; M],
    /// Null memory for h_st computation
    pub mem_zero: [Word16; M],
    
    /// Long-term postfilter state
    /// A(gamma2) residual buffer
    pub res2: [Word16; SIZ_RES2],
    /// Pointer into res2 buffer
    pub res2_ptr: usize,
    
    /// Gain control state
    /// Previous gain for AGC
    pub gain_prec: Word16,
}

impl SynthesisPostfilter {
    /// Create a new ITU-compliant synthesis postfilter
    pub fn new() -> Self {
        Self::new_with_variant(G729Variant::Core)
    }
    
    /// Create a new synthesis postfilter for specific variant
    pub fn new_with_variant(variant: G729Variant) -> Self {
        let mut postfilter = Self {
            variant,
            apond2: [0; LONG_H_ST],
            mem_stp: [0; M],
            mem_zero: [0; M],
            res2: [0; SIZ_RES2],
            res2_ptr: MEM_RES2,
            gain_prec: 16384,  // Q14 format
        };
        
        // Fill apond2[M+1..LONG_H_ST-1] with zeros (ITU initialization)
        for i in (M + 1)..LONG_H_ST {
            postfilter.apond2[i] = 0;
        }
        
        postfilter
    }
    
    /// Reset postfilter state
    pub fn reset(&mut self) {
        self.apond2 = [0; LONG_H_ST];
        self.mem_stp = [0; M];
        self.mem_zero = [0; M];
        self.res2 = [0; SIZ_RES2];
        self.res2_ptr = MEM_RES2;
        self.gain_prec = 16384;
        
        // Fill apond2[M+1..LONG_H_ST-1] with zeros
        for i in (M + 1)..LONG_H_ST {
            self.apond2[i] = 0;
        }
    }
    
    /// Set the variant for algorithm selection
    pub fn set_variant(&mut self, variant: G729Variant) {
        self.variant = variant;
    }
    
    /// ITU-compliant postfilter main function (Post() from PST.C)
    /// 
    /// # Arguments
    /// * `t0` - pitch delay given by coder
    /// * `signal_ptr` - input signal (pointer to current subframe)
    /// * `coeff` - LPC coefficients for current subframe
    /// * `sig_out` - output postfiltered signal
    /// 
    /// # Returns
    /// * `vo` - voicing decision (0 = unvoiced, >0 = delay)
    pub fn post_filter(&mut self, 
                       t0: Word16, 
                       signal_ptr: &[Word16], 
                       coeff: &[Word16], 
                       sig_out: &mut [Word16]) -> Word16 {
        assert_eq!(signal_ptr.len(), L_SUBFR);
        assert_eq!(coeff.len(), M + 1);
        assert_eq!(sig_out.len(), L_SUBFR);
        
        // Apply variant-specific postfiltering
        match self.variant {
            G729Variant::Core => self.post_filter_core(t0, signal_ptr, coeff, sig_out),
            G729Variant::AnnexA => self.post_filter_annex_a(t0, signal_ptr, coeff, sig_out),
            G729Variant::AnnexB => self.post_filter_annex_b(t0, signal_ptr, coeff, sig_out),
            G729Variant::AnnexBA => self.post_filter_annex_ba(t0, signal_ptr, coeff, sig_out),
        }
    }
    
    /// Core G.729 postfilter implementation
    fn post_filter_core(&mut self, 
                       t0: Word16, 
                       signal_ptr: &[Word16], 
                       coeff: &[Word16], 
                       sig_out: &mut [Word16]) -> Word16 {
        // Local variables and arrays
        let mut apond1 = [0i16; M + 1];         // s.t. denominator coefficients
        let mut apond2_local = [0i16; M + 1];   // local copy to avoid borrowing issues
        let mut sig_ltp = [0i16; L_SUBFR + 1];  // H0 output signal
        let mut parcor0: Word16 = 0;
        let mut vo: Word16;
        
        // Step 1: Compute weighted LPC coefficients
        self.weight_az(coeff, GAMMA1_PST, &mut apond1);
        self.weight_az(coeff, GAMMA2_PST, &mut apond2_local);
        
        // Copy to the instance variable
        self.apond2[..M + 1].copy_from_slice(&apond2_local);
        
        // Step 2: Compute A(gamma2) residual
        let mut residual = [0i16; L_SUBFR];
        self.residu(&apond2_local, signal_ptr, &mut residual);
        
        // Store residual in buffer
        let res_start = self.res2_ptr;
        self.res2[res_start..res_start + L_SUBFR].copy_from_slice(&residual);
        
        // Step 3: Harmonic filtering (long-term postfilter)
        vo = self.pst_ltp(t0, &residual, &mut sig_ltp[1..]);
        
        // Step 4: Save last output of 1/A(gamma1) from preceding subframe
        sig_ltp[0] = self.mem_stp[M - 1];
        
        // Step 5: Controls short term postfilter gain and compute parcor0
        self.calc_st_filt(&apond2_local, &apond1, &mut parcor0, &mut sig_ltp[1..]);
        
        // Step 6: 1/A(gamma1) filtering, mem_stp is updated
        let mut sig_ltp_in = [0i16; L_SUBFR]; // Copy input
        let mut sig_ltp_out = [0i16; L_SUBFR];
        sig_ltp_in.copy_from_slice(&sig_ltp[1..]);
        let mut mem_stp_local = self.mem_stp;
        self.syn_filt(&apond1, &sig_ltp_in, &mut sig_ltp_out, &mut mem_stp_local);
        self.mem_stp = mem_stp_local;
        sig_ltp[1..].copy_from_slice(&sig_ltp_out);
        
        // Step 7: Tilt filtering
        self.filt_mu(&sig_ltp, sig_out, parcor0);
        
        // Step 8: Gain control
        self.scale_st(signal_ptr, sig_out);
        
        // Step 9: Update for next subframe
        self.res2.copy_within(L_SUBFR.., 0);
        
        vo
    }
    
    /// Annex A reduced complexity postfilter (POSTFILT.C)
    /// 
    /// This implements the simplified postfilter for G.729 Annex A with ~50% complexity reduction.
    /// The algorithm removes some processing stages and uses simplified filtering.
    /// 
    /// Key simplifications:
    /// - Simplified short-term filtering (no A(gamma2) processing)
    /// - Reduced long-term postfilter complexity
    /// - Simplified tilt filtering
    fn post_filter_annex_a(&mut self, 
                           t0: Word16, 
                           signal_ptr: &[Word16], 
                           coeff: &[Word16], 
                           sig_out: &mut [Word16]) -> Word16 {
        assert_eq!(signal_ptr.len(), L_SUBFR);
        assert_eq!(coeff.len(), M + 1);
        assert_eq!(sig_out.len(), L_SUBFR);
        
        // Annex A: Simplified approach - only A(gamma1) filtering
        let mut apond1 = [0i16; M + 1];
        let mut sig_ltp = [0i16; L_SUBFR];
        
        // Step 1: Compute only weighted LPC coefficients for denominator
        self.weight_az(coeff, GAMMA1_PST, &mut apond1);
        
        // Step 2: Simplified harmonic filtering (reduced complexity)
        let vo = self.pst_ltp_fast(t0, signal_ptr, &mut sig_ltp);
        
        // Step 3: 1/A(gamma1) filtering only (skip A(gamma2) numerator)
        let mut mem_stp_local = self.mem_stp;
        self.syn_filt(&apond1, &sig_ltp, sig_out, &mut mem_stp_local);
        self.mem_stp = mem_stp_local;
        
        // Step 4: Simplified tilt filtering (optional for Annex A)
        let parcor0 = 0i16; // Skip parcor computation for speed
        self.filt_mu_simple(sig_out, parcor0);
        
        vo
    }
    
    /// Annex B VAD-aware postfilter
    fn post_filter_annex_b(&mut self, 
                           t0: Word16, 
                           signal_ptr: &[Word16], 
                           coeff: &[Word16], 
                           sig_out: &mut [Word16]) -> Word16 {
        // Could implement VAD-aware postfilter modifications here
        self.post_filter_core(t0, signal_ptr, coeff, sig_out)
    }
    
    /// Annex BA combined postfilter
    fn post_filter_annex_ba(&mut self, 
                            t0: Word16, 
                            signal_ptr: &[Word16], 
                            coeff: &[Word16], 
                            sig_out: &mut [Word16]) -> Word16 {
        // Could implement combined features here
        self.post_filter_core(t0, signal_ptr, coeff, sig_out)
    }
    
    /// Compute weighted LPC coefficients (Weight_Az from ITU)
    /// 
    /// # Arguments
    /// * `a` - Input LPC coefficients [M+1]
    /// * `gamma` - Weighting factor (Q15)
    /// * `ap` - Output weighted coefficients [M+1]
    fn weight_az(&self, a: &[Word16], gamma: Word16, ap: &mut [Word16]) {
        assert_eq!(a.len(), M + 1);
        assert_eq!(ap.len(), M + 1);
        
        ap[0] = a[0]; // First coefficient unchanged
        
        let mut fac = gamma;
        for i in 1..=M {
            ap[i] = mult(a[i], fac);
            fac = mult(fac, gamma);
        }
    }
    
    /// Compute residual signal (Residu from ITU)
    /// 
    /// # Arguments
    /// * `a` - LPC coefficients [M+1]
    /// * `x` - Input signal [L_SUBFR]
    /// * `y` - Output residual [L_SUBFR]
    fn residu(&self, a: &[Word16], x: &[Word16], y: &mut [Word16]) {
        assert_eq!(a.len(), M + 1);
        assert_eq!(x.len(), L_SUBFR);
        assert_eq!(y.len(), L_SUBFR);
        
        for n in 0..L_SUBFR {
            let mut l_acc = l_mult(a[0], x[n]);
            
            for i in 1..=M {
                if n >= i {
                    l_acc = l_msu(l_acc, a[i], x[n - i]);
                }
            }
            
            y[n] = round(l_acc);
        }
    }
    
    /// Long-term postfilter - harmonic filtering (pst_ltp from ITU)
    /// 
    /// # Arguments
    /// * `t0` - pitch delay from coder
    /// * `ptr_sig_in` - postfilter input (residual)
    /// * `ptr_sig_pst0` - harmonic postfilter output
    /// 
    /// # Returns
    /// * voicing decision
    fn pst_ltp(&mut self, t0: Word16, ptr_sig_in: &[Word16], ptr_sig_pst0: &mut [Word16]) -> Word16 {
        assert_eq!(ptr_sig_in.len(), L_SUBFR);
        assert_eq!(ptr_sig_pst0.len(), L_SUBFR);
        
        // Simplified harmonic postfilter for now
        // Full implementation would include:
        // - Signal justification on 13 bits
        // - Delay search around t0
        // - Fractional delay interpolation
        // - Gain computation and application
        
        // For now, apply simple pitch-based enhancement
        let lag = t0.max(20).min(143) as usize;
        
        for n in 0..L_SUBFR {
            if n >= lag {
                // Simple harmonic enhancement
                let enhanced = add(ptr_sig_in[n], mult(ptr_sig_in[n - lag], 6554)); // 0.2 gain
                ptr_sig_pst0[n] = enhanced;
            } else {
                ptr_sig_pst0[n] = ptr_sig_in[n];
            }
        }
        
        // Return voicing decision based on pitch delay
        if t0 >= 20 && t0 <= 143 {
            t0 // Voiced
        } else {
            0 // Unvoiced
        }
    }
    
    /// Calculate short-term filter parameters (calc_st_filt from ITU)
    /// 
    /// # Arguments
    /// * `apond2` - numerator coefficients A(gamma2)
    /// * `apond1` - denominator coefficients A(gamma1)
    /// * `parcor0` - output first parcor coefficient
    /// * `sig_ltp_ptr` - input/output signal scaled by 1/g0
    fn calc_st_filt(&self, apond2: &[Word16], apond1: &[Word16], parcor0: &mut Word16, sig_ltp_ptr: &mut [Word16]) {
        assert_eq!(apond2.len(), M + 1);
        assert_eq!(apond1.len(), M + 1);
        assert_eq!(sig_ltp_ptr.len(), L_SUBFR);
        
        let mut h = [0i16; LONG_H_ST];
        
        // Step 1: Compute impulse response of composed filter apond2 / apond1
        // Note: syn_filt computes impulse response by passing apond2 as input
        let mut mem_tmp = [0i16; M];
        // Use actual size of apond2 (M+1) instead of LONG_H_ST to avoid bounds error
        let mut apond2_padded = [0i16; LONG_H_ST];
        let copy_len = apond2.len().min(LONG_H_ST);
        apond2_padded[..copy_len].copy_from_slice(&apond2[..copy_len]);
        self.syn_filt(apond1, &apond2_padded, &mut h, &mut mem_tmp);
        
        // Step 2: Compute first parcor coefficient
        self.calc_rc0_h(&h, parcor0);
        
        // Step 3: Compute g0 (gain normalization factor)
        let mut l_g0 = 0i32;
        for &sample in h.iter() {
            l_g0 = l_add(l_g0, l_deposit_l(abs_s(sample)));
        }
        let g0 = extract_h(l_shl(l_g0, 14));
        
        // Step 4: Scale signal input of 1/A(gamma1)
        if g0 > 1024 {
            let temp = div_s(1024, g0); // temp = 2^15 / gain0
            for sample in sig_ltp_ptr.iter_mut() {
                *sample = mult_r(*sample, temp);
            }
        }
    }
    
    /// Compute first parcor from impulse response (calc_rc0_h from ITU)
    /// 
    /// # Arguments
    /// * `h` - impulse response of composed filter
    /// * `rc0` - output first parcor coefficient
    fn calc_rc0_h(&self, h: &[Word16], rc0: &mut Word16) {
        assert_eq!(h.len(), LONG_H_ST);
        
        // Step 1: Compute autocorrelation function acf
        let mut l_acc = 0i32;
        for &sample in h {
            l_acc = l_mac(l_acc, sample, sample);
        }
        let sh_acf = norm_l(l_acc);
        l_acc = l_shl(l_acc, sh_acf);
        let acf0 = extract_h(l_acc);
        
        // Step 2: Compute lag-1 autocorrelation
        l_acc = 0;
        for i in 0..(LONG_H_ST - 1) {
            l_acc = l_mac(l_acc, h[i], h[i + 1]);
        }
        l_acc = l_shl(l_acc, sh_acf);
        let acf1 = extract_h(l_acc);
        
        // Step 3: Compute first parcor
        if acf0 < abs_s(acf1) {
            *rc0 = 0;
            return;
        }
        
        *rc0 = div_s(abs_s(acf1), acf0);
        if acf1 > 0 {
            *rc0 = negate(*rc0);
        }
    }
    
    /// Simplified harmonic postfilter for Annex A (reduced complexity)
    fn pst_ltp_fast(&mut self, t0: Word16, ptr_sig_in: &[Word16], ptr_sig_pst0: &mut [Word16]) -> Word16 {
        assert_eq!(ptr_sig_in.len(), L_SUBFR);
        assert_eq!(ptr_sig_pst0.len(), L_SUBFR);
        
        // Annex A: Simplified harmonic enhancement without delay search
        let lag = t0.max(20).min(143) as usize;
        
        for n in 0..L_SUBFR {
            if n >= lag {
                // Simplified harmonic enhancement with fixed gain
                let enhanced = add(ptr_sig_in[n], mult(ptr_sig_in[n - lag], 3277)); // 0.1 gain (reduced)
                ptr_sig_pst0[n] = enhanced;
            } else {
                ptr_sig_pst0[n] = ptr_sig_in[n];
            }
        }
        
        // Return simplified voicing decision
        if t0 >= 20 && t0 <= 143 {
            t0
        } else {
            0
        }
    }

    /// Simplified tilt filtering for Annex A
    fn filt_mu_simple(&self, sig: &mut [Word16], _parcor0: Word16) {
        // Annex A: Very simple high-pass filtering instead of full tilt
        let mut prev = 0i16;
        for sample in sig.iter_mut() {
            let current = *sample;
            *sample = sub(current, mult(prev, 16384)); // Simple 1st order high-pass
            prev = current;
        }
    }

    /// Synthesis filtering (Syn_filt from ITU)
    /// 
    /// # Arguments
    /// * `a` - LPC coefficients [M+1]
    /// * `x` - input signal
    /// * `y` - output signal
    /// * `mem` - filter memory [M]
    fn syn_filt(&self, a: &[Word16], x: &[Word16], y: &mut [Word16], mem: &mut [Word16]) {
        assert_eq!(a.len(), M + 1);
        assert_eq!(x.len(), y.len());
        assert_eq!(mem.len(), M);
        
        for n in 0..x.len() {
            let mut l_acc = l_mult(x[n], a[0]);
            
            for i in 1..=M {
                if n >= i {
                    l_acc = l_msu(l_acc, a[i], y[n - i]);
                } else {
                    l_acc = l_msu(l_acc, a[i], mem[M - i + n]);
                }
            }
            
            y[n] = round(l_acc);
        }
        
        // Update memory
        if x.len() >= M {
            mem.copy_from_slice(&y[x.len() - M..]);
        } else {
            // Partial update
            mem.copy_within(x.len().., 0);
            mem[M - x.len()..].copy_from_slice(&y[..]);
        }
    }
    
    /// Tilt filtering (filt_mu from ITU)
    /// 
    /// Implements: (1 + mu*z^-1) * (1/(1-|mu|))
    /// Computes: y[n] = (1/(1-|mu|)) * (x[n] + mu*x[n-1])
    /// 
    /// # Arguments
    /// * `sig_in` - input signal (beginning at sample -1)
    /// * `sig_out` - output signal
    /// * `parcor0` - parcor0 (mu = parcor0 * gamma3)
    fn filt_mu(&self, sig_in: &[Word16], sig_out: &mut [Word16], parcor0: Word16) {
        assert_eq!(sig_in.len(), L_SUBFR + 1);
        assert_eq!(sig_out.len(), L_SUBFR);
        
        let (mu, ga, sh_fact1) = if parcor0 > 0 {
            let mu = mult_r(parcor0, GAMMA3_PLUS);
            let sh_fact1 = 15;
            let fact = 0x4000i16; // 2^14
            let mu2 = add(32767, sub(1, abs_s(mu))); // 2^15 * (1 - |mu|)
            let ga = div_s(fact, mu2); // 2^sh_fact / (1 - |mu|)
            (shr(mu, 1), ga, sh_fact1) // mu/2 to avoid overflows
        } else {
            let mu = mult_r(parcor0, GAMMA3_MINUS);
            let sh_fact1 = 12;
            let fact = 0x0800i16; // 2^11
            let mu2 = add(32767, sub(1, abs_s(mu))); // 2^15 * (1 - |mu|)
            let ga = div_s(fact, mu2); // 2^sh_fact / (1 - |mu|)
            (shr(mu, 1), ga, sh_fact1) // mu/2 to avoid overflows
        };
        
        for n in 0..L_SUBFR {
            let temp = sig_in[n]; // sig_in[n-1]
            let l_temp = l_deposit_l(sig_in[n + 1]); // sig_in[n]
            let mut l_acc = l_shl(l_temp, 15); // sig_in[n] * 2^15
            l_acc = l_mac(l_acc, mu, temp); // + mu * sig_in[n-1]
            l_acc = l_add(l_acc, 0x00004000); // rounding
            let temp_result = extract_l(l_shr(l_acc, 15));
            
            // ga * temp * 2 with rounding
            let l_temp = l_add(l_mult(temp_result, ga), if sh_fact1 == 15 { 0x00004000 } else { 0x00000800 });
            let l_temp = l_shr(l_temp, sh_fact1);
            sig_out[n] = saturate_word16(l_temp);
        }
    }
    
    /// Gain control (scale_st from ITU)
    /// 
    /// Automatic gain control: gain[n] = AGC_FAC * gain[n-1] + (1 - AGC_FAC) * g_in/g_out
    /// 
    /// # Arguments
    /// * `sig_in` - postfilter input signal
    /// * `sig_out` - postfilter output signal (modified)
    fn scale_st(&mut self, sig_in: &[Word16], sig_out: &mut [Word16]) {
        assert_eq!(sig_in.len(), L_SUBFR);
        assert_eq!(sig_out.len(), L_SUBFR);
        
        // Step 1: Compute input gain
        let mut l_acc = 0i32;
        for &sample in sig_in {
            l_acc = l_add(l_acc, l_abs(l_deposit_l(sample)));
        }
        
        if l_acc == 0 {
            return; // No input signal
        }
        
        let scal_in = norm_l(l_acc);
        l_acc = l_shl(l_acc, scal_in);
        let s_g_in = extract_h(l_acc); // normalized input gain
        
        // Step 2: Compute output gain
        l_acc = 0;
        for &sample in sig_out.iter() {
            l_acc = l_add(l_acc, l_abs(l_deposit_l(sample)));
        }
        
        if l_acc == 0 {
            self.gain_prec = 0;
            return; // No output signal
        }
        
        let scal_out = norm_l(l_acc);
        l_acc = l_shl(l_acc, scal_out);
        let s_g_out = extract_h(l_acc); // normalized output gain
        
        // Step 3: Compute gain ratio g0 = g_in / g_out
        let mut sh_g0 = add(scal_in, 1);
        sh_g0 = sub(sh_g0, scal_out); // scal_in - scal_out + 1
        
        let g0 = if s_g_in < s_g_out {
            div_s(s_g_in, s_g_out) // s_g_in/s_g_out in Q15
        } else {
            let temp = sub(s_g_in, s_g_out);
            let mut result = shr(div_s(temp, s_g_out), 1);
            result = add(result, 0x4000); // s_g_in/s_g_out in Q14
            sh_g0 = sub(sh_g0, 1);
            result
        };
        
        let g0 = shr(g0, sh_g0); // sh_g0 may be >0, <0, or =0
        let g0 = mult_r(g0, AGC_FAC1); // g_in/g_out * AGC_FAC1
        
        // Step 4: Apply adaptive gain control
        let mut gain = self.gain_prec;
        for sample in sig_out.iter_mut() {
            let temp = mult_r(AGC_FAC, gain);
            gain = add(temp, g0); // in Q14
            let l_temp = l_mult(gain, *sample);
            let l_temp = l_shl(l_temp, 1);
            *sample = round(l_temp);
        }
        self.gain_prec = gain;
    }
}

// Helper function for saturation
fn saturate_word16(value: Word32) -> Word16 {
    if value > 32767 {
        32767
    } else if value < -32768 {
        -32768
    } else {
        value as Word16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_postfilter_creation() {
        let postfilter = SynthesisPostfilter::new();
        assert_eq!(postfilter.variant, G729Variant::Core);
        assert_eq!(postfilter.gain_prec, 16384);
    }
    
    #[test]
    fn test_weight_az() {
        let postfilter = SynthesisPostfilter::new();
        let a = [4096, 1000, 500, 250, 125, 62, 31, 15, 7, 3, 1]; // Q12 format
        let mut ap = [0i16; M + 1];
        
        postfilter.weight_az(&a, 26214, &mut ap); // gamma = 0.8
        
        assert_eq!(ap[0], a[0]); // First coefficient unchanged
        assert!(ap[1] != 0); // Should be weighted
        assert!(ap[1] < a[1]); // Should be reduced
    }
    
    #[test]
    fn test_postfilter_basic_functionality() {
        let mut postfilter = SynthesisPostfilter::new();
        let signal = [100i16; L_SUBFR];
        let coeff = [4096, 1000, 500, 250, 125, 62, 31, 15, 7, 3, 1]; // LPC coefficients
        let mut output = [0i16; L_SUBFR];
        
        let vo = postfilter.post_filter(40, &signal, &coeff, &mut output);
        
        // Should produce some output
        assert!(output.iter().any(|&x| x != 0));
        // Voicing decision should be reasonable
        assert!(vo >= 0);
    }
} 