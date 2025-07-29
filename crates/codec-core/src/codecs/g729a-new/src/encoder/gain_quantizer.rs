use crate::common::basic_operators::*;
use crate::common::oper_32b::*;

// Constants from G.729A spec
const NCODE1: usize = 8;
const NCODE2: usize = 16;
const NCAN1: usize = 4;
const NCAN2: usize = 8;

const GPCLIP2: i16 = 15564; // Q14, corresponds to 0.95
const GP0999: i16 = 16383;  // Q14, corresponds to 0.999
const INV_COEF: i16 = -17103; // Q15, 1/(-0.5217)
const MAX_32: i32 = 0x7FFFFFFF; // Maximum 32-bit signed value

// MA prediction coefficients {0.68, 0.58, 0.34, 0.19} in Q13
const PRED: [i16; 4] = [5571, 4751, 2785, 1556];

// Gain codebooks (from TAB_LD8A.C)
const GBK1: [[i16; 2]; NCODE1] = [
    [1, 1516],      // Q14, Q13
    [1551, 2425],
    [1831, 5022],
    [57, 5404],
    [1921, 9291],
    [3242, 9949],
    [356, 14756],
    [2678, 27162],
];

const GBK2: [[i16; 2]; NCODE2] = [
    [826, 2005],    // Q14, Q13
    [1994, 0],
    [5142, 592],
    [6160, 2395],
    [8091, 4861],
    [9120, 525],
    [10573, 2966],
    [11569, 1196],
    [13260, 3256],
    [14194, 1630],
    [15132, 4914],
    [15161, 14276],
    [15434, 237],
    [16112, 3392],
    [17299, 1861],
    [18973, 5935],
];

// Map tables for index calculation
const MAP1: [i16; NCODE1] = [5, 1, 4, 7, 3, 0, 6, 2];
const MAP2: [i16; NCODE2] = [4, 6, 0, 2, 12, 14, 8, 10, 15, 11, 9, 13, 7, 3, 1, 5];

// Preselection thresholds
const THR1: [i16; NCODE1 - NCAN1] = [10808, 12374, 19778, 32567]; // Q14
const THR2: [i16; NCODE2 - NCAN2] = [14087, 16188, 20274, 21321, 23525, 25232, 27873, 30542]; // Q15

// Coefficients for preselection
const COEF: [[i16; 2]; 2] = [[31881, 26416], [31548, 27816]]; // Q10, Q14, Q16, Q19
const L_COEF: [[i32; 2]; 2] = [[2089405952, 1731217536], [2067549984, 1822990272]]; // Q26, Q30, Q32, Q35

pub struct GainQuantizer {
    past_qua_en: [i16; 4], // Past quantized energies Q10
}

impl GainQuantizer {
    pub fn new() -> Self {
        Self {
            past_qua_en: [-14336; 4], // -14.0 in Q10
        }
    }

    /// Gain prediction - predicts the fixed-codebook gain
    fn gain_predict(&self, code: &[i16], l_subfr: i16) -> (i16, i16) {
        let mut l_tmp = 0i32;
        
        // Energy coming from code
        for i in 0..(l_subfr as usize) {
            l_tmp = l_mac(l_tmp, code[i], code[i]);
        }

        // Compute: means_ener - 10log10(ener_code/L_subfr)
        let (exp, frac) = log2(l_tmp);
        l_tmp = mpy_32_16(exp, frac, -24660); // -3.0103 in Q13
        l_tmp = l_mac(l_tmp, 32588, 32); // 127.298 in Q14

        // Compute gcode0
        l_tmp = l_shl(l_tmp, 10); // From Q14 to Q24
        for i in 0..4 {
            l_tmp = l_mac(l_tmp, PRED[i], self.past_qua_en[i]); // Q13*Q10 -> Q24
        }

        let gcode0 = extract_h(l_tmp); // From Q24 to Q8

        // gcode0 = pow(10.0, gcode0/20) = pow(2, 0.166*gcode0)
        l_tmp = l_mult(gcode0, 5439); // *0.166 in Q15, result in Q24
        l_tmp = l_shr(l_tmp, 8); // From Q24 to Q16
        let (exp, frac) = l_extract(l_tmp);

        let gcode0 = extract_l(pow2(14, frac)); // Put 14 as exponent
        let exp_gcode0 = sub(14, exp);

        (gcode0, exp_gcode0)
    }

    /// Preselection for gain codebook
    fn gbk_presel(&self, best_gain: &[i16; 2], gcode0: i16) -> (i16, i16) {
        // x = (best_gain[1]-(coef[0][0]*best_gain[0]+coef[1][1])*gcode0) * inv_coef;
        let l_cfbg = l_mult(COEF[0][0], best_gain[0]); // Q20
        let mut l_acc = l_shr(L_COEF[1][1], 15); // Q20
        l_acc = l_add(l_cfbg, l_acc);
        let acc_h = extract_h(l_acc); // Q4
        let l_preg = l_mult(acc_h, gcode0); // Q9
        l_acc = l_shl(l_deposit_l(best_gain[1]), 7); // Q9
        l_acc = l_sub(l_acc, l_preg);
        let acc_h = extract_h(l_shl(l_acc, 2)); // Q[-5]
        let l_tmp_x = l_mult(acc_h, INV_COEF); // Q15

        // y = (coef[1][0]*(-coef[0][1]+best_gain[0]*coef[0][0])*gcode0
        //                                    -coef[0][0]*best_gain[1]) * inv_coef;
        l_acc = l_shr(L_COEF[0][1], 10); // Q20
        l_acc = l_sub(l_cfbg, l_acc);
        let mut acc_h = extract_h(l_acc); // Q4
        acc_h = mult(acc_h, gcode0); // Q[-7]
        let l_tmp = l_mult(acc_h, COEF[1][0]); // Q10

        let l_preg = l_mult(COEF[0][0], best_gain[1]); // Q13
        l_acc = l_sub(l_tmp, l_shr(l_preg, 3)); // Q10

        let acc_h = extract_h(l_shl(l_acc, 2)); // Q[-4]
        let l_tmp_y = l_mult(acc_h, INV_COEF); // Q16

        let sft_y = (14 + 4 + 1) - 16; // (Q[thr1]+Q[gcode0]+1)-Q[l_tmp_y]
        let sft_x = (15 + 4 + 1) - 15; // (Q[thr2]+Q[gcode0]+1)-Q[l_tmp_x]

        let mut cand1 = 0i16;
        let mut cand2 = 0i16;

        if gcode0 > 0 {
            // Pre-select codebook #1
            while cand1 < (NCODE1 - NCAN1) as i16 {
                let l_temp = l_sub(l_tmp_y, l_shr(l_mult(THR1[cand1 as usize], gcode0), sft_y));
                if l_temp > 0 {
                    cand1 = add(cand1, 1);
                } else {
                    break;
                }
            }

            // Pre-select codebook #2
            while cand2 < (NCODE2 - NCAN2) as i16 {
                let l_temp = l_sub(l_tmp_x, l_shr(l_mult(THR2[cand2 as usize], gcode0), sft_x));
                if l_temp > 0 {
                    cand2 = add(cand2, 1);
                } else {
                    break;
                }
            }
        } else {
            // Pre-select codebook #1 (gcode0 <= 0)
            while cand1 < (NCODE1 - NCAN1) as i16 {
                let l_temp = l_sub(l_tmp_y, l_shr(l_mult(THR1[cand1 as usize], gcode0), sft_y));
                if l_temp < 0 {
                    cand1 = add(cand1, 1);
                } else {
                    break;
                }
            }

            // Pre-select codebook #2 (gcode0 <= 0)
            while cand2 < (NCODE2 - NCAN2) as i16 {
                let l_temp = l_sub(l_tmp_x, l_shr(l_mult(THR2[cand2 as usize], gcode0), sft_x));
                if l_temp < 0 {
                    cand2 = add(cand2, 1);
                } else {
                    break;
                }
            }
        }

        (cand1, cand2)
    }

    /// Update past quantized energies
    fn gain_update(&mut self, l_gbk12: i32) {
        // Shift past energies
        for i in (1..4).rev() {
            self.past_qua_en[i] = self.past_qua_en[i - 1];
        }

        // C reference implementation:
        // Log2( L_gbk12, &exp, &frac );               /* L_gbk12:Q13       */
        // L_acc = L_Comp( sub(exp,13), frac);         /* L_acc:Q16           */
        // tmp = extract_h( L_shl( L_acc,13 ) );       /* tmp:Q13           */
        // past_qua_en[0] = mult( tmp, 24660 );        /* past_qua_en[]:Q10 */
        
        let (exp, frac) = log2(l_gbk12);              // L_gbk12:Q13
        let l_acc = l_comp(sub(exp, 13), frac);       // L_acc:Q16
        let tmp = extract_h(l_shl(l_acc, 13));        // tmp:Q13
        self.past_qua_en[0] = mult(tmp, 24660);       // past_qua_en[]:Q10
    }

    /// Main gain quantization function
    pub fn quantize_gain(
        &mut self,
        code: &[i16],           // Innovative vector Q13
        g_coeff: &[i16; 5],     // Correlations
        exp_coeff: &[i16; 5],   // Q-Format of g_coeff
        l_subfr: i16,           // Subframe length
        tameflag: i16,          // Taming flag
    ) -> (i16, i16, i16) {
        // (index, gain_pit Q14, gain_cod Q1)

        // Gain prediction
        let (gcode0, exp_gcode0) = self.gain_predict(code, l_subfr);

        // Calculate best gain (unquantized optimal gains)
        // tmp = -1./(4.*coeff[0]*coeff[2]-coeff[4]*coeff[4])
        let l_tmp1 = l_mult(g_coeff[0], g_coeff[2]);
        let exp1 = add(add(exp_coeff[0], exp_coeff[2]), 1 - 2);
        let l_tmp2 = l_mult(g_coeff[4], g_coeff[4]);
        let exp2 = add(add(exp_coeff[4], exp_coeff[4]), 1);

        let (l_tmp, exp) = if sub(exp1, exp2) > 0 {
            (l_sub(l_shr(l_tmp1, sub(exp1, exp2)), l_tmp2), exp2)
        } else {
            (l_sub(l_tmp1, l_shr(l_tmp2, sub(exp2, exp1))), exp1)
        };

        let sft = norm_l(l_tmp);
        let denom = extract_h(l_shl(l_tmp, sft));
        let exp_denom = sub(add(exp, sft), 16);

        let inv_denom = negate(div_s(16384, denom));
        let exp_inv_denom = sub(14 + 15, exp_denom);

        // best_gain[0] = (2.*coeff[2]*coeff[1]-coeff[3]*coeff[4])*tmp
        let l_tmp1 = l_mult(g_coeff[2], g_coeff[1]);
        let exp1 = add(exp_coeff[2], exp_coeff[1]);
        let l_tmp2 = l_mult(g_coeff[3], g_coeff[4]);
        let exp2 = add(add(exp_coeff[3], exp_coeff[4]), 1);

        let (l_tmp, exp) = if sub(exp1, exp2) > 0 {
            (l_sub(l_shr(l_tmp1, add(sub(exp1, exp2), 1)), l_shr(l_tmp2, 1)), sub(exp2, 1))
        } else {
            (l_sub(l_shr(l_tmp1, 1), l_shr(l_tmp2, add(sub(exp2, exp1), 1))), sub(exp1, 1))
        };

        let sft = norm_l(l_tmp);
        let nume = extract_h(l_shl(l_tmp, sft));
        let exp_nume = sub(add(exp, sft), 16);

        let sft = sub(add(exp_nume, exp_inv_denom), (9 + 16 - 1));
        let l_acc = l_shr(l_mult(nume, inv_denom), sft);
        let mut best_gain_0 = extract_h(l_acc); // Q9

        if tameflag == 1 && sub(best_gain_0, GPCLIP2) > 0 {
            best_gain_0 = GPCLIP2;
        }

        // best_gain[1] = (2.*coeff[0]*coeff[3]-coeff[1]*coeff[4])*tmp
        let l_tmp1 = l_mult(g_coeff[0], g_coeff[3]);
        let exp1 = add(exp_coeff[0], exp_coeff[3]);
        let l_tmp2 = l_mult(g_coeff[1], g_coeff[4]);
        let exp2 = add(add(exp_coeff[1], exp_coeff[4]), 1);

        let (l_tmp, exp) = if sub(exp1, exp2) > 0 {
            (l_sub(l_shr(l_tmp1, add(sub(exp1, exp2), 1)), l_shr(l_tmp2, 1)), sub(exp2, 1))
        } else {
            (l_sub(l_shr(l_tmp1, 1), l_shr(l_tmp2, add(sub(exp2, exp1), 1))), sub(exp1, 1))
        };

        let sft = norm_l(l_tmp);
        let nume = extract_h(l_shl(l_tmp, sft));
        let exp_nume = sub(add(exp, sft), 16);

        let sft = sub(add(exp_nume, exp_inv_denom), (2 + 16 - 1));
        let l_acc = l_shr(l_mult(nume, inv_denom), sft);
        let best_gain_1 = extract_h(l_acc); // Q2

        let best_gain = [best_gain_0, best_gain_1];

        // Change Q-format of gcode0 (Q[exp_gcode0] -> Q4)
        let gcode0_org = if sub(exp_gcode0, 4) >= 0 {
            shr(gcode0, sub(exp_gcode0, 4))
        } else {
            let l_acc = l_deposit_l(gcode0);
            let l_acc = l_shl(l_acc, sub((4 + 16), exp_gcode0));
            extract_h(l_acc) // Q4
        };

        // Preselection for gain codebook
        let (cand1, cand2) = self.gbk_presel(&best_gain, gcode0_org);

        // Prepare coefficients for codebook search
        let exp_min = [
            add(exp_coeff[0], 13),
            add(exp_coeff[1], 14),
            add(exp_coeff[2], sub(shl(exp_gcode0, 1), 21)),
            add(exp_coeff[3], sub(exp_gcode0, 3)),
            add(exp_coeff[4], sub(exp_gcode0, 4)),
        ];

        let mut e_min = exp_min[0];
        for i in 1..5 {
            if sub(exp_min[i], e_min) < 0 {
                e_min = exp_min[i];
            }
        }

        // Align coeff[] and save in special 32-bit double precision
        let mut coeff = [0i16; 5];
        let mut coeff_lsf = [0i16; 5];
        for i in 0..5 {
            let j = sub(exp_min[i], e_min);
            let l_tmp = l_deposit_h(g_coeff[i]);
            let l_tmp = l_shr(l_tmp, j);
            let (hi, lo) = l_extract(l_tmp);
            coeff[i] = hi;
            coeff_lsf[i] = lo;
        }

        // Codebook search
        let mut l_dist_min = MAX_32;
        let mut index1 = cand1;
        let mut index2 = cand2;

        if tameflag == 1 {
            for i in 0..NCAN1 {
                for j in 0..NCAN2 {
                    let g_pitch = add(GBK1[cand1 as usize + i][0], GBK2[cand2 as usize + j][0]); // Q14
                    if g_pitch < GP0999 {
                        let l_acc = l_deposit_l(GBK1[cand1 as usize + i][1]);
                        let l_accb = l_deposit_l(GBK2[cand2 as usize + j][1]); // Q13
                        let l_tmp = l_add(l_acc, l_accb);
                        let tmp = extract_l(l_shr(l_tmp, 1)); // Q12

                        let g_code = mult(gcode0, tmp); // Q[exp_gcode0+12-15]
                        let g2_pitch = mult(g_pitch, g_pitch); // Q13
                        let g2_code = mult(g_code, g_code); // Q[2*exp_gcode0-6-15]
                        let g_pit_cod = mult(g_code, g_pitch); // Q[exp_gcode0-3+14-15]

                        let mut l_tmp = mpy_32_16(coeff[0], coeff_lsf[0], g2_pitch);
                        l_tmp = l_add(l_tmp, mpy_32_16(coeff[1], coeff_lsf[1], g_pitch));
                        l_tmp = l_add(l_tmp, mpy_32_16(coeff[2], coeff_lsf[2], g2_code));
                        l_tmp = l_add(l_tmp, mpy_32_16(coeff[3], coeff_lsf[3], g_code));
                        l_tmp = l_add(l_tmp, mpy_32_16(coeff[4], coeff_lsf[4], g_pit_cod));

                        let l_temp = l_sub(l_tmp, l_dist_min);

                        if l_temp < 0 {
                            l_dist_min = l_tmp;
                            index1 = add(cand1, i as i16);
                            index2 = add(cand2, j as i16);
                        }
                    }
                }
            }
        } else {
            for i in 0..NCAN1 {
                for j in 0..NCAN2 {
                    let g_pitch = add(GBK1[cand1 as usize + i][0], GBK2[cand2 as usize + j][0]); // Q14
                    let l_acc = l_deposit_l(GBK1[cand1 as usize + i][1]);
                    let l_accb = l_deposit_l(GBK2[cand2 as usize + j][1]); // Q13
                    let l_tmp = l_add(l_acc, l_accb);
                    let tmp = extract_l(l_shr(l_tmp, 1)); // Q12

                    let g_code = mult(gcode0, tmp); // Q[exp_gcode0+12-15]
                    let g2_pitch = mult(g_pitch, g_pitch); // Q13
                    let g2_code = mult(g_code, g_code); // Q[2*exp_gcode0-6-15]
                    let g_pit_cod = mult(g_code, g_pitch); // Q[exp_gcode0-3+14-15]

                    let mut l_tmp = mpy_32_16(coeff[0], coeff_lsf[0], g2_pitch);
                    l_tmp = l_add(l_tmp, mpy_32_16(coeff[1], coeff_lsf[1], g_pitch));
                    l_tmp = l_add(l_tmp, mpy_32_16(coeff[2], coeff_lsf[2], g2_code));
                    l_tmp = l_add(l_tmp, mpy_32_16(coeff[3], coeff_lsf[3], g_code));
                    l_tmp = l_add(l_tmp, mpy_32_16(coeff[4], coeff_lsf[4], g_pit_cod));

                    let l_temp = l_sub(l_tmp, l_dist_min);

                    if l_temp < 0 {
                        l_dist_min = l_tmp;
                        index1 = add(cand1, i as i16);
                        index2 = add(cand2, j as i16);
                    }
                }
            }
        }

        // Read the quantized gains
        let gain_pit = add(GBK1[index1 as usize][0], GBK2[index2 as usize][0]); // Q14

        // gain_code = (gbk1[index1][1]+gbk2[index2][1]) * gcode0
        let l_acc = l_deposit_l(GBK1[index1 as usize][1]);
        let l_accb = l_deposit_l(GBK2[index2 as usize][1]);
        let l_gbk12 = l_add(l_acc, l_accb); // Q13
        let tmp = extract_l(l_shr(l_gbk12, 1)); // Q12
        let l_acc = l_mult(tmp, gcode0); // Q[exp_gcode0+12+1]

        let l_acc = l_shl(l_acc, add(negate(exp_gcode0), (-12 - 1 + 1 + 16)));
        let gain_cod = extract_h(l_acc); // Q1

        // Update past quantized energies
        self.gain_update(l_gbk12);

        // Return index and quantized gains
        let index = add(mult(MAP1[index1 as usize], NCODE2 as i16), MAP2[index2 as usize]);
        (index, gain_pit, gain_cod)
    }
}
