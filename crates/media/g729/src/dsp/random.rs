/// Public function `random`.
#[inline(always)]
pub fn random(seed: &mut i16) -> i16 {
    let mut s = i32::from(*seed);
    s = (s.wrapping_mul(31821).wrapping_add(13849)) & 0xFFFF;
    *seed = s as i16;
    *seed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_random_deterministic() {
        let mut seed = 21845;
        let a = random(&mut seed);
        let b = random(&mut seed);
        assert_ne!(a, b);
    }
}
