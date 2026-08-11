//! Codec Transcoding
//!
//! This module provides real-time transcoding between different audio codecs,
//! enabling mixed-codec calls and codec negotiation fallbacks.

use crate::codec::audio::common::AudioCodec;
use crate::codec::audio::payload_type::PCM_S16LE;
use crate::codec::spec::AudioCodecSpec;
use crate::error::{CodecError, Error, Result};
use crate::processing::format::{ConversionParams, FormatConverter};
use crate::types::{PayloadType, SampleRate};
use std::collections::HashMap;
use tracing::{debug, trace};

/// The spec for a statically-assigned payload type.
///
/// This is the only place a bare payload type is turned into a codec, and it
/// refuses to do so for a dynamic one. That refusal is the point: the old
/// `Transcoder::create_codec` special-cased 111 as Opus, which is true of some
/// negotiations and false of others, and there is no such number for AMR at
/// all.
fn spec_for(payload_type: PayloadType) -> Result<AudioCodecSpec> {
    if payload_type == PCM_S16LE {
        return Ok(AudioCodecSpec::new("PCM_S16LE", PCM_S16LE, 16_000, 1));
    }
    AudioCodecSpec::from_static_payload_type(payload_type).ok_or_else(|| {
        Error::Codec(CodecError::UnsupportedPayloadType { payload_type })
    })
}

/// Transcoding path between two codecs
///
/// Keyed on the full [`AudioCodecSpec`] rather than the payload type, because
/// a dynamic payload type does not name a codec: two AMR legs differing only
/// in `octet-align` share PT 96 and must not share a session, and PT 96 is a
/// different codec on the next call entirely.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TranscodingPath {
    /// Source codec.
    pub from: AudioCodecSpec,
    /// Target codec.
    pub to: AudioCodecSpec,
}

/// Transcoding session for converting between two specific codecs
pub struct TranscodingSession {
    /// Source codec for decoding
    source_codec: Box<dyn AudioCodec>,
    /// Target codec for encoding
    target_codec: Box<dyn AudioCodec>,
    /// Format converter for sample rate/channel conversion.
    ///
    /// One per session, owned rather than shared. A resampler is filter state
    /// as well as a ratio, so two sessions sharing one would interleave their
    /// histories even when their rates agree -- and when their rates differ,
    /// as they do for the two directions of an 8 kHz <-> 48 kHz call, they
    /// would fight over which ratio the single cached resampler holds.
    format_converter: FormatConverter,
    /// Transcoding statistics
    stats: TranscodingStats,
}

/// Transcoding statistics
#[derive(Debug, Clone, Default)]
pub struct TranscodingStats {
    /// Frames transcoded
    pub frames_transcoded: u64,
    /// Total processing time (microseconds)
    pub total_processing_time_us: u64,
    /// Average processing time per frame (microseconds)
    pub avg_processing_time_us: f32,
    /// Transcoding errors
    pub errors: u64,
}

/// Main transcoding engine
pub struct Transcoder {
    /// Active transcoding sessions
    sessions: HashMap<TranscodingPath, TranscodingSession>,
    /// Enable performance statistics
    enable_stats: bool,
}

impl Default for Transcoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Transcoder {
    /// Create a new transcoder.
    ///
    /// This used to take an `Arc<RwLock<FormatConverter>>`. Every caller built
    /// a fresh one to hand in, and sharing one across sessions was wrong
    /// anyway -- see [`TranscodingSession::format_converter`]. Each session now
    /// builds its own.
    pub fn new() -> Self {
        debug!("Creating Transcoder");

        Self {
            sessions: HashMap::new(),
            enable_stats: true,
        }
    }

    /// Get or create a transcoding session for two statically-assigned
    /// payload types.
    ///
    /// # Errors
    ///
    /// When either payload type is dynamic — there is nothing to infer from
    /// the number, so the caller must use
    /// [`get_or_create_session_for`](Self::get_or_create_session_for) with the
    /// specs the negotiation produced.
    pub fn get_or_create_session(
        &mut self,
        from_codec: PayloadType,
        to_codec: PayloadType,
    ) -> Result<&mut TranscodingSession> {
        let from = spec_for(from_codec)?;
        let to = spec_for(to_codec)?;
        self.get_or_create_session_for(&from, &to)
    }

    /// Get or create a transcoding session between two negotiated codecs.
    ///
    /// # Errors
    ///
    /// When either codec cannot be constructed.
    pub fn get_or_create_session_for(
        &mut self,
        from: &AudioCodecSpec,
        to: &AudioCodecSpec,
    ) -> Result<&mut TranscodingSession> {
        let path = TranscodingPath {
            from: from.clone(),
            to: to.clone(),
        };

        // Return existing session if available
        if self.sessions.contains_key(&path) {
            return Ok(self.sessions.get_mut(&path).unwrap());
        }

        // Create new transcoding session
        let session = TranscodingSession {
            source_codec: from.build()?,
            target_codec: to.build()?,
            format_converter: FormatConverter::new(),
            stats: TranscodingStats::default(),
        };
        self.sessions.insert(path.clone(), session);

        debug!("Created transcoding session: {} -> {}", from.name, to.name);
        Ok(self.sessions.get_mut(&path).unwrap())
    }

    /// Transcode audio data between two statically-assigned payload types.
    ///
    /// # Errors
    ///
    /// As [`get_or_create_session`](Self::get_or_create_session), plus
    /// whatever the codecs report.
    pub async fn transcode(
        &mut self,
        encoded_data: &[u8],
        from_codec: PayloadType,
        to_codec: PayloadType,
    ) -> Result<Vec<u8>> {
        // Short-circuit if no transcoding needed
        if from_codec == to_codec {
            return Ok(encoded_data.to_vec());
        }
        let from = spec_for(from_codec)?;
        let to = spec_for(to_codec)?;
        self.transcode_between(encoded_data, &from, &to).await
    }

    /// Transcode audio data between two negotiated codecs.
    ///
    /// # Errors
    ///
    /// When either codec cannot be constructed, or when the payload does not
    /// decode or re-encode.
    pub async fn transcode_between(
        &mut self,
        encoded_data: &[u8],
        from: &AudioCodecSpec,
        to: &AudioCodecSpec,
    ) -> Result<Vec<u8>> {
        // Identical codecs relay unchanged -- including their fmtp, since a
        // difference there is a difference in the bytes.
        if from == to {
            return Ok(encoded_data.to_vec());
        }

        let start_time = std::time::Instant::now();
        let enable_stats = self.enable_stats; // Copy the flag to avoid borrowing issues

        // Get transcoding session
        let session = self.get_or_create_session_for(from, to)?;

        // Perform transcoding
        let result = session.transcode(encoded_data).await;

        // Update statistics
        if enable_stats {
            let processing_time = start_time.elapsed().as_micros() as u64;
            session.stats.total_processing_time_us += processing_time;

            match &result {
                Ok(_) => {
                    session.stats.frames_transcoded += 1;
                    session.stats.avg_processing_time_us = session.stats.total_processing_time_us
                        as f32
                        / session.stats.frames_transcoded as f32;
                }
                Err(_) => session.stats.errors += 1,
            }
        }

        result
    }

    /// Get transcoding statistics for a statically-assigned pair.
    pub fn get_stats(
        &self,
        from_codec: PayloadType,
        to_codec: PayloadType,
    ) -> Option<&TranscodingStats> {
        let path = TranscodingPath {
            from: spec_for(from_codec).ok()?,
            to: spec_for(to_codec).ok()?,
        };
        self.sessions.get(&path).map(|session| &session.stats)
    }

    /// Get transcoding statistics for a negotiated pair.
    #[must_use]
    pub fn get_stats_for(
        &self,
        from: &AudioCodecSpec,
        to: &AudioCodecSpec,
    ) -> Option<&TranscodingStats> {
        let path = TranscodingPath {
            from: from.clone(),
            to: to.clone(),
        };
        self.sessions.get(&path).map(|session| &session.stats)
    }

    /// Get all supported transcoding paths
    pub fn get_supported_paths(&self) -> Vec<TranscodingPath> {
        let mut supported: Vec<AudioCodecSpec> = vec![
            AudioCodecSpec::new("PCMU", 0, 8_000, 1),
            AudioCodecSpec::new("PCMA", 8, 8_000, 1),
            AudioCodecSpec::new("PCM_S16LE", PCM_S16LE, 16_000, 1),
        ];
        #[cfg(feature = "g729")]
        supported.push(AudioCodecSpec::new("G729", 18, 8_000, 1));
        #[cfg(feature = "opus")]
        supported.push(AudioCodecSpec::new("opus", 111, 48_000, 2));
        // Listed at their conventional dynamic payload types. A real session
        // uses whatever it negotiated; this list answers "which pairs can this
        // build transcode", not "which payload types are in use".
        #[cfg(feature = "amr-nb")]
        supported.push(AudioCodecSpec::new("AMR", 96, 8_000, 1));
        #[cfg(feature = "amr-wb")]
        supported.push(AudioCodecSpec::new("AMR-WB", 97, 16_000, 1));

        let mut paths = Vec::new();
        for from in &supported {
            for to in &supported {
                if from != to {
                    paths.push(TranscodingPath {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
            }
        }

        paths
    }

    /// Clear all transcoding sessions (useful for memory management)
    pub fn clear_sessions(&mut self) {
        self.sessions.clear();
        debug!("Cleared all transcoding sessions");
    }
}

impl TranscodingSession {
    /// Transcode audio data
    pub async fn transcode(&mut self, encoded_data: &[u8]) -> Result<Vec<u8>> {
        // Step 1: Decode source audio
        let source_frame = self.source_codec.decode(encoded_data)?;

        // Step 2: Format conversion if needed
        let target_info = self.target_codec.get_info();
        let converted_frame = if source_frame.sample_rate != target_info.sample_rate
            || source_frame.channels != target_info.channels
        {
            trace!(
                "Converting format: {}Hz/{}ch -> {}Hz/{}ch",
                source_frame.sample_rate,
                source_frame.channels,
                target_info.sample_rate,
                target_info.channels
            );

            // Use FormatConverter's public API
            let conversion_params = ConversionParams::new(
                SampleRate::from_hz(target_info.sample_rate).unwrap_or(SampleRate::Rate8000),
                target_info.channels,
            );

            let conversion_result = self
                .format_converter
                .convert_frame(&source_frame, &conversion_params)?;
            conversion_result.frame
        } else {
            source_frame
        };

        // Step 3: Encode to target format
        let encoded = self.target_codec.encode(&converted_frame)?;

        trace!(
            "Transcoded {} bytes -> {} bytes",
            encoded_data.len(),
            encoded.len()
        );
        Ok(encoded)
    }
}

/// Utility functions for common transcoding scenarios
impl Transcoder {
    /// Opus at its conventional payload type, 48 kHz stereo.
    ///
    /// The convenience helpers below assume PT 111 means Opus, which is a
    /// convention and not a fact — a real session uses whatever it negotiated,
    /// and PT 111 is something else on plenty of calls. Naming the assumption
    /// here is the point: it used to live inside `create_codec`'s `111 =>`
    /// arm, where it read as a definition.
    #[cfg(feature = "opus")]
    fn conventional_opus() -> AudioCodecSpec {
        AudioCodecSpec::new("opus", 111, 48_000, 2)
    }

    /// Transcode G.711 PCMU to Opus
    #[cfg(feature = "opus")]
    pub async fn pcmu_to_opus(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        let from = spec_for(0)?;
        self.transcode_between(pcmu_data, &from, &Self::conventional_opus())
            .await
    }

    /// Transcode Opus to G.711 PCMU
    #[cfg(feature = "opus")]
    pub async fn opus_to_pcmu(&mut self, opus_data: &[u8]) -> Result<Vec<u8>> {
        let to = spec_for(0)?;
        self.transcode_between(opus_data, &Self::conventional_opus(), &to)
            .await
    }

    /// Transcode G.711 PCMU to PCMA
    pub async fn pcmu_to_pcma(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcmu_data, 0, 8).await
    }

    /// Transcode G.711 PCMA to PCMU
    pub async fn pcma_to_pcmu(&mut self, pcma_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcma_data, 8, 0).await
    }

    /// Transcode G.711 PCMU to G.729
    pub async fn pcmu_to_g729(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcmu_data, 0, 18).await
    }

    /// Transcode G.729 to G.711 PCMU
    pub async fn g729_to_pcmu(&mut self, g729_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(g729_data, 18, 0).await
    }

    /// Transcode G.729 to Opus
    #[cfg(all(feature = "g729", feature = "opus"))]
    pub async fn g729_to_opus(&mut self, g729_data: &[u8]) -> Result<Vec<u8>> {
        let from = spec_for(18)?;
        self.transcode_between(g729_data, &from, &Self::conventional_opus())
            .await
    }

    /// Transcode Opus to G.729
    #[cfg(all(feature = "g729", feature = "opus"))]
    pub async fn opus_to_g729(&mut self, opus_data: &[u8]) -> Result<Vec<u8>> {
        let to = spec_for(18)?;
        self.transcode_between(opus_data, &Self::conventional_opus(), &to)
            .await
    }

    /// Transcode raw 16 kHz mono PCM to G.711 PCMU.
    pub async fn pcm_s16le_to_pcmu(&mut self, pcm_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcm_data, PCM_S16LE, 0).await
    }

    /// Transcode G.711 PCMU to raw 16 kHz mono PCM.
    pub async fn pcmu_to_pcm_s16le(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcmu_data, 0, PCM_S16LE).await
    }

    /// Transcode raw 16 kHz mono PCM to Opus.
    pub async fn pcm_s16le_to_opus(&mut self, pcm_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcm_data, PCM_S16LE, 111).await
    }

    /// Transcode Opus to raw 16 kHz mono PCM.
    pub async fn opus_to_pcm_s16le(&mut self, opus_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(opus_data, 111, PCM_S16LE).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_transcoder() -> Transcoder {
        Transcoder::new()
    }

    #[test]
    fn test_transcoder_creation() {
        let transcoder = create_test_transcoder();
        assert_eq!(transcoder.sessions.len(), 0);

        let paths = transcoder.get_supported_paths();
        assert!(!paths.is_empty());
        assert!(paths.iter().any(|p| p.from.name == "PCMU" && p.to.name == "PCMA"));
        #[cfg(feature = "opus")]
        assert!(paths.iter().any(|p| p.from.name == "PCMU" && p.to.name == "opus"));
        #[cfg(not(feature = "opus"))]
        assert!(!paths
            .iter()
            .any(|path| path.from.name == "opus" || path.to.name == "opus"));
        assert!(paths.iter().any(|p| p.from.name == "PCM_S16LE" && p.to.name == "PCMU"));
        #[cfg(feature = "opus")]
        assert!(paths.iter().any(|p| p.from.name == "opus" && p.to.name == "PCM_S16LE"));
    }

    #[tokio::test]
    async fn test_same_codec_transcoding() {
        let mut transcoder = create_test_transcoder();
        let test_data = vec![0x80, 0x90, 0xA0, 0xB0];

        // Same codec should return identical data
        let result = transcoder.transcode(&test_data, 0, 0).await.unwrap();
        assert_eq!(result, test_data);
    }

    #[tokio::test]
    async fn test_pcmu_to_pcma_transcoding() {
        let mut transcoder = create_test_transcoder();

        // Create test PCMU data (80 bytes for 10ms frame)
        let pcmu_data = vec![0xFF; 80]; // PCMU 10ms frame

        let result = transcoder.pcmu_to_pcma(&pcmu_data).await;
        assert!(result.is_ok());

        let pcma_data = result.unwrap();
        assert_eq!(pcma_data.len(), 80); // Same length for G.711 variants (10ms frame)
        assert_ne!(pcma_data, pcmu_data); // Should be different encoding
    }

    #[tokio::test]
    async fn test_pcm_s16le_and_pcmu_transcoding_frame_sizes() {
        let mut transcoder = create_test_transcoder();
        let pcm = (0..320)
            .map(|sample| ((sample as f32 * 0.2).sin() * 10_000.0) as i16)
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();

        let pcmu = transcoder.pcm_s16le_to_pcmu(&pcm).await.unwrap();
        assert_eq!(pcmu.len(), 160);

        let decoded = transcoder.pcmu_to_pcm_s16le(&pcmu).await.unwrap();
        assert_eq!(decoded.len(), 640);
    }

    #[tokio::test]
    async fn test_transcoding_statistics() {
        let mut transcoder = create_test_transcoder();
        let test_data = vec![0xFF; 80]; // 10ms frame

        // Perform many transcodings to get measurable timing
        // (Our G.711 optimizations are so fast we need more iterations!)
        for _ in 0..100 {
            transcoder.pcmu_to_pcma(&test_data).await.unwrap();
        }

        let stats = transcoder.get_stats(0, 8).unwrap();
        assert_eq!(stats.frames_transcoded, 100);
        assert_eq!(stats.errors, 0);
        // Even with optimizations, 100 transcodings should be measurable.
        // `avg_processing_time_us` is `f64`; the lower bound check is
        // documentation. `total_processing_time_us` is `u64` so the
        // tautological `>= 0` check has been removed.
        assert!(stats.avg_processing_time_us >= 0.0);
    }

    #[tokio::test]
    async fn test_unsupported_codec() {
        let mut transcoder = create_test_transcoder();
        let test_data = vec![0x80, 0x90];

        // Try to transcode to unsupported codec
        let result = transcoder.transcode(&test_data, 0, 99).await;
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::UnsupportedPayloadType { payload_type: 99 })
            ));
        }
    }

    #[test]
    fn test_session_management() {
        let mut transcoder = create_test_transcoder();

        // Create a session
        transcoder.get_or_create_session(0, 8).unwrap();
        assert_eq!(transcoder.sessions.len(), 1);

        // Reuse existing session
        transcoder.get_or_create_session(0, 8).unwrap();
        assert_eq!(transcoder.sessions.len(), 1);

        // Create different session
        transcoder.get_or_create_session(8, 0).unwrap();
        assert_eq!(transcoder.sessions.len(), 2);

        // Clear sessions
        transcoder.clear_sessions();
        assert_eq!(transcoder.sessions.len(), 0);
    }

    #[cfg(feature = "g729")]
    #[tokio::test]
    async fn test_g729_transcoding() {
        let mut transcoder = create_test_transcoder();

        // Create test G.729 data (10 bytes for one frame)
        let g729_data = vec![0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x10, 0x20];

        let result = transcoder.g729_to_pcmu(&g729_data).await;
        if let Err(e) = &result {
            println!("G.729 to PCMU error: {:?}", e);
        }
        assert!(result.is_ok());

        let pcmu_data = result.unwrap();
        println!(
            "G.729 -> PCMU: {} bytes -> {} bytes",
            g729_data.len(),
            pcmu_data.len()
        );
        assert_eq!(pcmu_data.len(), 80); // G.729 10ms -> G.711 10ms (both 80 samples)

        // Test reverse transcoding with properly sized PCMU data (80 bytes for 10ms)
        let pcmu_test_data = vec![0xFF; 80]; // PCMU 10ms frame
        let result = transcoder.pcmu_to_g729(&pcmu_test_data).await;
        if let Err(e) = &result {
            println!("PCMU to G.729 error: {:?}", e);
        }
        assert!(result.is_ok());

        let transcoded_g729 = result.unwrap();
        println!(
            "PCMU -> G.729: {} bytes -> {} bytes",
            pcmu_test_data.len(),
            transcoded_g729.len()
        );
        assert!(
            matches!(transcoded_g729.len(), 0 | 2 | 10),
            "G.729AB payload should be no-data, SID, or speech"
        );
    }

    /// AMR transcoding, tested by properties rather than by bytes.
    ///
    /// A transcoded signal is lossy twice over and cannot be compared against
    /// anything exactly, so these assert what a broken path fails: the tone
    /// survives, the frame accounting holds, and a perturbed input moves the
    /// output. The last is the one that stops this passing vacuously — a
    /// transcoder that returned a fixed buffer would satisfy the first two.
    #[cfg(all(feature = "amr-nb", feature = "amr-wb"))]
    mod amr {
        use super::*;
        use crate::codec::spec::AudioCodecSpec;
        use crate::types::AudioFrame;

        fn amr_nb() -> AudioCodecSpec {
            AudioCodecSpec::new("AMR", 96, 8_000, 1).with_fmtp(Some("octet-align=1"))
        }

        fn amr_wb() -> AudioCodecSpec {
            AudioCodecSpec::new("AMR-WB", 97, 16_000, 1).with_fmtp(Some("octet-align=1"))
        }

        /// A 440 Hz tone, one 20 ms frame at `rate`.
        fn tone(rate: u32, frame: usize) -> Vec<i16> {
            let samples = rate as usize / 50;
            (0..samples)
                .map(|i| {
                    let t = ((frame * samples + i) as f64) / f64::from(rate);
                    (t * 440.0 * std::f64::consts::TAU).sin().mul_add(8000.0, 0.0) as i16
                })
                .collect()
        }

        /// The Goertzel magnitude of `freq` in `samples`, normalised by length.
        fn tone_strength(samples: &[i16], rate: u32, freq: f64) -> f64 {
            let n = samples.len() as f64;
            if n < 8.0 {
                return 0.0;
            }
            let k = (freq / f64::from(rate) * n).round();
            let coeff = 2.0 * (std::f64::consts::TAU * k / n).cos();
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for &x in samples {
                let s = f64::from(x) + coeff * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            (s1 * s1 + s2 * s2 - coeff * s1 * s2).sqrt().max(0.0) / n
        }

        /// Encode PCM with `spec`'s codec, one frame at a time.
        fn encode_all(spec: &AudioCodecSpec, frames: usize) -> Vec<Vec<u8>> {
            let mut codec = spec.build().expect("builds");
            (0..frames)
                .map(|f| {
                    let pcm = tone(spec.clock_rate, f);
                    codec
                        .encode(&AudioFrame::new(pcm, spec.clock_rate, 1, 0))
                        .expect("encodes")
                })
                .collect()
        }

        /// Transcode a stream and return the decoded output PCM.
        async fn transcode_stream(
            from: &AudioCodecSpec,
            to: &AudioCodecSpec,
            payloads: &[Vec<u8>],
        ) -> (Vec<i16>, usize) {
            let mut transcoder = Transcoder::new();
            let mut decoder = to.build().expect("builds");
            let mut pcm = Vec::new();
            let mut produced = 0usize;
            for payload in payloads {
                let out = transcoder
                    .transcode_between(payload, from, to)
                    .await
                    .expect("transcodes");
                produced += 1;
                pcm.extend_from_slice(&decoder.decode(&out).expect("decodes").samples);
            }
            (pcm, produced)
        }

        /// Every pair involving AMR carries a tone across, in both directions.
        #[tokio::test]
        async fn every_amr_pair_carries_a_tone() {
            let pairs: Vec<(AudioCodecSpec, AudioCodecSpec)> = vec![
                (amr_nb(), AudioCodecSpec::new("PCMU", 0, 8_000, 1)),
                (AudioCodecSpec::new("PCMU", 0, 8_000, 1), amr_nb()),
                (amr_wb(), AudioCodecSpec::new("PCMA", 8, 8_000, 1)),
                (AudioCodecSpec::new("PCMA", 8, 8_000, 1), amr_wb()),
                (amr_nb(), amr_wb()),
                (amr_wb(), amr_nb()),
            ];

            for (from, to) in pairs {
                // Twenty frames: the codecs need a few to converge, and the
                // first is always the weakest.
                let payloads = encode_all(&from, 20);
                let (pcm, produced) = transcode_stream(&from, &to, &payloads).await;

                assert_eq!(produced, payloads.len(), "{} -> {}: frame count", from.name, to.name);
                assert_eq!(
                    pcm.len(),
                    payloads.len() * to.frame_samples_20ms(),
                    "{} -> {}: sample count",
                    from.name,
                    to.name
                );

                // Measure over the settled tail, and against the noise floor
                // at an unrelated frequency rather than an absolute number.
                let tail = &pcm[pcm.len() / 2..];
                let signal = tone_strength(tail, to.clock_rate, 440.0);
                let noise = tone_strength(tail, to.clock_rate, 1_900.0);
                assert!(
                    signal > noise * 4.0,
                    "{} -> {}: 440 Hz ({signal:.1}) did not stand out from 1900 Hz ({noise:.1})",
                    from.name,
                    to.name
                );
            }
        }

        /// Perturbing the input moves the output.
        ///
        /// Without this the tone test above passes for a transcoder that
        /// ignores its input entirely and emits a fixed buffer — which is
        /// exactly what a wrongly-wired path tends to look like, because
        /// silence and a constant both decode to something tone-free.
        #[tokio::test]
        async fn a_changed_input_changes_the_output() {
            let from = amr_nb();
            let to = amr_wb();

            let mut baseline = encode_all(&from, 10);
            let (plain, _) = transcode_stream(&from, &to, &baseline).await;

            // Re-encode with a different signal rather than flipping a bit:
            // a flipped codec bit may be a legal index that decodes near the
            // original, while a different waveform must not.
            let mut codec = from.build().expect("builds");
            baseline = (0..10)
                .map(|f| {
                    let pcm: Vec<i16> = tone(from.clock_rate, f)
                        .iter()
                        .map(|&s| s.saturating_mul(-1))
                        .collect();
                    codec
                        .encode(&AudioFrame::new(pcm, from.clock_rate, 1, 0))
                        .expect("encodes")
                })
                .collect();
            let (perturbed, _) = transcode_stream(&from, &to, &baseline).await;

            assert_eq!(plain.len(), perturbed.len());
            assert_ne!(plain, perturbed, "the transcoder ignored its input");
        }

        /// A payload from the wrong variant is refused, not decoded as the
        /// other one.
        ///
        /// AMR-NB's 12.2 kbit/s frame and AMR-WB's 8.85 are both 31 octets, so
        /// a length check alone cannot tell them apart — and decoding one as
        /// the other produces speech-shaped noise rather than an error.
        #[tokio::test]
        async fn a_wrong_variant_payload_is_refused() {
            let nb = amr_nb();
            let wb = amr_wb();
            let nb_payloads = encode_all(&nb, 4);

            let mut transcoder = Transcoder::new();
            let pcmu = AudioCodecSpec::new("PCMU", 0, 8_000, 1);
            for payload in &nb_payloads {
                // Fed to a wideband session, which must not accept it.
                assert!(
                    transcoder.transcode_between(payload, &wb, &pcmu).await.is_err(),
                    "a narrowband payload decoded as wideband"
                );
            }
            // And the right variant still works, so the refusal is about the
            // variant rather than about the payload being malformed.
            assert!(transcoder
                .transcode_between(&nb_payloads[0], &nb, &pcmu)
                .await
                .is_ok());
        }

        /// Two AMR legs differing only in framing get different sessions.
        #[tokio::test]
        async fn framing_is_part_of_the_session_key() {
            let aligned = amr_nb();
            let efficient =
                AudioCodecSpec::new("AMR", 96, 8_000, 1).with_fmtp(Some("octet-align=0"));
            let pcmu = AudioCodecSpec::new("PCMU", 0, 8_000, 1);

            let mut transcoder = Transcoder::new();
            let payload = encode_all(&aligned, 1).remove(0);
            transcoder
                .transcode_between(&payload, &aligned, &pcmu)
                .await
                .expect("the aligned session transcodes");

            let other = encode_all(&efficient, 1).remove(0);
            transcoder
                .transcode_between(&other, &efficient, &pcmu)
                .await
                .expect("the efficient session transcodes");

            // Two sessions, not one reused with the wrong framing.
            assert!(transcoder.get_stats_for(&aligned, &pcmu).is_some());
            assert!(transcoder.get_stats_for(&efficient, &pcmu).is_some());
            assert_ne!(aligned, efficient);
        }
    }

    #[cfg(feature = "g729")]
    #[test]
    fn test_g729_transcoding_paths() {
        let transcoder = create_test_transcoder();
        let paths = transcoder.get_supported_paths();

        // Paths are named by codec now, not by payload type -- a dynamic
        // payload type does not identify one.
        let has = |from: &str, to: &str| {
            paths
                .iter()
                .any(|p| p.from.name.eq_ignore_ascii_case(from) && p.to.name.eq_ignore_ascii_case(to))
        };
        assert!(has("G729", "PCMU"));
        assert!(has("PCMU", "G729"));
        #[cfg(feature = "opus")]
        {
            assert!(has("G729", "opus"));
            assert!(has("opus", "G729"));
        }
    }

    /// Full Opus<->G.711 round trip through the real codecs, exercising BOTH
    /// converted directions: PCMU (8 kHz mono) -> Opus (48 kHz stereo)
    /// [up-sample + up-mix] and back [down-mix + down-sample]. A 1 kHz tone
    /// must survive (this path had no test before, and is where the resampler
    /// bug lived). Requires the `opus` feature (Opus is off by default, like G.729).
    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn pcmu_opus_roundtrip_preserves_tone() {
                use crate::types::AudioFrame;

        fn goertzel_mag(samples: &[i16], sample_rate: f64, freq: f64) -> f64 {
            let n = samples.len() as f64;
            let k = (freq / sample_rate * n).round();
            let coeff = 2.0 * (2.0 * std::f64::consts::PI * k / n).cos();
            let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
            for &x in samples {
                let s = x as f64 + coeff * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            (s1 * s1 + s2 * s2 - coeff * s1 * s2).sqrt()
        }

        let frames = 10usize;
        let spf = 160usize; // 20 ms @ 8 kHz
        let mut pcmu = crate::codec::factory::CodecFactory::create_codec_default(0).unwrap();
        let mut transcoder = create_test_transcoder();

        let mut recovered: Vec<i16> = Vec::new();
        for f in 0..frames {
            let samples: Vec<i16> = (0..spf)
                .map(|j| {
                    let i = (f * spf + j) as f64;
                    (10_000.0 * (2.0 * std::f64::consts::PI * 1000.0 * i / 8000.0).sin()) as i16
                })
                .collect();
            let frame = AudioFrame::new(samples, 8000, 1, (f * spf) as u32);
            let pcmu_in = pcmu.encode(&frame).unwrap();
            // PCMU(8k mono) -> Opus(48k stereo): up-sample + up-mix + encode.
            let opus = transcoder.pcmu_to_opus(&pcmu_in).await.unwrap();
            // Opus(48k stereo) -> PCMU(8k mono): decode + down-mix + down-sample.
            let pcmu_out = transcoder.opus_to_pcmu(&opus).await.unwrap();
            let decoded = pcmu.decode(&pcmu_out).unwrap();
            recovered.extend_from_slice(&decoded.samples);
        }

        // Skip warm-up frames (codec + filter state), analyze steady state.
        let steady = &recovered[2 * spf..];
        let tone = goertzel_mag(steady, 8000.0, 1000.0);
        let off = goertzel_mag(steady, 8000.0, 3000.0);
        assert!(
            tone > off * 3.0,
            "1 kHz tone lost through Opus<->G.711 round trip: tone={tone}, off={off}"
        );
    }
}
