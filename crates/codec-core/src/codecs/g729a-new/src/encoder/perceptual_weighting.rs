use crate::common::basic_operators::*;

const GAMMA1: Word16 = 30802; // 0.94 in Q15
const GAMMA2: Word16 = 19661; // 0.6 in Q15
const M: usize = 10;

fn weight_az(a: &[Word16], gamma: Word16, ap: &mut [Word16]) {
    ap[0] = a[0];
    let mut fac = gamma;
    for i in 1..M {
        ap[i] = round(l_mult(a[i], fac));
        fac = round(l_mult(fac, gamma));
    }
    ap[M] = round(l_mult(a[M], fac));
}

pub fn perceptual_weighting(a: &[Word16], p: &mut [Word16], f: &mut [Word16]) {
    weight_az(&a[..=M], GAMMA1, p);
    weight_az(&a[..=M], GAMMA2, f);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perceptual_weighting() {
        let a = [4096, -2043, 10, 32, -1, -10, 5, 1, -1, 0, 0];
        let mut p = [0; 11];
        let mut f = [0; 11];

        perceptual_weighting(&a, &mut p, &mut f);

        let expected_p = [4096, -1920, 9, 28, -1, -8, 4, 1, -1, 0, 0];
        let expected_f = [4096, -1226, 4, 11, 0, -2, 1, 0, 0, 0, 0];

        assert_eq!(p, expected_p);
        assert_eq!(f, expected_f);
    }
}
