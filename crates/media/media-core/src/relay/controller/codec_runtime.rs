//! Per-dialog codec selection and state.
//!
//! RTP payload numbers are negotiated per dialog.  Keeping the complete codec
//! identity beside the encoder/decoder prevents dynamic payloads from silently
//! falling back to PCMU and lets a re-INVITE replace codec state atomically.

use tokio::sync::Mutex;

use crate::codec::audio::common::AudioCodec;
use crate::codec::audio::G711Codec;
#[cfg(feature = "g729")]
use crate::codec::audio::{G729Annexes, G729Codec, G729Config};
#[cfg(feature = "opus")]
use crate::codec::audio::{OpusCodec, OpusConfig};
use crate::error::{CodecError, Error, Result};
use crate::types::AudioFrame;
#[cfg(any(feature = "g729", feature = "opus"))]
use crate::types::SampleRate;

use super::types::{
    MediaConfig, NegotiatedAudioCodec, AUDIO_CHANNELS_PARAMETER, RTP_CLOCK_RATE_PARAMETER,
    RTP_PAYLOAD_TYPE_PARAMETER,
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

fn normalized_codec_name(name: &str) -> String {
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
            _ => return Err(Error::unsupported_codec(requested)),
        };

    let codec = NegotiatedAudioCodec {
        name: name.to_string(),
        payload_type: supplied_payload_type.unwrap_or(default_payload_type),
        clock_rate: supplied_clock_rate.unwrap_or(default_clock_rate),
        channels: supplied_channels.unwrap_or(default_channels),
    };
    validate_codec_shape(&codec)?;
    Ok(codec)
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
        if codec.clock_rate != 48_000 || !matches!(codec.channels, 1 | 2) {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "Opus RTP requires 48000Hz and one or two channels, got {}Hz/{}ch",
                    codec.clock_rate, codec.channels
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

        Err(Error::unsupported_codec(&codec.name))
    }

    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<u8>> {
        match self {
            Self::Pcmu(codec) | Self::Pcma(codec) => codec.encode(frame),
            #[cfg(feature = "g729")]
            Self::G729(codec) => encode_g729(codec, frame),
            #[cfg(feature = "opus")]
            Self::Opus(codec) => codec.encode(frame),
        }
    }

    fn decode(&mut self, payload: &[u8]) -> Result<AudioFrame> {
        match self {
            Self::Pcmu(codec) | Self::Pcma(codec) => codec.decode(payload),
            #[cfg(feature = "g729")]
            Self::G729(codec) => decode_g729(codec, payload),
            #[cfg(feature = "opus")]
            Self::Opus(codec) => codec.decode(payload),
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
        let mut frame = self.decoder.lock().await.decode(payload)?;
        frame.timestamp = timestamp;
        Ok(frame)
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

    #[test]
    fn unsupported_codecs_fail_with_typed_error() {
        for name in ["G722", "not-a-codec"] {
            assert!(matches!(
                resolve_codec(&config(name)),
                Err(Error::Codec(CodecError::UnsupportedCodec { .. }))
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
