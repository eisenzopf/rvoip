use super::*;

#[test]
fn dsp_shl_saturates() {
    let mut ctx = DspContext::default();
    let r = shl(&mut ctx, Word16(20_000), 2);
    assert_eq!(r.0, MAX_16);
    assert!(ctx.overflow);
}

#[test]
fn dsp_norm_l_counts_leading_zeros() {
    assert_eq!(norm_l(Word32(0x1000_0000)), 2);
}
