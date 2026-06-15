use crate::bitstream::itu_params::{pack_sid_params, pack_speech_params};
use crate::dsp::Word16;
use crate::tables::bitstream::{SID_FRAME_BYTES, SPEECH_FRAME_BYTES};

/// Public function `pack_speech`.
pub fn pack_speech(params: &[u16; 11]) -> [u8; SPEECH_FRAME_BYTES] {
    let mut speech = [Word16(0); 11];
    for i in 0..11 {
        speech[i] = Word16(params[i] as i16);
    }
    pack_speech_params(&speech)
}

/// Public function `pack_sid`.
pub fn pack_sid(params: &[u16; 4]) -> [u8; SID_FRAME_BYTES] {
    let mut sid = [Word16(0); 4];
    for i in 0..4 {
        sid[i] = Word16(params[i] as i16);
    }
    pack_sid_params(&sid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitstream_pack_speech_roundtrip_shape() {
        let params = [0u16, 120, 210, 1, 6200, 15, 100, 19, 5300, 8, 110];
        let out = pack_speech(&params);
        assert_eq!(out.len(), SPEECH_FRAME_BYTES);
    }
}
