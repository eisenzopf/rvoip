//! This module implements the G.729 basic operators.
//! The functions are implemented to be bit-exact with the G.729 C reference code.
//! The C code uses a special 32-bit format (DPF) for some operations, which is
//! replicated here using i64 for intermediate calculations to prevent overflow.

pub type Word16 = i16;
pub type Word32 = i32;

pub static mut OVERFLOW: bool = false;

pub const MIN_32: Word32 = -2147483648;
pub const MAX_32: Word32 = 2147483647;


pub fn add(var1: Word16, var2: Word16) -> Word16 {
    let l_sum = (var1 as i32) + (var2 as i32);
    if l_sum > 32767 {
        32767
    } else if l_sum < -32768 {
        -32768
    } else {
        l_sum as Word16
    }
}

pub fn sub(var1: Word16, var2: Word16) -> Word16 {
    let l_diff = (var1 as i32) - (var2 as i32);
    if l_diff > 32767 {
        32767
    } else if l_diff < -32768 {
        -32768
    } else {
        l_diff as Word16
    }
}

pub fn abs_s(var1: Word16) -> Word16 {
    if var1 == -32768 {
        32767
    } else {
        var1.abs()
    }
}

pub fn shl(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        return shr(var1, -var2);
    }
    let resultat = (var1 as i32) * (1i32 << var2);
    if (var2 > 15 && var1 != 0) || (resultat != (resultat as Word16) as i32) {
        if var1 > 0 {
            32767
        } else {
            -32768
        }
    } else {
        resultat as Word16
    }
}

pub fn shr(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        return shl(var1, -var2);
    }
    if var2 >= 15 {
        if var1 < 0 {
            -1
        } else {
            0
        }
    } else {
        if var1 < 0 {
            !((!var1) >> var2)
        } else {
            var1 >> var2
        }
    }
}

fn sature(l_var1: Word32) -> Word16 {
    if l_var1 > 0x7fff {
        unsafe { OVERFLOW = true; }
        32767
    } else if l_var1 < -0x8000 {
        unsafe { OVERFLOW = true; }
        -32768
    } else {
        unsafe { OVERFLOW = false; }
        l_var1 as Word16
    }
}

pub fn mult(var1: Word16, var2: Word16) -> Word16 {
    let mut l_produit = (var1 as i32).wrapping_mul(var2 as i32);
    l_produit = (l_produit & 0xffff8000u32 as i32).wrapping_shr(15);
    if (l_produit & 0x00010000) != 0 {
        l_produit |= 0xffff0000u32 as i32;
    }
    sature(l_produit)
}

pub fn l_mult(var1: Word16, var2: Word16) -> Word32 {
    let mut l_var_out = (var1 as i32).wrapping_mul(var2 as i32);
    if l_var_out != 0x40000000 {
        l_var_out = l_var_out.wrapping_mul(2);
    } else {
        unsafe {
            OVERFLOW = true;
        }
        l_var_out = std::i32::MAX;
    }
    l_var_out
}

pub fn negate(var1: Word16) -> Word16 {
    if var1 == -32768 {
        32767
    } else {
        -var1
    }
}

pub fn extract_h(l_var1: Word32) -> Word16 {
    (l_var1 >> 16) as Word16
}

pub fn extract_l(l_var1: Word32) -> Word16 {
    (l_var1 & 0xFFFF) as Word16
}

pub fn round(l_var1: Word32) -> Word16 {
    let l_arrondi = l_add(l_var1, 0x00008000);
    extract_h(l_arrondi)
}

pub fn l_mac(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_produit = l_mult(var1, var2);
    let l_var_out = l_add(l_var3, l_produit);
    l_var_out
}

pub fn l_msu(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_produit = l_mult(var1, var2);
    l_sub(l_var3, l_produit)
}

pub fn l_add(l_var1: Word32, l_var2: Word32) -> Word32 {
    let temp = (l_var1 as i64) + (l_var2 as i64);
    if temp > MAX_32 as i64 {
        unsafe { OVERFLOW = true; }
        MAX_32
    } else if temp < MIN_32 as i64 {
        unsafe { OVERFLOW = true; }
        MIN_32
    } else {
        unsafe { OVERFLOW = false; }
        temp as Word32
    }
}

pub fn l_sub(l_var1: Word32, l_var2: Word32) -> Word32 {
    let temp = (l_var1 as i64) - (l_var2 as i64);
    if temp > MAX_32 as i64 {
        unsafe { OVERFLOW = true; }
        MAX_32
    } else if temp < MIN_32 as i64 {
        unsafe { OVERFLOW = true; }
        MIN_32
    } else {
        unsafe { OVERFLOW = false; }
        temp as Word32
    }
}

pub fn l_shl(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 <= 0 {
        return l_shr(l_var1, -var2);
    }
    let mut l_acc = l_var1;
    for _ in 0..var2 {
        if l_acc > 0x3FFFFFFF {
            unsafe { OVERFLOW = true; }
            return std::i32::MAX;
        } else if l_acc < -0x40000000 { // 0xc0000000
            unsafe { OVERFLOW = true; }
            return std::i32::MIN;
        }
        l_acc *= 2;
    }
    l_acc
}

pub fn l_shr(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 < 0 {
        return l_shl(l_var1, -var2);
    }
    if var2 >= 31 {
        if l_var1 < 0 {
            -1
        } else {
            0
        }
    } else {
        if l_var1 < 0 {
            !((!l_var1) >> var2)
        } else {
            l_var1 >> var2
        }
    }
}

pub fn mult_r(var1: Word16, var2: Word16) -> Word16 {
    let l_produit_arr = (var1 as i64) * (var2 as i64);
    let l_produit_arr = (l_produit_arr + 0x4000) >> 15;
    if l_produit_arr > 32767 {
        32767
    } else if l_produit_arr < -32768 {
        -32768
    } else {
        l_produit_arr as Word16
    }
}

pub fn l_deposit_h(var1: Word16) -> Word32 {
    (var1 as Word32) << 16
}

pub fn l_deposit_l(var1: Word16) -> Word32 {
    var1 as Word32
}

pub fn l_abs(l_var1: Word32) -> Word32 {
    if l_var1 == std::i32::MIN {
        std::i32::MAX
    } else {
        l_var1.abs()
    }
}

pub fn norm_s(var1: Word16) -> Word16 {
    if var1 == 0 {
        return 0;
    }
    if var1 == -1 {
        return 15;
    }
    let mut var_out = 0;
    let mut var1_mut = var1;
    if var1_mut < 0 {
        var1_mut = !var1_mut;
    }
    while var1_mut < 0x4000 {
        var_out += 1;
        var1_mut <<= 1;
    }
    var_out
}

pub fn norm_l(l_var1: Word32) -> Word16 {
    if l_var1 == 0 {
        return 0;
    }
    if l_var1 == -1 {
        return 31;
    }
    let mut var1 = l_var1;
    if var1 < 0 {
        var1 = !var1;
    }
    let mut var_out = 0;
    while var1 < 0x40000000 {
        var_out += 1;
        var1 <<= 1;
    }
    var_out
}

pub fn div_s(var1: Word16, var2: Word16) -> Word16 {
    if var1 < 0 || var2 <= 0 || var1 > var2 {
        // Error case, handled by panic.
        // In a real-world scenario, a more graceful error handling would be preferred.
        panic!("div_s error: invalid inputs var1={}, var2={}", var1, var2);
    }

    if var1 == var2 {
        return 32767; // Corresponds to MAX_16 in C
    }

    let mut l_num = var1 as Word32;
    let l_denom = var2 as Word32;
    let mut var_out: Word16 = 0;

    for _ in 0..15 {
        var_out <<= 1;
        l_num <<= 1;

        if l_num >= l_denom {
            l_num = l_sub(l_num, l_denom);
            var_out = add(var_out, 1);
        }
    }
    var_out
}

pub fn l_negate(l_var1: Word32) -> Word32 {
    if l_var1 == std::i32::MIN {
        std::i32::MAX
    } else {
        -l_var1
    }
}

const TABSQR: [Word16; 49] = [
    32767, 31790, 30894, 30070, 29309, 28602, 27945, 27330, 26755, 26214, 25705, 25225,
    24770, 24339, 23930, 23541, 23170, 22817, 22479, 22155, 21845, 21548, 21263, 20988,
    20724, 20470, 20225, 19988, 19760, 19539, 19326, 19119, 18919, 18725, 18536, 18354,
    18176, 18004, 17837, 17674, 17515, 17361, 17211, 17064, 16921, 16782, 16646, 16514,
    16384,
];

pub fn inv_sqrt(l_x: Word32) -> Word32 {
    if l_x == 0 {
        return 0x7fffffff;
    }
    let exp = norm_l(l_x);
    let l_x = l_shl(l_x, exp);
    
    let exp = sub(30, exp as Word16);
    let l_x = if (exp & 1) == 0 {
        l_shr(l_x, 1)
    } else {
        l_x
    };
    
    let exp = shr(exp, 1);
    let exp = add(exp, 1);
    
    let l_x = l_shr(l_x, 9);
    let i = extract_h(l_x);
    let l_x = l_shr(l_x, 1);
    let a = extract_l(l_x) & 0x7fff;
    
    let i = sub(i, 16);
    
    let l_y = l_deposit_h(TABSQR[i as usize]);
    let tmp = sub(TABSQR[i as usize], TABSQR[(i + 1) as usize]);
    let l_y = l_msu(l_y, tmp, a);
    
    l_shr(l_y, exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(10, 20), 30);
        assert_eq!(add(32767, 1), 32767);
        assert_eq!(add(-32768, -1), -32768);
    }

    #[test]
    fn test_sub() {
        assert_eq!(sub(20, 10), 10);
        assert_eq!(sub(-32768, 1), -32768);
        assert_eq!(sub(32767, -1), 32767);
    }

    #[test]
    fn test_abs_s() {
        assert_eq!(abs_s(10), 10);
        assert_eq!(abs_s(-10), 10);
        assert_eq!(abs_s(-32768), 32767);
    }

    #[test]
    fn test_shl() {
        assert_eq!(shl(10, 2), 40);
        assert_eq!(shl(16384, 1), 32767);
        assert_eq!(shl(-16384, 1), -32768);
    }

    #[test]
    fn test_shr() {
        assert_eq!(shr(40, 2), 10);
        assert_eq!(shr(-40, 2), -10);
    }

    #[test]
    fn test_mult() {
        assert_eq!(mult(16384, 16384), 8192);
        assert_eq!(mult(-16384, -16384), 8192);
    }

    #[test]
    fn test_l_mult() {
        assert_eq!(l_mult(16384, 16384), 536870912);
        assert_eq!(l_mult(-16384, -16384), 536870912);
    }

    #[test]
    fn test_l_mac() {
        assert_eq!(l_mac(10, 16384, 16384), 536870922);
    }

    #[test]
    fn test_l_msu() {
        assert_eq!(l_msu(10, 16384, 16384), -536870902);
    }

    #[test]
    fn test_round() {
        assert_eq!(round(536870912), 8192);
    }

    #[test]
    fn test_norm_s() {
        assert_eq!(norm_s(0), 0);
        assert_eq!(norm_s(-1), 15);
        assert_eq!(norm_s(1), 14);
        assert_eq!(norm_s(16384), 0);
        assert_eq!(norm_s(-16385), 0);
        assert_eq!(norm_s(-32768), 0);
    }
}
