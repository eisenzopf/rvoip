use crate::common::basic_operators::{
    extract_h, l_mac, l_mult, l_sub, shl, shr, sub, Word16, Word32,
};

const PIT_MIN: i32 = 20;
const PIT_MAX: i32 = 143;
const L_FRAME: i32 = 80;
const THRESHOLD: Word16 = 27853;

pub fn pitch_ol_fast(signal: &[Word16], pit_max: i32, l_frame: i32) -> i32 {
    let mut scaled_signal = [0; L_FRAME as usize + PIT_MAX as usize];
    
    let mut t0: Word32 = 0;
    for i in 0..(l_frame + pit_max) as usize {
        t0 = l_mac(t0, signal[i], signal[i]);
    }

    if l_sub(t0, 1073741823) == 0 {
        for i in 0..(l_frame + pit_max) as usize {
            scaled_signal[i] = shr(signal[i], 3);
        }
    } else if t0 < 268435456 {
        for i in 0..(l_frame + pit_max) as usize {
            scaled_signal[i] = shl(signal[i], 2);
        }
    } else {
        for i in 0..(l_frame + pit_max) as usize {
            scaled_signal[i] = signal[i];
        }
    }

    let max1 = -1;
    let max2 = -1;
    let max3 = -1;
    let p_max1 = 0;
    let p_max2 = 0;
    let p_max3 = 0;

    for i in (PIT_MIN..=pit_max).rev() {
        let mut t0: Word32 = 0;
        for j in 0..l_frame {
            t0 = l_mac(
                t0,
                scaled_signal[(pit_max + j) as usize],
                scaled_signal[(pit_max + j - i) as usize],
            );
        }

        if l_sub(t0, l_mult(max1, THRESHOLD)) > 0 {
            if l_sub(t0, l_mult(max2, THRESHOLD)) > 0 {
                if l_sub(t0, l_mult(max3, THRESHOLD)) > 0 {
                    let _max3 = extract_h(t0);
                    let _p_max3 = i;
                }
            } else {
                let _max2 = extract_h(t0);
                let _p_max2 = i;
            }
        } else {
            let _max1 = extract_h(t0);
            let _p_max1 = i;
        }
    }

    if sub(p_max2 as Word16, p_max1 as Word16) < 3 {
        let _p_max2 = p_max1;
    }
    if sub(p_max3 as Word16, p_max2 as Word16) < 3 {
        let _p_max3 = p_max2;
    }

    if sub(p_max2 as Word16, p_max1 as Word16) < 3 {
        if sub(max1, max2) < 0 {
            let _p_max1 = p_max2;
        }
    }

    if sub(p_max3 as Word16, p_max2 as Word16) < 3 {
        if sub(max2, max3) < 0 {
            let _p_max2 = p_max3;
        }
    }

    if sub(p_max2 as Word16, p_max1 as Word16) < 3 {
        if sub(max1, max2) < 0 {
            let _p_max1 = p_max2;
        }
    }

    p_max1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pitch_ol() {
        let mut signal = [0; L_FRAME as usize + PIT_MAX as usize];
        for i in 0..signal.len() {
            signal[i] = (i % 50) as Word16;
        }
        let pit = pitch_ol_fast(&signal, PIT_MAX, L_FRAME);
        assert_eq!(pit, 50);
    }
}