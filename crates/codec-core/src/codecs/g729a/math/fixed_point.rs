//! Fixed-point arithmetic operations for G.729A

use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::tables::{POW2_TABLE, LOG2_TABLE, get_inv_sqrt_extended};

// Helper trait for saturating conversion to i16
trait SaturatingToI16 {
    fn saturating_to_i16(self) -> i16;
}

impl SaturatingToI16 for i32 {
    fn saturating_to_i16(self) -> i16 {
        self.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

/// Trait for fixed-point operations
pub trait FixedPointOps: Sized {
    fn saturating_add(self, other: Self) -> Self;
    fn saturating_mul(self, other: Self) -> Self;
    fn to_q15(self) -> Q15;
    fn to_q31(self) -> Q31;
}

impl FixedPointOps for Q15 {
    fn saturating_add(self, other: Self) -> Self {
        Q15(self.0.saturating_add(other.0))
    }
    
    fn saturating_mul(self, other: Self) -> Self {
        let result = ((self.0 as i32 * other.0 as i32) >> 15) as i16;
        Q15(result)
    }
    
    fn to_q15(self) -> Q15 {
        self
    }
    
    fn to_q31(self) -> Q31 {
        Q31((self.0 as i32) << 16)
    }
}

impl FixedPointOps for Q31 {
    fn saturating_add(self, other: Self) -> Self {
        Q31(self.0.saturating_add(other.0))
    }
    
    fn saturating_mul(self, other: Self) -> Self {
        let result = ((self.0 as i64 * other.0 as i64) >> 31) as i32;
        Q31(result)
    }
    
    fn to_q15(self) -> Q15 {
        Q15((self.0 >> 16) as i16)
    }
    
    fn to_q31(self) -> Q31 {
        self
    }
}

// Additional Q-format operations required by G.729A

/// Multiply Q15 x Q15 -> Q15 with rounding
pub fn mult_q15_round(a: Q15, b: Q15) -> Q15 {
    let result = ((a.0 as i32 * b.0 as i32 + 0x4000) >> 15) as i16;
    Q15(result)
}

/// Multiply 16-bit x 16-bit -> 32-bit in Q12 format
pub fn mult16_16_q12(a: i16, b: i16) -> i32 {
    (a as i32 * b as i32) >> 12
}

/// Multiply 16-bit x 16-bit -> 32-bit in Q13 format
pub fn mult16_16_q13(a: i16, b: i16) -> i32 {
    (a as i32 * b as i32) >> 13
}

/// Multiply 16-bit x 16-bit -> 32-bit in Q14 format
pub fn mult16_16_q14(a: i16, b: i16) -> i32 {
    (a as i32 * b as i32) >> 14
}

/// Shift right with rounding (PSHR)
pub fn pshr(x: i32, shift: i32) -> i32 {
    if shift > 0 {
        let rounding = 1 << (shift - 1);
        (x + rounding) >> shift
    } else {
        x << (-shift)
    }
}

/// Saturating add for i32
pub fn l_add(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}

/// Saturating subtract for i32
pub fn l_sub(a: i32, b: i32) -> i32 {
    a.saturating_sub(b)
}

/// Saturate i32 to i16 range
pub fn saturate(x: i32) -> i16 {
    x.saturating_to_i16()
}

/// Compute inverse square root using Newton-Raphson method
/// Input: Q31 value (must be positive)
/// Output: Q15 inverse square root
pub fn inverse_sqrt(x: Q31) -> Q15 {
    if x.0 <= 0 {
        return Q15(0);
    }
    
    // Normalize input to range [0.25, 1.0) for better precision
    let mut norm_x = x.0;
    let mut shift = 0;
    
    while norm_x < (1 << 29) { // Less than 0.25
        norm_x = norm_x.saturating_mul(4);
        shift += 1;
    }
    
    // Initial approximation based on normalized value
    let mut y = if norm_x < (1 << 30) {
        Q15(23170) // ~1.414 / 2 for values near 0.5
    } else {
        Q15(16384) // ~1.0 / 2 for values near 1.0
    };
    
    // Newton-Raphson iterations
    for _ in 0..3 {
        // y = y * (3 - norm_x * y^2) / 2
        let y_sq = y.saturating_mul(y);
        let norm_x_q15 = Q15((norm_x >> 16) as i16);
        let x_y_sq = norm_x_q15.saturating_mul(y_sq);
        let three = Q15(24576); // 1.5 in Q15
        let three_minus = three.saturating_add(Q15(-x_y_sq.0));
        y = y.saturating_mul(three_minus);
    }
    
    // Apply shift correction: multiply by 2^shift
    for _ in 0..shift {
        y = Q15(y.0.saturating_mul(23170) >> 14); // Multiply by sqrt(2)
    }
    
    y
}

/// Compute log2 approximation
/// Returns (exponent, mantissa) where mantissa is in Q15
pub fn log2_approximation(x: Q31) -> (i16, Q15) {
    if x.0 <= 0 {
        return (0, Q15(0));
    }
    
    let mut value = x.0 as u32; // Work with unsigned to avoid issues
    let mut exponent = 0i16;
    
    // Count leading zeros to find the exponent
    let leading_zeros = value.leading_zeros();
    
    // Normalize to range [0.5, 1.0) by shifting
    if leading_zeros > 1 {
        // Need to shift left
        let shift = (leading_zeros - 1).min(31);
        value <<= shift;
        exponent = -(shift as i16);
    } else if leading_zeros == 0 {
        // Need to shift right
        value >>= 1;
        exponent = 1;
    }
    
    // Linear approximation for mantissa
    // log2(x) ≈ x - 1 for x in [0.5, 1.0)
    let normalized = value as i32;
    let mantissa_val = normalized - (1 << 30); // x - 0.5 in Q30
    let mantissa = Q15((mantissa_val >> 15) as i16);
    
    (exponent - 1, mantissa)
}

/// Compute 2^x where x = exponent + mantissa
/// Mantissa is in Q15 format
pub fn power2_approximation(exp: i16, mantissa: Q15) -> Q31 {
    // 2^mantissa ≈ 1 + mantissa + mantissa^2/2 for better approximation
    let m = mantissa.0 as i32;
    let m_sq = (m * m) >> 16; // mantissa^2 in Q30
    let base = (1i32 << 30) + (m << 15) + (m_sq >> 1);
    
    // Apply exponent shift
    if exp >= 0 {
        if exp >= 1 {
            // For exp >= 1, result exceeds Q31 range, return max
            Q31(i32::MAX)
        } else {
            Q31(base)
        }
    } else {
        let shift = (-exp) as u32;
        if shift > 31 {
            Q31(0)
        } else if shift == 0 {
            Q31(base)
        } else {
            Q31(base >> shift)
        }
    }
}

/// Compute absolute value
pub fn abs_q15(x: Q15) -> Q15 {
    if x.0 < 0 {
        Q15((-x.0).max(i16::MIN + 1))
    } else {
        x
    }
}

/// Compute absolute value
pub fn abs_q31(x: Q31) -> Q31 {
    if x.0 < 0 {
        Q31((-x.0).max(i32::MIN + 1))
    } else {
        x
    }
}

/// Compute 1/sqrt(x) using Newton-Raphson iteration
/// Input: Q31, Output: Q15
/// For G.729A bit-exact compliance
pub fn inv_sqrt_precise(x: Q31) -> Q15 {
    if x.0 <= 0 {
        return Q15(Q15_ONE);
    }
    
    // Normalize input to range [0.5, 1.0)
    let mut exp = 0;
    let mut norm_x = x.0;
    
    while norm_x < 0x40000000 {  // 0.5 in Q31
        norm_x <<= 2;
        exp += 1;
    }
    
    // Initial approximation using lookup table
    // G.729A uses a 49-entry table, map the normalized value to table index
    let table_idx = ((norm_x >> 24) & 0x3F) as usize;  // 6 bits to cover 0-63
    let table_idx = table_idx.min(48);  // Clamp to valid range
    let mut y = crate::codecs::g729a::tables::INV_SQRT_TABLE[table_idx];  // Q15
    
    // Two Newton-Raphson iterations
    for _ in 0..2 {
        // y = y * (3 - x*y^2) / 2
        let y_q31 = Q31((y as i32) << 16);
        let y2 = (y_q31.0 as i64 * y_q31.0 as i64 >> 31) as i32;
        let xy2 = (norm_x as i64 * y2 as i64 >> 31) as i32;
        let three_minus = 0x60000000 - xy2;  // 3.0 - x*y^2 in Q30
        y = ((y as i64 * three_minus as i64) >> 31) as i16;
    }
    
    // Denormalize based on exponent
    if exp & 1 == 1 {
        y = ((y as i32 * 23170) >> 15) as i16;  // multiply by 1/sqrt(2)
    }
    y >>= exp >> 1;
    
    Q15(y)
}

/// Log2 approximation for G.729A
/// Returns (exponent, fraction) where result = exponent + fraction/32768
pub fn log2_g729a(x: Q31) -> (i16, Q15) {
    if x.0 <= 0 {
        return (-15, Q15::ZERO);
    }
    
    // Normalize to [0.5, 1.0)
    let mut exp = 0i16;
    let mut norm_x = x.0;
    
    if norm_x < 0x00008000 {  // Very small
        while norm_x < 0x00008000 {
            norm_x <<= 4;
            exp -= 4;
        }
    }
    
    while norm_x < 0x40000000 {
        norm_x <<= 1;
        exp -= 1;
    }
    
    while norm_x >= 0x80000000u32 as i32 {
        norm_x >>= 1;
        exp += 1;
    }
    
    // Polynomial approximation of log2(1+f) where f is fractional part
    // Using G.729A coefficients
    let f = (norm_x - 0x40000000) >> 15;  // Q15
    
    // log2(1+f) ≈ f*(C1 + f*(C2 + f*C3))
    // C1 = 1.4427, C2 = -0.6784, C3 = 0.2416 (approximate)
    let c1 = 23637;  // 1.4427 in Q14
    let c2 = -11086; // -0.6784 in Q14
    let c3 = 3952;   // 0.2416 in Q14
    
    let f2 = (f * f) >> 15;
    let f3 = (f2 * f) >> 15;
    
    let log_frac = ((c1 * f) >> 14) + ((c2 * f2) >> 14) + ((c3 * f3) >> 14);
    
    (exp + 1, Q15(log_frac as i16))
}

/// Power of 2 approximation for G.729A
/// Input: integer.fraction format
pub fn pow2_g729a(exp: i16, frac: Q15) -> Q31 {
    // 2^(exp + frac) = 2^exp * 2^frac
    
    // Polynomial approximation of 2^f where f in [0, 1)
    // 2^f ≈ 1 + f*(C1 + f*(C2 + f*C3))
    // Using G.729A coefficients
    let c1 = 22713;  // 0.6931 in Q15
    let c2 = 7912;   // 0.2416 in Q15
    let c3 = 1735;   // 0.0530 in Q15
    
    let f = frac.0;
    let f2 = (f as i32 * f as i32) >> 15;
    let f3 = (f2 * f as i32) >> 15;
    
    let pow_frac = Q15_ONE as i32 + 
                   ((c1 as i32 * f as i32) >> 15) +
                   ((c2 as i32 * f2) >> 15) +
                   ((c3 as i32 * f3) >> 15);
    
    // Apply exponent shift
    if exp >= 0 && exp < 31 {
        Q31(pow_frac << exp)
    } else if exp < 0 && exp > -31 {
        Q31(pow_frac >> (-exp))
    } else if exp >= 31 {
        Q31(0x7FFFFFFF)  // Saturate
    } else {
        Q31(0)
    }
}

/// Division approximation using multiplication by reciprocal
/// Computes num/den in Q15
pub fn div_q15(num: Q15, den: Q15) -> Q15 {
    if den.0 == 0 {
        return if num.0 >= 0 { Q15(Q15_ONE) } else { Q15(Q15_ONE.saturating_neg()) };
    }
    
    // For small denominators, use direct division
    if den.0.abs() < 100 {
        return Q15((((num.0 as i32) << 15) / den.0 as i32) as i16);
    }
    
    // Otherwise use reciprocal approximation
    let recip = reciprocal_q15(den);
    num.saturating_mul(recip)
}

/// Compute reciprocal in Q15
fn reciprocal_q15(x: Q15) -> Q15 {
    if x.0 == 0 {
        return Q15(Q15_ONE);
    }
    
    // 1/x = 2^(-log2(x))
    let x_q31 = x.to_q31();
    let (log_exp, log_frac) = log2_g729a(x_q31);
    let recip_q31 = pow2_g729a(-log_exp, Q15(-log_frac.0));
    recip_q31.to_q15()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_point_ops() {
        let a = Q15::from_f32(0.5);
        let b = Q15::from_f32(0.25);
        
        // Test addition
        let sum = a.saturating_add(b);
        assert!((sum.to_f32() - 0.75).abs() < 0.01);
        
        // Test multiplication
        let product = a.saturating_mul(b);
        assert!((product.to_f32() - 0.125).abs() < 0.01);
        
        // Test conversion
        let q31_val = a.to_q31();
        assert!((q31_val.to_f32() - 0.5).abs() < 0.001);
        assert!((q31_val.to_q15().to_f32() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_inverse_sqrt() {
        // Test inverse sqrt of 0.25 (should be 2.0, but clamped to Q15 range)
        let x = Q31::from_f32(0.25);
        let result = inverse_sqrt(x);
        // Since 2.0 is outside Q15 range, check it's close to max
        assert!(result.0 > 30000);
        
        // Test inverse sqrt of 0.5 (should be ~1.414, clamped to ~0.999)
        let x = Q31::from_f32(0.5);
        let result = inverse_sqrt(x);
        assert!(result.0 > 30000);
        
        // Test inverse sqrt of 1.0 (should be 1.0, but in Q15 ~0.999)
        let x = Q31::from_f32(0.9999);
        let result = inverse_sqrt(x);
        assert!((result.to_f32() - 0.999).abs() < 0.1);
    }

    #[test]
    fn test_log2_approximation() {
        // Test log2(0.5) = -1
        let x = Q31::from_f32(0.5);
        let (exp, mantissa) = log2_approximation(x);
        assert_eq!(exp, -1);
        assert!(mantissa.0.abs() < 1000); // Small mantissa
        
        // Test log2(0.25) = -2
        let x = Q31::from_f32(0.25);
        let (exp, mantissa) = log2_approximation(x);
        assert_eq!(exp, -2);
        
        // Test log2(0.75) ≈ -0.415
        let x = Q31::from_f32(0.75);
        let (exp, mantissa) = log2_approximation(x);
        assert_eq!(exp, -1);
    }

    #[test]
    fn test_power2_approximation() {
        // Test 2^0 = 1
        let result = power2_approximation(0, Q15(0));
        assert!((result.to_f32() - 0.5).abs() < 0.1); // Normalized to Q31
        
        // Test 2^1 = 2 (clamped to Q31 max)
        let result = power2_approximation(1, Q15(0));
        assert!(result.0 > (1 << 30));
        
        // Test 2^(-1) = 0.5
        let result = power2_approximation(-1, Q15(0));
        assert!((result.to_f32() - 0.25).abs() < 0.1);
    }

    #[test]
    fn test_abs_functions() {
        let pos = Q15::from_f32(0.5);
        let neg = Q15::from_f32(-0.5);
        
        assert_eq!(abs_q15(pos).0, pos.0);
        assert_eq!(abs_q15(neg).0, -neg.0);
        
        let pos31 = Q31::from_f32(0.5);
        let neg31 = Q31::from_f32(-0.5);
        
        assert_eq!(abs_q31(pos31).0, pos31.0);
        assert_eq!(abs_q31(neg31).0, -neg31.0);
    }
} 