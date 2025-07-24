use crate::common::basic_operators::{
    div_s, extract_h, extract_l, l_mac, l_msu, l_mult, l_shr, l_sub, mult, Word16, Word32,
};

pub fn l_extract(l_32: Word32) -> (Word16, Word16) {
    let hi = extract_h(l_32);
    let lo = extract_l(l_msu(l_shr(l_32, 1), hi, 16384));
    (hi, lo)
}

pub fn l_comp(hi: Word16, lo: Word16) -> Word32 {
    let l_32 = (hi as Word32) << 16;
    l_mac(l_32, lo, 1)
}

pub fn mpy_32(hi1: Word16, lo1: Word16, hi2: Word16, lo2: Word16) -> Word32 {
    let mut l_32 = l_mult(hi1, hi2);
    l_32 = l_mac(l_32, mult(hi1, lo2), 1);
    l_mac(l_32, mult(lo1, hi2), 1)
}

pub fn mpy_32_16(hi: Word16, lo: Word16, n: Word16) -> Word32 {
    let l_32 = l_mult(hi, n);
    l_mac(l_32, mult(lo, n), 1)
}

pub fn div_32(l_num: Word32, denom_hi: Word16, denom_lo: Word16) -> Word32 {
    // First approximation: 1 / L_denom = 1/denom_hi
    let approx = div_s(0x3fff, denom_hi); // result in Q14

    // 1/L_denom = approx * (2.0 - L_denom * approx)
    let l_32 = mpy_32_16(denom_hi, denom_lo, approx); // result in Q30
    let l_32 = l_sub(0x7fffffff, l_32); // result in Q30

    let (hi, lo) = l_extract(l_32);
    let l_32 = mpy_32_16(hi, lo, approx); // = 1/L_denom in Q29

    // L_num * (1/L_denom)
    let (hi, lo) = l_extract(l_32);
    let (n_hi, n_lo) = l_extract(l_num);
    let l_32 = mpy_32(n_hi, n_lo, hi, lo); // result in Q29
    l_32 << 2 // From Q29 to Q31
}
