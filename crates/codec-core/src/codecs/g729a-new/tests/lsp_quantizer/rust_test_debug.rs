// Inline all the code we need for testing
use std::io::{self, Write};

type Word16 = i16;
type Word32 = i32;

const M: usize = 10;
const MA_NP: usize = 4;
const MODE: usize = 2;
const NC: usize = 5;
const NC0: usize = 128;
const NC1: usize = 32;
const NC0_B: i16 = 7;
const NC1_B: i16 = 5;
const GAP1: i16 = 39;
const GAP2: i16 = 20;
const GAP3: i16 = 321;
const PI04: i16 = 10294;
const PI92: i16 = 32111;
const L_LIMIT: i16 = 40;
const M_LIMIT: i16 = 25708;
const CONST10: i16 = 16384;
const CONST12: i16 = 24576;
const MAX_32: Word32 = 2147483647;

// Basic operators
fn add(var1: Word16, var2: Word16) -> Word16 {
    let l_sum = (var1 as i32) + (var2 as i32);
    if l_sum > 32767 {
        32767
    } else if l_sum < -32768 {
        -32768
    } else {
        l_sum as Word16
    }
}

fn sub(var1: Word16, var2: Word16) -> Word16 {
    let l_diff = (var1 as i32) - (var2 as i32);
    if l_diff > 32767 {
        32767
    } else if l_diff < -32768 {
        -32768
    } else {
        l_diff as Word16
    }
}

fn mult(var1: Word16, var2: Word16) -> Word16 {
    let l_product = (var1 as i32) * (var2 as i32);
    let l_product_hi = l_product >> 15;
    l_product_hi as Word16
}

fn l_mult(var1: Word16, var2: Word16) -> Word32 {
    (var1 as Word32) * (var2 as Word32)
}

fn l_mac(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_product = l_mult(var1, var2);
    let l_sum = l_var3.saturating_add(l_product);
    if l_sum != l_var3 + l_product {
        if l_product >= 0 {
            std::i32::MAX
        } else {
            std::i32::MIN
        }
    } else {
        l_sum
    }
}

fn l_msu(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_product = l_mult(var1, var2);
    let l_sum = l_var3.saturating_sub(l_product);
    if l_sum != l_var3 - l_product {
        if l_product >= 0 {
            std::i32::MIN
        } else {
            std::i32::MAX
        }
    } else {
        l_sum
    }
}

fn l_sub(l_var1: Word32, l_var2: Word32) -> Word32 {
    let l_diff = l_var1.saturating_sub(l_var2);
    if l_diff != l_var1 - l_var2 {
        if l_var2 > 0 {
            std::i32::MIN
        } else {
            std::i32::MAX
        }
    } else {
        l_diff
    }
}

fn extract_h(l_var1: Word32) -> Word16 {
    (l_var1 >> 16) as Word16
}

fn extract_l(l_var1: Word32) -> Word16 {
    l_var1 as Word16
}

fn round(l_var1: Word32) -> Word16 {
    let l_rounded = l_var1.saturating_add(0x00008000);
    extract_h(l_rounded)
}

fn l_shl(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 <= 0 {
        l_var1 >> (-var2)
    } else if var2 >= 31 {
        if l_var1 > 0 {
            std::i32::MAX
        } else {
            std::i32::MIN
        }
    } else {
        let result = l_var1.checked_shl(var2 as u32);
        result.unwrap_or(if l_var1 > 0 { std::i32::MAX } else { std::i32::MIN })
    }
}

fn l_shr(l_var1: Word32, var2: Word16) -> Word32 {
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
        l_var1 >> var2
    }
}

fn shl(var1: Word16, var2: Word16) -> Word16 {
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

fn shr(var1: Word16, var2: Word16) -> Word16 {
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

fn l_deposit_h(var1: Word16) -> Word32 {
    (var1 as Word32) << 16
}

fn l_deposit_l(var1: Word16) -> Word32 {
    var1 as Word32
}

fn norm_s(var1: Word16) -> Word16 {
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

// Include table data inline
const LSPCB1: [[Word16; M]; NC0] = [
    // ... (truncated for brevity - copy from tab_ld8a.rs)
    [ 1486,  2168,  3751,  9074, 12134, 13944, 17983, 19173, 21190, 21820],
    [ 1730,  2640,  3450,  4870,  6126,  7876, 15644, 17817, 20294, 21902],
    // ... include all 128 entries
    [ 1141,  1815,  2624,  4623,  6495,  9588, 13968, 16428, 19351, 21286],
    // ... rest of the entries
];

const LSPCB2: [[Word16; M]; NC1] = [
    [ -435,  -815,  -742,  1033,  -518,   582, -1201,   829,    86,   385],
    // ... include all 32 entries
];

const FG: [[[Word16; M]; MA_NP]; MODE] = [
    // ... copy from tab_ld8a.rs
    [[/* data */]],
    [[/* data */]],
];

const FG_SUM: [[Word16; M]; MODE] = [
    // ... copy from tab_ld8a.rs
    [/* data */],
    [/* data */],
];

const FG_SUM_INV: [[Word16; M]; MODE] = [
    [17210, 15888, 16357, 16183, 16516, 15833, 15888, 15421, 14840, 15597],
    [ 9202,  7320,  6788,  7738,  8170,  8154,  8856,  8818,  8366,  8544]
];

const TABLE2: [Word16; 64] = [
  32767,  32729,  32610,  32413,  32138,  31786,  31357,  30853,
  30274,  29622,  28899,  28106,  27246,  26320,  25330,  24279,
  23170,  22006,  20788,  19520,  18205,  16846,  15447,  14010,
  12540,  11039,   9512,   7962,   6393,   4808,   3212,   1608,
      0,  -1608,  -3212,  -4808,  -6393,  -7962,  -9512, -11039,
 -12540, -14010, -15447, -16846, -18205, -19520, -20788, -22006,
 -23170, -24279, -25330, -26320, -27246, -28106, -28899, -29622,
 -30274, -30853, -31357, -31786, -32138, -32413, -32610, -32729
];

const SLOPE_COS: [Word16; 64] = [
   -632,  -1893,  -3150,  -4399,  -5638,  -6863,  -8072,  -9261,
 -10428, -11570, -12684, -13767, -14817, -15832, -16808, -17744,
 -18637, -19486, -20287, -21039, -21741, -22390, -22986, -23526,
 -24009, -24435, -24801, -25108, -25354, -25540, -25664, -25726,
 -25726, -25664, -25540, -25354, -25108, -24801, -24435, -24009,
 -23526, -22986, -22390, -21741, -21039, -20287, -19486, -18637,
 -17744, -16808, -15832, -14817, -13767, -12684, -11570, -10428,
  -9261,  -8072,  -6863,  -5638,  -4399,  -3150,  -1893,   -632
];

const SLOPE_ACOS: [Word16; 64] = [
 -26887,  -8812,  -5323,  -3813,  -2979,  -2444,  -2081,  -1811,
  -1608,  -1450,  -1322,  -1219,  -1132,  -1059,   -998,   -946,
   -901,   -861,   -827,   -797,   -772,   -750,   -730,   -713,
   -699,   -687,   -677,   -668,   -662,   -657,   -654,   -652,
   -652,   -654,   -657,   -662,   -668,   -677,   -687,   -699,
   -713,   -730,   -750,   -772,   -797,   -827,   -861,   -901,
   -946,   -998,  -1059,  -1132,  -1219,  -1322,  -1450,  -1608,
  -1811,  -2081,  -2444,  -2979,  -3813,  -5323,  -8812, -26887
];

// LSP quantizer struct
struct LspQuantizer {
    freq_prev: [[Word16; M]; MA_NP],
}

impl LspQuantizer {
    fn new() -> Self {
        let mut freq_prev = [[0; M]; MA_NP];
        let freq_prev_reset = [
            2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396,
        ];
        for i in 0..MA_NP {
            freq_prev[i].copy_from_slice(&freq_prev_reset);
        }
        
        eprintln!("DEBUG: Initial freq_prev:");
        for i in 0..MA_NP {
            eprint!("  freq_prev[{}]: ", i);
            for j in 0..M {
                eprint!("{:6} ", freq_prev[i][j]);
            }
            eprintln!();
        }
        
        Self { freq_prev }
    }

    fn qua_lsp(&mut self, lsp: &[Word16], lsp_q: &mut [Word16], ana: &mut [Word16]) {
        let mut lsf = [0; M];
        let mut lsf_q = [0; M];

        eprintln!("\nDEBUG: qua_lsp called");
        eprintln!("  Input lsp: {:?}", lsp);
        
        lsp_to_lsf(lsp, &mut lsf);
        eprintln!("  After lsp_to_lsf, lsf: {:?}", lsf);
        
        lsp_qua_cs(&lsf, &mut lsf_q, ana, &mut self.freq_prev);
        eprintln!("  After lsp_qua_cs, lsf_q: {:?}", lsf_q);
        eprintln!("  ana[0]={}, ana[1]={}", ana[0], ana[1]);
        
        lsf_to_lsp(&lsf_q, lsp_q);
        eprintln!("  Final lsp_q: {:?}", lsp_q);
    }
}

// Rest of functions...
// (Include all the functions from lspvq.rs with debug statements)

fn main() {
    // Include test data inline
    let a = [4096, -4174, 1, 17, 12, 13, 13, 13, 11, 9, 103];
    let mut lsp = [0; 10];
    let mut lsp_q = [0; 10];
    let mut ana: [Word16; 2] = [0; 2];
    let old_lsp = [0; 10];

    println!("rust_function_name,rust_output");

    let mut quantizer = LspQuantizer::new();
    
    // Inline az_lsp function here
    // ... implementation ...
    
    quantizer.qua_lsp(&lsp, &mut lsp_q, &mut ana);

    for i in 0..10 {
        println!("lsp_q,{}", lsp_q[i]);
    }
    println!("ana0,{}", ana[0]);
    println!("ana1,{}", ana[1]);
} 