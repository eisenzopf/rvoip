use crate::common::basic_operators::*;
use crate::common::oper_32b::{l_extract, mpy_32_16};

const B_COEFF: [Word16; 3] = [1899, -3798, 1899];
const A_COEFF: [Word16; 2] = [7807, -3733];

pub struct PreProc {
    x: [Word16; 2],
    y: [(Word16, Word16); 2],
}

impl PreProc {
    pub fn new() -> Self {
        PreProc {
            x: [0; 2],
            y: [(0, 0); 2],
        }
    }

    pub fn process(&mut self, signal: &mut [Word16]) {
        for sample in signal.iter_mut() {
            let input = *sample;
            let mut l_tmp: Word32;

            let (y0_hi, y0_lo) = self.y[0];
            let (y1_hi, y1_lo) = self.y[1];

            l_tmp = mpy_32_16(y0_hi, y0_lo, A_COEFF[0]);
            l_tmp = l_add(l_tmp, mpy_32_16(y1_hi, y1_lo, A_COEFF[1]));
            l_tmp = l_mac(l_tmp, input, B_COEFF[0]);
            l_tmp = l_mac(l_tmp, self.x[0], B_COEFF[1]);
            l_tmp = l_mac(l_tmp, self.x[1], B_COEFF[2]);

            self.x[1] = self.x[0];
            self.x[0] = input;

            l_tmp = l_shl(l_tmp, 3);
            *sample = round(l_tmp);

            self.y[1] = self.y[0];
            self.y[0] = l_extract(l_tmp);
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
        let expected_signal: [Word16; 80] = [
            1899, 1721, 1549, 1384, 1226, 1075, 933, 797, 670, 550, 438, 333, 236, 147, 64, -12, -81, -143, -199, -249, -293, -332, -365, -394, -418, -437, -453, -465, -473, -478, -479, -479, -475, -470, -462, -453, -442, -429, -416, -401, -385, -369, -352, -335, -318, -300, -283, -265, -248, -230, -213, -197, -181, -165, -150, -135, -121, -107, -94, -82, -71, -60, -49, -40, -31, -22, -15, -7, -1, 5, 10, 15, 20, 24, 27, 30, 32, 35, 36, 38
        ];
        assert_eq!(signal, expected_signal);
    }
}
