//! Per-dialog codec selection and state.
//!
//! RTP payload numbers are negotiated per dialog.  Keeping the complete codec
//! identity beside the encoder/decoder prevents dynamic payloads from silently
//! falling back to PCMU and lets a re-INVITE replace codec state atomically.

use tokio::sync::Mutex;

use crate::codec::audio::common::AudioCodec;
use crate::codec::audio::G711Codec;
#[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
use crate::codec::audio::amr::AmrAdapter;
#[cfg(feature = "g729")]
use crate::codec::audio::{G729Annexes, G729Codec, G729Config};
#[cfg(feature = "opus")]
use crate::codec::audio::{OpusCodec, OpusConfig};
use crate::error::{CodecError, Error, Result};
use crate::types::AudioFrame;
#[cfg(any(feature = "g729", feature = "opus"))]
use crate::types::SampleRate;

use super::types::{
    MediaConfig, NegotiatedAudioCodec, AMR_DTX_PARAMETER, AUDIO_CHANNELS_PARAMETER,
    NEGOTIATED_FMTP_PARAMETER, RTP_CLOCK_RATE_PARAMETER, RTP_PAYLOAD_TYPE_PARAMETER,
};

fn parse_parameter<T>(config: &MediaConfig, key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
{
    config
        .parameters
        .get(key)
        .map(|value| {
            value.parse::<T>().map_err(|_| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("invalid {key} value: {value}"),
                })
            })
        })
        .transpose()
}

pub(super) fn normalized_codec_name(name: &str) -> String {
    name.chars()
        .filter(|character| !matches!(character, '.' | '-' | '_' | ' '))
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

/// Resolve and validate the exact codec identity before any media state is
/// allocated or mutated.
pub(super) fn resolve_codec(config: &MediaConfig) -> Result<NegotiatedAudioCodec> {
    let requested = config.preferred_codec.as_deref().unwrap_or("PCMU");
    let normalized = normalized_codec_name(requested);
    let supplied_payload_type = parse_parameter::<u8>(config, RTP_PAYLOAD_TYPE_PARAMETER)?;
    let supplied_clock_rate = parse_parameter::<u32>(config, RTP_CLOCK_RATE_PARAMETER)?;
    let supplied_channels = parse_parameter::<u8>(config, AUDIO_CHANNELS_PARAMETER)?;

    let (name, default_payload_type, default_clock_rate, default_channels) =
        match normalized.as_str() {
            "PCMU" | "G711MU" | "G711U" => ("PCMU", 0, 8_000, 1),
            "PCMA" | "G711A" => ("PCMA", 8, 8_000, 1),
            "G722" => {
                return Err(Error::unsupported_codec(
                    "G.722 (encoder/decoder is not implemented)",
                ));
            }
            "G729" | "G729A" | "G729BA" | "G729AB" => {
                #[cfg(not(feature = "g729"))]
                return Err(Error::unsupported_codec(
                    "G.729 (enable the `g729` feature)",
                ));
                #[cfg(feature = "g729")]
                (requested, 18, 8_000, 1)
            }
            "OPUS" => {
                #[cfg(not(feature = "opus"))]
                return Err(Error::unsupported_codec("Opus (enable the `opus` feature)"));
                #[cfg(feature = "opus")]
                ("opus", 111, 48_000, 2)
            }
            // The canonical SDP names, which are what every layer below this
            // compares against -- not the normalised forms this match arm
            // reads. AMR has no static payload type, so the defaults here are
            // only a fallback for a caller that supplied none; a real session
            // always carries the negotiated one.
            "AMR" => {
                #[cfg(not(feature = "amr-nb"))]
                return Err(Error::unsupported_codec("AMR (enable the `amr-nb` feature)"));
                #[cfg(feature = "amr-nb")]
                ("AMR", 96, 8_000, 1)
            }
            "AMRWB" => {
                #[cfg(not(feature = "amr-wb"))]
                return Err(Error::unsupported_codec("AMR-WB (enable the `amr-wb` feature)"));
                #[cfg(feature = "amr-wb")]
                ("AMR-WB", 97, 16_000, 1)
            }
            _ => return Err(Error::unsupported_codec(requested)),
        };

    let codec = NegotiatedAudioCodec {
        name: name.to_string(),
        payload_type: supplied_payload_type.unwrap_or(default_payload_type),
        clock_rate: supplied_clock_rate.unwrap_or(default_clock_rate),
        channels: supplied_channels.unwrap_or(default_channels),
        fmtp: config.parameters.get(NEGOTIATED_FMTP_PARAMETER).cloned(),
        dtx: config
            .parameters
            .get(AMR_DTX_PARAMETER)
            .is_some_and(|value| value.eq_ignore_ascii_case("true")),
    };
    validate_codec_shape(&codec)?;
    Ok(codec)
}

/// Whether a canonical codec name is one of the two AMR encodings.
fn is_amr(name: &str) -> bool {
    name.eq_ignore_ascii_case("AMR") || name.eq_ignore_ascii_case("AMR-WB")
}

fn validate_codec_shape(codec: &NegotiatedAudioCodec) -> Result<()> {
    if codec.payload_type > 127 {
        return Err(CodecError::InvalidParameters {
            details: format!("RTP payload type {} is outside 0..=127", codec.payload_type),
        }
        .into());
    }
    if codec.channels == 0 {
        return Err(CodecError::InvalidParameters {
            details: "audio channel count must be at least one".to_string(),
        }
        .into());
    }

    if codec.name.eq_ignore_ascii_case("opus") {
        if codec.payload_type < 96 || codec.payload_type == 101 {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "Opus requires a negotiated dynamic RTP payload type, got {}",
                    codec.payload_type
                ),
            }
            .into());
        }
        if codec.clock_rate != 48_000 || !matches!(codec.channels, 1 | 2) {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "Opus RTP requires 48000Hz and one or two channels, got {}Hz/{}ch",
                    codec.clock_rate, codec.channels
                ),
            }
            .into());
        }
    } else if codec.name.eq_ignore_ascii_case("PCMU") && codec.payload_type != 0 {
        return Err(CodecError::InvalidParameters {
            details: format!(
                "PCMU requires static RTP payload type 0, got {}",
                codec.payload_type
            ),
        }
        .into());
    } else if codec.name.eq_ignore_ascii_case("PCMA") && codec.payload_type != 8 {
        return Err(CodecError::InvalidParameters {
            details: format!(
                "PCMA requires static RTP payload type 8, got {}",
                codec.payload_type
            ),
        }
        .into());
    } else if normalized_codec_name(&codec.name).starts_with("G729") && codec.payload_type != 18 {
        return Err(CodecError::InvalidParameters {
            details: format!(
                "G.729 requires static RTP payload type 18, got {}",
                codec.payload_type
            ),
        }
        .into());
    } else if is_amr(&codec.name) {
        // **This arm must stay immediately before the catch-all below.** That
        // catch-all demands 8000 Hz, and AMR-WB is 16000; reaching it refuses
        // every wideband session with a message about narrowband's rate.
        if codec.payload_type < 96 || codec.payload_type == 101 {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "{} requires a negotiated dynamic RTP payload type, got {}",
                    codec.name, codec.payload_type
                ),
            }
            .into());
        }
        let wanted = if codec.name.eq_ignore_ascii_case("AMR-WB") {
            16_000
        } else {
            8_000
        };
        if codec.clock_rate != wanted || codec.channels != 1 {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "{} RTP requires {wanted}Hz mono, got {}Hz/{}ch",
                    codec.name, codec.clock_rate, codec.channels
                ),
            }
            .into());
        }
    } else if codec.clock_rate != 8_000 || codec.channels != 1 {
        return Err(CodecError::InvalidParameters {
            details: format!(
                "{} RTP requires 8000Hz mono, got {}Hz/{}ch",
                codec.name, codec.clock_rate, codec.channels
            ),
        }
        .into());
    }
    Ok(())
}

enum StatefulCodec {
    Pcmu(G711Codec),
    Pcma(G711Codec),
    #[cfg(feature = "g729")]
    G729(G729Codec),
    #[cfg(feature = "opus")]
    Opus(OpusCodec),
    #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
    Amr(Box<AmrAdapter>),
}

impl StatefulCodec {
    fn new(codec: &NegotiatedAudioCodec) -> Result<Self> {
        if codec.name.eq_ignore_ascii_case("PCMU") {
            return Ok(Self::Pcmu(G711Codec::mu_law(
                codec.clock_rate,
                u16::from(codec.channels),
            )?));
        }
        if codec.name.eq_ignore_ascii_case("PCMA") {
            return Ok(Self::Pcma(G711Codec::a_law(
                codec.clock_rate,
                u16::from(codec.channels),
            )?));
        }

        #[cfg(feature = "g729")]
        if normalized_codec_name(&codec.name).starts_with("G729") {
            let annex_b = !matches!(normalized_codec_name(&codec.name).as_str(), "G729A");
            return Ok(Self::G729(G729Codec::new(
                SampleRate::Rate8000,
                1,
                G729Config {
                    annexes: G729Annexes {
                        annex_a: true,
                        annex_b,
                    },
                    frame_size_ms: 10.0,
                    enable_vad: annex_b,
                    enable_cng: annex_b,
                },
            )?));
        }

        #[cfg(feature = "opus")]
        if codec.name.eq_ignore_ascii_case("opus") {
            let sample_rate = SampleRate::from_hz(codec.clock_rate).ok_or_else(|| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("invalid Opus sample rate: {}", codec.clock_rate),
                })
            })?;
            return Ok(Self::Opus(OpusCodec::new(
                sample_rate,
                codec.channels,
                OpusConfig::default(),
            )?));
        }

        #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
        if is_amr(&codec.name) {
            // The fmtp is the framing, so it is passed through rather than
            // defaulted: `octet-align` decides the payload's bit layout.
            let mut adapter =
                AmrAdapter::new(codec.payload_type, &codec.name, codec.fmtp.as_deref())?;
            adapter.set_allow_dtx(codec.dtx);
            return Ok(Self::Amr(Box::new(adapter)));
        }

        Err(Error::unsupported_codec(&codec.name))
    }

    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<u8>> {
        match self {
            Self::Pcmu(codec) | Self::Pcma(codec) => codec.encode(frame),
            #[cfg(feature = "g729")]
            Self::G729(codec) => encode_g729(codec, frame),
            #[cfg(feature = "opus")]
            Self::Opus(codec) => codec.encode(frame),
            #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
            Self::Amr(codec) => codec.encode(frame),
        }
    }

    /// Any codec mode request the last decode observed.
    ///
    /// Only AMR has one; every other codec here returns `None`.
    fn take_mode_request(&mut self) -> Option<u8> {
        match self {
            #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
            Self::Amr(codec) => codec.take_mode_request(),
            _ => None,
        }
    }

    /// Apply a peer's codec mode request to this (encoding) side.
    #[cfg_attr(
        not(any(feature = "amr-nb", feature = "amr-wb")),
        allow(unused_variables)
    )]
    fn apply_mode_request(&mut self, cmr: u8) {
        match self {
            #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
            Self::Amr(codec) => codec.apply_mode_request(Some(cmr)),
            _ => {}
        }
    }

    /// Emit a CMR to the peer on the next packed payload. AMR only; other
    /// codecs have no such field, so this is a no-op for them.
    #[cfg_attr(
        not(any(feature = "amr-nb", feature = "amr-wb")),
        allow(unused_variables)
    )]
    fn request_peer_mode(&mut self, mode_index: u8) {
        match self {
            #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
            Self::Amr(codec) => codec.request_peer_mode(mode_index),
            _ => {}
        }
    }

    /// The mode of the last speech frame decoded from the peer, if this codec
    /// tracks one. AMR only.
    fn last_decoded_mode(&self) -> Option<u8> {
        match self {
            #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
            Self::Amr(codec) => Some(codec.last_decoded_mode()),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    fn decode(&mut self, payload: &[u8]) -> Result<AudioFrame> {
        match self {
            Self::Pcmu(codec) | Self::Pcma(codec) => codec.decode(payload),
            #[cfg(feature = "g729")]
            Self::G729(codec) => decode_g729(codec, payload),
            #[cfg(feature = "opus")]
            Self::Opus(codec) => codec.decode(payload),
            #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
            Self::Amr(codec) => codec.decode(payload),
        }
    }
}

#[cfg(feature = "g729")]
fn encode_g729(codec: &mut G729Codec, frame: &AudioFrame) -> Result<Vec<u8>> {
    const SAMPLES_PER_FRAME: usize = 80;
    if frame.samples.len() % SAMPLES_PER_FRAME != 0 {
        return Err(CodecError::InvalidFrameSize {
            expected: SAMPLES_PER_FRAME,
            actual: frame.samples.len(),
        }
        .into());
    }
    let mut encoded = Vec::with_capacity(frame.samples.len() / SAMPLES_PER_FRAME * 10);
    for samples in frame.samples.chunks_exact(SAMPLES_PER_FRAME) {
        encoded.extend(codec.encode(&AudioFrame::new(
            samples.to_vec(),
            frame.sample_rate,
            frame.channels,
            frame.timestamp,
        ))?);
    }
    Ok(encoded)
}

#[cfg(feature = "g729")]
fn decode_g729(codec: &mut G729Codec, payload: &[u8]) -> Result<AudioFrame> {
    const SPEECH_FRAME_BYTES: usize = 10;
    if payload.is_empty() || payload.len() == 2 || payload.len() % SPEECH_FRAME_BYTES != 0 {
        return codec.decode(payload);
    }
    let mut samples = Vec::with_capacity(payload.len() / SPEECH_FRAME_BYTES * 80);
    for chunk in payload.chunks_exact(SPEECH_FRAME_BYTES) {
        samples.extend(codec.decode(chunk)?.samples);
    }
    Ok(AudioFrame::new(samples, 8_000, 1, 0))
}

/// Encoder and decoder state for one exact negotiated codec generation.
pub(super) struct DialogCodecRuntime {
    pub(super) format: NegotiatedAudioCodec,
    encoder: Mutex<StatefulCodec>,
    decoder: Mutex<StatefulCodec>,
}

impl DialogCodecRuntime {
    pub(super) fn new(format: NegotiatedAudioCodec) -> Result<Self> {
        let encoder = StatefulCodec::new(&format)?;
        let decoder = StatefulCodec::new(&format)?;
        Ok(Self {
            format,
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
        })
    }

    pub(super) async fn encode(&self, frame: &AudioFrame) -> Result<Vec<u8>> {
        if frame.sample_rate != self.format.clock_rate || frame.channels != self.format.channels {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "{} frame is {}Hz/{}ch, negotiated format is {}Hz/{}ch",
                    self.format.name,
                    frame.sample_rate,
                    frame.channels,
                    self.format.clock_rate,
                    self.format.channels
                ),
            }
            .into());
        }
        self.encoder.lock().await.encode(frame)
    }

    pub(super) async fn decode(&self, payload: &[u8], timestamp: u32) -> Result<AudioFrame> {
        let (mut frame, mode_request) = {
            let mut decoder = self.decoder.lock().await;
            let frame = decoder.decode(payload)?;
            (frame, decoder.take_mode_request())
        };
        frame.timestamp = timestamp;

        // A codec mode request arrives on the *receive* path and constrains
        // the *transmit* one, and those are separate objects behind separate
        // locks. Hand it across here, after the decoder's lock is released so
        // the two are never held at once.
        if let Some(cmr) = mode_request {
            self.encoder.lock().await.apply_mode_request(cmr);
        }

        Ok(frame)
    }

    /// Ask the peer to change the mode it sends us: the encoder stamps the CMR
    /// on its next payload. The request rides the *transmit* side, which is
    /// why it goes to the encoder lock.
    pub(super) async fn request_peer_mode(&self, mode_index: u8) {
        self.encoder.lock().await.request_peer_mode(mode_index);
    }

    /// The mode of the last speech frame we decoded from the peer — how a
    /// caller confirms a requested change actually took effect on the wire.
    pub(super) async fn last_decoded_mode(&self) -> Option<u8> {
        self.decoder.lock().await.last_decoded_mode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::SocketAddr;

    fn config(codec: &str) -> MediaConfig {
        MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some(codec.to_string()),
            parameters: HashMap::new(),
        }
    }

    /// A config with an AMR codec, its dynamic payload type and its fmtp.
    #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
    fn amr_config(codec: &str, payload_type: u8, clock_rate: u32, fmtp: &str) -> MediaConfig {
        let mut config = config(codec);
        config.parameters.insert(
            RTP_PAYLOAD_TYPE_PARAMETER.to_string(),
            payload_type.to_string(),
        );
        config.parameters.insert(
            RTP_CLOCK_RATE_PARAMETER.to_string(),
            clock_rate.to_string(),
        );
        config
            .parameters
            .insert(NEGOTIATED_FMTP_PARAMETER.to_string(), fmtp.to_string());
        config
    }

    /// Both variants resolve, and the fmtp reaches the resolved codec.
    ///
    /// The fmtp is the part that matters: `octet-align` selects the payload's
    /// bit layout, so a session that resolved without it would build a framing
    /// the peer cannot parse — audible as nothing at all, and indistinguishable
    /// from a network fault.
    #[test]
    #[cfg(all(feature = "amr-nb", feature = "amr-wb"))]
    fn both_amr_variants_resolve_with_their_framing() {
        let nb = resolve_codec(&amr_config("AMR", 96, 8_000, "octet-align=1")).expect("AMR");
        assert_eq!(nb.name, "AMR");
        assert_eq!(nb.clock_rate, 8_000);
        assert_eq!(nb.fmtp.as_deref(), Some("octet-align=1"));

        let wb = resolve_codec(&amr_config("AMR-WB", 97, 16_000, "octet-align=1;mode-set=0,2"))
            .expect("AMR-WB");
        assert_eq!(wb.name, "AMR-WB");
        assert_eq!(wb.clock_rate, 16_000, "AMR-WB is a 16 kHz codec");
        assert_eq!(wb.fmtp.as_deref(), Some("octet-align=1;mode-set=0,2"));

        // The SDP spellings a peer may actually send.
        for spelling in ["amr", "AMR-wb", "amrwb", "AMR_WB"] {
            let resolved = resolve_codec(&amr_config(spelling, 96, 0, ""));
            // Wrong clock rates are refused; what matters here is that the
            // name was *recognised* rather than falling through to
            // `unsupported_codec`.
            assert!(
                !matches!(resolved, Err(Error::Codec(CodecError::UnsupportedCodec { .. }))),
                "`{spelling}` was not recognised as AMR"
            );
        }
    }

    /// AMR-WB's 16 kHz survives the shape check.
    ///
    /// The catch-all in `validate_codec_shape` demands 8000 Hz, so an AMR arm
    /// placed after it refuses every wideband session — with a message about
    /// narrowband's rate, which is the kind of error that gets diagnosed as a
    /// negotiation bug somewhere else entirely.
    #[test]
    #[cfg(feature = "amr-wb")]
    fn wideband_is_not_refused_by_the_narrowband_catch_all() {
        assert!(resolve_codec(&amr_config("AMR-WB", 97, 16_000, "")).is_ok());
        // And the rate is still checked: 8000 is wrong for wideband.
        let err = resolve_codec(&amr_config("AMR-WB", 97, 8_000, "")).unwrap_err();
        assert!(err.to_string().contains("16000Hz"), "{err}");
    }

    /// AMR has no static payload type, and a static one is refused.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn amr_requires_a_dynamic_payload_type() {
        for payload_type in [0u8, 8, 18, 95, 101] {
            let err = resolve_codec(&amr_config("AMR", payload_type, 8_000, ""))
                .expect_err("a non-dynamic payload type must be refused");
            assert!(err.to_string().contains("dynamic"), "{payload_type}: {err}");
        }
        assert!(resolve_codec(&amr_config("AMR", 96, 8_000, "")).is_ok());
        assert!(resolve_codec(&amr_config("AMR", 127, 8_000, "")).is_ok());
    }

    /// A resolved AMR codec encodes and decodes through the runtime.
    ///
    /// The whole point of the wiring: `resolve_codec` to `StatefulCodec` to a
    /// payload and back. Asserted for both variants because they differ in
    /// frame size, clock rate and payload length, and a wiring that hard-coded
    /// narrowband's would pass a narrowband-only test.
    #[test]
    #[cfg(all(feature = "amr-nb", feature = "amr-wb"))]
    fn a_resolved_amr_codec_round_trips_through_the_runtime() {
        for (name, payload_type, clock_rate, samples) in
            [("AMR", 96u8, 8_000u32, 160usize), ("AMR-WB", 97, 16_000, 320)]
        {
            let resolved = resolve_codec(&amr_config(name, payload_type, clock_rate, "octet-align=1"))
                .expect("resolves");
            let mut codec = StatefulCodec::new(&resolved).expect("constructs");

            let pcm: Vec<i16> = (0..samples)
                .map(|i| (((i as f32) * 0.05).sin() * 5000.0) as i16)
                .collect();
            let frame = AudioFrame::new(pcm, clock_rate, 1, 0);

            let payload = codec.encode(&frame).expect("encodes");
            assert!(!payload.is_empty(), "{name}: empty payload");

            let decoded = codec.decode(&payload).expect("decodes");
            assert_eq!(decoded.samples.len(), samples, "{name}: decoded frame length");
            assert!(
                decoded.samples.iter().any(|&s| s != 0),
                "{name}: the round trip produced silence"
            );
        }
    }

    /// A mis-framed buffer is refused rather than silently mis-encoded.
    ///
    /// It has to be an error the caller sees at the boundary: `audio_generation`
    /// treats an encode failure as fatal and stops sending for the rest of the
    /// call, so a codec that quietly accepted 80 samples would be worse than
    /// one that refuses them.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn a_misframed_amr_buffer_is_refused() {
        let resolved = resolve_codec(&amr_config("AMR", 96, 8_000, "")).expect("resolves");
        let mut codec = StatefulCodec::new(&resolved).expect("constructs");
        for length in [80usize, 159, 161, 320] {
            let frame = AudioFrame::new(vec![0i16; length], 8_000, 1, 0);
            assert!(codec.encode(&frame).is_err(), "{length} samples should be refused");
        }
    }

    /// A peer's codec mode request has to reach the *encoder*.
    ///
    /// `DialogCodecRuntime` holds the encoder and the decoder as two separate
    /// `StatefulCodec` allocations, so a CMR applied inside the decoding
    /// object changes nothing about what this end transmits. The unit test in
    /// the adapter passed because it called `apply_mode_request` on the same
    /// object it then encoded with; production never does.
    ///
    /// A CMR is the one flow-control lever a peer has over our bitrate. On a
    /// congested link a handset asks us to drop rate and, before this, we
    /// carried on at the negotiated maximum indefinitely.
    #[tokio::test]
    #[cfg(feature = "amr-nb")]
    async fn a_peers_mode_request_reaches_the_encoder() {
        use codec_core::codecs::amr::mode::AmrVariant;
        use codec_core::codecs::amr::payload::{AmrPayloadCodec, AmrPayloadConfig};

        // mode-set 0 and 7 only, so the request below is satisfiable and the
        // two modes have plainly different frame sizes.
        let resolved = resolve_codec(&amr_config("AMR", 96, 8_000, "octet-align=1; mode-set=0,7"))
            .expect("resolves");
        let runtime = DialogCodecRuntime::new(resolved).expect("constructs");

        let pcm: Vec<i16> = (0..160)
            .map(|i| ((f64::from(i) * 0.05).sin() * 5000.0) as i16)
            .collect();
        let frame = AudioFrame::new(pcm, 8_000, 1, 0);

        let before = runtime.encode(&frame).await.expect("encodes").len();

        // A payload from the peer carrying CMR = 0 (request 4.75 kbit/s).
        // Its speech frame is whatever we just produced, so the packet is
        // well-formed and the only thing under test is the request.
        let packer = AmrPayloadCodec::new(AmrPayloadConfig {
            variant: AmrVariant::NarrowBand,
            octet_aligned: true,
            crc: false,
            robust_sorting: false,
            interleaving: false,
        })
        .expect("packer");
        // Unpack one of our own payloads and re-pack it with the request
        // set, so the speech frame is genuinely well-formed and the only
        // difference from an ordinary packet is the CMR nibble.
        let speech = runtime.encode(&frame).await.expect("encodes");
        let mut packet = packer.unpack(&speech).expect("our own payload unpacks");
        packet.cmr = Some(0);
        let inbound = packer.pack(&packet).expect("packs");

        runtime.decode(&inbound, 0).await.expect("decodes");

        let after = runtime.encode(&frame).await.expect("encodes").len();
        assert_ne!(
            before, after,
            "the peer asked for 4.75 kbit/s and the encoder kept sending {before}-byte frames"
        );
        assert!(
            after < before,
            "the request was for a lower rate, so frames should have shrunk: {before} -> {after}"
        );
    }

    /// A session can actually turn DTX on, and does not by default.
    ///
    /// All the DTX bit-exactness work lives in codec-core and every test there
    /// sets `parameters.amr.dtx` by hand. Nothing in the production path ever
    /// did, and there was no config surface that could — so the encoder ran
    /// 200 frames of digital silence and emitted 200 identical full-rate
    /// frames.
    ///
    /// Both halves are asserted. Off by default matters because DTX changes
    /// what goes on the wire and a deployment should opt in; on-when-asked
    /// matters because otherwise the switch is decorative.
    #[tokio::test]
    #[cfg(feature = "amr-nb")]
    async fn a_session_can_enable_dtx_and_does_not_by_default() {
        // Digital silence is what DTX exists for. Not a homing frame: an
        // all-0x0008 frame would trip the encoder's own homing test.
        let silence = AudioFrame::new(vec![0i16; 160], 8_000, 1, 0);

        async fn run(dtx: bool, silence: &AudioFrame) -> std::collections::BTreeSet<usize> {
            let mut config = amr_config("AMR", 96, 8_000, "octet-align=1");
            if dtx {
                config
                    .parameters
                    .insert(AMR_DTX_PARAMETER.to_string(), "true".to_string());
            }
            let resolved = resolve_codec(&config).expect("resolves");
            assert_eq!(resolved.dtx, dtx, "the parameter did not reach the format");
            let runtime = DialogCodecRuntime::new(resolved).expect("constructs");

            let mut sizes = std::collections::BTreeSet::new();
            for _ in 0..200 {
                sizes.insert(runtime.encode(silence).await.expect("encodes").len());
            }
            sizes
        }

        let without = run(false, &silence).await;
        assert_eq!(
            without.len(),
            1,
            "DTX is off by default, so every frame should be full-rate: {without:?}"
        );

        let with = run(true, &silence).await;
        assert!(
            with.len() > 1,
            "200 frames of silence with DTX on produced one payload size: {with:?}"
        );
        // A SID is 5 octets of payload plus framing, and a NO_DATA carries
        // none — both are far shorter than a speech frame, so the smallest
        // size seen must have dropped well below the no-DTX one.
        let speech = *without.iter().next().expect("one size");
        assert!(
            *with.iter().next().expect("at least one") < speech,
            "nothing shorter than a speech frame was emitted: {with:?} vs {speech}"
        );
    }

    #[test]
    fn unsupported_codecs_fail_with_typed_error() {
        for name in ["G722", "not-a-codec"] {
            assert!(matches!(
                resolve_codec(&config(name)),
                Err(Error::Codec(CodecError::UnsupportedCodec { .. }))
            ));
        }
    }

    #[test]
    fn static_codec_payload_identity_is_enforced() {
        for invalid in [
            config("PCMU").with_negotiated_audio_codec("PCMU", 8, 8_000, 1),
            config("PCMA").with_negotiated_audio_codec("PCMA", 0, 8_000, 1),
        ] {
            assert!(matches!(
                resolve_codec(&invalid),
                Err(Error::Codec(CodecError::InvalidParameters { .. }))
            ));
        }
    }

    #[cfg(feature = "opus")]
    #[test]
    fn opus_requires_a_dynamic_non_dtmf_payload() {
        for payload_type in [0, 8, 18, 101] {
            let invalid =
                config("opus").with_negotiated_audio_codec("opus", payload_type, 48_000, 2);
            assert!(matches!(
                resolve_codec(&invalid),
                Err(Error::Codec(CodecError::InvalidParameters { .. }))
            ));
        }
    }

    #[cfg(not(feature = "opus"))]
    #[test]
    fn opus_fails_closed_without_backend() {
        assert!(matches!(
            resolve_codec(&config("OpUs")),
            Err(Error::Codec(CodecError::UnsupportedCodec { .. }))
        ));
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn opus_runtime_preserves_negotiated_shape() {
        let config = config("OPUS").with_negotiated_audio_codec("OPUS", 96, 48_000, 1);
        let runtime = DialogCodecRuntime::new(resolve_codec(&config).unwrap()).unwrap();
        let input = AudioFrame::new(vec![0; 960], 48_000, 1, 7);
        let payload = runtime.encode(&input).await.unwrap();
        let output = runtime.decode(&payload, 23).await.unwrap();
        assert_eq!(runtime.format.payload_type, 96);
        assert_eq!(output.sample_rate, 48_000);
        assert_eq!(output.channels, 1);
        assert_eq!(output.timestamp, 23);
    }
}
