use crate::common::basic_operators::*;

const GAMMA1: Word16 = 30802; // 0.94 in Q15
const GAMMA2: Word16 = 19661; // 0.6 in Q15

pub fn perceptual_weighting(a: &[Word16], p: &mut [Word16], f: &mut [Word16]) {
    // Perceptual weighting filter coefficients
    for i in 0..=10 {
        let mut temp = GAMMA1;
        for j in 1..=i {
            let term1 = mult(p[j as usize], a[(i - j) as usize]);
            let term2 = mult(temp, a[j as usize]);
            p[i as usize] = add(term1, term2);
            temp = mult(temp, GAMMA1);
        }
    }

    // Perceptual weighting filter denominator coefficients
    for i in 0..=10 {
        let mut temp = GAMMA2;
        for j in 1..=i {
            let term1 = mult(f[j as usize], a[(i - j) as usize]);
            let term2 = mult(temp, a[j as usize]);
            f[i as usize] = add(term1, term2);
            temp = mult(temp, GAMMA2);
        }
    }
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

        let expected_p = [
            0, -1921, -218, 28, 2, -9, 2, 0, -1, -1, 0,
        ];
        let expected_f = [
            0, -1226, -141, 7, -1, -2, -1, 0, -1, -1, 0,
        ];
        assert_eq!(p, expected_p);
        assert_eq!(f, expected_f);
    }
}
