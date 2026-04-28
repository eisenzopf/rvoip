//! Generic analyzer for Asterisk CallbackPeer example WAV files.

mod common;

use common::{
    analyze_samples, assert_audio_path, assert_samples_tone, endpoint_config, load_env,
    print_analysis, read_wav, ExampleResult, ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ,
    ENDPOINT_1003_TONE_HZ, ENDPOINT_2001_TONE_HZ, ENDPOINT_2002_TONE_HZ, SAMPLE_RATE,
};

const POST_RESUME_TONE_HZ: f32 = 660.0;
const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize;
const MIN_CALLER_SAMPLES: usize = WINDOW_SAMPLES * 2;

fn main() -> ExampleResult<()> {
    load_env();
    common::init_tracing();

    match std::env::var("CALLBACK_ANALYZE")?.as_str() {
        "tls_hold" => analyze_hold(true),
        "udp_hold" => analyze_hold(false),
        "tls_dtmf" => analyze_dtmf(),
        "tls_transfer" => analyze_transfer(),
        other => Err(format!("unknown CALLBACK_ANALYZE '{}'", other).into()),
    }
}

fn analyze_hold(tls: bool) -> ExampleResult<()> {
    let cfg = if tls {
        endpoint_config("1001", 5070, 16000, 16100)?
    } else {
        endpoint_config("2001", 5080, 17000, 17100)?
    };
    let (caller_wav, callee_wav, caller_tone, callee_tone, label) = if tls {
        (
            cfg.output_dir
                .join("tls_srtp_hold_resume_1001_received.wav"),
            cfg.output_dir
                .join("tls_srtp_hold_resume_1002_received.wav"),
            ENDPOINT_1001_TONE_HZ,
            ENDPOINT_1002_TONE_HZ,
            "TLS/SRTP callback hold/resume",
        )
    } else {
        (
            cfg.output_dir.join("hold_resume_2001_received.wav"),
            cfg.output_dir.join("hold_resume_2002_received.wav"),
            ENDPOINT_2001_TONE_HZ,
            ENDPOINT_2002_TONE_HZ,
            "UDP callback hold/resume",
        )
    };

    let caller = assert_audio_path(&caller_wav, callee_tone, caller_tone)?;
    let callee_samples = read_wav(&callee_wav)?;
    if callee_samples.len() < MIN_CALLER_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            callee_wav.display(),
            callee_samples.len(),
            MIN_CALLER_SAMPLES
        )
        .into());
    }
    let first_window = &callee_samples[..WINDOW_SAMPLES];
    let last_window = &callee_samples[callee_samples.len() - WINDOW_SAMPLES..];
    let pre_hold = assert_samples_tone(
        "callee pre-hold caller tone",
        first_window,
        caller_tone,
        POST_RESUME_TONE_HZ,
    )?;
    let post_resume = assert_samples_tone(
        "callee post-resume caller tone",
        last_window,
        POST_RESUME_TONE_HZ,
        caller_tone,
    )?;
    let during_hold_probe = analyze_samples(&callee_samples, 550.0, caller_tone)?;

    println!("=== Asterisk {} audio analysis ===", label);
    print_analysis(
        "caller received callee reference tone",
        &caller_wav,
        &caller,
    );
    print_analysis("callee pre-hold caller tone", &callee_wav, &pre_hold);
    print_analysis("callee post-resume caller tone", &callee_wav, &post_resume);
    println!(
        "callee during-hold 550Hz probe magnitude {:.1} (informational)",
        during_hold_probe.expected_magnitude
    );
    println!("{} audio path verification passed.", label);
    Ok(())
}

fn analyze_dtmf() -> ExampleResult<()> {
    let cfg = endpoint_config("1001", 5070, 16000, 16100)?;
    let caller_wav = cfg.output_dir.join("tls_srtp_dtmf_1001_received.wav");
    let callee_wav = cfg.output_dir.join("tls_srtp_dtmf_1002_received.wav");
    let caller = assert_audio_path(&caller_wav, ENDPOINT_1002_TONE_HZ, ENDPOINT_1001_TONE_HZ)?;
    let callee = assert_audio_path(&callee_wav, ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ)?;
    println!("=== Asterisk TLS/SRTP callback DTMF audio analysis ===");
    print_analysis("1001 received 1002 tone", &caller_wav, &caller);
    print_analysis("1002 received 1001 tone", &callee_wav, &callee);
    println!("TLS/SRTP callback DTMF audio path verification passed.");
    Ok(())
}

fn analyze_transfer() -> ExampleResult<()> {
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
    if transferee_samples.len() < MIN_CALLER_SAMPLES {
        return Err(format!("{} too short", transferee_wav.display()).into());
    }
    let first_window = &transferee_samples[..WINDOW_SAMPLES];
    let last_window = &transferee_samples[transferee_samples.len() - WINDOW_SAMPLES..];
    let initial = assert_samples_tone(
        "1002 initial leg received 1001 tone",
        first_window,
        ENDPOINT_1001_TONE_HZ,
        ENDPOINT_1003_TONE_HZ,
    )?;
    let transferred = assert_samples_tone(
        "1002 transferred leg received 1003 tone",
        last_window,
        ENDPOINT_1003_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;
    println!("=== Asterisk TLS/SRTP callback blind transfer audio analysis ===");
    print_analysis("1001 received 1002 tone", &transferor_wav, &transferor);
    print_analysis("1003 received 1002 tone", &target_wav, &target);
    print_analysis("1002 initial window", &transferee_wav, &initial);
    print_analysis("1002 final window", &transferee_wav, &transferred);
    println!("TLS/SRTP callback blind transfer audio path verification passed.");
    Ok(())
}
