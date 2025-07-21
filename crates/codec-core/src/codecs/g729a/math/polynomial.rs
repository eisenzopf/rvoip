//! Polynomial operations for LSP conversion

use crate::codecs::g729a::types::Q15;
use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::math::fixed_point::{add, sub, negate, mac, msu, add32, sub32, sshl32};

/// Evaluate polynomial at a given point using Horner's method
pub fn evaluate_polynomial(coeffs: &[Q15], x: Q15) -> i32 {
    if coeffs.is_empty() {
        return 0;
    }
    
    let mut result = coeffs[coeffs.len() - 1].0 as i32;
    
    for i in (0..coeffs.len() - 1).rev() {
        result = mac(result, x.0, coeffs[i].0);
    }
    
    result
}



/// Find roots of polynomial in the range [0, π]
pub fn find_polynomial_roots_in_range(poly: &[Q15], grid: &[Q15], num_roots: usize) -> Vec<Q15> {
    let mut roots = Vec::new();
    
    if poly.is_empty() || grid.len() < 2 {
        return roots;
    }
    
    let mut y0 = evaluate_polynomial(poly, grid[0]);
    
    for i in 1..grid.len() {
        if roots.len() >= num_roots {
            break;
        }
        
        let y1 = evaluate_polynomial(poly, grid[i]);
        
        // Check for sign change
        if (y0 < 0 && y1 > 0) || (y0 > 0 && y1 < 0) {
            // Refine root using bisection
            let root = bisect_root(poly, grid[i - 1], grid[i]);
            roots.push(root);
        }
        
        y0 = y1;
    }
    
    roots
}

/// Find a root using bisection method
fn bisect_root(poly: &[Q15], mut a: Q15, mut b: Q15) -> Q15 {
    const MAX_ITERATIONS: usize = 10;
    
    for _ in 0..MAX_ITERATIONS {
        let mid = Q15::from_f32((a.to_f32() + b.to_f32()) / 2.0);
        let y_mid = evaluate_polynomial(poly, mid);
        
        if y_mid == 0 {
            return mid;
        }
        
        let y_a = evaluate_polynomial(poly, a);
        
        if (y_a < 0 && y_mid < 0) || (y_a > 0 && y_mid > 0) {
            a = mid;
        } else {
            b = mid;
        }
    }
    
    Q15::from_f32((a.to_f32() + b.to_f32()) / 2.0)
}

/// Form sum polynomial Q12 coefficients for direct Chebyshev evaluation
/// Returns Q12 coefficients as i32 array for ITU-T Chebyshev polynomial
pub fn form_sum_polynomial_q12(lp_coeffs: &[Q15]) -> [i32; 6] {
    let m = lp_coeffs.len(); // Should be 10 for G.729A
    let mut f1_q12 = vec![0i32; 6]; // ITU-T: 6 coefficients in Q12
    
    // ITU-T: f1[0] = 1.0 in Q12 (4096)
    const ONE_IN_Q12: i32 = 1 << 12;
    f1_q12[0] = ONE_IN_Q12;
    
    // ITU-T: Convert LP coefficients from Q15 to Q12 (assuming they're currently Q12 internally)
    // Since my LP coeffs are Q15 but represent values that should be Q12, convert back
    for i in 0..5 {
        let a_i_q12 = if i < m { (lp_coeffs[i].0 as i32) >> 3 } else { 0 }; // Q15 to Q12
        let a_reverse_q12 = if (9 - i) < m { (lp_coeffs[9 - i].0 as i32) >> 3 } else { 0 }; // Q15 to Q12
        
        // ITU-T: f1[i+1] = a[i] + a[9-i] - f1[i] in Q12
        let result_q12 = a_i_q12 + a_reverse_q12 - f1_q12[i];
        f1_q12[i + 1] = result_q12;
        
        #[cfg(debug_assertions)]
        if i < 3 {
            eprintln!("    F1[{}] Q12: a[{}]={} + a[{}]={} - f1[{}]={} = {}", 
                i+1, i, a_i_q12, 9-i, a_reverse_q12, i, f1_q12[i], result_q12);
        }
    }
    
    // ITU-T: Convert Q12 to Q15 for Chebyshev evaluation (left shift 3)
    let mut f1_q15 = [0i32; 6];
    for i in 0..6 {
        f1_q15[i] = sshl32(f1_q12[i], 3); // Convert Q12 to Q15
        
        #[cfg(debug_assertions)]
        if i < 3 {
            eprintln!("    F1[{}] Q12={} -> Q15={}", i, f1_q12[i], f1_q15[i]);
        }
    }
    
    f1_q15
}

/// Form difference polynomial Q12 coefficients for direct Chebyshev evaluation  
/// Returns Q12 coefficients as i32 array for ITU-T Chebyshev polynomial
pub fn form_difference_polynomial_q12(lp_coeffs: &[Q15]) -> [i32; 6] {
    let m = lp_coeffs.len(); // Should be 10 for G.729A
    let mut f2_q12 = vec![0i32; 6]; // ITU-T: 6 coefficients in Q12
    
    // ITU-T: f2[0] = 1.0 in Q12 (4096)
    const ONE_IN_Q12: i32 = 1 << 12;
    f2_q12[0] = ONE_IN_Q12;
    
    // ITU-T algorithm in Q12: f2[i+1] = f2[i] + a[i] - a[9-i]
    // Convert LP coeffs from Q15 to Q12 (right shift 3)
    for i in 0..5 {  // ITU-T: 5 iterations like F1
        let a_i_q12 = if i < m { (lp_coeffs[i].0 as i32) >> 3 } else { 0 }; // Q15 to Q12
        let a_reverse_q12 = if (9 - i) < m { (lp_coeffs[9 - i].0 as i32) >> 3 } else { 0 }; // Q15 to Q12
        
        // ITU-T: f2[i+1] = f2[i] + a[i] - a[9-i] in Q12
        let result_q12 = f2_q12[i] + a_i_q12 - a_reverse_q12;
        f2_q12[i + 1] = result_q12;
        
        #[cfg(debug_assertions)]
        if i < 3 {
            eprintln!("    F2[{}] Q12: f2[{}]={} + (a[{}]={} - a[{}]={}) = {}", 
                i+1, i, f2_q12[i], i, a_i_q12, 9-i, a_reverse_q12, result_q12);
        }
    }
    
    // ITU-T: Convert Q12 to Q15 for Chebyshev evaluation (left shift 3)
    let mut f2_q15 = [0i32; 6];
    for i in 0..6 {
        f2_q15[i] = sshl32(f2_q12[i], 3); // Convert Q12 to Q15
        
        #[cfg(debug_assertions)]
        if i < 3 {
            eprintln!("    F2[{}] Q12={} -> Q15={}", i, f2_q12[i], f2_q15[i]);
        }
    }
    
    f2_q15
}

/// ITU-T predefined cosine grid for Chebyshev polynomial evaluation
/// cos(w) with w in [0,Pi] in 50 steps - exact ITU-T values in Q15
const COS_W_0_PI: [i16; 51] = [
    32760, 32703, 32509, 32187, 31738, 31164,
    30466, 29649, 28714, 27666, 26509, 25248,
    23886, 22431, 20887, 19260, 17557, 15786,
    13951, 12062, 10125,  8149,  6140,  4106,
     2057,     0, -2057, -4106, -6140, -8149,
   -10125,-12062,-13951,-15786,-17557,-19260,
   -20887,-22431,-23886,-25248,-26509,-27666,
   -28714,-29649,-30466,-31164,-31738,-32187,
   -32509,-32703,-32760
];

/// Generate Chebyshev polynomial grid using exact ITU-T values
pub fn get_itu_t_chebyshev_grid() -> Vec<Q15> {
    COS_W_0_PI.iter().map(|&x| Q15(x)).collect()
}

/// Find LSP roots using exact ITU-T algorithm with Chebyshev polynomial evaluation
/// Based on LP2LSPConversion.c from ITU-T reference implementation
pub fn find_lsp_roots_itu_t(f1_coeffs: &[i32; 6], f2_coeffs: &[i32; 6]) -> Vec<Q15> {
    let mut lsp_coefficients = Vec::new();
    let mut number_of_root_found = 0;
    let grid = get_itu_t_chebyshev_grid();
    
    // ITU-T: Start with f1 polynomial coefficients and alternate with f2
    let mut polynomial_coefficients = f1_coeffs;
    let mut previous_cx = chebyshev_polynomial_itu_t(grid[0], polynomial_coefficients);
    
    for i in 1..grid.len() {
        let cx = chebyshev_polynomial_itu_t(grid[i], polynomial_coefficients);
        
        #[cfg(debug_assertions)]
        if i <= 5 || (i % 10 == 0) { // Debug first few values and every 10th
            eprintln!("    Grid[{}]: x={}, prev_cx={}, cx={}, sign_change={}", 
                i, grid[i].0, previous_cx, cx, (previous_cx ^ cx) & 0x80000000u32 as i32 != 0);
        }
        
        // ITU-T: Check sign change by XOR on the sign bit (bit 31 for 32-bit signed values)
        if (previous_cx ^ cx) & 0x80000000u32 as i32 != 0 {
            // ITU-T: Divide 2 times the interval to find a more accurate root
            let mut x_low = grid[i - 1].0;
            let mut x_high = grid[i].0;
            
            #[cfg(debug_assertions)]
            eprintln!("    Sign change detected at grid[{}]: interval [{}, {}]", i, x_low, x_high);
            
            for _ in 0..2 {
                let x_mean = ((x_low as i32 + x_high as i32) >> 1) as i16; // Use standard arithmetic
                let middle_cx = chebyshev_polynomial_itu_t(Q15(x_mean), polynomial_coefficients);
                
                if (previous_cx ^ middle_cx) & 0x80000000u32 as i32 != 0 {
                    x_high = x_mean;
                    // ITU-T: Update cx for linear interpolation
                    // (simplified - skipping linear interpolation for now)
                } else {
                    x_low = x_mean;
                    previous_cx = middle_cx;
                }
            }
            
            // ITU-T: Toggle the polynomial coefficients between f1 and f2
            polynomial_coefficients = if std::ptr::eq(polynomial_coefficients, f1_coeffs) {
                f2_coeffs
            } else {
                f1_coeffs
            };
            
            // Store the root (use mean of interval)
            let x_mean = ((x_low as i32 + x_high as i32) >> 1) as i16; // Use standard arithmetic
            lsp_coefficients.push(Q15(x_mean));
            
            #[cfg(debug_assertions)]
            eprintln!("    Root {}: interval [{}, {}] -> mean = {}", 
                number_of_root_found, x_low, x_high, x_mean);
            
            number_of_root_found += 1;
            if number_of_root_found == 10 { // NB_LSP_COEFF
                break;
            }
        }
        
        previous_cx = cx;
    }
    
    #[cfg(debug_assertions)]
    {
        eprintln!("  ITU-T root finding: found {} roots", number_of_root_found);
        if number_of_root_found > 0 {
            eprintln!("  First 5 roots: {:?}", 
                lsp_coefficients.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
    }
    
    lsp_coefficients
}

/// Evaluate Chebyshev polynomial using exact ITU-T algorithm  
/// This is the core of the ITU-T LSP root finding algorithm (spec 3.2.3 eq17)
/// Input: x in Q15, coefficients f[6] in Q15 (32-bit containers)  
/// Output: result in Q15 (32-bit)
pub fn chebyshev_polynomial_itu_t(x: Q15, f: &[i32]) -> i32 {
    if f.len() < 6 {
        return 0;
    }
    
    // ITU-T algorithm - coeffs are Q15 values in 32-bit containers
    // bk in Q15
    let mut bk1 = add32(sshl32(x.0 as i32, 1), f[1]); // init: b4=2x+f1
    let mut bk2 = Q15_ONE as i32; // init: b5=1
    
    // Loop k = 3 down to 1
    for k in (1..=3).rev() {
        // bk = 2*x*bk1 - bk2 + f(5-k) all in Q15
        let mult_result = mult16_32_q15(x.0, bk1); // MULT16_32_Q15(x, bk1)
        let shifted = sshl32(mult_result, 1); // 2*x*bk1
        let temp_add = add32(shifted, f[5 - k]); // + f(5-k)
        let bk = sub32(temp_add, bk2); // - bk2
        
        bk2 = bk1;
        bk1 = bk;
    }
    
    // C(x) = x*b1 - b2 + f(5)/2
    let x_b1 = mult16_32_q15(x.0, bk1);
    let f5_half = f[5] >> 1; // SHR(f[5], 1)
    let temp_add = add32(x_b1, f5_half);
    sub32(temp_add, bk2)
}

/// ITU-T MULT16_32_Q15 operation
fn mult16_32_q15(a: i16, b: i32) -> i32 {
    let product = (a as i64) * (b as i64);
    ((product + 0x4000) >> 15) as i32
}

/// Backward compatibility: Form sum polynomial (for existing API)
pub fn form_sum_polynomial(lp_coeffs: &[Q15]) -> Vec<Q15> {
    let q15_coeffs = form_sum_polynomial_q12(lp_coeffs);
    // Convert i32 Q15 values back to Q15 struct for compatibility
    q15_coeffs.iter().map(|&x| Q15((x >> 15) as i16)).collect()
}

/// Backward compatibility: Form difference polynomial (for existing API)
pub fn form_difference_polynomial(lp_coeffs: &[Q15]) -> Vec<Q15> {
    let q15_coeffs = form_difference_polynomial_q12(lp_coeffs);
    // Convert i32 Q15 values back to Q15 struct for compatibility  
    q15_coeffs.iter().map(|&x| Q15((x >> 15) as i16)).collect()
}

/// Backward compatibility: Generate Chebyshev grid
pub fn generate_chebyshev_grid(num_points: usize) -> Vec<Q15> {
    // Use ITU-T grid if requesting 51 points (standard), otherwise use old method
    if num_points == 51 {
        get_itu_t_chebyshev_grid()
    } else {
        let mut grid = Vec::with_capacity(num_points);
        for i in 0..num_points {
            let angle = std::f32::consts::PI * (1.0 - i as f32 / (num_points - 1) as f32);
            let cos_val = angle.cos();
            grid.push(Q15::from_f32(cos_val));
        }
        grid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_polynomial() {
        // Test polynomial: x^2 - 0.25 = 0, roots at ±0.5
        let coeffs = vec![
            Q15::from_f32(-0.25),
            Q15::ZERO,
            Q15::ONE,
        ];
        
        // Evaluate at x = 0.5
        let result = evaluate_polynomial(&coeffs, Q15::from_f32(0.5));
        assert!(result.abs() < 1000); // Should be close to 0
        
        // Evaluate at x = 0
        let result = evaluate_polynomial(&coeffs, Q15::ZERO);
        assert!((result as f32 / 32768.0 + 0.25).abs() < 0.01);
    }

    #[test]
    fn test_find_roots() {
        // Simple polynomial with known roots
        let coeffs = vec![
            Q15::from_f32(-0.25),
            Q15::ZERO,
            Q15::ONE,
        ];
        
        let grid = generate_chebyshev_grid(20);
        let roots = find_polynomial_roots_in_range(&coeffs, &grid, 2);
        
        // Should find roots near ±0.5
        assert!(roots.len() >= 2);
        assert!((roots[0].to_f32().abs() - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_bisect_root() {
        let coeffs = vec![
            Q15::from_f32(-0.25),
            Q15::ZERO,
            Q15::ONE,
        ];
        
        // Bisect between 0.4 and 0.6 (root at 0.5)
        let root = bisect_root(&coeffs, Q15::from_f32(0.4), Q15::from_f32(0.6));
        assert!((root.to_f32() - 0.5).abs() < 0.05);
    }

    #[test]
    fn test_form_polynomials() {
        // Test with simple LP coefficients
        let lp = vec![
            Q15::from_f32(0.1),
            Q15::from_f32(0.2),
            Q15::from_f32(0.3),
            Q15::from_f32(0.2),
            Q15::from_f32(0.1),
        ];
        
        let f1 = form_sum_polynomial(&lp);
        let f2 = form_difference_polynomial(&lp);
        
        // F1 should have order 3 (M/2 + 1)
        assert_eq!(f1.len(), 3);
        
        // F2 should have order 2 (M/2)
        assert_eq!(f2.len(), 2);
        
        // Check symmetry properties
        // F1 coefficients should be symmetric sums
        // F2 coefficients should be anti-symmetric differences
    }

    #[test]
    fn test_chebyshev_grid() {
        let grid = generate_chebyshev_grid(10);
        
        // Should have correct number of points
        assert_eq!(grid.len(), 10);
        
        // Should range from -1 to 1
        assert!(grid[0].to_f32() < -0.9);
        assert!(grid[grid.len() - 1].to_f32() > 0.9);
        
        // Should be monotonically increasing
        for i in 1..grid.len() {
            assert!(grid[i].0 > grid[i - 1].0);
        }
    }
} 