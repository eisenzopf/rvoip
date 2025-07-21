use crate::common::basic_operators::*;

const NC: i32 = 5;
const N_COEF: i32 = 11;

fn get_lsp_pol(a: &[Word16], f: &mut [Word16]) {
    let mut t0: Word32;

    f[0] = round(L_shl(4096, 11));

    for i in 1..=NC {
        t0 = L_mult(a[i as usize], 4096);
        t0 = L_mac(t0, a[(N_COEF - i) as usize], 4096);
        f[i as usize] = round(L_shr(t0, 13));
    }
}

fn chebyshev(x: Word16, f: &[Word16]) -> Word16 {
    let mut b1: Word16 = 0;
    let mut b2: Word16 = 0;
    let mut b0: Word16;
    let mut t0: Word32;

    t0 = L_mac(L_mult(x, 512), f[1], 256);
    b2 = round(L_shr(t0, 9));

    for i in 2..NC {
        t0 = L_mult(x, b2);
        t0 = L_mac(t0, f[i as usize], 256);
        t0 = L_mac(t0, b1, -32768);
        b0 = round(L_shr(t0, 9));
        b1 = b2;
        b2 = b0;
    }

    t0 = L_mult(x, b2);
    t0 = L_mac(t0, f[NC as usize], 256);
    t0 = L_mac(t0, b1, -32768);
    round(L_shr(t0, 9))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp() {
        let a = [4096, -2043, 10, 32, -1, -10, 5, 1, -1, 0, 0];
        let mut f = [0; 6];
        get_lsp_pol(&a, &mut f);
        let x = chebyshev(16384, &f);
        assert_eq!(x, -16384);
    }
}
