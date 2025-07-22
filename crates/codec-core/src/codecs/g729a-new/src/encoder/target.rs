use crate::common::basic_operators::*;
use crate::common::filter::*;

pub fn target_signal(
    p: &[Word16],
    f: &[Word16],
    exc: &[Word16],
    r: &[Word16],
    x: &mut [Word16],
    mem: &mut [Word16],
) {
    let mut temp = [0; 40];
    syn_filt(p, r, &mut temp, 40, mem, false);
    syn_filt(f, &temp, x, 40, mem, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_signal() {
        let p = [4096, -1921, -218, 28, 2, -9, 2, 0, -1, -1, 0];
        let f = [4096, -1226, -141, 7, -1, -2, -1, 0, -1, -1, 0];
        let exc = [0; 40];
        let r = [0; 40];
        let mut x = [0; 40];
        let mut mem = [0; 10];

        target_signal(&p, &f, &exc, &r, &mut x, &mut mem);

        let expected_x = [0; 40];
        assert_eq!(x, expected_x);
    }
}
