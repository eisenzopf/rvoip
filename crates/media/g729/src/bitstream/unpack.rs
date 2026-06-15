use crate::bitstream::itu_params::{unpack_sid_params, unpack_speech_params};
use crate::tables::bitstream::{SID_FRAME_BYTES, SPEECH_FRAME_BYTES};

/// Public function `unpack_speech`.
pub fn unpack_speech(bits: &[u8; SPEECH_FRAME_BYTES]) -> [u16; 11] {
    let prm = unpack_speech_params(bits);
    let mut out = [0u16; 11];
    for i in 0..11 {
        out[i] = prm[i].0 as u16;
    }
    out
}

/// Public function `unpack_sid`.
pub fn unpack_sid(bits: &[u8; SID_FRAME_BYTES]) -> [u16; 4] {
    let prm = unpack_sid_params(bits);
    let mut out = [0u16; 4];
    for i in 0..4 {
        out[i] = prm[i].0 as u16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitstream::pack::{pack_sid, pack_speech};

    #[test]
    fn bitstream_roundtrip_speech_identity() {
        let params = [0u16, 120, 210, 1, 6200, 15, 100, 19, 5300, 8, 110];
        let bits = pack_speech(&params);
        let decoded = unpack_speech(&bits);
        assert_eq!(decoded, params);
    }

    #[test]
    fn bitstream_roundtrip_sid_identity() {
        let params = [1u16, 12, 7, 18];
        let bits = pack_sid(&params);
        let decoded = unpack_sid(&bits);
        assert_eq!(decoded, params);
    }
}
