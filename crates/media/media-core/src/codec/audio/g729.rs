//! G.729 Audio Codec Implementation
//!
//! This module implements the G.729 codec, a low bit-rate audio codec
//! standardized by ITU-T, commonly used in VoIP for its excellent compression.

use super::common::{AudioCodec, CodecInfo};
use crate::error::{CodecError, Result};
use crate::types::{AudioFrame, SampleRate};
#[cfg(feature = "g729")]
use codec_core::codecs::g729::G729Codec as CodecCoreG729;
#[cfg(feature = "g729")]
use codec_core::types::{
    AudioCodec as CodecCoreAudioCodec, CodecConfig as CodecCoreConfig, CodecType as CodecCoreType,
    SampleRate as CodecCoreSampleRate,
};
use tracing::debug;
#[cfg(not(feature = "g729"))]
use tracing::warn;
/// G.729 codec configuration
#[derive(Debug, Clone)]
pub struct G729Config {
    /// Annexes supported (A, B, etc.)
    pub annexes: G729Annexes,
    /// Frame size in milliseconds (10ms standard)
    pub frame_size_ms: f32,
    /// Enable Voice Activity Detection (VAD)
    pub enable_vad: bool,
    /// Enable Comfort Noise Generation (CNG)
    pub enable_cng: bool,
}

/// G.729 annexes configuration
#[derive(Debug, Clone)]
pub struct G729Annexes {
    /// Annex A: Reduced complexity (G.729A)
    pub annex_a: bool,
    /// Annex B: Silence compression with VAD/CNG
    pub annex_b: bool,
}

/// One non-empty RFC 3551 G.729 RTP payload and the timestamp of its oldest
/// encoded frame.
///
/// Annex-B no-data frames deliberately produce no value. The next emitted
/// packet keeps its original timestamp, so the RTP clock still carries the
/// suppressed interval without putting an empty payload on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct G729RtpPacket {
    /// Codec payload bytes, without an RTP header.
    pub payload: Vec<u8>,
    /// RTP timestamp in the 8 kHz G.729 clock.
    pub timestamp: u32,
    /// RTP marker bit. Set on the first speech packet of a talkspurt.
    pub marker: bool,
}

impl Default for G729Config {
    fn default() -> Self {
        Self {
            annexes: G729Annexes {
                annex_a: true, // Use reduced complexity by default
                annex_b: true, // Enable silence compression
            },
            frame_size_ms: 10.0, // Standard 10ms frames
            enable_vad: true,    // Voice Activity Detection
            enable_cng: true,    // Comfort Noise Generation
        }
    }
}

/// G.729 audio codec implementation. The owned `config` is held so
/// the encoder/decoder can be re-initialised when the `g729` feature
/// is enabled; the stub build keeps the field for ABI parity.
pub struct G729Codec {
    /// Codec configuration
    #[allow(dead_code)]
    config: G729Config,
    /// Sample rate (fixed at 8kHz for G.729)
    sample_rate: u32,
    /// Number of channels (fixed at 1 for G.729)
    channels: u8,
    /// Frame size in samples (80 samples for 10ms at 8kHz)
    frame_size: usize,
    /// Codec-core G.729 adapter.
    #[cfg(feature = "g729")]
    inner: CodecCoreG729,
    /// Whether the next speech packet starts a new talkspurt.
    rtp_talkspurt_pending: bool,
    /// First timestamp not yet consumed from the caller's PCM timeline.
    ///
    /// The graph can suppress Annex-B no-data frames before they reach this
    /// encoder. Tracking the input timeline lets the next speech packet still
    /// carry the RTP marker even when this encoder never observed the silence.
    rtp_expected_timestamp: Option<u32>,
}

impl G729Codec {
    const RTP_SPEECH_FRAME_BYTES: usize = 10;
    const RTP_SID_FRAME_BYTES: usize = 2;
    const SAMPLES_PER_FRAME: usize = 80;
    // RFC 3551 says receivers SHOULD accept 200 ms, so never aggregate more
    // than twenty 10 ms frames into one packet when no negotiated ptime is
    // available at this layer.
    const MAX_RTP_FRAMES: usize = 20;

    /// Create a new G.729 codec
    pub fn new(sample_rate: SampleRate, channels: u8, config: G729Config) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();

        // G.729 only supports 8kHz mono
        if sample_rate_hz != 8000 {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "G.729 only supports 8kHz sample rate, got {}",
                    sample_rate_hz
                ),
            }
            .into());
        }

        if channels != 1 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 only supports mono audio, got {} channels", channels),
            }
            .into());
        }

        // Calculate frame size (10ms at 8kHz = 80 samples)
        let frame_size = (sample_rate_hz as f32 * config.frame_size_ms / 1000.0) as usize;

        if frame_size != 80 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 requires 80 sample frames (10ms), got {}", frame_size),
            }
            .into());
        }

        debug!(
            "Creating G.729 codec: {}Hz, {}ch, {}ms frames, VAD={}, CNG={}",
            sample_rate_hz, channels, config.frame_size_ms, config.enable_vad, config.enable_cng
        );

        #[cfg(feature = "g729")]
        let inner = CodecCoreG729::new(codec_core_config(&config)?).map_err(|e| {
            CodecError::InitializationFailed {
                reason: format!("G.729 codec-core initialization failed: {e}"),
            }
        })?;

        Ok(Self {
            config,
            sample_rate: sample_rate_hz,
            channels,
            frame_size,
            #[cfg(feature = "g729")]
            inner,
            rtp_talkspurt_pending: true,
            rtp_expected_timestamp: None,
        })
    }

    /// Whether this codec generation negotiated Annex-B VAD/DTX/CNG.
    pub fn annex_b_enabled(&self) -> bool {
        self.config.annexes.annex_b && self.config.enable_vad && self.config.enable_cng
    }

    /// Encode one or more complete 10 ms PCM frames into valid RFC 3551 RTP
    /// payloads.
    ///
    /// The codec primitive emits one speech (10-byte), SID (2-byte), or
    /// no-data (empty) component per 80 samples. This packetizer keeps the only
    /// legal aggregate order: zero or more speech frames followed by at most
    /// one SID. A no-data component flushes the current packet and advances the
    /// timestamp through the following component rather than emitting an empty
    /// RTP payload.
    pub fn encode_rtp_packets(&mut self, audio_frame: &AudioFrame) -> Result<Vec<G729RtpPacket>> {
        let components = self.encode_timestamped_components(audio_frame)?;
        let mut talkspurt_pending = self.rtp_talkspurt_pending
            || self
                .rtp_expected_timestamp
                .is_some_and(|expected| expected != audio_frame.timestamp);
        let packets = assemble_rtp_packets(components, &mut talkspurt_pending)?;
        self.rtp_talkspurt_pending = talkspurt_pending;
        self.rtp_expected_timestamp = Some(
            audio_frame
                .timestamp
                .wrapping_add(audio_frame.samples_per_channel() as u32),
        );
        Ok(packets)
    }

    /// Encode complete PCM frames as individual non-empty codec payloads.
    ///
    /// This is the graph-facing form: each returned payload is independently
    /// decodable by the single-frame codec primitive. No-data components are
    /// suppressed, while later frames retain their source timestamps.
    pub fn encode_payload_frames(
        &mut self,
        audio_frame: &AudioFrame,
    ) -> Result<Vec<G729RtpPacket>> {
        self.encode_timestamped_components(audio_frame)
            .map(|components| {
                components
                    .into_iter()
                    .filter_map(|(timestamp, payload)| {
                        (!payload.is_empty()).then_some(G729RtpPacket {
                            payload,
                            timestamp,
                            marker: false,
                        })
                    })
                    .collect()
            })
    }

    fn encode_timestamped_components(
        &mut self,
        audio_frame: &AudioFrame,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        if audio_frame.sample_rate != self.sample_rate || audio_frame.channels != self.channels {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "G.729 RTP input must be {}Hz/{}ch, got {}Hz/{}ch",
                    self.sample_rate, self.channels, audio_frame.sample_rate, audio_frame.channels
                ),
            }
            .into());
        }
        if audio_frame.samples.is_empty()
            || !audio_frame
                .samples
                .len()
                .is_multiple_of(Self::SAMPLES_PER_FRAME)
        {
            return Err(CodecError::InvalidFrameSize {
                expected: Self::SAMPLES_PER_FRAME,
                actual: audio_frame.samples.len(),
            }
            .into());
        }

        let mut components =
            Vec::with_capacity(audio_frame.samples.len() / Self::SAMPLES_PER_FRAME);
        for (index, samples) in audio_frame
            .samples
            .chunks_exact(Self::SAMPLES_PER_FRAME)
            .enumerate()
        {
            let timestamp = audio_frame
                .timestamp
                .wrapping_add((index * Self::SAMPLES_PER_FRAME) as u32);
            let payload = self.encode(&AudioFrame::new(
                samples.to_vec(),
                audio_frame.sample_rate,
                audio_frame.channels,
                timestamp,
            ))?;
            components.push((timestamp, payload));
        }
        Ok(components)
    }

    /// Decode one RFC 3551 RTP payload, including bundled Annex-B payloads.
    ///
    /// A packet is zero or more 10-byte speech frames optionally followed by
    /// one 2-byte SID frame. The complete shape and negotiated Annex-B policy
    /// are checked before the stateful decoder consumes any component.
    pub fn decode_rtp_payload(&mut self, payload: &[u8]) -> Result<AudioFrame> {
        let remainder = payload.len() % Self::RTP_SPEECH_FRAME_BYTES;
        let has_sid = remainder == Self::RTP_SID_FRAME_BYTES;
        if remainder != 0 && !has_sid {
            return Err(CodecError::DecodingFailed {
                reason: format!(
                    "invalid G.729 RTP payload length {}; expected 10*N bytes with an optional trailing 2-byte SID",
                    payload.len()
                ),
            }
            .into());
        }
        if (payload.is_empty() || has_sid) && !self.annex_b_enabled() {
            return Err(CodecError::DecodingFailed {
                reason: "G.729 Annex-B no-data/SID payload received after annexb=no negotiation"
                    .to_string(),
            }
            .into());
        }

        if payload.is_empty() {
            return self.decode(payload);
        }

        let speech_bytes = payload.len() - usize::from(has_sid) * Self::RTP_SID_FRAME_BYTES;
        let frame_count = speech_bytes / Self::RTP_SPEECH_FRAME_BYTES + usize::from(has_sid);
        let mut samples = Vec::with_capacity(frame_count * Self::SAMPLES_PER_FRAME);
        for speech in payload[..speech_bytes].chunks_exact(Self::RTP_SPEECH_FRAME_BYTES) {
            samples.extend(self.decode(speech)?.samples);
        }
        if has_sid {
            samples.extend(self.decode(&payload[speech_bytes..])?.samples);
        }
        Ok(AudioFrame::new(samples, self.sample_rate, self.channels, 0))
    }

    /// Simulate G.729 encoding (when actual codec not available)
    #[cfg(not(feature = "g729"))]
    fn simulate_encode(&self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        // G.729 produces 10 bytes per 10ms frame (8 kbps)
        // This is a simulation for testing purposes
        let mut encoded = Vec::with_capacity(10);

        // Simple simulation based on energy level
        let energy = audio_frame
            .samples
            .iter()
            .map(|&s| (s as f32).abs())
            .sum::<f32>()
            / audio_frame.samples.len() as f32;

        // Generate deterministic "encoded" data based on energy
        let energy_byte = (energy / 32768.0 * 255.0) as u8;
        for i in 0..10 {
            encoded.push(energy_byte.wrapping_add(i as u8));
        }

        debug!(
            "G.729 simulation: encoded {} samples -> {} bytes",
            audio_frame.samples.len(),
            encoded.len()
        );

        Ok(encoded)
    }

    /// Simulate G.729 decoding (when actual codec not available)
    #[cfg(not(feature = "g729"))]
    fn simulate_decode(&self, encoded_data: &[u8]) -> Result<AudioFrame> {
        // G.729 decodes to 80 samples per frame
        if encoded_data.len() != 10 {
            return Err(CodecError::InvalidFrameSize {
                expected: 10,
                actual: encoded_data.len(),
            }
            .into());
        }

        // Simple simulation: generate silence or tone based on first byte
        let mut samples = Vec::with_capacity(80);
        let energy_level = encoded_data[0] as i16 * 128; // Scale to 16-bit range

        for i in 0..80 {
            // Generate a simple pattern based on encoded data
            let sample = if energy_level > 16384 {
                // Generate a simple tone for non-silence
                (((i as f32 * 2.0 * std::f32::consts::PI * 440.0) / 8000.0).sin()
                    * energy_level as f32) as i16
            } else {
                // Generate silence
                0
            };
            samples.push(sample);
        }

        debug!(
            "G.729 simulation: decoded {} bytes -> {} samples",
            encoded_data.len(),
            samples.len()
        );

        Ok(AudioFrame::new(
            samples,
            self.sample_rate,
            self.channels,
            0, // Timestamp to be set by caller
        ))
    }
}

fn assemble_rtp_packets(
    components: impl IntoIterator<Item = (u32, Vec<u8>)>,
    talkspurt_pending: &mut bool,
) -> Result<Vec<G729RtpPacket>> {
    let mut packets = Vec::new();
    let mut payload = Vec::new();
    let mut packet_timestamp = 0;
    let mut packet_frames = 0usize;
    let mut packet_marker = false;

    let flush = |packets: &mut Vec<G729RtpPacket>,
                 payload: &mut Vec<u8>,
                 packet_timestamp: u32,
                 packet_marker: bool,
                 packet_frames: &mut usize| {
        if !payload.is_empty() {
            packets.push(G729RtpPacket {
                payload: std::mem::take(payload),
                timestamp: packet_timestamp,
                marker: packet_marker,
            });
        }
        *packet_frames = 0;
    };

    for (timestamp, component) in components {
        match component.len() {
            0 => {
                flush(
                    &mut packets,
                    &mut payload,
                    packet_timestamp,
                    packet_marker,
                    &mut packet_frames,
                );
                *talkspurt_pending = true;
            }
            G729Codec::RTP_SPEECH_FRAME_BYTES => {
                if packet_frames == G729Codec::MAX_RTP_FRAMES {
                    flush(
                        &mut packets,
                        &mut payload,
                        packet_timestamp,
                        packet_marker,
                        &mut packet_frames,
                    );
                }
                if payload.is_empty() {
                    packet_timestamp = timestamp;
                    packet_marker = *talkspurt_pending;
                    *talkspurt_pending = false;
                }
                payload.extend(component);
                packet_frames += 1;
            }
            G729Codec::RTP_SID_FRAME_BYTES => {
                if packet_frames == G729Codec::MAX_RTP_FRAMES {
                    flush(
                        &mut packets,
                        &mut payload,
                        packet_timestamp,
                        packet_marker,
                        &mut packet_frames,
                    );
                }
                if payload.is_empty() {
                    packet_timestamp = timestamp;
                    packet_marker = false;
                }
                payload.extend(component);
                packet_frames += 1;
                // SID is legal only at the end of a packet.
                flush(
                    &mut packets,
                    &mut payload,
                    packet_timestamp,
                    packet_marker,
                    &mut packet_frames,
                );
                *talkspurt_pending = true;
            }
            actual => {
                return Err(CodecError::EncodingFailed {
                    reason: format!(
                        "G.729 codec emitted an invalid {actual}-byte frame; expected 0, 2, or 10"
                    ),
                }
                .into());
            }
        }
    }
    flush(
        &mut packets,
        &mut payload,
        packet_timestamp,
        packet_marker,
        &mut packet_frames,
    );
    debug_assert!(packets.iter().all(|packet| !packet.payload.is_empty()));
    Ok(packets)
}

impl AudioCodec for G729Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        if audio_frame.samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: audio_frame.samples.len(),
            }
            .into());
        }

        #[cfg(feature = "g729")]
        {
            self.inner.encode(&audio_frame.samples).map_err(|e| {
                CodecError::EncodingFailed {
                    reason: format!("G.729 encoding failed: {e}"),
                }
                .into()
            })
        }

        #[cfg(not(feature = "g729"))]
        {
            warn!("G.729 codec not available - using simulation");
            self.simulate_encode(audio_frame)
        }
    }

    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        // G.729 frames are typically 10 bytes. Under Annex B, empty no-data
        // payloads are valid only when the real codec-core backend is enabled.
        if encoded_data.is_empty() && !cfg!(feature = "g729") {
            return Err(CodecError::InvalidFrameSize {
                expected: 10,
                actual: 0,
            }
            .into());
        }

        #[cfg(feature = "g729")]
        {
            let decoded_samples =
                self.inner
                    .decode(encoded_data)
                    .map_err(|e| CodecError::DecodingFailed {
                        reason: format!("G.729 decoding failed: {e}"),
                    })?;

            Ok(AudioFrame::new(
                decoded_samples,
                self.sample_rate,
                self.channels,
                0, // Timestamp to be set by caller
            ))
        }

        #[cfg(not(feature = "g729"))]
        {
            warn!("G.729 codec not available - using simulation");
            self.simulate_decode(encoded_data)
        }
    }

    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: "G.729".to_string(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: 8000, // 8 kbps
        }
    }

    fn reset(&mut self) {
        #[cfg(feature = "g729")]
        {
            let _ = self.inner.reset();
        }
        self.rtp_talkspurt_pending = true;
        self.rtp_expected_timestamp = None;
        debug!("G.729 codec reset");
    }
}

#[cfg(feature = "g729")]
fn codec_core_config(config: &G729Config) -> Result<CodecCoreConfig> {
    if !config.annexes.annex_a {
        return Err(CodecError::InvalidParameters {
            details: "Full-complexity base G.729 is not implemented; Annex A is required"
                .to_string(),
        }
        .into());
    }

    let annex_b = config.annexes.annex_b && config.enable_vad && config.enable_cng;
    let codec_type = if annex_b {
        CodecCoreType::G729BA
    } else {
        CodecCoreType::G729A
    };

    let mut core_config = CodecCoreConfig::new(codec_type)
        .with_sample_rate(CodecCoreSampleRate::Rate8000)
        .with_channels(1)
        .with_frame_size_ms(config.frame_size_ms)
        .with_g729_annex_a(true)
        .with_g729_annex_b(annex_b);

    core_config.parameters.g729.annex_a = true;
    core_config.parameters.g729.annex_b = annex_b;

    Ok(core_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec_with_annex_b(annex_b: bool) -> G729Codec {
        G729Codec::new(
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
        )
        .expect("G.729 codec")
    }

    #[cfg(feature = "g729")]
    fn speech_payload(frame_count: usize) -> Vec<u8> {
        let mut encoder = codec_with_annex_b(false);
        let mut payload = Vec::with_capacity(frame_count * 10);
        for frame_index in 0..frame_count {
            let samples = (0..80)
                .map(|sample_index| {
                    let index = frame_index * 80 + sample_index;
                    let phase = index as f64 * 2.0 * std::f64::consts::PI * 440.0 / 8_000.0;
                    (phase.sin() * 6_000.0) as i16
                })
                .collect();
            let frame = encoder
                .encode(&AudioFrame::new(samples, 8_000, 1, 0))
                .expect("speech frame");
            assert_eq!(frame.len(), 10);
            payload.extend(frame);
        }
        payload
    }

    #[test]
    fn test_g729_creation() {
        let config = G729Config::default();
        let codec = G729Codec::new(SampleRate::Rate8000, 1, config);
        assert!(codec.is_ok());

        let codec = codec.unwrap();
        assert_eq!(codec.sample_rate, 8000);
        assert_eq!(codec.channels, 1);
        assert_eq!(codec.frame_size, 80);
    }

    #[test]
    fn test_g729_invalid_sample_rate() {
        let config = G729Config::default();
        let result = G729Codec::new(SampleRate::Rate16000, 1, config);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::InvalidParameters { .. })
            ));
        }
    }

    #[test]
    fn test_g729_invalid_channels() {
        let config = G729Config::default();
        let result = G729Codec::new(SampleRate::Rate8000, 2, config);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::InvalidParameters { .. })
            ));
        }
    }

    #[test]
    fn test_g729_encode_decode() {
        let config = G729Config::default();
        let mut codec = G729Codec::new(SampleRate::Rate8000, 1, config).unwrap();

        // Create test frame (80 samples for 10ms at 8kHz)
        let samples: Vec<i16> = (0..80).map(|i| (i * 100) as i16).collect();
        let frame = AudioFrame::new(samples.clone(), 8000, 1, 1000);

        // Test encoding
        let encoded = codec.encode(&frame);
        assert!(encoded.is_ok());

        let encoded_data = encoded.unwrap();
        assert_eq!(encoded_data.len(), 10); // G.729 produces 10 bytes per frame

        // Test decoding
        let decoded = codec.decode(&encoded_data);
        assert!(decoded.is_ok());

        let decoded_frame = decoded.unwrap();
        assert_eq!(decoded_frame.samples.len(), 80);
        assert_eq!(decoded_frame.sample_rate, 8000);
        assert_eq!(decoded_frame.channels, 1);
    }

    #[test]
    fn test_g729_invalid_frame_size() {
        let config = G729Config::default();
        let mut codec = G729Codec::new(SampleRate::Rate8000, 1, config).unwrap();

        // Test with wrong frame size
        let samples: Vec<i16> = vec![0; 160]; // Wrong size (should be 80)
        let frame = AudioFrame::new(samples, 8000, 1, 1000);

        let result = codec.encode(&frame);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::InvalidFrameSize { .. })
            ));
        }
    }

    #[test]
    fn test_g729_codec_info() {
        let config = G729Config::default();
        let codec = G729Codec::new(SampleRate::Rate8000, 1, config).unwrap();

        let info = codec.get_info();
        assert_eq!(info.name, "G.729");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 8000);
    }

    #[test]
    fn test_g729_config_default() {
        let config = G729Config::default();
        assert!(config.annexes.annex_a);
        assert!(config.annexes.annex_b);
        assert_eq!(config.frame_size_ms, 10.0);
        assert!(config.enable_vad);
        assert!(config.enable_cng);
    }

    #[cfg(feature = "g729")]
    #[test]
    fn rtp_decoder_accepts_bundled_speech_with_trailing_sid() {
        for (speech_frames, expected_samples) in [(1, 160), (2, 240)] {
            let mut payload = speech_payload(speech_frames);
            payload.extend([0, 0]);

            let decoded = codec_with_annex_b(true)
                .decode_rtp_payload(&payload)
                .expect("valid Annex-B RTP payload");
            assert_eq!(payload.len(), speech_frames * 10 + 2);
            assert_eq!(decoded.samples.len(), expected_samples);
            assert_eq!(decoded.samples_per_channel(), expected_samples);
            assert_eq!(
                decoded.duration,
                std::time::Duration::from_millis((expected_samples / 8) as u64)
            );
        }
    }

    #[cfg(feature = "g729")]
    #[test]
    fn rtp_decoder_rejects_malformed_lengths_and_sid_after_annexb_no() {
        for payload_len in [1, 3, 4, 8, 11, 13, 21, 23] {
            let error = codec_with_annex_b(true)
                .decode_rtp_payload(&vec![0; payload_len])
                .expect_err("malformed payload must fail before decoding");
            assert!(
                error
                    .to_string()
                    .contains("invalid G.729 RTP payload length"),
                "{payload_len}: {error}"
            );
        }

        for payload in [vec![0, 0], {
            let mut payload = speech_payload(1);
            payload.extend([0, 0]);
            payload
        }] {
            let error = codec_with_annex_b(false)
                .decode_rtp_payload(&payload)
                .expect_err("annexb=no must reject SID");
            assert!(error.to_string().contains("annexb=no"), "{error}");
        }
    }

    #[cfg(feature = "g729")]
    #[test]
    fn rtp_encoder_marks_speech_after_an_input_timestamp_gap() {
        let mut codec = codec_with_annex_b(false);
        let samples = (0..80)
            .map(|sample| {
                let phase = sample as f64 * std::f64::consts::TAU * 440.0 / 8_000.0;
                (phase.sin() * 6_000.0) as i16
            })
            .collect::<Vec<_>>();

        let mut encode_at = |timestamp| {
            codec
                .encode_rtp_packets(&AudioFrame::new(samples.clone(), 8_000, 1, timestamp))
                .expect("speech packet")
                .into_iter()
                .next()
                .expect("Annex-A speech is never suppressed")
        };

        let first = encode_at(1_000);
        let contiguous = encode_at(1_080);
        let after_gap = encode_at(1_240);
        assert!(first.marker, "the first speech packet starts a talkspurt");
        assert!(
            !contiguous.marker,
            "contiguous speech stays in the talkspurt"
        );
        assert!(
            after_gap.marker,
            "a suppressed 10 ms interval starts a new talkspurt"
        );
        assert_eq!(after_gap.timestamp, 1_240);
    }

    #[test]
    fn rtp_packet_assembler_preserves_dtx_timestamps_and_never_emits_empty_payloads() {
        let speech = |value| vec![value; 10];
        let sid = || vec![0; 2];

        let mut talkspurt_pending = true;
        let packets = assemble_rtp_packets(
            [
                (0, speech(1)),
                (80, speech(2)),
                (160, sid()),
                (240, Vec::new()),
                (320, speech(3)),
                (400, sid()),
                (480, speech(4)),
            ],
            &mut talkspurt_pending,
        )
        .expect("valid components");

        assert_eq!(
            packets,
            vec![
                G729RtpPacket {
                    payload: [speech(1), speech(2), sid()].concat(),
                    timestamp: 0,
                    marker: true,
                },
                G729RtpPacket {
                    payload: [speech(3), sid()].concat(),
                    timestamp: 320,
                    marker: true,
                },
                G729RtpPacket {
                    payload: speech(4),
                    timestamp: 480,
                    marker: true,
                },
            ]
        );
        assert!(packets.iter().all(|packet| !packet.payload.is_empty()));
        assert!(!talkspurt_pending);

        let mut pending = true;
        assert!(assemble_rtp_packets([(0, vec![0; 1])], &mut pending).is_err());
        assert!(
            assemble_rtp_packets([(0, Vec::new()), (80, Vec::new())], &mut pending)
                .expect("no-data components")
                .is_empty()
        );
        assert!(pending);
    }

    #[test]
    fn rtp_encoder_rejects_zero_pcm_input() {
        let error = codec_with_annex_b(true)
            .encode_rtp_packets(&AudioFrame::new(Vec::new(), 8_000, 1, 123))
            .expect_err("zero PCM does not describe a G.729 frame interval");
        assert!(matches!(
            error,
            crate::error::Error::Codec(CodecError::InvalidFrameSize { actual: 0, .. })
        ));
    }
}
