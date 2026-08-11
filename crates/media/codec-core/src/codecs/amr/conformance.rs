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

    /// The normative *DTX* vector: real SID frames on the wire.
    ///
    /// `tst.inp` never goes quiet enough to emit one, so the two tests above
    /// exercise DTX only through its effect on 23.85 kbit/s's gain. This one
    /// is `test_enc.bat`'s tenth line — `coder -dtx 2 dtx.inp` — and it is 80
    /// speech frames, a `SID_FIRST`, then a long comfort-noise tail. It is the
    /// only normative check that a SID this codec *transmits* is the SID the
    /// specification says to transmit.
    #[test]
    #[ignore = "needs the 3GPP sequences; see the module header"]
    fn wideband_dtx_matches_the_normative_vector() {
        use crate::codecs::amr::sid_cadence::SidCadence;
        use crate::codecs::amr::{AmrMode, AmrVariant};

        let root = reference_root();
        let raw = std::fs::read(root.join("testv/dtx.inp")).expect("dtx.inp reads");
        let input: Vec<[i16; 320]> = raw
            .chunks_exact(640)
            .map(|chunk| {
                let mut frame = [0i16; 320];
                for (slot, pair) in frame.iter_mut().zip(chunk.chunks_exact(2)) {
                    *slot = i16::from_le_bytes([pair[0], pair[1]]);
                }
                frame
            })
            .collect();

        // The serial length follows the frame *type*, not the mode word: a
        // comfort-noise frame keeps the speech mode in its header and carries
        // 35 bits.
        let vector = std::fs::read(root.join("testv/tst_md.cod")).expect("tst_md.cod reads");
        let words: Vec<i16> = vector
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let rate = Rate::from_index(2).expect("12.65 kbit/s");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");

        let mut encoder = WbEncoder::new();
        encoder.set_allow_dtx(true);
        let mut cadence = SidCadence::new(AmrVariant::WideBand);

        let mut at = 0usize;
        let mut frame = 0usize;
        let mut seen = (0usize, 0usize, 0usize, 0usize);
        while at < words.len() {
            assert_eq!(words[at], 0x6b21, "frame {frame} lost the serial sync");
            let tx_type = words[at + 1];
            let bits = if tx_type == 0 { rate.bits() } else { 35 };
            let want: Vec<u8> = words[at + 3..at + 3 + bits]
                .iter()
                .map(|&w| u8::from(w == 127))
                .collect();

            let source = input.get(frame).unwrap_or_else(|| {
                panic!("dtx.inp is shorter than tst_md.cod at frame {frame}")
            });
            let homing = homing::is_encoder_homing_frame(source);
            let (comfort_noise, payload) = encoder.encode_frame_typed(source, rate);
            let scheduled = cadence.next(comfort_noise, mode);

            // 0 speech, 1 SID_FIRST, 2 SID_UPDATE, 3 NO_DATA.
            let want_type = match scheduled {
                crate::codecs::amr::AmrFrameType::Speech(_) => 0,
                crate::codecs::amr::AmrFrameType::Sid(_) => {
                    if cadence.last_sid_was_an_update() {
                        2
                    } else {
                        1
                    }
                }
                _ => 3,
            };
            assert_eq!(want_type, tx_type, "frame {frame} transmit type");
            match tx_type {
                0 => seen.0 += 1,
                1 => seen.1 += 1,
                2 => seen.2 += 1,
                _ => seen.3 += 1,
            }

            // The reference writes the built SID bits even on a NO_DATA
            // frame, so every frame's payload is comparable. A SID sorts
            // through its own table and `AmrMode` cannot name the pseudo-mode,
            // so unsort it directly.
            let got: Vec<u8> = if tx_type == 0 {
                super::super::wb::bitstream::CodecBits::unpack(mode, &payload)
                    .expect("speech payload unpacks")
                    .bits()
                    .to_vec()
            } else {
                let sort = &crate::codecs::amr::wb::sort_tables::SORT_SID;
                let mut codec = vec![0u8; sort.len()];
                for (i, &target) in sort.iter().enumerate() {
                    codec[target as usize] = (payload[i / 8] >> (7 - (i % 8))) & 1;
                }
                codec
            };
            assert_eq!(got, want, "frame {frame} payload");

            if homing {
                encoder = WbEncoder::new();
                encoder.set_allow_dtx(true);
                cadence.reset();
            }
            at += 3 + bits;
            frame += 1;
        }

        assert_eq!(seen, (80, 1, 15, 104), "the vector's frame mix moved");
    }

    /// The decoder against `tst_md.out`: the normative comfort-noise output.
    ///
    /// Speech, comfort noise and gaps, 200 frames, sample for sample.
    ///
    /// Two defects had to go before this passed, and the committed fixture
    /// exposed neither.
    ///
    /// The background energy history has to be captured from an excitation in
    /// *one* exponent. `rescale_to` rescales the whole history when a subframe
    /// needs a different one, so the reference's buffer is uniform by the end
    /// and a single `Scale_sig` undoes it; snapshotting each subframe as it is
    /// built mixes four exponents and corrupts exactly the frames where the
    /// scaling moved — three of eight ring slots here.
    ///
    /// And `CN_dithering` draws the generator *twice* per perturbation and
    /// sums the halves, giving a triangular variate rather than a uniform one,
    /// and enforces ISF spacing inline against the coefficient just written
    /// rather than in a pass afterwards. Written from a summary it was wrong
    /// in both respects, and only a stream whose encoder set the dithering bit
    /// could show it.
    #[test]
    #[ignore = "needs the 3GPP sequences; see the module header"]
    fn wideband_dtx_decoding_matches_the_normative_vector() {
        use crate::codecs::amr::wb::decoder::Decoder;
        use crate::codecs::amr::wb::dtx::RxFrameType;
        use crate::codecs::amr::wb::gain::FrameQuality;
        use crate::codecs::amr::{AmrMode, AmrVariant};

        let root = reference_root();
        let vector = std::fs::read(root.join("testv/tst_md.cod")).expect("tst_md.cod reads");
        let words: Vec<i16> = vector
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let raw = std::fs::read(root.join("testv/tst_md.out")).expect("tst_md.out reads");
        let want: Vec<i16> = raw
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();

        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");
        let bits = Rate::from_index(2).expect("12.65 kbit/s").bits();
        let sort = crate::codecs::amr::wb::bitstream::sort_table_for(mode);
        let comfort_order = &crate::codecs::amr::wb::sort_tables::SORT_SID;

        let mut decoder = Decoder::new();
        let mut homed = true;
        let mut at = 0usize;
        let mut frame = 0usize;
        let mut compared = 0usize;
        while at < words.len() {
            let tx_type = words[at + 1];
            let width = if tx_type == 0 { bits } else { 35 };
            let codec: Vec<u8> = words[at + 3..at + 3 + width]
                .iter()
                .map(|&w| u8::from(w == 127))
                .collect();

            let out = if tx_type == 0 {
                let mut payload = vec![0u8; bits.div_ceil(8)];
                for (i, &source) in sort.iter().enumerate() {
                    payload[i / 8] |= codec[source as usize] << (7 - (i % 8));
                }
                let is_homing = homed
                    && homing::is_decoder_homing_frame_first(&payload, 2);
                let out = if is_homing {
                    [homing::HOMING_SAMPLE; 320]
                } else {
                    decoder
                        .decode_frame(mode, &payload, FrameQuality::Good)
                        .unwrap_or_else(|| panic!("frame {frame} refused"))
                };
                homed = if homed {
                    is_homing
                } else {
                    homing::is_decoder_homing_frame(&payload, 2)
                };
                if homed {
                    decoder = Decoder::new();
                }
                out
            } else {
                homed = false;
                let mut payload = vec![0u8; 5];
                for (i, &source) in comfort_order.iter().enumerate() {
                    payload[i / 8] |= codec[source as usize] << (7 - (i % 8));
                }
                let rx = match tx_type {
                    1 => RxFrameType::SidFirst,
                    2 => RxFrameType::SidUpdate,
                    _ => RxFrameType::NoData,
                };
                let data: &[u8] = if tx_type == 3 { &[] } else { &payload };
                decoder
                    .decode_comfort_noise(rx, data, bits)
                    .unwrap_or_else(|| panic!("frame {frame} comfort noise refused"))
            };

            for (i, (&got, &theirs)) in out.iter().zip(&want[frame * 320..]).enumerate() {
                assert_eq!(
                    got & !3,
                    theirs,
                    "frame {frame} (tx type {tx_type}) sample {i}"
                );
                compared += 1;
            }
            at += 3 + width;
            frame += 1;
        }
        assert_eq!(frame, 200, "the vector is 200 frames");
        assert_eq!(compared, 200 * 320);
    }
}
