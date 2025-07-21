use crate::common::basic_operators::{round, L_mac, L_mult, Word16, Word32};

// Corresponds to H(z) = (0.46363718 - 0.92724705z^-1 + 0.46363718z^-2) / (1 - 1.9059465z^-1 + 0.9114024z^-2)
// The numerator coefficients are scaled by 2, as specified in the G.729 documentation.
const B_COEFF: [Word16; 3] = [1899, -3798, 1899]; // {0.46363718, -0.92724705, 0.46363718} in Q12
const A_COEFF: [Word16; 2] = [15613, -7466]; // {1.9059465, -0.9114024} in Q13

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
            let input_scaled = *sample >> 1;
            let mut L_tmp: Word32;

            L_tmp = L_mult(self.y[0], A_COEFF[0]);
            L_tmp = L_mac(L_tmp, self.y[1], A_COEFF[1]);
            L_tmp = L_mac(L_tmp, input_scaled, B_COEFF[0]);
            L_tmp = L_mac(L_tmp, self.x[0], B_COEFF[1]);
            L_tmp = L_mac(L_tmp, self.x[1], B_COEFF[2]);

            self.x[1] = self.x[0];
            self.x[0] = input_scaled;

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
        let expected_signal: [Word16; 80] = [
            950, 861, 775, 692, 613, 538, 467, 400, 337, 278, 223, 172, 125, 81, 40, 2, -33, -65, -94, -120, -143, -163, -180, -195, -208, -219, -228, -235, -240, -243, -244, -244, -243, -241, -238, -234, -229, -223, -216, -208, -200, -192, -184, -176, -168, -160, -152, -144, -136, -128, -120, -112, -104, -96, -88, -80, -72, -64, -56, -48, -40, -32, -25, -18, -12, -6, 0, 5, 10, 15, 19, 23, 27, 30, 33, 36, 39, 42, 45, 47
        ];
        assert_eq!(signal, expected_signal);
    }
}