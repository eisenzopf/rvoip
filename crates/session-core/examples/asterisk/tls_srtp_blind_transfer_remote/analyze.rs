//! Analyze WAV files from the Asterisk TLS/SRTP blind transfer example.

#[path = "../common.rs"]
mod common;

use common::{
    assert_audio_path, assert_samples_tone, endpoint_config, init_tracing, load_env,
    print_analysis, read_wav, ExampleResult, ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ,
    ENDPOINT_1003_TONE_HZ, SAMPLE_RATE,
};

const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize;
const MIN_TRANSFEREE_SAMPLES: usize = WINDOW_SAMPLES * 2;

fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1001", 5070, 16000, 16100)?;
    let transferor_wav = cfg
        .output_dir
        .join("tls_srtp_blind_transfer_1001_received.wav");
    let transferee_wav = cfg
        .output_dir
        .join("tls_srtp_blind_transfer_1002_received.wav");
    let target_wav = cfg
        .output_dir
        .join("tls_srtp_blind_transfer_1003_received.wav");

    let transferor = assert_audio_path(
        &transferor_wav,
        ENDPOINT_1002_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;
    let target = assert_audio_path(&target_wav, ENDPOINT_1002_TONE_HZ, ENDPOINT_1003_TONE_HZ)?;

    let transferee_samples = read_wav(&transferee_wav)?;
    if transferee_samples.len() < MIN_TRANSFEREE_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            transferee_wav.display(),
            transferee_samples.len(),
            MIN_TRANSFEREE_SAMPLES
        )
        .into());
    }
    let first_window = &transferee_samples[..WINDOW_SAMPLES];
    let last_window = &transferee_samples[transferee_samples.len() - WINDOW_SAMPLES..];
    let transferee_initial = assert_samples_tone(
        "1002 initial leg received 1001 tone",
        first_window,
        ENDPOINT_1001_TONE_HZ,
        ENDPOINT_1003_TONE_HZ,
    )?;
    let transferee_transferred = assert_samples_tone(
        "1002 transferred leg received 1003 tone",
        last_window,
        ENDPOINT_1003_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;

    println!("=== Asterisk TLS/SRTP blind transfer audio analysis ===");
    print_analysis(
        "1001 received 1002 initial-leg tone",
        &transferor_wav,
        &transferor,
    );
    print_analysis(
        "1003 received 1002 transferred-leg tone",
        &target_wav,
        &target,
    );
    print_analysis(
        "1002 initial window received 1001 tone",
        &transferee_wav,
        &transferee_initial,
    );
    print_analysis(
        "1002 final window received 1003 tone",
        &transferee_wav,
        &transferee_transferred,
    );
    println!("TLS/SRTP blind transfer audio path verification passed.");

    Ok(())
}
