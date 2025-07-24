use crate::common::basic_operators::*;
use crate::common::oper_32b::*;

const HAMWINDOW: [Word16; 240] = [
    2621, 2623, 2629, 2638, 2651, 2668, 2689, 2713, 2741, 2772, 2808, 2847, 2890, 2936,
    2986, 3040, 3097, 3158, 3223, 3291, 3363, 3438, 3517, 3599, 3685, 3774, 3867, 3963,
    4063, 4166, 4272, 4382, 4495, 4611, 4731, 4853, 4979, 5108, 5240, 5376, 5514, 5655,
    5800, 5947, 6097, 6250, 6406, 6565, 6726, 6890, 7057, 7227, 7399, 7573, 7750, 7930,
    8112, 8296, 8483, 8672, 8863, 9057, 9252, 9450, 9650, 9852, 10055, 10261, 10468,
    10677, 10888, 11101, 11315, 11531, 11748, 11967, 12187, 12409, 12632, 12856, 13082,
    13308, 13536, 13764, 13994, 14225, 14456, 14688, 14921, 15155, 15389, 15624, 15859,
    16095, 16331, 16568, 16805, 17042, 17279, 17516, 17754, 17991, 18228, 18465, 18702,
    18939, 19175, 19411, 19647, 19882, 20117, 20350, 20584, 20816, 21048, 21279, 21509,
    21738, 21967, 22194, 22420, 22644, 22868, 23090, 23311, 23531, 23749, 23965, 24181,
    24394, 24606, 24816, 25024, 25231, 25435, 25638, 25839, 26037, 26234, 26428, 26621,
    26811, 26999, 27184, 27368, 27548, 27727, 27903, 28076, 28247, 28415, 28581, 28743,
    28903, 29061, 29215, 29367, 29515, 29661, 29804, 29944, 30081, 30214, 30345, 30472,
    30597, 30718, 30836, 30950, 31062, 31170, 31274, 31376, 31474, 31568, 31659, 31747,
    31831, 31911, 31988, 32062, 32132, 32198, 32261, 32320, 32376, 32428, 32476, 32521,
    32561, 32599, 32632, 32662, 32688, 32711, 32729, 32744, 32755, 32763, 32767, 32767,
    32741, 32665, 32537, 32359, 32129, 31850, 31521, 31143, 30716, 30242, 29720, 29151,
    28538, 27879, 27177, 26433, 25647, 24821, 23957, 23055, 22117, 21145, 20139, 19102,
    18036, 16941, 15820, 14674, 13505, 12315, 11106, 9879, 8637, 7381, 6114, 4838, 3554,
    2264, 971,
];

const LAG_H: [Word16; 10] = [
    32728, 32619, 32438, 32187, 31867, 31480, 31029, 30517, 29946, 29321,
];
const LAG_L: [Word16; 10] = [
    11904, 17280, 30720, 25856, 24192, 28992, 24384, 7360, 19520, 14784,
];

pub struct Lpc {
    old_a: [Word16; 11],
    old_rc: [Word16; 2],
}

impl Lpc {
    pub fn new() -> Self {
        Lpc {
            old_a: [4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            old_rc: [0, 0],
        }
    }

    pub fn autocorrelation(&self, x: &[Word16], m: i16, r_h: &mut [Word16], r_l: &mut [Word16]) {
        let mut y = [0i16; 240];
        let mut sum: i32;

        // Windowing of signal
        for i in 0..240 {
            y[i] = mult_r(x[i], HAMWINDOW[i]);
        }

        // Compute r[0] and test for overflow
        loop {
            unsafe {
                crate::common::basic_operators::OVERFLOW = false;
            }
            sum = 1; // Avoid case of all zeros
            for i in 0..240 {
                sum = l_mac(sum, y[i], y[i]);
                if unsafe { crate::common::basic_operators::OVERFLOW } {
                    break;
                }
            }

            // If overflow divide y[] by 4
            if unsafe { crate::common::basic_operators::OVERFLOW } {
                for i in 0..240 {
                    y[i] = shr(y[i], 2);
                }
            } else {
                break;
            }
        }

        // Normalization of r[0]
        let norm = norm_l(sum);
        sum = l_shl(sum, norm);
        (r_h[0], r_l[0]) = l_extract(sum);

        // r[1] to r[m]
        for i in 1..=m {
            sum = 0;
            for j in 0..(240 - i) {
                sum = l_mac(sum, y[j as usize], y[(j + i) as usize]);
            }
            sum = l_shl(sum, norm);
            (r_h[i as usize], r_l[i as usize]) = l_extract(sum);
        }
    }

    pub fn lag_window(&self, m: i16, r_h: &mut [Word16], r_l: &mut [Word16]) {
        for i in 1..=m {
            let x = mpy_32(
                r_h[i as usize],
                r_l[i as usize],
                LAG_H[(i - 1) as usize],
                LAG_L[(i - 1) as usize],
            );
            (r_h[i as usize], r_l[i as usize]) = l_extract(x);
        }
    }

    pub fn levinson(
        &mut self,
        rh: &[Word16],
        rl: &[Word16],
        a: &mut [Word16],
        rc: &mut [Word16],
    ) {
        let (mut ah, mut al) = ([0; 11], [0; 11]);
        let (mut anh, mut anl) = ([0; 11], [0; 11]);

        // K = A[1] = -R[1] / R[0]
        let mut t1 = l_comp(rh[1], rl[1]);
        let mut t2 = l_abs(t1);
        let mut t0 = div_32(t2, rh[0], rl[0]);
        if t1 > 0 {
            t0 = l_negate(t0);
        }
        let (mut kh, mut kl) = l_extract(t0);
        rc[0] = kh;
        t0 = l_shr(t0, 4);
        (ah[1], al[1]) = l_extract(t0);

        // Alpha = R[0] * (1-K**2)
        t0 = mpy_32(kh, kl, kh, kl);
        t0 = l_abs(t0);
        t0 = l_sub(0x7fffffff, t0);
        let (hi, lo) = l_extract(t0);
        t0 = mpy_32(rh[0], rl[0], hi, lo);

        // Normalize Alpha
        let mut alp_exp = norm_l(t0);
        t0 = l_shl(t0, alp_exp);
        let (mut alp_h, mut alp_l) = l_extract(t0);

        // ITERATIONS I=2 to M
        for i in 2..=10 {
            // t0 = SUM ( R[j]*A[i-j] ,j=1,i-1 ) + R[i]
            t0 = 0;
            for j in 1..i {
                t0 = l_add(t0, mpy_32(rh[j], rl[j], ah[i - j], al[i - j]));
            }
            t0 = l_shl(t0, 4);
            t1 = l_comp(rh[i], rl[i]);
            t0 = l_add(t0, t1);

            // K = -t0 / Alpha
            t1 = l_abs(t0);
            t2 = div_32(t1, alp_h, alp_l);
            if t0 > 0 {
                t2 = l_negate(t2);
            }
            t2 = l_shl(t2, alp_exp);
            (kh, kl) = l_extract(t2);
            rc[i - 1] = kh;

            // Test for unstable filter
            if sub(abs_s(kh), 32750) > 0 {
                for j in 0..=10 {
                    a[j] = self.old_a[j];
                }
                rc[0] = self.old_rc[0];
                rc[1] = self.old_rc[1];
                return;
            }

            // Compute new LPC coeff.
            for j in 1..i {
                t0 = mpy_32(kh, kl, ah[i - j], al[i - j]);
                t0 = l_add(t0, l_comp(ah[j], al[j]));
                (anh[j], anl[j]) = l_extract(t0);
            }
            t2 = l_shr(t2, 4);
            (anh[i], anl[i]) = l_extract(t2);

            // Alpha = Alpha * (1-K**2)
            t0 = mpy_32(kh, kl, kh, kl);
            t0 = l_abs(t0);
            t0 = l_sub(0x7fffffff, t0);
            let (hi, lo) = l_extract(t0);
            t0 = mpy_32(alp_h, alp_l, hi, lo);

            // Normalize Alpha
            let j = norm_l(t0);
            t0 = l_shl(t0, j);
            (alp_h, alp_l) = l_extract(t0);
            alp_exp = add(alp_exp, j);

            // A[j] = An[j]
            for j in 1..=i {
                ah[j] = anh[j];
                al[j] = anl[j];
            }
        }

        // Truncate A[i] in Q27 to Q12 with rounding
        a[0] = 4096;
        for i in 1..=10 {
            t0 = l_comp(ah[i], al[i]);
            a[i] = round(l_shl(t0, 1));
            self.old_a[i] = a[i];
        }
        self.old_rc[0] = rc[0];
        self.old_rc[1] = rc[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lpc() {
        let speech = [8192; 240];
        let mut r_h = [0; 11];
        let mut r_l = [0; 11];
        let mut a = [0; 11];
        let mut rc = [0; 10];

        let mut lpc = Lpc::new();
        let mut r_h = [32129, 32013, 31668, 31101, 30322, 29346, 28194, 26888, 25454, 23918, 22304];
        let mut r_l = [2816, 22272, 18432, 2048, 29184, 16384, 23040, 22016, 28160, 13312, 256];
        lpc.levinson(&r_h, &r_l, &mut a, &mut rc);

        let expected_a = [4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(a, expected_a);
    }
}
