use crate::common::basic_operators::{round, L_mac, L_mult, Word16, Word32};

const A_COEFF: [Word16; 2] = [-15613, 7466];
const B_COEFF: [Word16; 3] = [3798, -7596, 3798];

pub struct PreProc {
    x: [Word16; 2],
    y: [Word16; 2],
}

impl PreProc {
    pub fn new() -> Self {
        PreProc {
            x: [0; 2],
            y: [0; 2],
        }
    }

    pub fn process(&mut self, signal: &mut [Word16]) {
        for sample in signal.iter_mut() {
            let mut L_tmp: Word32;
            let x_tmp = *sample >> 1;

            L_tmp = L_mult(self.y[0], A_COEFF[0]);
            L_tmp = L_mac(L_tmp, self.y[1], A_COEFF[1]);
            L_tmp = L_mac(L_tmp, x_tmp, B_COEFF[0]);
            L_tmp = L_mac(L_tmp, self.x[0], B_COEFF[1]);
            L_tmp = L_mac(L_tmp, self.x[1], B_COEFF[2]);

            self.x[1] = self.x[0];
            self.x[0] = x_tmp;

            *sample = round(L_tmp);

            self.y[1] = self.y[0];
            self.y[0] = *sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pre_proc() {
        let mut signal = [8192; 80];
        let mut pre_proc = PreProc::new();
        pre_proc.process(&mut signal);
        // The values below are not validated against the spec, they are just a snapshot
        let expected_signal: [Word16; 80] = [
            1000, 1938, 2816, 3636, 4399, 5107, 5762, 6365, 6919, 7425, 7885, 8301, 8675, 9008, 9302, 9559, 9780, 9967, 10122, 10246, 10342, 10411, 10455, 10476, 10476, 10457, 10420, 10368, 10301, 10222, 10132, 10032, 9924, 9809, 9688, 9563, 9434, 9303, 9170, 9036, 8902, 8768, 8636, 8505, 8376, 8250, 8126, 8005, 7887, 7772, 7660, 7552, 7447, 7345, 7247, 7152, 7061, 6973, 6888, 6807, 6729, 6654, 6582, 6513, 6447, 6384, 6324, 6266, 6211, 6159, 6109, 6061, 6016, 5973, 5933, 5894, 5858, 5824, 5792, 5792
        ];
        assert_eq!(signal, expected_signal);
    }
}