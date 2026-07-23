//! Exhaustive G.711 companding stability regression.
//!
//! Every linear PCM input must produce a stable quantized value. A-law has one
//! canonical codepoint per value; mu-law has distinct positive-zero and
//! negative-zero codepoints, so its decoded value is the exact invariant.

use crate::codecs::g711::{alaw_compress, alaw_expand, ulaw_compress, ulaw_expand};

#[test]
fn test_all_linear_inputs_produce_stable_codepoints() {
    for raw in u16::MIN..=u16::MAX {
        let linear = raw as i16;

        let alaw = alaw_compress(linear);
        assert_eq!(
            alaw_compress(alaw_expand(alaw)),
            alaw,
            "A-law codepoint was not stable for PCM {linear}"
        );

        let ulaw = ulaw_compress(linear);
        let ulaw_linear = ulaw_expand(ulaw);
        assert_eq!(
            ulaw_expand(ulaw_compress(ulaw_linear)),
            ulaw_linear,
            "mu-law quantized value was not stable for PCM {linear}"
        );
    }
}
