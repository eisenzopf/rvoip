use rvoip_rtp_core::packet::extension::RtpHeaderExtensions;

#[test]
fn aligned_rfc8285_one_byte_extensions_have_no_synthetic_end_marker() {
    let mut extensions = RtpHeaderExtensions::new_one_byte();
    extensions.add_extension(1, vec![1, 2, 3]).unwrap();

    assert_eq!(
        extensions.serialize().unwrap().as_ref(),
        &[0x12, 1, 2, 3],
        "an already aligned RFC 8285 extension block must not grow an extra word"
    );
}
