//! Polynomial operations for LSP conversion

use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::constants::LP_ORDER;
use crate::codecs::g729a::math::fixed_point::FixedPointOps;

/// Evaluate polynomial at a given point using Horner's method
pub fn evaluate_polynomial(coeffs: &[Q15], x: Q15) -> Q31 {
    if coeffs.is_empty() {
        return Q31::ZERO;
    }
    
    let mut result = coeffs[coeffs.len() - 1].to_q31();
    
    for i in (0..coeffs.len() - 1).rev() {
        result = result.saturating_mul(x.to_q31());
        result = result.saturating_add(coeffs[i].to_q31());
    }
    
    result
}

/// Find polynomial roots using grid search and bisection
pub fn find_polynomial_roots(poly: &[Q15], grid: &[Q15], num_roots: usize) -> Vec<Q15> {
    let mut roots = Vec::with_capacity(num_roots);
    let mut last_idx = 0;
    
    for i in 1..grid.len() {
        let y0 = evaluate_polynomial(poly, grid[i - 1]);
        let y1 = evaluate_polynomial(poly, grid[i]);
        
        // Check for sign change
        if (y0.0 < 0 && y1.0 > 0) || (y0.0 > 0 && y1.0 < 0) {
            // Refine root using bisection
            let root = bisect_root(poly, grid[i - 1], grid[i]);
            roots.push(root);
            
            if roots.len() >= num_roots {
                break;
            }
        }
    }
    
    // Fill remaining roots if needed
    while roots.len() < num_roots {
        roots.push(Q15::ZERO);
    }
    
    roots
}

/// Bisection method to refine root location
fn bisect_root(poly: &[Q15], left: Q15, right: Q15) -> Q15 {
    let mut a = left;
    let mut b = right;
    
    // 4 iterations for reasonable precision
    for _ in 0..4 {
        let mid = Q15((a.0.saturating_add(b.0)) / 2);
        let y_mid = evaluate_polynomial(poly, mid);
        
        if y_mid.0 == 0 {
            return mid;
        }
        
        let y_a = evaluate_polynomial(poly, a);
        
        if (y_a.0 < 0 && y_mid.0 < 0) || (y_a.0 > 0 && y_mid.0 > 0) {
            a = mid;
        } else {
            b = mid;
        }
    }
    
    Q15((a.0.saturating_add(b.0)) / 2)
}

/// Form sum polynomial F1(z) = A(z) + z^(-M-1) * A(z^-1)
pub fn form_sum_polynomial(lp_coeffs: &[Q15]) -> Vec<Q15> {
    let m = lp_coeffs.len();
    let mut f1 = vec![Q15::ZERO; m + 1];
    
    // F1 has order M/2 + 1
    for i in 0..=m/2 {
        if i < m {
            let sum = if i == 0 {
                Q15::ONE.saturating_add(lp_coeffs[m - 1])
            } else if i == m/2 {
                lp_coeffs[i - 1].saturating_add(lp_coeffs[m - i - 1])
            } else {
                lp_coeffs[i - 1].saturating_add(lp_coeffs[m - i - 1])
            };
            f1[i] = sum;
        }
    }
    
    f1.truncate(m/2 + 1);
    f1
}

/// Form difference polynomial F2(z) = A(z) - z^(-M-1) * A(z^-1)
pub fn form_difference_polynomial(lp_coeffs: &[Q15]) -> Vec<Q15> {
    let m = lp_coeffs.len();
    let mut f2 = vec![Q15::ZERO; m + 1];
    
    // F2 has order M/2
    for i in 0..m/2 {
        let diff = if i == 0 {
            Q15::ONE.saturating_add(Q15(lp_coeffs[m - 1].0.saturating_neg()))
        } else {
            lp_coeffs[i - 1].saturating_add(Q15(lp_coeffs[m - i - 1].0.saturating_neg()))
        };
        f2[i] = diff;
    }
    
    f2.truncate(m/2);
    f2
}

/// Generate Chebyshev polynomial grid for root finding
pub fn generate_chebyshev_grid(num_points: usize) -> Vec<Q15> {
    let mut grid = Vec::with_capacity(num_points);
    
    // Generate cosine values from pi to 0
    for i in 0..num_points {
        let angle = std::f32::consts::PI * (1.0 - i as f32 / (num_points - 1) as f32);
        let cos_val = angle.cos();
        grid.push(Q15::from_f32(cos_val));
    }
    
    grid
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
        assert!(result.0.abs() < 1000); // Should be close to 0
        
        // Evaluate at x = 0
        let result = evaluate_polynomial(&coeffs, Q15::ZERO);
        assert!((result.to_f32() + 0.25).abs() < 0.01);
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
        let roots = find_polynomial_roots(&coeffs, &grid, 2);
        
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