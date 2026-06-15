use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use g729::bitstream::itu_serial::read_serial_frame;
use g729::constants::PRM_SIZE;
use g729::{FRAME_SAMPLES, G729Config, G729Decoder};

fn read_pcm_s16le(path: &Path) -> Result<Vec<i16>, String> {
    let raw = fs::read(path).map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    let mut out = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.chunks_exact(2) {
        out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(out)
}

fn decode_serial(path: &Path) -> Result<Vec<i16>, String> {
    let file = File::open(path).map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    let mut decoder = G729Decoder::new(G729Config::default());
    let mut out = Vec::new();

    loop {
        let mut parm = [0i16; PRM_SIZE + 2 + 4];
        let mut bfi = 0i16;
        let status = read_serial_frame(&mut reader, &mut parm, &mut bfi)
            .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
        if status == 0 {
            break;
        }

        let mut pcm = [0i16; FRAME_SAMPLES];
        decoder
            .decode_parm(&mut parm, &mut pcm)
            .map_err(|e| format!("decode_parm failed: {e}"))?;
        out.extend_from_slice(&pcm);
    }

    Ok(out)
}

#[derive(Clone, Copy)]
pub(crate) enum VectorMode {
    AnnexA,
    AnnexB,
}

impl VectorMode {
    pub(crate) fn parse(arg: Option<String>) -> Result<Self, String> {
        match arg.as_deref() {
            None | Some("annex-a") => Ok(Self::AnnexA),
            Some("annex-b") => Ok(Self::AnnexB),
            Some(other) => Err(format!("unknown vector mode '{other}'")),
        }
    }
}

pub(crate) fn run_test_vectors(
    input: &Path,
    expected: &Path,
    _mode: VectorMode,
) -> Result<(), String> {
    let actual = decode_serial(input)?;
    let reference = read_pcm_s16le(expected)?;

    if reference.len() != actual.len() {
        return Err(format!(
            "sample count mismatch: expected {} samples, decoded {} samples",
            reference.len(),
            actual.len()
        ));
    }

    for (idx, (&exp, &got)) in reference.iter().zip(actual.iter()).enumerate() {
        if exp != got {
            return Err(format!(
                "mismatch at sample {idx}: expected {exp}, got {got}"
            ));
        }
    }

    println!(
        "PASS: {} matched {} sample(s)",
        expected.display(),
        reference.len()
    );
    Ok(())
}
