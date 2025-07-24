use crate::common::basic_operators::*;
use crate::common::tab_ld8a::*;

// static memory
static mut FREQ_PREV: [[Word16; M]; MA_NP] = [[0; M]; MA_NP];

pub fn lsp_encw_reset() {
    let freq_prev_reset = [
        2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396,
    ];
    for i in 0..MA_NP {
        unsafe {
            FREQ_PREV[i].copy_from_slice(&freq_prev_reset);
        }
    }
}

pub fn qua_lsp(
  lsp: &[Word16],       // (i) Q15 : Unquantized LSP
  lsp_q: &mut [Word16],     // (o) Q15 : Quantized LSP
  ana: &mut [Word16]        // (o)     : indexes
)
{
  let mut lsf = [0; M];
  let mut lsf_q = [0; M];  // domain 0.0<= lsf <PI in Q13

  // Convert LSPs to LSFs
  lsp_to_lsf(lsp, &mut lsf);

  unsafe {
    lsp_qua_cs(&lsf, &mut lsf_q, ana, &mut FREQ_PREV);
  }

  // Convert LSFs to LSPs
  lsf_to_lsp(&lsf_q, lsp_q);
}

fn lsp_qua_cs(
  flsp_in: &[Word16],    // (i) Q13 : Original LSP parameters
  lspq_out: &mut [Word16],   // (o) Q13 : Quantized LSP parameters
  code: &mut [Word16],         // (o)     : codes of the selected LSP
  freq_prev: &mut [[Word16; M]; MA_NP]
)
{
  let mut wegt = [0; M];       // Q11->normalized : weighting coefficients

  get_wegt( flsp_in, &mut wegt );

  relspwed( flsp_in, &wegt, lspq_out, &LSPCB1, &LSPCB2, &FG,
    freq_prev, &FG_SUM, &FG_SUM_INV, code);
}

fn relspwed(
  lsp: &[Word16],                 // (i) Q13 : unquantized LSP parameters
  wegt: &[Word16],                // (i) norm: weighting coefficients
  lspq: &mut [Word16],                // (o) Q13 : quantized LSP parameters
  lspcb1: &[[Word16; M]],           // (i) Q13 : first stage LSP codebook
  lspcb2: &[[Word16; M]],           // (i) Q13 : Second stage LSP codebook
  fg: &[[[Word16; M]; MA_NP]],    // (i) Q15 : MA prediction coefficients
  freq_prev: &mut [[Word16; M]],   // (i/o) Q13 : previous LSP vector
  fg_sum: &[[Word16; M]],       // (i) Q15 : present MA prediction coef.
  fg_sum_inv: &[[Word16; M]],   // (i) Q12 : inverse coef.
  code_ana: &mut [Word16]             // (o)     : codes of the selected LSP
)
{
  let mut cand = [0; MODE];
  let mut tindex1 = [0; MODE];
  let mut tindex2 = [0; MODE];
  let mut l_tdist = [0; MODE];         // Q26
  let mut rbuf = [0; M];               // Q13
  let mut buf = [0; M];                // Q13

  for mode in 0..MODE {
    let mut cand_cur = 0;
    lsp_prev_extract(lsp, &mut rbuf, &fg[mode], freq_prev, &fg_sum_inv[mode]);

    lsp_pre_select(&rbuf, lspcb1, &mut cand_cur );
    cand[mode] = cand_cur;

    let mut index = 0;
    lsp_select_1(&rbuf, &lspcb1[cand_cur as usize], wegt, lspcb2, &mut index);

    tindex1[mode] = index;

    for j in 0..NC {
      buf[j] = add( lspcb1[cand_cur as usize][j], lspcb2[index as usize][j] );
    }

    lsp_expand_1(&mut buf, GAP1);

    lsp_select_2(&rbuf, &lspcb1[cand_cur as usize], wegt, lspcb2, &mut index);

    tindex2[mode] = index;

    for j in NC..M {
      buf[j] = add( lspcb1[cand_cur as usize][j], lspcb2[index as usize][j] );
    }

    lsp_expand_2(&mut buf, GAP1);

    lsp_expand_1_2(&mut buf, GAP2);

    lsp_get_tdist(wegt, &buf, &mut l_tdist[mode], &rbuf, &fg_sum[mode]);
  }

  let mut mode_index = 0;
  lsp_last_select(&l_tdist, &mut mode_index);

  code_ana[0] = (mode_index << NC0_B) | cand[mode_index as usize];
  code_ana[1] = (tindex1[mode_index as usize] << NC1_B) | tindex2[mode_index as usize];

  lsp_get_quant(lspcb1, lspcb2, cand[mode_index as usize],
      tindex1[mode_index as usize], tindex2[mode_index as usize],
      &fg[mode_index as usize], freq_prev, lspq, &fg_sum[mode_index as usize]) ;
}

fn get_wegt(
  flsp: &[Word16],    // (i) Q13 : M LSP parameters
  wegt: &mut [Word16]     // (o) Q11->norm : M weighting coefficients
)
{
    let mut buf = [0; M]; // in Q13

    buf[0] = sub( flsp[1], PI04+8192 );           // 8192:1.0(Q13)

    for i in 1..M-1 {
        let tmp = sub( flsp[i+1], flsp[i-1] );
        buf[i] = sub( tmp, 8192 );
    }

    buf[M-1] = sub( PI92-8192, flsp[M-2] );

    for i in 0..M {
        if buf[i] > 0 {
            wegt[i] = 2048;                    // 2048:1.0(Q11)
        }
        else {
            let l_acc = l_mult( buf[i], buf[i] );           // L_acc in Q27
            let tmp = extract_h( l_shl( l_acc, 2 ) );       // tmp in Q13

            let l_acc = l_mult( tmp, CONST10 );             // L_acc in Q25
            let tmp = extract_h( l_shl( l_acc, 2 ) );       // tmp in Q11

            wegt[i] = add( tmp, 2048 );                 // wegt in Q11
        }
    }

    let mut l_acc = l_mult( wegt[4], CONST12 );             // L_acc in Q26
    wegt[4] = extract_h( l_shl( l_acc, 1 ) );       // wegt in Q11

    l_acc = l_mult( wegt[5], CONST12 );             // L_acc in Q26
    wegt[5] = extract_h( l_shl( l_acc, 1 ) );       // wegt in Q11

    let mut tmp = 0;
    for i in 0..M {
        if sub(wegt[i], tmp) > 0 {
            tmp = wegt[i];
        }
    }

    let sft = norm_s(tmp);
    for i in 0..M {
        wegt[i] = shl(wegt[i], sft);                  // wegt in Q(11+sft)
    }
}

fn lsp_to_lsf(lsp: &[Word16], lsf: &mut [Word16]) {
    let mut ind = 63;
    for i in (0..M).rev() {
        while sub(TABLE2[ind as usize], lsp[i]) < 0 {
            ind -= 1;
            if ind <= 0 {
                break;
            }
        }
        let offset = sub(lsp[i], TABLE2[ind as usize]);
        let l_tmp = l_mult(SLOPE_ACOS[ind as usize], offset);
        let freq = add(shl(ind, 9), extract_l(l_shr(l_tmp, 12)));
        lsf[i] = mult(freq, 25736);
    }
}

fn lsf_to_lsp(lsf: &[Word16], lsp: &mut [Word16]) {
    for i in 0..M {
        let freq = mult(lsf[i], 20861);
        let mut ind = shr(freq, 8);
        let offset = freq & 0x00ff;
        if sub(ind, 63) > 0 {
            ind = 63;
        }
        let l_tmp = l_mult(SLOPE_COS[ind as usize], offset);
        lsp[i] = add(TABLE2[ind as usize], extract_l(l_shr(l_tmp, 13)));
    }
}

fn lsp_prev_extract(
    lsp: &[Word16],
    lsp_ele: &mut [Word16],
    fg: &[[Word16; M]],
    freq_prev: &[[Word16; M]],
    fg_sum_inv: &[Word16],
) {
    for j in 0..M {
        let mut l_temp = l_deposit_h(lsp[j]);
        for k in 0..MA_NP {
            l_temp = l_msu(l_temp, freq_prev[k][j], fg[k][j]);
        }
        let temp = extract_h(l_temp);
        l_temp = l_mult(temp, fg_sum_inv[j]);
        lsp_ele[j] = extract_h(l_shl(l_temp, 3));
    }
}

fn lsp_pre_select(rbuf: &[Word16], lspcb1: &[[Word16; M]], cand: &mut Word16) {
    *cand = 0;
    let mut l_dmin = MAX_32;
    for i in 0..NC0 {
        let mut l_tmp = 0;
        for j in 0..M {
            let tmp = sub(rbuf[j], lspcb1[i][j]);
            l_tmp = l_mac(l_tmp, tmp, tmp);
        }
        if l_sub(l_tmp, l_dmin) < 0 {
            l_dmin = l_tmp;
            *cand = i as Word16;
        }
    }
}

fn lsp_select_1(
    rbuf: &[Word16],
    lspcb1: &[Word16],
    wegt: &[Word16],
    lspcb2: &[[Word16; M]],
    index: &mut Word16,
) {
    let mut buf = [0; M];
    for j in 0..NC {
        buf[j] = sub(rbuf[j], lspcb1[j]);
    }
    *index = 0;
    let mut l_dmin = MAX_32;
    for k1 in 0..NC1 {
        let mut l_dist = 0;
        for j in 0..NC {
            let tmp = sub(buf[j], lspcb2[k1][j]);
            let tmp2 = mult(wegt[j], tmp);
            l_dist = l_mac(l_dist, tmp2, tmp);
        }
        if l_sub(l_dist, l_dmin) < 0 {
            l_dmin = l_dist;
            *index = k1 as Word16;
        }
    }
}

fn lsp_select_2(
    rbuf: &[Word16],
    lspcb1: &[Word16],
    wegt: &[Word16],
    lspcb2: &[[Word16; M]],
    index: &mut Word16,
) {
    let mut buf = [0; M];
    for j in NC..M {
        buf[j] = sub(rbuf[j], lspcb1[j]);
    }
    *index = 0;
    let mut l_dmin = MAX_32;
    for k1 in 0..NC1 {
        let mut l_dist = 0;
        for j in NC..M {
            let tmp = sub(buf[j], lspcb2[k1][j]);
            let tmp2 = mult(wegt[j], tmp);
            l_dist = l_mac(l_dist, tmp2, tmp);
        }
        if l_sub(l_dist, l_dmin) < 0 {
            l_dmin = l_dist;
            *index = k1 as Word16;
        }
    }
}

fn lsp_expand_1(buf: &mut [Word16], gap: Word16) {
    for j in 1..NC {
        let diff = sub(buf[j - 1], buf[j]);
        let tmp = shr(add(diff, gap), 1);
        if tmp > 0 {
            buf[j - 1] = sub(buf[j - 1], tmp);
            buf[j] = add(buf[j], tmp);
        }
    }
}

fn lsp_expand_2(buf: &mut [Word16], gap: Word16) {
    for j in NC..M {
        let diff = sub(buf[j - 1], buf[j]);
        let tmp = shr(add(diff, gap), 1);
        if tmp > 0 {
            buf[j - 1] = sub(buf[j - 1], tmp);
            buf[j] = add(buf[j], tmp);
        }
    }
}

fn lsp_expand_1_2(buf: &mut [Word16], gap: Word16) {
    for j in 1..M {
        let diff = sub(buf[j - 1], buf[j]);
        let tmp = shr(add(diff, gap), 1);
        if tmp > 0 {
            buf[j - 1] = sub(buf[j - 1], tmp);
            buf[j] = add(buf[j], tmp);
        }
    }
}

fn lsp_get_tdist(
    wegt: &[Word16],
    buf: &[Word16],
    l_tdist: &mut Word32,
    rbuf: &[Word16],
    fg_sum: &[Word16],
) {
    *l_tdist = 0;
    for j in 0..M {
        let tmp = sub(buf[j], rbuf[j]);
        let tmp = mult(tmp, fg_sum[j]);
        let l_acc = l_mult(wegt[j], tmp);
        let tmp2 = extract_h(l_shl(l_acc, 4));
        *l_tdist = l_mac(*l_tdist, tmp2, tmp);
    }
}

fn lsp_last_select(l_tdist: &[Word32], mode_index: &mut Word16) {
    *mode_index = 0;
    if l_sub(l_tdist[1], l_tdist[0]) < 0 {
        *mode_index = 1;
    }
}

fn lsp_get_quant(
    lspcb1: &[[Word16; M]],
    lspcb2: &[[Word16; M]],
    code0: Word16,
    code1: Word16,
    code2: Word16,
    fg: &[[Word16; M]],
    freq_prev: &mut [[Word16; M]],
    lspq: &mut [Word16],
    fg_sum: &[Word16],
) {
    let mut buf = [0; M];
    for j in 0..NC {
        buf[j] = add(lspcb1[code0 as usize][j], lspcb2[code1 as usize][j]);
    }
    for j in NC..M {
        buf[j] = add(lspcb1[code0 as usize][j], lspcb2[code2 as usize][j]);
    }

    lsp_expand_1_2(&mut buf, GAP1);
    lsp_expand_1_2(&mut buf, GAP2);

    lsp_prev_compose(&buf, lspq, fg, freq_prev, fg_sum);

    lsp_prev_update(&buf, freq_prev);

    lsp_stability(lspq);
}

fn lsp_prev_compose(
    lsp_ele: &[Word16],
    lsp: &mut [Word16],
    fg: &[[Word16; M]],
    freq_prev: &[[Word16; M]],
    fg_sum: &[Word16],
) {
    for j in 0..M {
        let mut l_acc = l_mult(lsp_ele[j], fg_sum[j]);
        for k in 0..MA_NP {
            l_acc = l_mac(l_acc, freq_prev[k][j], fg[k][j]);
        }
        lsp[j] = extract_h(l_acc);
    }
}

fn lsp_prev_update(lsp_ele: &[Word16], freq_prev: &mut [[Word16; M]]) {
    for k in (1..MA_NP).rev() {
        let (src, dest) = freq_prev.split_at_mut(k);
        dest[0].copy_from_slice(&src[k - 1]);
    }
    freq_prev[0].copy_from_slice(lsp_ele);
}

fn lsp_stability(buf: &mut [Word16]) {
    for j in 0..M - 1 {
        let l_acc = l_deposit_l(buf[j + 1]);
        let l_accb = l_deposit_l(buf[j]);
        let l_diff = l_sub(l_acc, l_accb);

        if l_diff < 0 {
            let tmp = buf[j + 1];
            buf[j + 1] = buf[j];
            buf[j] = tmp;
        }
    }

    if sub(buf[0], L_LIMIT) < 0 {
        buf[0] = L_LIMIT;
    }
    for j in 0..M - 1 {
        let l_acc = l_deposit_l(buf[j + 1]);
        let l_accb = l_deposit_l(buf[j]);
        let l_diff = l_sub(l_acc, l_accb);

        if l_sub(l_diff, GAP3 as Word32) < 0 {
            buf[j + 1] = add(buf[j], GAP3);
        }
    }

    if sub(buf[M - 1], M_LIMIT) > 0 {
        buf[M - 1] = M_LIMIT;
    }
}
