use crate::common::basic_operators::*;
use crate::common::filter::syn_filt;
use crate::common::tab_ld8a::{L_SUBFR, M};

// Gamma factors for perceptual weighting filter
const GAMMA1: Word16 = 24576; // 0.75 in Q15
const GAMMA2: Word16 = 18022; // 0.55 in Q15

/// Weight the LPC coefficients with a gamma factor
fn weight_az(a: &[Word16], gamma: Word16, ap: &mut [Word16]) {
    ap[0] = a[0];
    let mut fac = gamma;

    for i in 1..=M {
        let temp = l_mult(a[i], fac);
        ap[i] = round(temp);
        fac = mult(fac, gamma);
    }
}

/// Compute the impulse response h(n) = W(z)/A(z)
/// 
/// This function computes the impulse response of the cascade filter:
/// - W(z) = A(z/gamma1) / A(z/gamma2)
/// - H(z) = W(z) / A(z)
/// 
/// # Arguments
/// * `a` - LP filter coefficients A(z) in Q12
/// * `h` - Output impulse response h(n) in Q12
pub fn compute_impulse_response(a: &[Word16], h: &mut [Word16]) {
    let mut ap1 = [0i16; M + 1];
    let mut ap2 = [0i16; M + 1];
    
    // Compute A(z/gamma1) and A(z/gamma2)
    weight_az(a, GAMMA1, &mut ap1);
    weight_az(a, GAMMA2, &mut ap2);
    
    // Set up impulse input
    let mut x = [0i16; L_SUBFR];
    x[0] = 4096; // 1.0 in Q12
    
    // Filter through 1/A(z/gamma2)
    let mut mem = [0i16; M];
    syn_filt(&ap2, &x, h, L_SUBFR as i32, &mut mem, false);
    
    // Filter through A(z/gamma1)
    let mut temp = [0i16; L_SUBFR];
    for i in 0..L_SUBFR {
        let mut s = l_mult(h[i], ap1[0]);
        for j in 1..=M.min(i) {
            s = l_mac(s, ap1[j], h[i - j]);
        }
        temp[i] = round(l_shl(s, 3));
    }
    
    // Filter through 1/A(z)
    mem = [0i16; M];
    syn_filt(a, &temp, h, L_SUBFR as i32, &mut mem, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impulse_response_basic() {
        // Simple test with unit filter (a[0]=1, rest=0)
        let a = [4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // A(z) = 1
        let mut h = [0i16; L_SUBFR];
        
        compute_impulse_response(&a, &mut h);
        
        // With A(z)=1, W(z) should be close to 1, so h[0] should be near 4096
        assert!(h[0] > 0, "First impulse response sample should be positive");
        
        // Response should decay
        let energy_start = (h[0] as i32 * h[0] as i32) >> 12;
        let energy_end = (h[L_SUBFR-1] as i32 * h[L_SUBFR-1] as i32) >> 12;
        assert!(energy_start > energy_end, "Impulse response should decay");
    }
    
    #[test] 
    fn test_impulse_response_with_real_coeffs() {
        // Test with realistic LP coefficients
        let a = [4096, -2043, 10, 32, -1, -10, 5, 1, -1, 0, 0];
        let mut h = [0i16; L_SUBFR];
        
        compute_impulse_response(&a, &mut h);
        
        // First sample should be significant
        assert!(h[0] > 1000, "First sample should have significant energy");
        
        // Should have some energy throughout (not all zeros)
        let mut total_energy = 0i32;
        for i in 0..L_SUBFR {
            total_energy += (h[i] as i32 * h[i] as i32) >> 12;
        }
        assert!(total_energy > 1000, "Total energy should be significant");
    }
}