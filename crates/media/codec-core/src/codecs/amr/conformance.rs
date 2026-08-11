//! The normative 3GPP conformance sequences, when they are present.
//!
//! # Why these are not in the tree
//!
//! `tst.inp`, `tst_m0.cod` .. `tst_m8.cod` and their narrowband counterparts
//! are 3GPP copyright. The reference *implementations* are fetched and built
//! by `tools/build-amr-reference.sh` for in-house design and never
//! redistributed, and the same applies to the sequences: only generated output
//! is committed. So these tests are opt-in, and they **panic rather than skip**
//! when the sequences are absent. A conformance test that quietly passes
//! because it found nothing to check is worse than no test at all — this
//! branch has been bitten four times by comparisons that silently compared
//! nothing.
//!
//! # Running them
//!
//! ```text
//! RVOIP_AMRWB_REFERENCE=$TMPDIR/rvoip-amr-reference \
//!   cargo test -p rvoip-codec-core --all-features -- --ignored conformance
//! ```
//!
//! The directory is the one `build-amr-reference.sh` populates; the sequences
//! live in its `testv/` subdirectory.
//!
//! # What they establish that the committed fixtures do not
//!
//! The committed fixtures are 50 frames of one deterministic signal, encoded
//! by the reference *implementation*. These are 200 frames of the
//! specification's own input, compared against the specification's own output.
//! Agreement with an implementation is strong evidence; agreement with the
//! normative sequence is the claim itself.
//!
//! All nine wideband encoder vectors are produced with `-dtx`
//! (`testv/test_enc.bat`), and eight of the nine are reproducible without it —
//! only 23.85 kbit/s differs, because it is the one rate whose high-band
//! correction gain is scaled by the DTX hangover counter.

#[cfg(test)]
mod tests {
    use crate::codecs::amr::wb::enc::encoder::{Rate, WbEncoder};
    use crate::codecs::amr::wb::homing;
    use std::path::{Path, PathBuf};

    /// Where the fetched reference lives, or a panic naming the variable.
    fn reference_root() -> PathBuf {
        let named = std::env::var("RVOIP_AMRWB_REFERENCE").unwrap_or_else(|_| {
            let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
            format!("{}/rvoip-amr-reference", tmp.trim_end_matches('/'))
        });
        let root = PathBuf::from(named);
        assert!(
            root.join("testv/tst.inp").is_file(),
            "the TS 26.173 conformance sequences are not at {}. They are 3GPP \
             copyright and are deliberately not committed; run \
             tools/build-amr-reference.sh and set RVOIP_AMRWB_REFERENCE. This test \
             panics rather than skipping so a green run cannot be mistaken for \
             conformance evidence.",
            root.display()
        );
        root
    }

    /// One frame of the input sequence.
    fn frames(root: &Path) -> Vec<[i16; 320]> {
        let raw = std::fs::read(root.join("testv/tst.inp")).expect("tst.inp reads");
        assert_eq!(raw.len() % 640, 0, "tst.inp is not a whole number of frames");
        raw.chunks_exact(640)
            .map(|chunk| {
                let mut frame = [0i16; 320];
                for (slot, pair) in frame.iter_mut().zip(chunk.chunks_exact(2)) {
                    *slot = i16::from_le_bytes([pair[0], pair[1]]);
                }
                frame
            })
            .collect()
    }

    /// The ETSI serial `.cod` form the vectors ship in.
    ///
    /// Three 16-bit header words — a `0x6b21` sync, the transmit frame type
    /// and the mode — then one word per codec bit, `127` for a one and `-127`
    /// for a zero. Not the MIME format the rest of this crate speaks, and read
    /// directly rather than converted: a converter is one more thing that
    /// could be wrong in the same direction as the code under test.
    fn read_serial(path: &Path, bits_per_frame: usize) -> Vec<Vec<u8>> {
        let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let words: Vec<i16> = raw
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let stride = 3 + bits_per_frame;
        assert_eq!(
            words.len() % stride,
            0,
            "{} is not a whole number of {bits_per_frame}-bit frames",
            path.display()
        );
        words
            .chunks_exact(stride)
            .map(|frame| {
                assert_eq!(frame[0], 0x6b21, "missing the serial sync word");
                frame[3..].iter().map(|&w| u8::from(w == 127)).collect()
            })
            .collect()
    }

    /// The eight rates whose normative vectors do not depend on DTX, encoded
    /// from the specification's own input and compared bit for bit.
    #[test]
    #[ignore = "needs the 3GPP sequences; see the module header"]
    fn wideband_encoder_matches_the_normative_vectors() {
        let root = reference_root();
        let input = frames(&root);
        assert_eq!(input.len(), 200, "tst.inp is not 200 frames");

        let mut compared = 0usize;
        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let want = read_serial(&root.join(format!("testv/tst_m{mode}.cod")), rate.bits());
            assert_eq!(want.len(), input.len(), "mode {mode} vector length");

            let mut encoder = WbEncoder::new();
            encoder.set_allow_dtx(true);
            for (n, frame) in input.iter().enumerate() {
                // The sequence opens with homing frames; each drives a reset.
                let homing = homing::is_encoder_homing_frame(frame);
                let (_, payload) = encoder.encode_frame_typed(frame, rate);
                let got = super::super::wb::bitstream::CodecBits::unpack(
                    crate::codecs::amr::AmrMode::new(
                        crate::codecs::amr::AmrVariant::WideBand,
                        mode,
                    )
                    .expect("a speech mode"),
                    &payload,
                )
                .expect("payload unpacks");
                assert_eq!(
                    got.bits(),
                    &want[n][..],
                    "mode {mode} frame {n} differs from the normative vector"
                );
                compared += want[n].len();
                if homing {
                    encoder = WbEncoder::new();
                    encoder.set_allow_dtx(true);
                }
            }
        }
        assert!(compared > 500_000, "only {compared} bits compared");
    }

    /// The decoder, against the specification's own output for each vector.
    ///
    /// `tst_m*.out` is what TS 26.173's decoder makes of `tst_m*.cod`. All
    /// nine, sample for sample, masked to the fourteen bits AMR-WB defines.
    #[test]
    #[ignore = "needs the 3GPP sequences; see the module header"]
    fn wideband_decoder_matches_the_normative_vectors() {
        use crate::codecs::amr::wb::decoder::Decoder;
        use crate::codecs::amr::wb::gain::FrameQuality;

        let root = reference_root();
        let mut compared = 0usize;
        for mode in 0..9u8 {
            let amr =
                crate::codecs::amr::AmrMode::new(crate::codecs::amr::AmrVariant::WideBand, mode)
                    .expect("a speech mode");
            let bits = Rate::from_index(mode).expect("a speech mode").bits();
            let coded = read_serial(&root.join(format!("testv/tst_m{mode}.cod")), bits);
            let raw = std::fs::read(root.join(format!("testv/tst_m{mode}.out")))
                .unwrap_or_else(|e| panic!("tst_m{mode}.out: {e}"));
            let want: Vec<i16> = raw
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert_eq!(want.len(), coded.len() * 320, "mode {mode} output length");

            let mut decoder = Decoder::new();
            // The driver's two-state homing protocol, transcribed from
            // `decoder.c`. Once homed, a following homing frame is recognised
            // from its *first subframe only* and is answered with the encoder
            // homing frame directly, without decoding -- the rest of that
            // frame already carries the next one's content.
            //
            // It starts *homed*: `reset_flag_old` is initialised to 1, so a
            // sequence opening with a homing frame is answered rather than
            // decoded. Starting from false decodes frame 0 and emits silence
            // where the vector has 0x0008.
            let mut homed = true;
            for (n, frame) in coded.iter().enumerate() {
                // Re-sort the codec bits into a payload, which is what the
                // decoder's public entry point takes.
                let sort = crate::codecs::amr::wb::bitstream::sort_table_for(amr);
                let mut payload = vec![0u8; bits.div_ceil(8)];
                for (i, &source) in sort.iter().enumerate() {
                    payload[i / 8] |= frame[source as usize] << (7 - (i % 8));
                }

                let mut is_homing = homed
                    && homing::is_decoder_homing_frame_first(&payload, mode as usize);
                let out = if is_homing {
                    [homing::HOMING_SAMPLE; 320]
                } else {
                    decoder
                        .decode_frame(amr, &payload, FrameQuality::Good)
                        .unwrap_or_else(|| panic!("mode {mode} frame {n} refused"))
                };
                for (i, (&got, &theirs)) in out.iter().zip(&want[n * 320..]).enumerate() {
                    assert_eq!(
                        got & !3,
                        theirs,
                        "mode {mode} frame {n} sample {i} differs from the vector"
                    );
                    compared += 1;
                }
                if !homed {
                    is_homing = homing::is_decoder_homing_frame(&payload, mode as usize);
                }
                if is_homing {
                    decoder = Decoder::new();
                }
                homed = is_homing;
            }
        }
        assert!(compared > 500_000, "only {compared} samples compared");
    }
}
