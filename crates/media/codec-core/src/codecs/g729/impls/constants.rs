/// Public constant `L_FRAME`.
pub const L_FRAME: usize = 80;
/// Public constant `L_SUBFR`.
pub const L_SUBFR: usize = 40;
/// Public constant `M`.
pub const M: usize = 10;
/// Public constant `MP1`.
pub const MP1: usize = M + 1;
/// Public constant `NP`.
pub const NP: usize = 12;
/// Public constant `L_TOTAL`.
pub const L_TOTAL: usize = 240;
/// Public constant `L_WINDOW`.
pub const L_WINDOW: usize = 240;
/// Public constant `L_NEXT`.
pub const L_NEXT: usize = 40;

/// Public constant `PIT_MIN`.
pub const PIT_MIN: i16 = 20;
/// Public constant `PIT_MAX`.
pub const PIT_MAX: i16 = 143;
/// Public constant `L_INTERPOL`.
pub const L_INTERPOL: usize = 11;
/// Public constant `L_INTER10`.
pub const L_INTER10: usize = 10;
/// Public constant `UP_SAMP`.
pub const UP_SAMP: usize = 3;
/// Public constant `FIR_SIZE_SYN`.
pub const FIR_SIZE_SYN: usize = UP_SAMP * L_INTER10 + 1;

/// Public constant `SHARPMAX`.
pub const SHARPMAX: i16 = 13017; // 0.8 Q14
/// Public constant `SHARPMIN`.
pub const SHARPMIN: i16 = 3277; // 0.2 Q14
/// Public constant `TILT_WSP`.
pub const TILT_WSP: i16 = 22938; // 0.7 Q15 (encoder weighted-speech tilt)
/// Public constant `GAMMA1`.
pub const GAMMA1: i16 = 24576; // 0.75 Q15

/// Public constant `PRM_SIZE`.
pub const PRM_SIZE: usize = 11;
/// Public constant `SERIAL_SIZE`.
pub const SERIAL_SIZE: usize = 82;

/// Public constant `MA_NP`.
pub const MA_NP: usize = 4;
/// Public constant `MODE`.
pub const MODE: usize = 2;
/// Public constant `NC`.
pub const NC: usize = 5;
/// Public constant `NC0_B`.
pub const NC0_B: usize = 7;
/// Public constant `NC1_B`.
pub const NC1_B: usize = 5;
/// Public constant `NC0`.
pub const NC0: usize = 1 << NC0_B;
/// Public constant `NC1`.
pub const NC1: usize = 1 << NC1_B;

/// Public constant `NCODE1_B`.
pub const NCODE1_B: usize = 3;
/// Public constant `NCODE2_B`.
pub const NCODE2_B: usize = 4;
/// Public constant `NCODE1`.
pub const NCODE1: usize = 1 << NCODE1_B;
/// Public constant `NCODE2`.
pub const NCODE2: usize = 1 << NCODE2_B;
/// Public constant `NCAN1`.
pub const NCAN1: usize = 4;
/// Public constant `NCAN2`.
pub const NCAN2: usize = 8;
/// Public constant `INV_COEF`.
pub const INV_COEF: i16 = -17103; // Q19

/// Public constant `DIM_RR`.
pub const DIM_RR: usize = 616;
/// Public constant `NB_POS`.
pub const NB_POS: usize = 8;
/// Public constant `STEP`.
pub const STEP: usize = 5;
/// Public constant `MSIZE`.
pub const MSIZE: usize = 64;

/// Public constant `PI04`.
pub const PI04: i16 = 1029; // Q13
/// Public constant `PI92`.
pub const PI92: i16 = 23677; // Q13
/// Public constant `CONST10`.
pub const CONST10: i16 = 10 * (1 << 11); // Q11
/// Public constant `CONST12`.
pub const CONST12: i16 = 19661; // Q14

/// Public constant `L_H`.
pub const L_H: usize = 22;
/// Public constant `GAMMAP`.
pub const GAMMAP: i16 = 16384;
/// Public constant `INV_GAMMAP`.
pub const INV_GAMMAP: i16 = 21845;
/// Public constant `GAMMAP_2`.
pub const GAMMAP_2: i16 = 10923;
/// Public constant `GAMMA2_PST`.
pub const GAMMA2_PST: i16 = 18022;
/// Public constant `GAMMA1_PST`.
pub const GAMMA1_PST: i16 = 22938;
/// Public constant `MU`.
pub const MU: i16 = 26214;
/// Public constant `AGC_FAC`.
pub const AGC_FAC: i16 = 29491;
/// Public constant `AGC_FAC1`.
pub const AGC_FAC1: i16 = i16::MAX - AGC_FAC;

/// Public constant `L_LIMIT`.
pub const L_LIMIT: i16 = 40;
/// Public constant `M_LIMIT`.
pub const M_LIMIT: i16 = 25681;
/// Public constant `GAP1`.
pub const GAP1: i16 = 10;
/// Public constant `GAP2`.
pub const GAP2: i16 = 5;
/// Public constant `GAP3`.
pub const GAP3: i16 = 321;
/// Public constant `GRID_POINTS`.
pub const GRID_POINTS: usize = 50;

/// Public constant `GPCLIP`.
pub const GPCLIP: i16 = 15564; // Q14
/// Public constant `GPCLIP2`.
pub const GPCLIP2: i16 = 481; // Q9
/// Public constant `GP0999`.
pub const GP0999: i16 = 16383; // Q14
/// Public constant `L_THRESH_ERR`.
pub const L_THRESH_ERR: i32 = 983_040_000; // Q14

/// Public constant `FRAC1`.
pub const FRAC1: i16 = 19043;
/// Public constant `K0`.
pub const K0: i16 = 24576;
/// Public constant `G_MAX`.
pub const G_MAX: i16 = 5000;

/// Public constant `BIT_0`.
pub const BIT_0: i16 = 0x007f;
/// Public constant `BIT_1`.
pub const BIT_1: i16 = 0x0081;
