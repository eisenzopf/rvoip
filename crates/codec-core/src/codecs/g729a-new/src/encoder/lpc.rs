use crate::common::basic_operators::*;

const LAG_WINDOW: [Word16; 10] = [32766, 32761, 32752, 32739, 32722, 32701, 32676, 32647, 32614, 32577];

pub fn autocorrelation(speech: &[Word16], r: &mut [Word32]) {
    let mut s_prime = [0i16; 240];
    // Hamming window
    for i in 0..240 {
        let window_val = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / 239.0).cos();
        s_prime[i] = (speech[i] as f64 * window_val) as Word16;
    }

    for k in 0..=10 {
        let mut sum: Word32 = 0;
        for n in k..240 {
            sum = L_mac(sum, s_prime[n], s_prime[n - k]);
        }
        r[k] = sum;
    }

    if r[0] < 1 {
        r[0] = 1;
    }

    let r0_scaled = L_shr(r[0], 1);
    let noise_floor = L_mult(round(r0_scaled), 1);
    r[0] = L_mac(r[0], round(noise_floor), 1);

    for k in 1..=10 {
        r[k] = L_mult(r[k] as Word16, LAG_WINDOW[k-1]);
    }
}

pub fn levinson_durbin(r: &[Word32], a: &mut [Word16]) {
    let mut rc = [0i16; 10];
    let mut ac = [0i16; 11];
    let mut old_a = [0i16; 11];
    let mut e: Word32 = r[0];

    ac[0] = 4096; // 1.0 in Q12

    for i in 0..10 {
        let mut s: Word32 = 0;
        for j in 0..=i {
            s = L_mac(s, ac[j], r[i + 1 - j] as Word16);
        }
        if e == 0 {
            rc[i] = 0;
        } else {
            rc[i] = -round(L_shl(s, 16).wrapping_div(e));
        }

        for j in 0..=i {
            old_a[j] = ac[j];
        }

        for j in 1..=i + 1 {
            ac[j] = round(L_mac(L_mult(old_a[j], 4096), rc[i], old_a[i + 1 - j]));
        }

        e = L_mac(L_mult(e as Word16, 4096), -round(L_mult(rc[i], rc[i])), e as Word16);
    }

    for i in 0..11 {
        a[i] = ac[i];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lpc() {
        let speech = [8192; 240];
        let mut r = [0; 11];
        let mut a = [0; 11];

        autocorrelation(&speech, &mut r);
        levinson_durbin(&r, &mut a);

        let expected_a = [4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(a, expected_a);
    }
}
