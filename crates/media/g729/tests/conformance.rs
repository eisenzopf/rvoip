//! ITU G.729 conformance tests integrated in Rust.

#![cfg(all(feature = "std", feature = "itu_serial"))]

mod common;

use g729::{EncoderConfig, FrameType, G729Encoder};

#[cfg(feature = "annex_b")]
use common::itu_format::decode_annex_b_bit_file;
use common::itu_format::{
    compare_word_frames, decode_annex_a_bit_file, read_serial_frames, sid_words_from_payload,
    speech_words_from_payload,
};
use common::pcm_format::{compare_pcm, read_pcm_file};
use common::{annex_a_vectors, annex_b_vectors};

fn annex_a_pair(name: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let d = annex_a_vectors();
    let bit = d.join(format!("{}.BIT", name));

    let pst_upper = d.join(format!("{}.PST", name));
    let pst_lower = d.join(format!("{}.pst", name));
    let pst = if pst_upper.exists() {
        pst_upper
    } else {
        pst_lower
    };

    if !bit.exists() || !pst.exists() {
        eprintln!(
            "skipping Annex A decoder vector {name}: missing {} or {}",
            bit.display(),
            pst.display()
        );
        return None;
    }

    Some((bit, pst))
}

fn run_annex_a_decoder(name: &str) {
    let Some((bit, pst)) = annex_a_pair(name) else {
        return;
    };
    let test_pcm = decode_annex_a_bit_file(&bit);
    let ref_pcm = read_pcm_file(&pst);

    let (matches, total, max_error, first_diverge) = compare_pcm(&ref_pcm, &test_pcm, name);
    assert_eq!(
        matches, total,
        "{}: not bit exact (max_error={}, first_diverge={:?})",
        name, max_error, first_diverge
    );
}

fn run_annex_a_encoder(name: &str) {
    let d = annex_a_vectors();
    let in_path = d.join(format!("{}.IN", name));
    let bit_path = d.join(format!("{}.BIT", name));
    if !in_path.exists() || !bit_path.exists() {
        eprintln!(
            "skipping Annex A encoder vector {name}: missing {} or {}",
            in_path.display(),
            bit_path.display()
        );
        return;
    }

    let ref_frames = read_serial_frames(&bit_path);
    let pcm = read_pcm_file(&in_path);

    let mut encoder = G729Encoder::new(EncoderConfig { annex_b: false });
    let mut test_frames = Vec::new();

    for chunk in pcm.chunks_exact(80) {
        let mut frame = [0i16; 80];
        frame.copy_from_slice(chunk);

        let mut packed = [0u8; 10];
        let ftype = encoder.encode(&frame, &mut packed);
        let words = match ftype {
            FrameType::Speech => speech_words_from_payload(&packed),
            FrameType::Sid => sid_words_from_payload(&packed),
            FrameType::NoData => Vec::new(),
        };
        test_frames.push(words);
    }

    let (matches, total, first_diverge) = compare_word_frames(&ref_frames, &test_frames, name);
    assert_eq!(
        matches, total,
        "{}: not bit exact (first_diverge={:?})",
        name, first_diverge
    );
}

#[test]
fn conformance_annex_a_dec_algthm() {
    run_annex_a_decoder("ALGTHM");
}

#[test]
fn conformance_annex_a_dec_erasure() {
    run_annex_a_decoder("ERASURE");
}

#[test]
fn conformance_annex_a_dec_fixed() {
    run_annex_a_decoder("FIXED");
}

#[test]
fn conformance_annex_a_dec_lsp() {
    run_annex_a_decoder("LSP");
}

#[test]
fn conformance_annex_a_dec_overflow() {
    run_annex_a_decoder("OVERFLOW");
}

#[test]
fn conformance_annex_a_dec_parity() {
    run_annex_a_decoder("PARITY");
}

#[test]
fn conformance_annex_a_dec_pitch() {
    run_annex_a_decoder("PITCH");
}

#[test]
fn conformance_annex_a_dec_speech() {
    run_annex_a_decoder("SPEECH");
}

#[test]
fn conformance_annex_a_dec_tame() {
    run_annex_a_decoder("TAME");
}

#[test]
fn conformance_annex_a_dec_test() {
    let d = annex_a_vectors();
    if d.join("TEST.BIT").exists() {
        run_annex_a_decoder("TEST");
    }
}

#[test]
fn conformance_annex_a_enc_algthm() {
    run_annex_a_encoder("ALGTHM");
}

#[test]
fn conformance_annex_a_enc_fixed() {
    run_annex_a_encoder("FIXED");
}

#[test]
fn conformance_annex_a_enc_lsp() {
    run_annex_a_encoder("LSP");
}

#[test]
fn conformance_annex_a_enc_pitch() {
    run_annex_a_encoder("PITCH");
}

#[test]
fn conformance_annex_a_enc_speech() {
    run_annex_a_encoder("SPEECH");
}

#[test]
fn conformance_annex_a_enc_tame() {
    run_annex_a_encoder("TAME");
}

#[test]
fn conformance_annex_a_enc_test() {
    let d = annex_a_vectors();
    if d.join("TEST.IN").exists() && d.join("TEST.BIT").exists() {
        run_annex_a_encoder("TEST");
    }
}

#[cfg(feature = "annex_b")]
fn run_annex_b_decoder(bit_name: &str, out_name: &str) {
    let d = annex_b_vectors();
    let bit = d.join(bit_name);
    let out = d.join(out_name);
    if !bit.exists() || !out.exists() {
        eprintln!(
            "skipping Annex B decoder vector {bit_name}: missing {} or {}",
            bit.display(),
            out.display()
        );
        return;
    }

    let test_pcm = decode_annex_b_bit_file(&bit);
    let ref_pcm = read_pcm_file(&out);

    let (matches, total, max_error, first_diverge) = compare_pcm(&ref_pcm, &test_pcm, bit_name);
    assert_eq!(
        matches, total,
        "{}: not bit exact (max_error={}, first_diverge={:?})",
        bit_name, max_error, first_diverge
    );
}

#[cfg(feature = "annex_b")]
fn run_annex_b_encoder(seq: usize) {
    let d = annex_b_vectors();
    let in_path = d.join(format!("tstseq{}.bin", seq));
    let bit_path = d.join(format!("tstseq{}a.bit", seq));
    if !in_path.exists() || !bit_path.exists() {
        eprintln!(
            "skipping Annex B encoder vector {seq}: missing {} or {}",
            in_path.display(),
            bit_path.display()
        );
        return;
    }

    let ref_frames = read_serial_frames(&bit_path);
    let pcm = read_pcm_file(&in_path);

    let mut encoder = G729Encoder::new(EncoderConfig { annex_b: true });
    let mut test_frames = Vec::new();

    for chunk in pcm.chunks_exact(80) {
        let mut frame = [0i16; 80];
        frame.copy_from_slice(chunk);

        let mut packed = [0u8; 10];
        let ftype = encoder.encode(&frame, &mut packed);
        let words = match ftype {
            FrameType::Speech => speech_words_from_payload(&packed),
            FrameType::Sid => sid_words_from_payload(&packed),
            FrameType::NoData => Vec::new(),
        };
        test_frames.push(words);
    }

    let name = format!("AnnexB_Enc_tstseq{}", seq);
    let (matches, total, first_diverge) = compare_word_frames(&ref_frames, &test_frames, &name);
    assert_eq!(
        matches, total,
        "{}: not bit exact (first_diverge={:?})",
        name, first_diverge
    );
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_dec_tstseq1() {
    run_annex_b_decoder("tstseq1a.bit", "tstseq1a.out");
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_dec_tstseq2() {
    run_annex_b_decoder("tstseq2a.bit", "tstseq2a.out");
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_dec_tstseq3() {
    run_annex_b_decoder("tstseq3a.bit", "tstseq3a.out");
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_dec_tstseq4() {
    run_annex_b_decoder("tstseq4a.bit", "tstseq4a.out");
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_dec_tstseq5() {
    run_annex_b_decoder("tstseq5.bit", "tstseq5a.out");
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_dec_tstseq6() {
    run_annex_b_decoder("tstseq6.bit", "tstseq6a.out");
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_enc_tstseq1() {
    run_annex_b_encoder(1);
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_enc_tstseq2() {
    run_annex_b_encoder(2);
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_enc_tstseq3() {
    run_annex_b_encoder(3);
}

#[cfg(feature = "annex_b")]
#[test]
fn conformance_annex_b_enc_tstseq4() {
    run_annex_b_encoder(4);
}
