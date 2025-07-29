use crate::common::basic_operators::*;
use crate::common::oper_32b::*;
use crate::common::tab_ld8a::{M, MP1};

/// Convert LSP to LP filter coefficients
/// 
/// This function converts line spectral pairs (LSP) to linear prediction
/// coefficients a[]. The LSP values are in the cosine domain.
/// 
/// # Arguments
/// * `lsp` - Line spectral pairs in Q15 (cosine domain)
/// * `a` - Output LP coefficients in Q12
pub fn lsp_az(lsp: &[Word16], a: &mut [Word16]) {
    let mut f1 = [0i32; 6];
    let mut f2 = [0i32; 6];
    
    get_lsp_pol(lsp, &mut f1, 0);  // First polynomial F1(z) - even indices
    get_lsp_pol(lsp, &mut f2, 1);  // Second polynomial F2(z) - odd indices
    
    // Expand F1(z) and F2(z)
    for i in (1..=5).rev() {
        f1[i] = l_add(f1[i], f1[i-1]);  // f1[i] += f1[i-1]
        f2[i] = l_sub(f2[i], f2[i-1]);  // f2[i] -= f2[i-1]
    }
    
    // A(z) = (F1(z) + F2(z))/2
    a[0] = 4096;  // 1.0 in Q12
    
    let mut j = 10;
    for i in 1..=5 {
        let t0 = l_add(f1[i], f2[i]);           // f1[i] + f2[i]
        a[i] = round(l_shl(t0, 3));             // from Q24 to Q12 (shift left 3 = right 13 + round)
        
        let t0 = l_sub(f1[i], f2[i]);           // f1[i] - f2[i]
        a[j] = round(l_shl(t0, 3));             // from Q24 to Q12 (shift left 3 = right 13 + round)
        j -= 1;
    }
}

/// Find the polynomial F1(z) or F2(z) from the LSPs
/// 
/// # Arguments
/// * `lsp` - Line spectral frequencies (cosine domain) in Q15
/// * `f` - Output polynomial coefficients in Q24
/// * `offset` - 0 for F1 (even), 1 for F2 (odd)
fn get_lsp_pol(lsp: &[Word16], f: &mut [Word32], offset: usize) {
    // All computation in Q24
    f[0] = l_mult(4096, 2048);              // f[0] = 1.0 in Q24
    f[1] = l_msu(0, lsp[offset], 512);      // f[1] = -2.0 * lsp[offset] in Q24
    
    let mut lsp_idx = offset + 2;
    for i in 2..=5 {
        f[i] = f[i-2];
        
        let current_lsp = lsp[lsp_idx];  // Store current LSP value
        for j in (1..i).rev() {
            let (hi, lo) = l_extract(f[j-1]);
            let mut t0 = mpy_32_16(hi, lo, current_lsp);   // t0 = f[j-1] * lsp
            t0 = l_shl(t0, 1);
            if j >= 2 {
                f[j] = l_add(f[j], f[j-2]);                 // f[j] += f[j-2]
            }
            f[j] = l_sub(f[j], t0);                         // f[j] -= t0
        }
        
        f[0] = l_msu(f[0], current_lsp, 512);              // f[0] -= lsp<<9
        lsp_idx += 2;                                       // Advance lsp pointer
    }
}

/// Interpolate LSP parameters and convert to LP coefficients
/// 
/// This function interpolates between old and new LSP parameters
/// and converts the result to LP coefficients for each subframe.
/// 
/// # Arguments
/// * `lsp_old` - Previous frame LSP values
/// * `lsp_new` - Current frame LSP values  
/// * `az` - Output LP coefficients (2 sets of MP1 coefficients)
pub fn int_qlpc(lsp_old: &[Word16], lsp_new: &[Word16], az: &mut [Word16]) {
    let mut lsp = [0i16; M];
    
    // First subframe: 0.75*old + 0.25*new
    for i in 0..M {
        lsp[i] = add(mult(lsp_old[i], 24576), mult(lsp_new[i], 8192));  // Q15
    }
    lsp_az(&lsp, &mut az[0..MP1]);
    
    // Second subframe: use new LSP
    lsp_az(lsp_new, &mut az[MP1..2*MP1]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_az_basic() {
        // Test with simple LSP values
        let lsp = [30000, 25000, 20000, 15000, 10000, 5000, 0, -5000, -10000, -15000];
        let mut a = [0i16; MP1];
        
        lsp_az(&lsp, &mut a);
        
        // First coefficient should always be 4096 (1.0 in Q12)
        assert_eq!(a[0], 4096);
        
        // Other coefficients should be non-zero for typical LSP values
        let mut non_zero_count = 0;
        for i in 1..MP1 {
            if a[i] != 0 {
                non_zero_count += 1;
            }
        }
        assert!(non_zero_count > 0, "LP coefficients should not all be zero");
    }
    
    #[test]
    fn test_int_qlpc() {
        let lsp_old = [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000];
        let lsp_new = [29000, 25000, 20000, 14000, 7000, -1000, -9000, -16000, -22000, -27000];
        let mut az = [0i16; 2 * MP1];
        
        int_qlpc(&lsp_old, &lsp_new, &mut az);
        
        // Check both subframes have valid a[0]
        assert_eq!(az[0], 4096);
        assert_eq!(az[MP1], 4096);
        
        // The two subframes should be different (interpolation effect)
        let mut different = false;
        for i in 1..MP1 {
            if az[i] != az[MP1 + i] {
                different = true;
                break;
            }
        }
        assert!(different, "Subframe coefficients should differ due to interpolation");
    }
}