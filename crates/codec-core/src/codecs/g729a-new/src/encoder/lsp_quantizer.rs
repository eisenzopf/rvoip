use crate::common::basic_operators::*;
use crate::common::oper_32b::*;

const NC: usize = 5;
const GRID_POINTS: usize = 60;
const GRID: [Word16; 51] = [
    32760, 32703, 32509, 32187, 31738, 31164, 30466, 29649, 28714, 27666, 26509, 25248,
    23886, 22431, 20887, 19260, 17557, 15786, 13951, 12062, 10125, 8149, 6140, 4106,
    2057, 0, -2057, -4106, -6140, -8149, -10125, -12062, -13951, -15786, -17557,
    -19260, -20887, -22431, -23886, -25248, -26509, -27666, -28714, -29649, -30466,
    -31164, -31738, -32187, -32509, -32703, -32760,
];

fn chebyshev(x: Word16, f: &[Word16], n: usize) -> Word16 {
    let mut b0_h: Word16;
    let mut b0_l: Word16;
    let mut b1_h: Word16;
    let mut b1_l: Word16;
    let mut b2_h: Word16;
    let mut b2_l: Word16;
    let mut t0: Word32;

    b2_h = 256;
    b2_l = 0;

    t0 = l_mult(x, 512);
    t0 = l_mac(t0, f[1], 4096);
    (b1_h, b1_l) = l_extract(t0);

    for i in 2..n {
        t0 = mpy_32_16(b1_h, b1_l, x);
        t0 = l_shl(t0, 1);
        t0 = l_mac(t0, b2_h, -32768i32 as Word16);
        t0 = l_msu(t0, b2_l, 1);
        t0 = l_mac(t0, f[i], 4096);
        (b0_h, b0_l) = l_extract(t0);

        b2_l = b1_l;
        b2_h = b1_h;
        b1_l = b0_l;
        b1_h = b0_h;
    }

    t0 = mpy_32_16(b1_h, b1_l, x);
    t0 = l_mac(t0, b2_h, -32768i32 as Word16);
    t0 = l_msu(t0, b2_l, 1);
    t0 = l_mac(t0, f[n], 2048);

    t0 = l_shl(t0, 6);
    extract_h(t0)
}

pub fn az_lsp(a: &[Word16], lsp: &mut [Word16], old_lsp: &[Word16]) {
    let mut f1 = [0; NC + 1];
    let mut f2 = [0; NC + 1];

    f1[0] = 2048;
    f2[0] = 2048;

    for i in 0..NC {
        let t0 = l_mult(a[i + 1], 16384);
        let t0 = l_mac(t0, a[10 - i], 16384);
        let x = extract_h(t0);
        f1[i + 1] = sub(x, f1[i]);

        let t0 = l_mult(a[i + 1], 16384);
        let t0 = l_msu(t0, a[10 - i], 16384);
        let x = extract_h(t0);
        f2[i + 1] = add(x, f2[i]);
    }

    let mut nf = 0;
    let mut ip = 0;
    let mut xlow = GRID[0];
    let mut ylow = chebyshev(xlow, &f1, NC);
    let mut j = 0;

    while nf < 10 && j < GRID_POINTS {
        j += 1;
        let mut xhigh = xlow;
        let mut yhigh = ylow;
        xlow = GRID[j];

        ylow = if ip == 0 {
            chebyshev(xlow, &f1, NC)
        } else {
            chebyshev(xlow, &f2, NC)
        };

        if l_mult(ylow, yhigh) <= 0 {
            for _ in 0..2 {
                let xmid = add(shr(xlow, 1), shr(xhigh, 1));
                let ymid = if ip == 0 {
                    chebyshev(xmid, &f1, NC)
                } else {
                    chebyshev(xmid, &f2, NC)
                };
                if l_mult(ylow, ymid) <= 0 {
                    yhigh = ymid;
                    xhigh = xmid;
                } else {
                    ylow = ymid;
                    xlow = xmid;
                }
            }

            let x = sub(xhigh, xlow);
            let y = sub(yhigh, ylow);
            let xint = if y == 0 {
                xlow
            } else {
                let sign = y;
                let y = abs_s(y);
                let exp = norm_s(y);
                let y = shl(y, exp as Word16);
                let y = div_s(16383, y);
                let mut t0 = l_mult(x, y);
                t0 = l_shr(t0, sub(20, exp as Word16));
                let y = extract_l(t0);
                let y = if sign < 0 { negate(y) } else { y };
                let t0 = l_mult(ylow, y);
                let t0 = l_shr(t0, 11);
                sub(xlow, extract_l(t0))
            };
            lsp[nf] = xint;
            xlow = xint;
            nf += 1;
            ip = 1 - ip;

            ylow = if ip == 0 {
                chebyshev(xlow, &f1, NC)
            } else {
                chebyshev(xlow, &f2, NC)
            };
        }
    }

    if nf < 10 {
        for i in 0..10 {
            lsp[i] = old_lsp[i];
        }
    }
}

