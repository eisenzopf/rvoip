use crate::constants::NP;
use crate::tables::annexa::fg;

/// Public constant `LBF_CORR`.
pub const LBF_CORR: [i16; NP + 1] = [
    7869, 7011, 4838, 2299, 321, -660, -782, -484, -164, 3, 39, 21, 4,
];

/// Public constant `SHIFT_FX`.
pub const SHIFT_FX: [i16; 33] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5,
    0,
];

/// Public constant `FACTOR_FX`.
pub const FACTOR_FX: [i16; 33] = [
    32767, 16913, 17476, 18079, 18725, 19418, 20165, 20972, 21845, 22795, 23831, 24966, 26214,
    27594, 29127, 30840, 32767, 17476, 18725, 20165, 21845, 23831, 26214, 29127, 32767, 18725,
    21845, 26214, 32767, 21845, 32767, 32767, 0,
];

/// Public constant `FACT`.
pub const FACT: [i16; 3] = [410, 26, 13];
/// Public constant `MARG`.
pub const MARG: [i16; 3] = [0, 0, 1];

/// Public constant `TAB_SIDGAIN`.
pub const TAB_SIDGAIN: [i16; 32] = [
    2, 5, 8, 13, 20, 32, 50, 64, 80, 101, 127, 160, 201, 253, 318, 401, 505, 635, 800, 1007, 1268,
    1596, 2010, 2530, 3185, 4009, 5048, 6355, 8000, 10071, 12679, 15962,
];

/// Public constant `NOISE_FG_SUM`.
pub const NOISE_FG_SUM: [[i16; 10]; 2] = [
    [7798, 8447, 8205, 8293, 8126, 8477, 8447, 8703, 9043, 8604],
    [
        10514, 12402, 12833, 11914, 11447, 11670, 11132, 11311, 11844, 11447,
    ],
];

/// Public constant `NOISE_FG_SUM_INV`.
pub const NOISE_FG_SUM_INV: [[i16; 10]; 2] = [
    [
        17210, 15888, 16357, 16183, 16516, 15833, 15888, 15421, 14840, 15597,
    ],
    [
        12764, 10821, 10458, 11264, 11724, 11500, 12056, 11865, 11331, 11724,
    ],
];

/// Public constant `PTR_TAB_1`.
pub const PTR_TAB_1: [i16; 32] = [
    96, 52, 20, 54, 86, 114, 82, 68, 36, 121, 48, 92, 18, 120, 94, 124, 50, 125, 4, 100, 28, 76,
    12, 117, 81, 22, 90, 116, 127, 21, 108, 66,
];

/// Public constant `PTR_TAB_2`.
pub const PTR_TAB_2: [[i16; 16]; 2] = [
    [31, 21, 9, 3, 10, 2, 19, 26, 4, 3, 11, 29, 15, 27, 21, 12],
    [16, 1, 0, 0, 8, 25, 22, 20, 19, 23, 20, 31, 4, 31, 20, 31],
];

/// Public constant `MP_WEIGHT`.
pub const MP_WEIGHT: [i16; 2] = [8644, 16572];

/// Public constant `A_GAIN0`.
pub const A_GAIN0: i16 = 28672;
/// Public constant `A_GAIN1`.
pub const A_GAIN1: i16 = 4096;

/// Public constant `fn`.
#[inline(always)]
pub const fn noise_fg(mode: usize, k: usize, j: usize) -> i16 {
    if mode == 0 {
        fg(0, k, j)
    } else {
        let a = fg(0, k, j) as i32;
        let b = fg(1, k, j) as i32;
        ((a * 19660 + b * 13107) >> 15) as i16
    }
}
