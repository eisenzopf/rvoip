//! Long-run stability for both AMR variants.
//!
//! # What this covers that nothing else does
//!
//! The conformance sequences prove *correctness* — bit-exactness against the
//! normative references — over a few hundred frames. The fuzz targets prove
//! *robustness* against arbitrary input for a bounded time. Neither answers
//! the question an operator actually has: does a call that runs for hours,
//! changing rate and dropping packets the whole way, stay healthy?
//!
//! The failure modes this is aimed at are the ones that need time to appear:
//!
//! - **Unbounded growth.** Every buffer in this codec is fixed-size by
//!   construction, so an allocation that grows with frame count would be a
//!   bug — but "by construction" is exactly the kind of claim that rots. The
//!   test watches resident memory across the run.
//! - **State drift.** The LSF and gain predictors, the pitch history, the DTX
//!   hangover rings and the concealment state all carry across frames, and
//!   across mode changes they carry across a change in what their numbers
//!   *mean*. A slow divergence — a predictor walking toward saturation over
//!   thousands of frames — produces audio that degrades rather than breaks,
//!   which no single-frame assertion catches.
//! - **Concealment recovery.** Loss is interleaved throughout rather than
//!   clustered at the start, so the decoder has to recover repeatedly from a
//!   state that is itself the product of earlier recoveries.
//!
//! # Why it is `#[ignore]`d
//!
//! The default run is ~30 seconds of speech per variant, which is too slow for
//! `cargo test` on every change and far too short to be a real soak. The knob
//! is `RVOIP_AMR_SOAK_SECS` (speech-seconds simulated per variant); the driver
//! `tools/run-amr-soak.sh` sets it. See that script for the recorded baseline.
//!
//! # What it does not do
//!
//! It does not assert audio *quality* frame by frame — that is the
//! conformance suite's job and it does it bit-exactly. Here the output
//! assertions are structural (length, finite range, not-all-silence over a
//! window), because the point is that ten thousand frames later the codec is
//! still producing well-formed frames of the right size from a state that has
//! been churned the whole way.

use super::mode::{AmrMode, AmrVariant};
use super::AmrCodec;
use crate::types::{CodecConfig, CodedFrame, FrameKind, VariableRateCodec};

/// Speech-seconds simulated per variant. Deliberately small by default so a
/// developer who runs `--ignored` by hand is not stuck for an hour.
fn soak_seconds() -> u64 {
    std::env::var("RVOIP_AMR_SOAK_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30)
}

/// Resident set size in kilobytes, or `None` where the platform is not one we
/// know how to ask. Absence must not fail the test: the growth check is one of
/// several signals, and a soak that refuses to run on an unfamiliar platform
/// is a soak that stops being run.
///
/// This measures the whole process, so it is a coarse ceiling rather than a
/// per-codec delta: the two variants' tests share one test binary, and under
/// the default parallel harness each would see the other's allocations.
/// `tools/run-amr-soak.sh` passes `--test-threads=1` so the readings are at
/// least sequential. The claim being checked is "does not grow with frame
/// count", which the driver verifies by comparing runs of different lengths.
fn resident_kb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                return rest.split_whitespace().next()?.parse().ok();
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        // `ps` rather than task_info(): no unsafe, no extra dependency, and
        // this is sampled a handful of times across a multi-minute run.
        let pid = std::process::id();
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8(output.stdout).ok()?.trim().parse().ok()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// A deterministic signal that keeps the analysis doing real work: two
/// harmonics plus a little noise, with a slow amplitude envelope so the
/// energy-driven decisions (scaling, VAD, gain prediction) keep moving rather
/// than settling. Silence for one stretch in every eight so DTX has something
/// to detect when it is enabled.
// Synthesis arithmetic, not codec arithmetic: f64 phase accumulation and a
// clamped narrowing to i16. The clamp bounds the value before the cast, and
// the index-to-f64 conversion is exact for any run length reachable here
// (2^52 frames is 2.8 million years of speech).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::suboptimal_flops
)]
fn frame_samples(variant: AmrVariant, index: usize) -> Vec<i16> {
    let samples = variant.frame_samples();
    let rate = f64::from(variant.sample_rate());
    // A silent stretch every 8th block of 50 frames.
    if (index / 50) % 8 == 7 {
        return vec![0i16; samples];
    }
    let mut seed =
        0x2545_f491u32.wrapping_add(u32::try_from(index % u32::MAX as usize).unwrap_or(0));
    (0..samples)
        .map(|i| {
            let t = ((index * samples + i) as f64) / rate;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = f64::from((seed >> 22) as i16 % 128) * 1.5;
            // Envelope period is deliberately not a multiple of the frame
            // duration, so frames do not all see the same phase of it.
            let envelope = 0.55 + 0.45 * (t * 0.37 * std::f64::consts::TAU).sin();
            let tone = (t * 230.0 * std::f64::consts::TAU).sin() * 7000.0
                + (t * 690.0 * std::f64::consts::TAU).sin() * 2600.0;
            ((tone * envelope) + noise).clamp(-32000.0, 32000.0) as i16
        })
        .collect()
}

// Frame counts converted to f64 for the energy average; exact at any run
// length this test can reach.
#[allow(clippy::cast_precision_loss)]
fn soak_variant(variant: AmrVariant) {
    let seconds = soak_seconds();
    let frames_per_second = 50; // 20 ms frame-blocks
    let total_frames = usize::try_from(seconds).unwrap_or(usize::MAX) * frames_per_second;
    let modes = AmrMode::all(variant);
    assert!(!modes.is_empty(), "variant must have modes");

    let mut config = match variant {
        AmrVariant::NarrowBand => CodecConfig::amr_nb(),
        AmrVariant::WideBand => CodecConfig::amr_wb(),
    };
    // All modes permitted: the run drives the whole ladder.
    config.parameters.amr.mode_set = 0;
    config.parameters.amr.dtx = true;

    let mut encoder = AmrCodec::new(&config).expect("encoder constructs");
    let mut decoder = AmrCodec::new(&config).expect("decoder constructs");

    let expected_samples = variant.frame_samples();
    let baseline_kb = resident_kb();
    let mut peak_kb = baseline_kb.unwrap_or(0);
    let mut decoded_frames = 0usize;
    let mut concealed_frames = 0usize;
    let mut dtx_frames = 0usize;
    // Rolling energy over the last second of decoded audio, so a codec that
    // silently degrades into permanent silence is caught even though any one
    // frame being quiet is legitimate.
    let mut window_energy: f64 = 0.0;
    let mut window_frames = 0usize;
    let mut silent_windows = 0usize;

    for index in 0..total_frames {
        // Mode churn: walk the ladder, changing every 7 frames so the change
        // lands at varying offsets within DTX hangover and the gain
        // predictor's own history.
        if index % 7 == 0 {
            let mode = modes[(index / 7) % modes.len()];
            encoder
                .set_mode(mode.index())
                .expect("every mode in the set is selectable");
        }

        let pcm = frame_samples(variant, index);
        let coded = encoder.encode_frame(&pcm).expect("speech frame encodes");

        // Loss pattern: an isolated frame every 43, and a burst of three every
        // 349 — both prime-ish so they drift against the mode-change and DTX
        // cycles rather than aligning with them.
        let isolated = index % 43 == 42;
        let burst = (index % 349) >= 346;
        let lost = isolated || burst;

        let output = if lost {
            concealed_frames += 1;
            decoder
                .decode_frame(&CodedFrame {
                    kind: FrameKind::Lost,
                    mode: coded.mode,
                    quality_ok: false,
                    data: Vec::new(),
                })
                .expect("concealment produces a frame")
        } else {
            if coded.kind != FrameKind::Speech {
                dtx_frames += 1;
            }
            decoder.decode_frame(&coded).expect("coded frame decodes")
        };

        assert_eq!(
            output.len(),
            expected_samples,
            "frame {index}: decoder must always produce a full frame"
        );
        decoded_frames += 1;

        let energy: f64 = output
            .iter()
            .map(|&sample| f64::from(sample) * f64::from(sample))
            .sum();
        window_energy += energy;
        window_frames += 1;
        if window_frames == frames_per_second {
            // The source is silent for one stretch in eight, so a silent
            // second is expected; a *run* of them is not. Count them and
            // assert on the total at the end.
            if window_energy / (window_frames * expected_samples) as f64 <= 1.0 {
                silent_windows += 1;
            }
            window_energy = 0.0;
            window_frames = 0;
        }

        // Sample memory a few times rather than every frame: `ps` is a fork.
        if index % 2_000 == 0 {
            if let Some(kb) = resident_kb() {
                peak_kb = peak_kb.max(kb);
            }
        }
    }

    assert_eq!(decoded_frames, total_frames);
    assert!(
        concealed_frames > 0,
        "the loss pattern must actually have concealed frames"
    );

    // Roughly one second in eight is silent by construction; allow generous
    // slack for DTX and window alignment, but catch a codec that has fallen
    // permanently silent.
    let seconds_run = total_frames / frames_per_second;
    assert!(
        silent_windows * 3 < seconds_run.max(3),
        "decoded audio was silent for {silent_windows} of {seconds_run} seconds — \
         the codec appears to have degraded into silence"
    );

    if let (Some(baseline), true) = (baseline_kb, peak_kb > 0) {
        // Every buffer here is fixed-size, so the steady state should be flat.
        // The allowance covers allocator behaviour and the sampling itself,
        // not growth proportional to frame count.
        let allowed = baseline + 64 * 1024;
        assert!(
            peak_kb <= allowed,
            "resident memory grew from {baseline} kB to {peak_kb} kB over \
             {total_frames} frames — expected a flat steady state"
        );
    }

    println!(
        "{variant:?}: {total_frames} frames ({seconds}s), {concealed_frames} concealed, \
         {dtx_frames} non-speech, rss {baseline_kb:?} kB -> {peak_kb} kB"
    );
}

#[test]
#[ignore = "long-running; see tools/run-amr-soak.sh"]
#[cfg(feature = "amr-nb")]
fn narrowband_survives_a_long_call() {
    soak_variant(AmrVariant::NarrowBand);
}

#[test]
#[ignore = "long-running; see tools/run-amr-soak.sh"]
#[cfg(feature = "amr-wb")]
fn wideband_survives_a_long_call() {
    soak_variant(AmrVariant::WideBand);
}
