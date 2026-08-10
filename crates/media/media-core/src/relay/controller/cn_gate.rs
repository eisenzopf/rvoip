//! VAD-driven Comfort Noise (RFC 3389) gating for the audio TX path.
//!
//! Wraps the lower-level [`super::cn_transmitter::CnTransmitter`] in a
//! per-session state machine that:
//!
//! - runs a simple energy/ZCR VAD over each outbound PCM frame,
//! - on the first speech→silence transition emits one PT 13 packet
//!   carrying the noise level (CN), then suppresses subsequent audio
//!   packets while silence persists,
//! - re-emits CN every ~200 ms (RFC 3389 §4.1) so the receiver's PLC
//!   model gets a fresh level reference,
//! - resumes normal audio TX on the silence→speech transition.
//!
//! Construction is per-session; one instance lives in
//! [`MediaSessionController`](super::MediaSessionController) keyed by
//! `DialogId`. The gate is consulted from
//! [`encode_and_send_audio_frame`](super::rtp_management) before
//! encoding; the decision determines whether the PCM frame becomes a
//! G.711 packet on the wire or gets suppressed in favour of CN.
//!
//! Out of scope: spectral side-information (RFC 3389 §3.2) — `level`
//! only. Receivers that don't support spectral CN fall back to a flat
//! noise model from the level byte alone.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{debug, trace};

use rvoip_rtp_core::RtpSession;

use crate::error::Result;
use crate::processing::audio::{VadConfig, VadResult, VoiceActivityDetector};
use crate::types::AudioFrame;

use super::cn_transmitter::{CnTransmitter, DEFAULT_NOISE_LEVEL_DBOV};

/// Re-emit CN at this cadence while silence persists. RFC 3389 §4.1
/// recommends ~200 ms — short enough that brief packet loss doesn't
/// strand the receiver in stale-noise PLC, long enough that the
/// bandwidth saving over continuous audio (one 1-byte packet vs ~50
/// 160-byte packets per second) is meaningful.
const CN_REFRESH_INTERVAL: Duration = Duration::from_millis(200);

/// Whether a PT 13 comfort-noise packet may legally ride in this codec's
/// stream.
///
/// The gate used to ask whether the clock rate was 8 kHz, which answers a
/// different question. RFC 3389 CN is a *separate payload type* interleaved
/// into the stream, so it is only available where the receiver's depacketizer
/// tolerates a foreign payload type and the codec does not already signal
/// silence in band. Clock rate says nothing about either. AMR-NB is 8 kHz and
/// fails both halves: RFC 4867 §4 leaves no room for PT 13 between AMR frames,
/// and AMR carries its own SID frames — so the old test would have started
/// suppressing audio and emitting unparseable packets the moment AMR reached
/// the relay path.
///
/// This is an allow-list rather than a deny-list, so a codec added later is
/// excluded until someone classifies it deliberately. That failure mode costs
/// a bandwidth optimisation; the other one corrupts a call.
pub(super) fn supports_rfc3389_comfort_noise(codec_name: &str) -> bool {
    let normalized = super::codec_runtime::normalized_codec_name(codec_name);
    matches!(normalized.as_str(), "PCMU" | "PCMA") || normalized.starts_with("G729")
}

/// Decision returned by [`CnGate::process_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CnGateDecision {
    /// Frame is speech (or VAD failed open) — caller should encode and
    /// send normally.
    SendAudio,
    /// Silence detected and a CN packet was already emitted in this
    /// silence window — caller should drop the audio frame entirely.
    SuppressAudio,
    /// Silence detected and a CN packet must be emitted now (either
    /// the first frame of a silence run, or the periodic refresh).
    /// Caller should drop the audio frame and call
    /// [`CnGate::emit_cn_now`] (which uses [`CnTransmitter`] under the
    /// hood). Returned `level` is the encoded -dBov noise level.
    EmitCnThenSuppress { level: u8 },
}

/// Per-session state for the VAD + CN gate.
///
/// Held inside an outer `Mutex` (one entry per dialog) by the media
/// controller; the gate's own mutation (smoothed-energy, last-CN
/// timestamp, in-silence flag) happens behind that single Mutex so
/// concurrent encode-and-send calls for the same dialog see a
/// consistent decision.
pub struct CnGate {
    vad: VoiceActivityDetector,
    cn: Arc<CnTransmitter>,
    /// `true` when the last frame analysed was classified as silence.
    /// Edge-triggers the first CN emission of each silence run.
    in_silence: bool,
    /// When CN was last emitted on the wire. Drives the §4.1 refresh.
    last_cn_emitted: Option<Instant>,
}

impl CnGate {
    /// Construct a gate for the supplied RTP session. The VAD uses the
    /// default config (`VadConfig::default`); callers wanting custom
    /// thresholds can use [`Self::new_with_vad_config`].
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Result<Self> {
        Self::new_with_vad_config(rtp_session, VadConfig::default())
    }

    /// Construct a gate with a custom VAD configuration.
    pub fn new_with_vad_config(
        rtp_session: Arc<Mutex<RtpSession>>,
        vad_config: VadConfig,
    ) -> Result<Self> {
        let vad = VoiceActivityDetector::new(vad_config)?;
        let cn = Arc::new(CnTransmitter::new(rtp_session));
        Ok(Self {
            vad,
            cn,
            in_silence: false,
            last_cn_emitted: None,
        })
    }

    /// Analyse a PCM audio frame and decide whether the caller should
    /// send it as audio, suppress it (silence already covered by a
    /// recent CN), or emit a fresh CN packet now and then suppress.
    ///
    /// Updates internal silence/refresh state. The frame is *not*
    /// consumed — the caller still owns the samples and may pass them
    /// on (or not) based on the decision.
    pub fn process_frame(&mut self, frame: &AudioFrame) -> CnGateDecision {
        // VAD requires a minimum frame length; below that we fail
        // open and send the audio so we don't break degenerate
        // single-sample test fixtures.
        let vad_result: VadResult = match self.vad.analyze_frame(frame) {
            Ok(r) => r,
            Err(_) => {
                trace!("VAD analyze_frame failed — falling open to SendAudio");
                self.in_silence = false;
                return CnGateDecision::SendAudio;
            }
        };

        if vad_result.is_voice {
            // Speech — leave silence state, drive normal audio.
            if self.in_silence {
                debug!(
                    "RFC 3389 CN gate: silence→speech transition (energy={:.4})",
                    vad_result.energy_level
                );
            }
            self.in_silence = false;
            self.last_cn_emitted = None;
            return CnGateDecision::SendAudio;
        }

        // Silence path. Decide whether to emit CN now or suppress.
        let now = Instant::now();
        let due_for_refresh = match self.last_cn_emitted {
            None => true, // First silent frame of this run.
            Some(t) => now.saturating_duration_since(t) >= CN_REFRESH_INTERVAL,
        };

        if !self.in_silence {
            debug!(
                "RFC 3389 CN gate: speech→silence transition (energy={:.4})",
                vad_result.energy_level
            );
        }
        self.in_silence = true;

        if due_for_refresh {
            self.last_cn_emitted = Some(now);
            // Map the VAD's noise estimate to a -dBov level. The VAD
            // reports normalised energy in 0.0..=1.0; we pick the
            // greater of an estimate from that and the default "room
            // noise" so receivers without spectral PLC always get an
            // audible mask. -dBov is in the inverse direction (higher
            // = quieter), so the floor is the LOUDER end (lower
            // number). Keep it simple: use the default noise level.
            CnGateDecision::EmitCnThenSuppress {
                level: DEFAULT_NOISE_LEVEL_DBOV,
            }
        } else {
            CnGateDecision::SuppressAudio
        }
    }

    /// Emit one CN packet at the supplied level. Mirrors
    /// [`CnTransmitter::send`] — exposed here so callers acting on
    /// [`CnGateDecision::EmitCnThenSuppress`] don't need a separate
    /// reference to the transmitter.
    pub async fn emit_cn_now(&self, level: u8) -> Result<()> {
        self.cn.send(level).await
    }

    /// Test-only inspector: are we currently in a silence run?
    #[cfg(test)]
    pub fn in_silence(&self) -> bool {
        self.in_silence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AudioFrame;

    fn loud_frame() -> AudioFrame {
        // 160 samples shaped to land inside the VAD's speech band:
        // - High RMS (~16k → energy ≈ 0.5)
        // - ZCR around 0.0625 (10 sign-flips per 160 samples) which
        //   sits between the simple VAD's lower bound (0.05) and the
        //   default upper bound (0.15).
        // Period of 32 samples (16 positive, 16 negative) is a 250 Hz
        // square-ish tone — well within the speech band.
        let samples = (0..160)
            .map(|i| if (i / 16) % 2 == 0 { 16_000 } else { -16_000 })
            .collect();
        AudioFrame::new(samples, 8000, 1, 0)
    }

    fn silent_frame() -> AudioFrame {
        AudioFrame::new(vec![0; 160], 8000, 1, 0)
    }

    /// Construct a gate bound to a real (loopback) RtpSession. The
    /// tests below only exercise the VAD path on `process_frame` —
    /// `emit_cn_now` is not invoked, so the session never has to
    /// actually transmit. Async because `RtpSession::new` is async.
    async fn make_gate() -> CnGate {
        use rvoip_rtp_core::session::{RtpSession, RtpSessionConfig};
        let cfg = RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: None,
            ssrc: Some(0xCAFE_F00D),
            payload_type: 0,
            clock_rate: 8000,
            jitter_buffer_size: None,
            max_packet_age_ms: None,
            enable_jitter_buffer: false,
            session_buffer_config: Default::default(),
            transport_buffer_config: Default::default(),
        };
        let session = RtpSession::new(cfg).await.unwrap();
        CnGate::new(Arc::new(Mutex::new(session))).unwrap()
    }

    #[test]
    fn comfort_noise_is_offered_only_where_a_foreign_payload_type_is_legal() {
        // G.711 in both flavours and every G.729 spelling the runtime
        // resolves. These are what the gate was written for.
        for name in ["PCMU", "pcmu", "PCMA", "G729", "G729A", "G729AB", "G.729"] {
            assert!(
                supports_rfc3389_comfort_noise(name),
                "{name} should still be gated"
            );
        }
        // AMR is the case that motivated this. Both variants, in the
        // spellings SDP actually carries. AMR-NB is 8 kHz, so the old
        // clock-rate test admitted it.
        for name in ["AMR", "AMR-WB", "AMR-NB", "amr-wb"] {
            assert!(
                !supports_rfc3389_comfort_noise(name),
                "{name} carries its own SID frames and RFC 4867 has no room for PT 13"
            );
        }
        // Opus was already excluded, by clock rate rather than on
        // purpose. Now it is on purpose, and so is anything unknown.
        for name in ["opus", "G722", "not-a-codec", ""] {
            assert!(!supports_rfc3389_comfort_noise(name), "{name}");
        }
    }

    /// The rule this replaced was `clock_rate == 8_000`. Assert the two
    /// actually disagree, so the change is not a restatement of the old
    /// one in different words.
    #[cfg(feature = "amr-nb")]
    #[test]
    fn the_clock_rate_proxy_would_have_admitted_amr_narrowband() {
        use codec_core::codecs::amr::AmrVariant;
        assert_eq!(AmrVariant::NarrowBand.clock_rate(), 8_000);
        assert!(!supports_rfc3389_comfort_noise("AMR"));
        // And the wideband variant is excluded for a reason that is not
        // its clock rate, which the old test happened to get right.
        assert_eq!(AmrVariant::WideBand.clock_rate(), 16_000);
        assert!(!supports_rfc3389_comfort_noise("AMR-WB"));
    }

    #[test]
    fn every_codec_the_runtime_resolves_is_classified_deliberately() {
        // The allow-list lives next door to `resolve_codec`'s match, and
        // this pins the two together: a codec that starts resolving
        // without being classified shows up here as an unexpected
        // `false`, which is the safe direction but should still be a
        // decision someone made rather than one they missed.
        use crate::relay::controller::codec_runtime::resolve_codec;
        use crate::relay::MediaConfig;
        use std::collections::HashMap;
        use std::net::SocketAddr;

        let expected = [
            ("PCMU", true),
            ("PCMA", true),
            #[cfg(feature = "g729")]
            ("G729", true),
            #[cfg(feature = "opus")]
            ("opus", false),
        ];
        let mut checked = 0;
        for (requested, gated) in expected {
            let config = MediaConfig {
                local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                remote_addr: None,
                preferred_codec: Some(requested.to_string()),
                parameters: HashMap::new(),
            };
            let resolved = resolve_codec(&config)
                .unwrap_or_else(|error| panic!("{requested} no longer resolves: {error}"));
            assert_eq!(
                supports_rfc3389_comfort_noise(&resolved.name),
                gated,
                "{requested} resolved to {} and changed CN classification",
                resolved.name
            );
            checked += 1;
        }
        assert_eq!(
            checked,
            expected.len(),
            "the loop skipped a codec instead of classifying it"
        );
    }

    #[tokio::test]
    async fn loud_frame_returns_send_audio() {
        let mut gate = make_gate().await;
        // Prime
        let _ = gate.process_frame(&loud_frame());
        let decision = gate.process_frame(&loud_frame());
        assert_eq!(decision, CnGateDecision::SendAudio);
        assert!(!gate.in_silence());
    }

    /// Drain the simple VAD's `hangover_frames` (default 5) by feeding
    /// silent frames until `is_voice` flips false. Returns the
    /// transition-frame decision so callers can assert on it.
    fn drain_hangover_with_silence(gate: &mut CnGate) -> CnGateDecision {
        let silent = silent_frame();
        // hangover_frames = 5 → up to 5 frames may still report voice
        // even after energy drops. Sweep through up to 6 to guarantee
        // the transition.
        let mut last = CnGateDecision::SendAudio;
        for _ in 0..6 {
            last = gate.process_frame(&silent);
            if matches!(last, CnGateDecision::EmitCnThenSuppress { .. }) {
                return last;
            }
        }
        last
    }

    #[tokio::test]
    async fn first_silent_frame_after_speech_emits_cn() {
        let mut gate = make_gate().await;
        // Establish speech baseline.
        let _ = gate.process_frame(&loud_frame());
        let _ = gate.process_frame(&loud_frame());
        // After the VAD's hangover frames (5 by default) run out,
        // the next silent frame should trigger speech→silence.
        let decision = drain_hangover_with_silence(&mut gate);
        assert!(
            matches!(decision, CnGateDecision::EmitCnThenSuppress { .. }),
            "expected EmitCnThenSuppress after hangover drain, got {:?}",
            decision
        );
        assert!(gate.in_silence());
    }

    #[tokio::test]
    async fn subsequent_silent_frames_within_refresh_window_are_suppressed() {
        let mut gate = make_gate().await;
        let _ = gate.process_frame(&loud_frame());
        let _ = gate.process_frame(&loud_frame());
        let first = drain_hangover_with_silence(&mut gate);
        assert!(matches!(first, CnGateDecision::EmitCnThenSuppress { .. }));
        // Immediately follow with another silent frame — should
        // suppress without re-emitting CN.
        let second = gate.process_frame(&silent_frame());
        assert_eq!(second, CnGateDecision::SuppressAudio);
    }

    #[tokio::test]
    async fn speech_after_silence_resumes_audio() {
        let mut gate = make_gate().await;
        let _ = gate.process_frame(&loud_frame());
        // Drive into silence first.
        let _ = drain_hangover_with_silence(&mut gate);
        // Now switch back to speech.
        let resume = gate.process_frame(&loud_frame());
        assert_eq!(resume, CnGateDecision::SendAudio);
        assert!(!gate.in_silence());
    }
}
