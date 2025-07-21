//! Window function tables for G.729A

use crate::codecs::g729a::types::Q15;

/// Hamming-cosine window for LPC analysis
/// 240 samples total (200 + 40 lookahead)
/// Values are in Q15 format
pub const HAMMING_WINDOW: [i16; 240] = [
    2621, 2623, 2629, 2638, 2651, 2668, 2689, 2713, 2741, 2772,
    2808, 2847, 2890, 2936, 2986, 3040, 3097, 3158, 3223, 3291,
    3363, 3438, 3517, 3599, 3685, 3774, 3867, 3963, 4063, 4166,
    4272, 4382, 4495, 4611, 4731, 4853, 4979, 5108, 5240, 5376,
    5514, 5655, 5800, 5947, 6097, 6250, 6406, 6565, 6726, 6890,
    7057, 7227, 7399, 7573, 7750, 7930, 8112, 8296, 8483, 8672,
    8863, 9057, 9252, 9450, 9650, 9852, 10055, 10261, 10468, 10677,
    10888, 11101, 11315, 11531, 11748, 11967, 12187, 12409, 12632, 12856,
    13082, 13308, 13536, 13764, 13994, 14225, 14456, 14688, 14921, 15155,
    15389, 15624, 15859, 16095, 16331, 16568, 16805, 17042, 17279, 17516,
    17754, 17991, 18228, 18465, 18702, 18939, 19175, 19411, 19647, 19882,
    20117, 20350, 20584, 20816, 21048, 21279, 21509, 21738, 21967, 22194,
    22420, 22644, 22868, 23090, 23311, 23531, 23749, 23965, 24181, 24394,
    24606, 24816, 25024, 25231, 25435, 25638, 25839, 26037, 26234, 26428,
    26621, 26811, 26999, 27184, 27368, 27548, 27727, 27903, 28076, 28247,
    28415, 28581, 28743, 28903, 29061, 29215, 29367, 29515, 29661, 29804,
    29944, 30081, 30214, 30345, 30472, 30597, 30718, 30836, 30950, 31062,
    31170, 31274, 31376, 31474, 31568, 31659, 31747, 31831, 31911, 31988,
    32062, 32132, 32198, 32261, 32320, 32376, 32428, 32476, 32521, 32561,
    32599, 32632, 32662, 32688, 32711, 32729, 32744, 32755, 32763, 32767,
    32767, 32741, 32665, 32537, 32359, 32129, 31850, 31521, 31143, 30716,
    30242, 29720, 29151, 28538, 27879, 27177, 26433, 25647, 24821, 23957,
    23055, 22117, 21145, 20139, 19102, 18036, 16941, 15820, 14674, 13505,
    12315, 11106, 9879, 8637, 7381, 6114, 4838, 3554, 2264, 971
];

/// Lag window for autocorrelation
/// Special double precision format - upper and lower parts
/// Bandwidth expansion = 60 Hz, noise floor = 1.0001
/// These values represent lag_window[1] through lag_window[10]
/// lag_window[0] = 1.0 (not stored)
pub const LAG_WINDOW_H: [i16; 10] = [
    32728,  // 0.99879038 upper
    32619,  // 0.99546897 upper
    32438,  // 0.98995781 upper
    32187,  // 0.98229337 upper
    31867,  // 0.97252619 upper
    31480,  // 0.96072036 upper
    31029,  // 0.94695264 upper
    30517,  // 0.93131179 upper
    29946,  // 0.91389757 upper
    29321   // 0.89481968 upper
];

pub const LAG_WINDOW_L: [i16; 10] = [
    11904,  // 0.99879038 lower
    17280,  // 0.99546897 lower
    30720,  // 0.98995781 lower
    25856,  // 0.98229337 lower
    24192,  // 0.97252619 lower
    28992,  // 0.96072036 lower
    24384,  // 0.94695264 lower
    7360,   // 0.93131179 lower
    19520,  // 0.91389757 lower
    14784   // 0.89481968 lower
];

/// Convert window values to Q15 type
pub fn get_hamming_window() -> Vec<Q15> {
    HAMMING_WINDOW.iter().map(|&val| Q15(val)).collect()
}

/// Get lag window coefficient at index (1-based)
/// Returns the full precision value by combining high and low parts
pub fn get_lag_window_coeff(idx: usize) -> i32 {
    if idx == 0 || idx > 10 {
        return 1 << 31; // 1.0 in Q31
    }
    
    let i = idx - 1;
    // Combine high and low parts to form Q31 value
    ((LAG_WINDOW_H[i] as i32) << 16) | (LAG_WINDOW_L[i] as i32 & 0xFFFF)
}

/// Get lag window coefficients as Q15 values for autocorrelation
/// Returns [1.0, w[1], w[2], ..., w[10]] in Q15 format
pub fn get_lag_window() -> Vec<Q15> {
    let mut lag_window = Vec::with_capacity(11);
    
    // lag_window[0] = 1.0 in Q15
    lag_window.push(Q15(32767));
    
    // Convert Q31 values to Q15 by right shift 16
    for i in 1..=10 {
        let q31_val = get_lag_window_coeff(i);
        let q15_val = (q31_val >> 16) as i16;
        lag_window.push(Q15(q15_val));
    }
    
    lag_window
} 