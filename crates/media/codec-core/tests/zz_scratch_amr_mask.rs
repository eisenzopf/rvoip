//! TEMPORARY scratch harness — delete after use.
#![cfg(feature = "amr-wb")]

use codec_core::codecs::amr::mode::{AmrMode, AmrVariant};
use codec_core::codecs::amr::storage;
use codec_core::codecs::amr::wb::params::FrameParams;

fn data(name: &str) -> Vec<u8> {
    let p = format!(
        "{}/src/codecs/amr/testdata/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn vad_flags() {
    for m in 0..9usize {
        let bits = data(&format!("amrwb_mode{m}.amr"));
        let (_, frames) = storage::read(&bits).expect("parse");
        let mode = AmrMode::new(AmrVariant::WideBand, m as u8).expect("mode");
        let flags: Vec<u8> = frames
            .iter()
            .map(|f| u8::from(FrameParams::parse(mode, &f.data).expect("p").vad_flag))
            .collect();
        // Emulate the reference's vad_hist counter.
        let mut hist = 0i32;
        let hists: Vec<i32> = flags
            .iter()
            .map(|&v| {
                if v == 0 {
                    hist += 1;
                } else {
                    hist = 0;
                }
                hist
            })
            .collect();
        println!("mode {m}: vad {flags:?}\n         vad_hist {hists:?}");
    }
}
