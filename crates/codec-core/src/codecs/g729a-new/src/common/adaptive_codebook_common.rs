use crate::common::basic_operators::{
    add, l_abs, l_mac, l_sub, l_shl, negate, norm_l, round, sub, 
    Word16, Word32,
};
use crate::common::tab_ld8a::{inter_3l, UP_SAMP, L_INTER10, L_SUBFR};

/// Compute correlations of impulse response h[] with the target vector x[].
/// This is equivalent to the C function Cor_h_X().
pub fn cor_h_x(h: &[Word16], x: &[Word16], d: &mut [Word16]) {
    let mut y32 = [0i32; L_SUBFR];
    let mut max: Word32 = 0;

    // First keep the result on 32 bits and find absolute maximum
    for i in 0..L_SUBFR {
        let mut s: Word32 = 0;
        for j in i..L_SUBFR {
            s = l_mac(s, x[j], h[j - i]);
        }
        y32[i] = s;

        let s_abs = l_abs(s);
        let l_temp = l_sub(s_abs, max);
        if l_temp > 0 {
            max = s_abs;
        }
    }

    // Find the number of right shifts to do on y32[]
    // so that maximum is on 13 bits
    let mut j = norm_l(max);
    if sub(j, 16) > 0 {
        j = 16;
    }

    j = sub(j, 3); // to be sure of no overflow

    for i in 0..L_SUBFR {
        d[i] = extract_l(l_shl(y32[i], j as Word16));
    }
}

/// Compute scalar product between two vectors.
/// This is equivalent to the C function Dot_Product().
pub fn dot_product(x: &[Word16], y: &[Word16], lg: Word16) -> Word32 {
    let mut sum: Word32 = 0;
    for i in 0..(lg as usize) {
        sum = l_mac(sum, x[i], y[i]);
    }
    sum
}

/// Long-term prediction with fractional delay.
/// This is equivalent to the C function Pred_lt_3().
pub fn pred_lt_3(exc: &mut [Word16], t0: Word16, mut frac: Word16, l_subfr: Word16) {
    frac = negate(frac);
    let mut x0_offset = -(t0 as isize);
    
    if frac < 0 {
        frac = add(frac, UP_SAMP);
        x0_offset -= 1;
    }

    for j in 0..(l_subfr as usize) {
        let x0_idx = j as isize + x0_offset;
        
        let mut s: Word32 = 0;
        let mut k = 0;
        
        for i in 0..(L_INTER10 as usize) {
            let x1_idx = (x0_idx - i as isize) as usize;
            let x2_idx = (x0_idx + 1 + i as isize) as usize;
            let c1_idx = (frac as usize) + k;
            let c2_idx = (sub(UP_SAMP, frac) as usize) + k;
            
            // Bounds checking for safety
            if x1_idx < exc.len() && x2_idx < exc.len() && 
               c1_idx < inter_3l.len() && c2_idx < inter_3l.len() {
                s = l_mac(s, exc[x1_idx], inter_3l[c1_idx]);
                s = l_mac(s, exc[x2_idx], inter_3l[c2_idx]);
            }
            
            k += UP_SAMP as usize;
        }

        exc[j] = round(s);
    }
}

/// Fast pitch search for adaptive codebook.
/// This is equivalent to the C function Pitch_fr3_fast().
pub fn pitch_fr3_fast(
    exc: &mut [Word16],
    xn: &[Word16], 
    h: &[Word16],
    l_subfr: Word16,
    t0_min: Word16,
    t0_max: Word16,
    i_subfr: Word16,
    pit_frac: &mut Word16,
) -> Word16 {
    let mut dn = [0i16; L_SUBFR];
    let mut exc_tmp = [0i16; L_SUBFR];
    
    // Compute correlation of target vector with impulse response
    cor_h_x(h, xn, &mut dn);
    
    // Find maximum integer delay
    let mut max: Word32 = crate::common::basic_operators::MIN_32;
    let mut t0 = t0_min; // Only to remove warning
    
    for t in t0_min..=t0_max {
        // Create a slice starting from exc[-t]
        let exc_offset = exc.len() - t as usize;
        if exc_offset >= l_subfr as usize {
            let exc_slice = &exc[exc_offset - l_subfr as usize..exc_offset];
            let corr = dot_product(&dn, exc_slice, l_subfr);
            let l_temp = l_sub(corr, max);
            if l_temp > 0 {
                max = corr;
                t0 = t;
            }
        }
    }
    
    // Test fractions
    
    // Fraction 0
    pred_lt_3(exc, t0, 0, l_subfr);
    let exc_slice = &exc[0..l_subfr as usize];
    max = dot_product(&dn, exc_slice, l_subfr);
    *pit_frac = 0;
    
    // If first subframe and lag > 84 do not search fractional pitch
    if i_subfr == 0 && sub(t0, 84) > 0 {
        return t0;
    }
    
    // Save current excitation
    for i in 0..(l_subfr as usize) {
        exc_tmp[i] = exc[i];
    }
    
    // Fraction -1/3
    pred_lt_3(exc, t0, -1, l_subfr);
    let exc_slice = &exc[0..l_subfr as usize];
    let corr = dot_product(&dn, exc_slice, l_subfr);
    let l_temp = l_sub(corr, max);
    if l_temp > 0 {
        max = corr;
        *pit_frac = -1;
        // Save this excitation
        for i in 0..(l_subfr as usize) {
            exc_tmp[i] = exc[i];
        }
    }
    
    // Fraction +1/3
    pred_lt_3(exc, t0, 1, l_subfr);
    let exc_slice = &exc[0..l_subfr as usize];
    let corr = dot_product(&dn, exc_slice, l_subfr);
    let l_temp = l_sub(corr, max);
    if l_temp > 0 {
        max = corr;
        *pit_frac = 1;
    } else {
        // Restore best excitation
        for i in 0..(l_subfr as usize) {
            exc[i] = exc_tmp[i];
        }
    }
    
    t0
}

// Helper function to extract lower 16 bits from 32-bit value
fn extract_l(l_var1: Word32) -> Word16 {
    (l_var1 & 0xffff) as Word16
}
