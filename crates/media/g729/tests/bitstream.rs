use g729::bitstream::{
    pack_sid_params, pack_speech_params, unpack_sid_params, unpack_speech_params,
};
use g729::dsp::Word16;

#[test]
fn bitstream_speech_pack_unpack_roundtrip() {
    let prm = [
        Word16(0),
        Word16(120),
        Word16(210),
        Word16(1),
        Word16(6200),
        Word16(15),
        Word16(100),
        Word16(19),
        Word16(5300),
        Word16(8),
        Word16(110),
    ];
    let bits = pack_speech_params(&prm);
    let out = unpack_speech_params(&bits);
    assert_eq!(prm, out);
}

#[test]
fn bitstream_sid_pack_unpack_roundtrip() {
    let prm = [Word16(1), Word16(12), Word16(7), Word16(18)];
    let bits = pack_sid_params(&prm);
    let out = unpack_sid_params(&bits);
    assert_eq!(prm, out);
}
