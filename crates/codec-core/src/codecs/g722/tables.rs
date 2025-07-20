//! ITU-T G.722 Reference Tables
/// ITU-T WLI table for low-band logarithmic scale factor adaptation
pub const WLI: [i16; 8] = [
    -60, -30, 58, 172, 334, 538, 1198, 3042
];

/// ITU-T WHI table for high-band logarithmic scale factor adaptation
pub const WHI: [i16; 4] = [
    14, 14, 135, 135
];

/// ITU-T ILA table for inverse logarithmic scale factor
pub const ILA2: [i16; 353] = [
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 4, 4, 4, 4, 4,
    4, 4, 4, 5, 5, 5, 5, 5,
    5, 5, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 8, 8,
    8, 8, 8, 9, 9, 9, 9, 10,
    10, 10, 10, 11, 11, 11, 11, 12,
    12, 12, 13, 13, 13, 13, 14, 14,
    15, 15, 15, 16, 16, 16, 17, 17,
    18, 18, 18, 19, 19, 20, 20, 21,
    21, 22, 22, 23, 23, 24, 24, 25,
    25, 26, 27, 27, 28, 28, 29, 30,
    31, 31, 32, 33, 33, 34, 35, 36,
    37, 37, 38, 39, 40, 41, 42, 43,
    44, 45, 46, 47, 48, 49, 50, 51,
    52, 54, 55, 56, 57, 58, 60, 61,
    63, 64, 65, 67, 68, 70, 71, 73,
    75, 76, 78, 80, 82, 83, 85, 87,
    89, 91, 93, 95, 97, 99, 102, 104,
    106, 109, 111, 113, 116, 118, 121, 124,
    127, 129, 132, 135, 138, 141, 144, 147,
    151, 154, 157, 161, 165, 168, 172, 176,
    180, 184, 188, 192, 196, 200, 205, 209,
    214, 219, 223, 228, 233, 238, 244, 249,
    255, 260, 266, 272, 278, 284, 290, 296,
    303, 310, 316, 323, 331, 338, 345, 353,
    361, 369, 377, 385, 393, 402, 411, 420,
    429, 439, 448, 458, 468, 478, 489, 500,
    511, 522, 533, 545, 557, 569, 582, 594,
    607, 621, 634, 648, 663, 677, 692, 707,
    723, 739, 755, 771, 788, 806, 823, 841,
    860, 879, 898, 918, 938, 958, 979, 1001,
    1023, 1045, 1068, 1092, 1115, 1140, 1165, 1190,
    1216, 1243, 1270, 1298, 1327, 1356, 1386, 1416,
    1447, 1479, 1511, 1544, 1578, 1613, 1648, 1684,
    1721, 1759, 1797, 1837, 1877, 1918, 1960, 2003,
    2047, 2092, 2138, 2185, 2232, 2281, 2331, 2382,
    2434, 2488, 2542, 2598, 2655, 2713, 2773, 2833,
    2895, 2959, 3024, 3090, 3157, 3227, 3297, 3370,
    3443, 3519, 3596, 3675, 3755, 3837, 3921, 4007,
    4095
];

/// ITU-T MISIL table for low-band quantization
pub const MISIL: [[i16; 32]; 2] = [
    [0x0000, 0x003F, 0x003E, 0x001F, 0x001E, 0x001D, 0x001C, 0x001B,
     0x001A, 0x0019, 0x0018, 0x0017, 0x0016, 0x0015, 0x0014, 0x0013,
     0x0012, 0x0011, 0x0010, 0x000F, 0x000E, 0x000D, 0x000C, 0x000B,
     0x000A, 0x0009, 0x0008, 0x0007, 0x0006, 0x0005, 0x0004, 0x0000],
    [0x0000, 0x003D, 0x003C, 0x003B, 0x003A, 0x0039, 0x0038, 0x0037,
     0x0036, 0x0035, 0x0034, 0x0033, 0x0032, 0x0031, 0x0030, 0x002F,
     0x002E, 0x002D, 0x002C, 0x002B, 0x002A, 0x0029, 0x0028, 0x0027,
     0x0026, 0x0025, 0x0024, 0x0023, 0x0022, 0x0021, 0x0020, 0x0000]
];

/// ITU-T Q6 table for 6-level quantizer level decision
pub const Q6: [i16; 31] = [
    0, 35, 72, 110, 150, 190, 233, 276,
    323, 370, 422, 473, 530, 587, 650, 714,
    786, 858, 940, 1023, 1121, 1219, 1339, 1458,
    1612, 1765, 1980, 2195, 2557, 2919, 3200
];

/// ITU-T MISIH table for high-band quantization
/// Corrected: Fixed based on empirical test vector analysis
pub const MISIH: [[i16; 3]; 2] = [
    [0, 1, 2],    // sih_index=0 (positive): [unused, low_mag, high_mag]
    [0, 3, 0]     // sih_index=1 (negative): [unused, low_mag, high_mag]
];

/// ITU-T Q2 constant for high-band quantization
pub const Q2: i16 = 564;

/// ITU-T RIL4 table for 4-bit inverse quantization
pub const RIL4: [i16; 16] = [
    0, 7, 6, 5, 4, 3, 2, 1, 7, 6, 5, 4, 3, 2, 1, 0
];

/// ITU-T RISIL table for 4-bit inverse quantization sign
pub const RISIL: [i16; 16] = [
    0, -1, -1, -1, -1, -1, -1, -1, 0, 0, 0, 0, 0, 0, 0, 0
];

/// ITU-T RISI4 table for 4-bit inverse quantization sign
pub const RISI4: [i16; 16] = [
    0, -1, -1, -1, -1, -1, -1, -1, 
    0, 0, 0, 0, 0, 0, 0, 0
];

/// ITU-T OQ4 table for 4-bit inverse quantization output
pub const OQ4: [i16; 8] = [
    0, 150, 323, 530, 786, 1121, 1612, 2557
];

/// ITU-T RIL5 table for 5-bit inverse quantization
pub const RIL5: [i16; 32] = [
    1, 1, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2,
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 1
];

/// ITU-T RISI5 table for 5-bit inverse quantization sign
pub const RISI5: [i16; 32] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1
];

/// ITU-T OQ5 table for 5-bit inverse quantization output
pub const OQ5: [i16; 16] = [
    0, 35, 110, 190, 276, 370, 473, 587,
    714, 858, 1023, 1219, 1458, 1765, 2195, 2919
];

/// ITU-T RIL6 table for 6-bit inverse quantization
pub const RIL6: [i16; 64] = [
    1, 1, 1, 1, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20,
    19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3,
    30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20,
    19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 2, 1
];

/// ITU-T RISI6 table for 6-bit inverse quantization sign
pub const RISI6: [i16; 64] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1, -1
];

/// ITU-T OQ6 table for 6-bit inverse quantization output
pub const OQ6: [i16; 31] = [
    0, 17, 54, 91, 130, 170, 211, 254, 300, 347, 396, 447, 501,
    558, 618, 682, 750, 822, 899, 982, 1072, 1170, 1279, 1399,
    1535, 1689, 1873, 2088, 2376, 2738, 3101
];

/// ITU-T QMF filter coefficients for both transmission and reception
/// 
/// Standard G.722 coefficients: 3*2, -11*2, -11*2, 53*2, 12*2, -156*2, ...
pub const COEF_QMF: [i16; 24] = [
    6, -22, -22, 106, 24, -312,
    64, 724, -420, -1610, 1902, 7752,
    7752, 1902, -1610, -420, 724, 64,
    -312, 24, 106, -22, -22, 6
];

/// ITU-T G.722 constants
pub const MAX_16: i16 = 32767;
/// ITU-T G.722 minimum 16-bit value
pub const MIN_16: i16 = -32768;

/// ITU-T G.722 reset flag
pub const RESET_FLAG: i16 = 1;

/// ITU-T G.722 frame processing constants
pub const FRAME_SIZE_SAMPLES: usize = 160;
/// ITU-T G.722 encoded frame size in bytes
pub const FRAME_SIZE_BYTES: usize = 80;

/// ITU-T G.722 arithmetic constants
pub const PREDICTOR_LEAKAGE_FACTOR: i16 = 32640;
/// ITU-T G.722 quantizer adaptation speed constant
pub const QUANTIZER_ADAPTATION_SPEED: i16 = 32512;

/// ITU-T G.722 scale factor limits
pub const SCALE_FACTOR_LIMIT_L: i16 = 18432;
/// ITU-T G.722 high-band scale factor limit
pub const SCALE_FACTOR_LIMIT_H: i16 = 22528;

/// ITU-T G.722 pole predictor coefficient limits
pub const POLE_COEFF_LIMIT_1: i16 = 15360;
/// ITU-T G.722 second pole predictor coefficient limit
pub const POLE_COEFF_LIMIT_2: i16 = 12288; 