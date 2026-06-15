use super::*;

#[test]
fn bitstream_itu_serial_roundtrip_words() {
    let payload = [1u8, 2, 3, 4, 5];
    let words = frame_to_words(&payload, 40);
    let back = words_to_frame(&words[2..], 40);
    assert_eq!(back, payload);
}

#[test]
fn bitstream_itu_serial_roundtrip_frames() {
    let frames = vec![
        (SYNC_WORD, vec![BIT_1; 80]),
        (SYNC_WORD, vec![BIT_0; 16]),
        (SYNC_WORD, vec![]),
    ];
    let raw = serialize_stream(&frames);
    let parsed = parse_stream(&raw);
    assert_eq!(parsed, frames);
}
