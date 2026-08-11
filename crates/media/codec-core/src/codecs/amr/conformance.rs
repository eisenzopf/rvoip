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
//! RVOIP_AMRNB_REFERENCE=$TMPDIR/rvoip-amrnb-reference \
//!   cargo test -p rvoip-codec-core --all-features -- --ignored conformance
//! ```
//!
//! The directories are the ones `build-amr-reference.sh` and
//! `build-amrnb-encoder-reference.sh` populate. The wideband sequences live in
//! a `testv/` subdirectory; the narrowband ones sit beside the sources in
//! `c-code/`.
//!
//! # The narrowband sequence is one stream, and it changes rate every frame
//!
//! `spch_dos.inp` is 425 frames driven through `allmodes.txt`, which cycles
//! all eight rates continuously — so the rate changes on 424 of the 425
//! frames, with DTX on throughout. Nothing in the committed fixtures does
//! that: they are eight separate constant-rate streams. A rate switch carries
//! the LSF predictor, the gain predictors, the pitch history and the DTX rings
//! across a change in what those numbers *mean*, and that is exactly the state
//! the fixtures cannot exercise.
//!
//! It contains **no homing frame**. Both tests below drive the homing protocol
//! the reference drivers implement, because getting it wrong would be a silent
//! divergence if a stream ever did contain one — but neither is evidence that
//! the protocol works. That evidence is in `nb::homing`'s own tests, which
//! build each mode's pattern and check the encoder emits it.
//!
//! (`spch_un2` is the same stream through VAD2, which is not implemented — the
//! makefiles' default and this port's is VAD1. `spch_unx` is the big-endian
//! copy of `spch_dos`.)
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

    /// Where the narrowband reference lives, or a panic naming the variable.
    fn narrowband_root() -> PathBuf {
        let named = std::env::var("RVOIP_AMRNB_REFERENCE").unwrap_or_else(|_| {
            let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
            format!("{}/rvoip-amrnb-reference", tmp.trim_end_matches('/'))
        });
        let root = PathBuf::from(named);
        assert!(
            root.join("c-code/spch_dos.inp").is_file(),
            "the TS 26.073 verification sequences are not at {}. They are 3GPP \
             copyright and are deliberately not committed; run \
             tools/build-amrnb-encoder-reference.sh and set RVOIP_AMRNB_REFERENCE. \
             This test panics rather than skipping so a green run cannot be \
             mistaken for conformance evidence.",
            root.display()
        );
        root
    }

    /// Little-endian 16-bit samples, in frames of 160.
    fn narrowband_frames(path: &Path) -> Vec<[i16; 160]> {
        let bytes = std::fs::read(path).expect("the input sequence is readable");
        bytes
            .chunks_exact(320)
            .map(|frame| {
                let mut out = [0i16; 160];
                for (slot, pair) in out.iter_mut().zip(frame.chunks_exact(2)) {
                    *slot = i16::from_le_bytes([pair[0], pair[1]]);
                }
                out
            })
            .collect()
    }

    /// The rate sequence `allmodes.txt` drives the encoder through.
    fn mode_sequence(path: &Path) -> Vec<u8> {
        let text = std::fs::read_to_string(path).expect("the mode file is readable");
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| match line {
                "MR475" => 0u8,
                "MR515" => 1,
                "MR59" => 2,
                "MR67" => 3,
                "MR74" => 4,
                "MR795" => 5,
                "MR102" => 6,
                "MR122" => 7,
                other => panic!("unknown mode `{other}` in the mode file"),
            })
            .collect()
    }

    /// One frame of the narrowband ETSI serial format.
    struct SerialFrame {
        /// `TXFrameType`: 0 speech, 1 `SID_FIRST`, 2 `SID_UPDATE`, 3 `NO_DATA`.
        tx_type: i16,
        /// The 244 bit slots, of which each rate uses a prefix.
        bits: Vec<i16>,
        /// The rate the encoder was asked for, or -1 on a `NO_DATA` frame.
        mode: i16,
    }

    /// Read `spch_dos.cod`: 250 little-endian words per frame.
    ///
    /// `[tx_type][244 bits][mode][4 unused]`. Note the bit values are plain 0
    /// and 1 here — narrowband's `BIT_0`/`BIT_1` — where wideband's serial
    /// format uses ±127. Decoding this one with `& 1` happens to work and
    /// decoding wideband's that way yields all ones, which is why neither
    /// reader is shared.
    fn read_narrowband_serial(path: &Path) -> Vec<SerialFrame> {
        const WORDS: usize = 1 + 244 + 5;
        let bytes = std::fs::read(path).expect("the bitstream is readable");
        assert_eq!(bytes.len() % (WORDS * 2), 0, "not a whole number of serial frames");

        bytes
            .chunks_exact(WORDS * 2)
            .map(|frame| {
                let word = |i: usize| i16::from_le_bytes([frame[i * 2], frame[i * 2 + 1]]);
                SerialFrame {
                    tx_type: word(0),
                    bits: (1..=244).map(word).collect(),
                    mode: word(245),
                }
            })
            .collect()
    }

    /// The narrowband encoder against the reference's own verification stream.
    ///
    /// 425 frames, DTX on, and the rate changing on 424 of them. Compared
    /// three ways per frame: the transmitted frame type, the rate word, and
    /// every one of the rate's bits — 36575 bits in all, the rest of the
    /// stream being the 157 gaps DTX leaves and carrying no bits to compare.
    ///
    /// The homing reset is driven the way the reference driver drives it, but
    /// this stream contains no homing frame; see the module header.
    #[test]
    #[ignore = "needs the 3GPP sequences; see the module header"]
    fn narrowband_encoder_matches_the_reference_vectors() {
        use crate::codecs::amr::nb::enc::encoder::{NbEncoder, Rate as NbRate};
        use crate::codecs::amr::nb::homing as nb_homing;
        use crate::codecs::amr::sid_cadence::SidCadence;
        use crate::codecs::amr::{AmrFrameType, AmrMode, AmrVariant};

        let root = narrowband_root();
        let frames = narrowband_frames(&root.join("c-code/spch_dos.inp"));
        let modes = mode_sequence(&root.join("c-code/allmodes.txt"));
        let want = read_narrowband_serial(&root.join("c-code/spch_dos.cod"));

        assert!(frames.len() >= 400, "only {} input frames", frames.len());
        assert!(want.len() >= frames.len(), "the bitstream is shorter than the input");
        assert!(modes.len() >= frames.len(), "the mode file is shorter than the input");

        let mut encoder = NbEncoder::new();
        encoder.set_allow_dtx(true);
        let mut cadence = SidCadence::new(AmrVariant::NarrowBand);

        let (mut compared, mut sids, mut gaps, mut homings, mut switches) =
            (0usize, 0usize, 0usize, 0usize, 0usize);

        for (n, frame) in frames.iter().enumerate() {
            let mode = modes[n];
            if n > 0 && modes[n - 1] != mode {
                switches += 1;
            }
            let rate = NbRate::from_index(mode).expect("a speech mode");
            let amr_mode = AmrMode::new(AmrVariant::NarrowBand, mode).expect("a speech mode");

            // The driver tests the input *before* encoding it.
            let homing_frame = nb_homing::is_encoder_homing_frame(frame);

            let (comfort_noise, mut data) = encoder.encode_frame_typed(frame, rate);
            let tx = cadence.next(comfort_noise, amr_mode);
            if let AmrFrameType::Sid(_) = tx {
                let update = cadence.last_sid_was_an_update();
                crate::codecs::amr::nb::bitstream::finish_sid_payload(&mut data, update, mode);
            }

            let (want_tx, want_mode) = match tx {
                AmrFrameType::Speech(_) => (0i16, i16::from(mode)),
                AmrFrameType::Sid(_) => {
                    sids += 1;
                    (if cadence.last_sid_was_an_update() { 2 } else { 1 }, i16::from(mode))
                }
                AmrFrameType::NoData => {
                    gaps += 1;
                    (3, -1)
                }
                AmrFrameType::SpeechLost => panic!("narrowband has no SPEECH_LOST"),
            };
            assert_eq!(want[n].tx_type, want_tx, "frame {n} type");
            assert_eq!(want[n].mode, want_mode, "frame {n} mode word");

            // `Prm2bits` writes one word per bit in **parameter** order,
            // most significant bit of each parameter first -- *not* in the
            // sorted payload order the MIME format uses. `PackBits` is what
            // applies the sort, and it is a different function on a different
            // path. Comparing the packed payload against these words directly
            // diverges at bit 16 of the very first frame.
            //
            // So this unpacks back to codec order, which also exercises the
            // sort table in both directions.
            if want_tx != 3 {
                let (unpack_mode, bits) = if want_tx == 0 { (mode, rate.bits()) } else { (8, 35) };
                let codec = crate::codecs::amr::nb::bitstream::unpack(unpack_mode, &data)
                    .expect("the payload unpacks");
                for (i, &want_bit) in want[n].bits[..bits].iter().enumerate() {
                    assert_eq!(
                        i16::from(codec[i]),
                        want_bit,
                        "frame {n} (mode {mode}, type {want_tx}) bit {i}"
                    );
                    compared += 1;
                }
            }

            if homing_frame {
                homings += 1;
                encoder = NbEncoder::new();
                encoder.set_allow_dtx(true);
                cadence = SidCadence::new(AmrVariant::NarrowBand);
            }
        }

        // Exact, not a floor: these are properties of a fixed stream, and a
        // floor would let a future change quietly compare fewer frames.
        assert_eq!(compared, 36_575, "the bit count changed; the stream is fixed");
        assert_eq!((sids, gaps), (26, 157), "the DTX frame mix changed");
        assert_eq!(switches, 424, "the mode file did not cycle every frame");
        assert_eq!(homings, 0, "this stream has no homing frame; see the module header");
    }

    /// The narrowband decoder against the reference's own output.
    ///
    /// `spch_dos.out` is what TS 26.073's decoder makes of `spch_dos.cod`.
    /// All 68000 samples of all 425 frames, masked to the thirteen bits AMR-NB
    /// defines — 242 speech frames at eight different rates, 26 SIDs and 157
    /// gaps, with the rate changing almost every frame.
    #[test]
    #[ignore = "needs the 3GPP sequences; see the module header"]
    fn narrowband_decoder_matches_the_reference_vectors() {
        use crate::codecs::amr::nb::decoder::Decoder;
        use crate::codecs::amr::nb::dtx::RxFrameType;
        use crate::codecs::amr::nb::homing as nb_homing;

        let root = narrowband_root();
        let coded = read_narrowband_serial(&root.join("c-code/spch_dos.cod"));
        let want = narrowband_frames(&root.join("c-code/spch_dos.out"));
        assert!(want.len() >= 400, "only {} output frames", want.len());

        let mut decoder = Decoder::new();
        // `reset_flag_old` starts at 1: the decoder begins homed.
        let mut homed = true;
        let (mut compared, mut kinds, mut homings) = (0usize, [0usize; 3], 0usize);
        let mut prev_mode = 0u8;

        for (n, frame) in coded.iter().enumerate().take(want.len()) {
            let mode = if frame.tx_type == 3 {
                prev_mode
            } else {
                let m = u8::try_from(frame.mode).expect("a speech mode");
                prev_mode = m;
                m
            };

            // The serial words are codec bits in parameter order; the codec
            // speaks the sorted payload. Apply the permutation -- and note
            // which table: a SID uses its own 35-entry one, never the speech
            // mode's.
            let sid = matches!(frame.tx_type, 1 | 2 | 6);
            let sort: &[u16] = if sid {
                &crate::codecs::amr::nb::tables::SORT_SID
            } else {
                crate::codecs::amr::nb::bitstream::sort_table_for(mode)
            };
            let mut payload = vec![0u8; sort.len().div_ceil(8)];
            for (i, &source) in sort.iter().enumerate() {
                let bit = frame.bits[source as usize] & 1;
                payload[i / 8] |= u8::try_from(bit).expect("one bit") << (7 - (i % 8));
            }

            let rx = match frame.tx_type {
                0 => RxFrameType::SpeechGood,
                1 => RxFrameType::SidFirst,
                2 => RxFrameType::SidUpdate,
                3 => RxFrameType::NoData,
                4 => RxFrameType::SpeechDegraded,
                5 => RxFrameType::SpeechBad,
                6 => RxFrameType::SidBad,
                other => panic!("frame {n}: unexpected TX type {other}"),
            };
            kinds[match rx {
                RxFrameType::SidFirst | RxFrameType::SidUpdate | RxFrameType::SidBad => 1,
                RxFrameType::NoData | RxFrameType::Onset => 2,
                _ => 0,
            }] += 1;

            // A decoder already homed compares only the first subframe; one
            // that is not compares the whole frame. Checked before decoding,
            // exactly where the driver checks it.
            let homing_now = if homed {
                nb_homing::is_decoder_homing_frame_first(&payload, mode)
            } else {
                false
            };

            let got = if homing_now && homed {
                [nb_homing::HOMING_SAMPLE; 160]
            } else {
                let params = match rx {
                    RxFrameType::SidUpdate | RxFrameType::SidBad => {
                        crate::codecs::amr::nb::bitstream::parse(8, &payload)
                            .expect("a SID parses")
                    }
                    RxFrameType::NoData | RxFrameType::SidFirst | RxFrameType::Onset => Vec::new(),
                    _ => crate::codecs::amr::nb::bitstream::parse(mode, &payload)
                        .expect("speech parses"),
                };
                decoder.decode_typed(rx, mode, &params)
            };

            for (i, &sample) in got.iter().enumerate() {
                assert_eq!(sample, want[n][i] & !7, "frame {n} sample {i}");
                compared += 1;
            }

            // And after: an unhomed decoder tests the whole frame.
            let reset = if homed {
                homing_now
            } else {
                nb_homing::is_decoder_homing_frame(&payload, mode)
            };
            if reset {
                decoder.reset();
                homings += 1;
            }
            homed = reset;
        }

        assert_eq!(compared, 425 * 160, "the sample count changed; the stream is fixed");
        assert_eq!(kinds, [242, 26, 157], "the frame-type mix changed");
        assert_eq!(homings, 0, "this stream has no homing frame; see the module header");
    }

    /// One frame of the input sequence.    /// One frame of the input sequence.
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
