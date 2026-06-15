include!("annexa_tables.rs");

/// Public constant `fn`.
#[inline(always)]
pub const fn lspcb1(code: usize, j: usize) -> i16 {
    LSPCB1[code * 10 + j]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn lspcb2(code: usize, j: usize) -> i16 {
    LSPCB2[code * 10 + j]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn fg(mode: usize, k: usize, j: usize) -> i16 {
    FG[mode * 40 + k * 10 + j]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn fg_sum(mode: usize, j: usize) -> i16 {
    FG_SUM[mode * 10 + j]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn fg_sum_inv(mode: usize, j: usize) -> i16 {
    FG_SUM_INV[mode * 10 + j]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn gbk1(index: usize, col: usize) -> i16 {
    GBK1[index * 2 + col]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn gbk2(index: usize, col: usize) -> i16 {
    GBK2[index * 2 + col]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn coef(row: usize, col: usize) -> i16 {
    COEF[row * 2 + col]
}

/// Public constant `fn`.
#[inline(always)]
pub const fn l_coef(row: usize, col: usize) -> i32 {
    L_COEF[row * 2 + col]
}
