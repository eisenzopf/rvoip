use crate::common::basic_operators::{
    abs_s, add, extract_l, inv_sqrt, l_deposit_h, l_mac, l_msu, l_sub, mult, shl,
    shr, sub, Word16, Word32, MIN_32,
};
use crate::common::oper_32b::{l_extract, mpy_32};

const L_FRAME: i32 = 80;
const PIT_MAX: i32 = 143;

/// G.729A compliant open-loop pitch analysis
/// 
/// This function implements the exact algorithm from the G.729A reference code,
/// including three-section search, energy normalization, and pitch multiple detection.
pub fn pitch_ol_fast_g729a(signal: &[Word16], pit_max: i32, l_frame: i32) -> i32 {
    // Scaled signal buffer
    let mut scaled_signal = [0i16; (L_FRAME + PIT_MAX) as usize];

    // Step 1: Verification for risk of overflow
    unsafe {
        crate::common::basic_operators::OVERFLOW = false;
    }
    let mut sum: Word32 = 0;

    // Sum every other sample (as in C code)
    let total_len = add(l_frame as Word16, pit_max as Word16) as i32;
    let mut i = 0;
    while i < total_len {
        sum = l_mac(sum, signal[i as usize], signal[i as usize]);
        i = add(i as Word16, 2) as i32;
    }

    // Step 2: Scaling of input signal
    let overflow = unsafe { crate::common::basic_operators::OVERFLOW };
    if overflow {
        // Scale down by 8 if overflow
        for i in 0..total_len {
            scaled_signal[i as usize] = shr(signal[i as usize], 3);
        }
    } else {
        let l_temp = l_sub(sum, 1048576); // 2^20
        if l_temp < 0 {
            // Scale up by 8 if sum < 2^20
            for i in 0..total_len {
                scaled_signal[i as usize] = shl(signal[i as usize], 3);
            }
        } else {
            // No scaling
            for i in 0..total_len {
                scaled_signal[i as usize] = signal[i as usize];
            }
        }
    }

    // Use offset to handle negative indices like in C
    let scal_sig_offset = pit_max as usize;

    // Step 3: First section (lag 20 to 39)
    let mut max: Word32 = MIN_32;
    let mut t1 = 20; // Initialize to avoid compiler warning

    for i in 20..40 {
        let mut sum: Word32 = 0;
        let mut j = 0;
        while j < l_frame {
            let idx1 = add(scal_sig_offset as Word16, j as Word16) as usize;
            let idx2 = add(scal_sig_offset as Word16, sub(j as Word16, i as Word16)) as usize;
            sum = l_mac(sum, scaled_signal[idx1], scaled_signal[idx2]);
            j = add(j as Word16, 2) as i32;
        }
        let l_temp = l_sub(sum, max);
        if l_temp > 0 {
            max = sum;
            t1 = i;
        }
    }

    // Compute energy of maximum
    let mut sum: Word32 = 1; // To avoid division by zero
    let mut i = 0;
    while i < l_frame {
        let idx = add(scal_sig_offset as Word16, sub(i as Word16, t1 as Word16)) as usize;
        let p = scaled_signal[idx];
        sum = l_mac(sum, p, p);
        i = add(i as Word16, 2) as i32;
    }

    // max1 = max/sqrt(energy)
    let inv_energy = inv_sqrt(sum); // 1/sqrt(energy), result in Q30
    let (max_h, max_l) = l_extract(max);
    let (ener_h, ener_l) = l_extract(inv_energy);
    let normalized = mpy_32(max_h, max_l, ener_h, ener_l);
    let mut max1 = extract_l(normalized);

    // Step 4: Second section (lag 40 to 79)
    max = MIN_32;
    let mut t2 = 40;

    for i in 40..80 {
        let mut sum: Word32 = 0;
        let mut j = 0;
        while j < l_frame {
            let idx1 = add(scal_sig_offset as Word16, j as Word16) as usize;
            let idx2 = add(scal_sig_offset as Word16, sub(j as Word16, i as Word16)) as usize;
            sum = l_mac(sum, scaled_signal[idx1], scaled_signal[idx2]);
            j = add(j as Word16, 2) as i32;
        }
        let l_temp = l_sub(sum, max);
        if l_temp > 0 {
            max = sum;
            t2 = i;
        }
    }

    // Compute energy of maximum
    sum = 1;
    i = 0;
    while i < l_frame {
        let idx = add(scal_sig_offset as Word16, sub(i as Word16, t2 as Word16)) as usize;
        let p = scaled_signal[idx];
        sum = l_mac(sum, p, p);
        i = add(i as Word16, 2) as i32;
    }

    // max2 = max/sqrt(energy)
    let inv_energy = inv_sqrt(sum);
    let (max_h, max_l) = l_extract(max);
    let (ener_h, ener_l) = l_extract(inv_energy);
    let normalized = mpy_32(max_h, max_l, ener_h, ener_l);
    let mut max2 = extract_l(normalized);

    // Step 5: Third section (lag 80 to 143)
    max = MIN_32;
    let mut t3 = 80;

    // Search every other lag
    let mut i = 80;
    while i < 143 {
        let mut sum: Word32 = 0;
        let mut j = 0;
        while j < l_frame {
            let idx1 = add(scal_sig_offset as Word16, j as Word16) as usize;
            let idx2 = add(scal_sig_offset as Word16, sub(j as Word16, i as Word16)) as usize;
            sum = l_mac(sum, scaled_signal[idx1], scaled_signal[idx2]);
            j = add(j as Word16, 2) as i32;
        }
        let l_temp = l_sub(sum, max);
        if l_temp > 0 {
            max = sum;
            t3 = i;
        }
        i = add(i as Word16, 2) as i32;
    }

    // Test around max3 (check t3+1 and t3-1)
    let i = t3;
    
    // Check i+1
    if i < 142 {
        let mut sum: Word32 = 0;
        let mut j = 0;
        let i_plus_1 = add(i as Word16, 1) as i32;
        while j < l_frame {
            let idx1 = add(scal_sig_offset as Word16, j as Word16) as usize;
            let idx2 = add(scal_sig_offset as Word16, sub(j as Word16, i_plus_1 as Word16)) as usize;
            sum = l_mac(sum, scaled_signal[idx1], scaled_signal[idx2]);
            j = add(j as Word16, 2) as i32;
        }
        let l_temp = l_sub(sum, max);
        if l_temp > 0 {
            max = sum;
            t3 = i_plus_1;
        }
    }

    // Check i-1
    if i > 80 {
        let mut sum: Word32 = 0;
        let mut j = 0;
        let i_minus_1 = sub(i as Word16, 1) as i32;
        while j < l_frame {
            let idx1 = add(scal_sig_offset as Word16, j as Word16) as usize;
            let idx2 = add(scal_sig_offset as Word16, sub(j as Word16, i_minus_1 as Word16)) as usize;
            sum = l_mac(sum, scaled_signal[idx1], scaled_signal[idx2]);
            j = add(j as Word16, 2) as i32;
        }
        let l_temp = l_sub(sum, max);
        if l_temp > 0 {
            max = sum;
            t3 = i_minus_1;
        }
    }

    // Compute energy of maximum
    sum = 1;
    let mut k = 0;
    while k < l_frame {
        let idx = add(scal_sig_offset as Word16, sub(k as Word16, t3 as Word16)) as usize;
        let p = scaled_signal[idx];
        sum = l_mac(sum, p, p);
        k = add(k as Word16, 2) as i32;
    }

    // max3 = max/sqrt(energy)
    let inv_energy = inv_sqrt(sum);
    let (max_h, max_l) = l_extract(max);
    let (ener_h, ener_l) = l_extract(inv_energy);
    let normalized = mpy_32(max_h, max_l, ener_h, ener_l);
    let max3 = extract_l(normalized);

    // Step 6: Test for multiples
    // if( abs(T2*2 - T3) < 5) max2 += max3 * 0.25
    let i = sub(shl(t2 as Word16, 1), t3 as Word16);
    let j = sub(abs_s(i), 5);
    if j < 0 {
        max2 = add(max2, shr(max3, 2));
    }

    // if( abs(T2*3 - T3) < 7) max2 += max3 * 0.25
    let i = add(i, t2 as Word16);
    let j = sub(abs_s(i), 7);
    if j < 0 {
        max2 = add(max2, shr(max3, 2));
    }

    // if( abs(T1*2 - T2) < 5) max1 += max2 * 0.20
    let i = sub(shl(t1 as Word16, 1), t2 as Word16);
    let j = sub(abs_s(i), 5);
    if j < 0 {
        max1 = add(max1, mult(max2, 6554)); // 0.20 in Q15
    }

    // if( abs(T1*3 - T2) < 7) max1 += max2 * 0.20
    let i = add(i, t1 as Word16);
    let j = sub(abs_s(i), 7);
    if j < 0 {
        max1 = add(max1, mult(max2, 6554));
    }

    // Step 7: Compare the 3 sections maxima
    if sub(max1, max2) < 0 {
        max1 = max2;
        t1 = t2;
    }
    if sub(max1, max3) < 0 {
        t1 = t3;
    }

    t1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pitch_ol_fast_basic() {
        // Create a simple periodic signal
        let mut signal = [0i16; (L_FRAME + PIT_MAX) as usize];
        
        // Generate a signal with period 50
        for i in 0..signal.len() {
            signal[i] = if i % 50 < 25 { 1000 } else { -1000 };
        }
        
        let pitch = pitch_ol_fast_g729a(&signal, PIT_MAX, L_FRAME);
        
        // The pitch detector should find something close to 50
        // (might not be exactly 50 due to the algorithm's characteristics)
        assert!(pitch >= 45 && pitch <= 55, "Expected pitch around 50, got {}", pitch);
    }
} 