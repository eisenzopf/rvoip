use crate::common::basic_operators::*;

const M: usize = 10;

pub fn syn_filt(a: &[Word16], x: &[Word16], y: &mut [Word16], lg: i32, mem: &mut [Word16], update: bool) {
    let mut yy = [0; 100];
    for i in 0..M {
        yy[i] = mem[i];
    }

    for i in 0..lg as usize {
        let mut s = l_mult(x[i], a[0]);
        for j in 1..=M {
            s = l_msu(s, a[j], yy[i + M - j]);
        }
        s = l_shl(s, 3);
        yy[i + M] = round(s);
    }

    for i in 0..lg as usize {
        y[i] = yy[i + M];
    }

    if update {
        for i in 0..M {
            mem[i] = y[lg as usize - M + i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syn_filt() {
        let a = [4096, -2043, 10, 32, -1, -10, 5, 1, -1, 0, 0];
        let x = [8192; 40];
        let mut y = [0; 40];
        let mut mem = [0; 10];

        syn_filt(&a, &x, &mut y, 40, &mut mem, true);

        let expected_y = [
            8192, 12278, 14296, 15229, 15659, 15877, 15977, 16022, 16043, 16053, 16058, 16060, 16061, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062, 16062,
        ];
        assert_eq!(y, expected_y);
    }
}
