//! ITU-T G.729A Filtering Functions
//!
//! This module implements filtering functions based on the ITU reference implementation
//! FILTER.C from the official ITU-T G.729 Release 3.

use crate::codecs::g729a::types::*;
use crate::codecs::g729a::basic_ops::*;

/// Synthesis filter
/// 
/// Based on ITU-T G.729A Syn_filt function from FILTER.C
/// 
/// # Arguments
/// * `a` - Q12: a[m+1] prediction coefficients (m=10)
/// * `x` - Input signal
/// * `y` - Output signal  
/// * `lg` - Size of filtering
/// * `mem` - Memory associated with this filtering (i/o)
/// * `update` - 0=no update, 1=update of memory
pub fn syn_filt(
    a: &[Word16], 
    x: &[Word16], 
    y: &mut [Word16], 
    lg: Word16, 
    mem: &mut [Word16], 
    update: Word16
) {
    assert_eq!(a.len(), MP1);
    assert_eq!(mem.len(), M);
    assert_eq!(x.len(), lg as usize);
    assert_eq!(y.len(), lg as usize);
    
    // Create temporary buffer (M + lg)
    let mut tmp = vec![0i16; M + (lg as usize)];
    
    // Copy mem[] to beginning of tmp[]
    for i in 0..M {
        tmp[i] = mem[i];
    }
    
    // Do the filtering
    // yy pointer starts at tmp[M]
    for i in 0..(lg as usize) {
        let mut s = l_mult(x[i], a[0]);
        
        // yy is currently at tmp[M + i]
        // yy[-j] means tmp[M + i - j]
        for j in 1..=M {
            let idx = (M as isize) + (i as isize) - (j as isize);
            if idx >= 0 && (idx as usize) < tmp.len() {
                s = l_msu(s, a[j], tmp[idx as usize]);
            }
        }
        
        s = l_shl(s, 3);
        tmp[M + i] = round(s);
    }
    
    // Copy output: y[i] = tmp[i + M]
    for i in 0..(lg as usize) {
        y[i] = tmp[i + M];
    }
    
    // Update memory if requested
    if update != 0 {
        let lg_usize = lg as usize;
        for i in 0..M {
            mem[i] = y[lg_usize - M + i];
        }
    }
}

/// Weighted synthesis filter
/// 
/// This is a variant used for perceptual weighting
/// 
/// # Arguments
/// * `a` - Q12: prediction coefficients
/// * `x` - Input signal
/// * `y` - Output signal
/// * `lg` - Size of filtering  
/// * `mem` - Filter memory
/// * `update` - Update memory flag
pub fn weight_syn_filt(
    a: &[Word16],
    x: &[Word16], 
    y: &mut [Word16],
    lg: Word16,
    mem: &mut [Word16],
    update: Word16
) {
    // For now, this is the same as syn_filt
    // In practice, this would apply perceptual weighting
    syn_filt(a, x, y, lg, mem, update);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syn_filt_basic() {
        let a = [4096i16, 100, -200, 50, -25, 12, -6, 3, -1, 0, 0]; // Simple coefficients
        let x = [1000i16, 500, -300, 200, -100, 50, 25, 12, 6, 3, 1, 0]; // Input signal (12 samples > M=10)
        let mut y = [0i16; 12]; // Output buffer
        let mut mem = [0i16; M]; // Filter memory
        
        syn_filt(&a, &x, &mut y, 12, &mut mem, 1);
        
        // Output should be non-zero
        assert!(y.iter().any(|&sample| sample != 0));
        
        // Memory should be updated
        assert!(mem.iter().any(|&m| m != 0));
    }

    #[test]
    fn test_syn_filt_memory_update() {
        let a = [4096i16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // Unity coefficients
        let x = [1000i16, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 11000, 12000]; // 12 samples
        let mut y = [0i16; 12];
        let mut mem = [0i16; M];
        
        // Test with update = 0 (no memory update)
        syn_filt(&a, &x, &mut y, 12, &mut mem, 0);
        let mem_before = mem;
        
        syn_filt(&a, &x, &mut y, 12, &mut mem, 0);
        assert_eq!(mem, mem_before); // Memory should not change
        
        // Test with update = 1 (memory update)
        syn_filt(&a, &x, &mut y, 12, &mut mem, 1);
        assert_ne!(mem, mem_before); // Memory should change
    }
} 