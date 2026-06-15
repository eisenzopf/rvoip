use crate::dsp::types::Word16;

/// Public constant `BITSNO`.
pub const BITSNO: [u8; 11] = [8, 10, 8, 1, 13, 4, 7, 5, 13, 4, 7];
/// Public constant `BITSNO2`.
pub const BITSNO2: [u8; 4] = [1, 5, 4, 5];

#[inline(always)]
fn read_bits(payload: &[u8], start_bit: usize, n_bits: u8) -> u16 {
    let mut v = 0u16;
    for i in 0..n_bits as usize {
        let bit_pos = start_bit + i;
        let byte = payload[bit_pos / 8];
        let bit = (byte >> (7 - (bit_pos % 8))) & 1;
        v = (v << 1) | u16::from(bit);
    }
    v
}

#[inline(always)]
fn write_bits(payload: &mut [u8], start_bit: usize, n_bits: u8, value: u16) {
    for i in 0..n_bits as usize {
        let bit_pos = start_bit + i;
        let bit = ((value >> ((n_bits as usize - 1) - i)) & 1) as u8;
        let byte = &mut payload[bit_pos / 8];
        let mask = 1 << (7 - (bit_pos % 8));
        if bit != 0 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
}

/// Public function `unpack_speech_params`.
pub fn unpack_speech_params(payload: &[u8; 10]) -> [Word16; 11] {
    let mut params = [Word16(0); 11];
    let mut bit_off = 0usize;
    for (i, &nb) in BITSNO.iter().enumerate() {
        params[i] = Word16(read_bits(payload, bit_off, nb) as i16);
        bit_off += nb as usize;
    }
    params
}

/// Public function `pack_speech_params`.
pub fn pack_speech_params(params: &[Word16; 11]) -> [u8; 10] {
    let mut payload = [0u8; 10];
    let mut bit_off = 0usize;
    for (i, &nb) in BITSNO.iter().enumerate() {
        write_bits(&mut payload, bit_off, nb, params[i].0 as u16);
        bit_off += nb as usize;
    }
    payload
}

/// Public function `unpack_sid_params`.
pub fn unpack_sid_params(payload: &[u8; 2]) -> [Word16; 4] {
    let mut params = [Word16(0); 4];
    let mut bit_off = 0usize;
    for (i, &nb) in BITSNO2.iter().enumerate() {
        params[i] = Word16(read_bits(payload, bit_off, nb) as i16);
        bit_off += nb as usize;
    }
    params
}

/// Public function `pack_sid_params`.
pub fn pack_sid_params(params: &[Word16; 4]) -> [u8; 2] {
    let mut payload = [0u8; 2];
    let mut bit_off = 0usize;
    for (i, &nb) in BITSNO2.iter().enumerate() {
        write_bits(&mut payload, bit_off, nb, params[i].0 as u16);
        bit_off += nb as usize;
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_params_roundtrip() {
        let p = [
            Word16(12),
            Word16(666),
            Word16(123),
            Word16(1),
            Word16(4095),
            Word16(9),
            Word16(65),
            Word16(17),
            Word16(777),
            Word16(2),
            Word16(99),
        ];
        let bits = pack_speech_params(&p);
        let back = unpack_speech_params(&bits);
        assert_eq!(p, back);
    }

    #[test]
    fn sid_params_roundtrip() {
        let p = [Word16(1), Word16(12), Word16(7), Word16(18)];
        let bits = pack_sid_params(&p);
        let back = unpack_sid_params(&bits);
        assert_eq!(p, back);
    }
}
