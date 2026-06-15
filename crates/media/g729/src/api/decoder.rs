#[cfg(feature = "annex_b")]
use crate::annex_b::cng::CngState;
use crate::api::config::DecoderConfig;
use crate::api::frame::FrameType;
#[cfg(feature = "itu_serial")]
use crate::bitstream::itu_params::{pack_sid_params, pack_speech_params};
#[cfg(feature = "annex_b")]
use crate::codec::decode::decode_annex_b_frame;
use crate::codec::decode::{decode_frame_typed, decode_speech_frame};
use crate::codec::state::DecoderState;
#[cfg(feature = "itu_serial")]
use crate::constants::PRM_SIZE;
#[cfg(feature = "itu_serial")]
use crate::dsp::Word16;
use crate::error::CodecError;
use crate::{FRAME_SAMPLES, SID_FRAME_BYTES, SPEECH_FRAME_BYTES};

/// Stateful G.729 decoder instance.
pub struct G729Decoder {
    state: DecoderState,
    config: DecoderConfig,
    consecutive_erasures: usize,
    #[cfg(feature = "annex_b")]
    cng: CngState,
}

impl G729Decoder {
    /// Create a new decoder with the supplied runtime configuration.
    pub fn new(config: impl Into<DecoderConfig>) -> Self {
        Self {
            state: DecoderState::default(),
            config: config.into(),
            consecutive_erasures: 0,
            #[cfg(feature = "annex_b")]
            cng: CngState::default(),
        }
    }

    /// Decode a packed bitstream frame and infer its type from input length.
    ///
    /// This method is intentionally tolerant: unknown payload lengths are
    /// treated as `FrameType::NoData` (erasure/no-data behavior).
    /// Use [`Self::decode_frame`] when strict input-length validation is required.
    pub fn decode(&mut self, bitstream: &[u8], output: &mut [i16; FRAME_SAMPLES]) {
        let frame_type = match bitstream.len() {
            SPEECH_FRAME_BYTES => FrameType::Speech,
            SID_FRAME_BYTES => FrameType::Sid,
            0 => FrameType::NoData,
            _ => FrameType::NoData,
        };
        self.decode_with_type(bitstream, frame_type, output);
    }

    /// Decode a frame with an explicitly provided frame type.
    pub fn decode_with_type(
        &mut self,
        bitstream: &[u8],
        frame_type: FrameType,
        output: &mut [i16; FRAME_SAMPLES],
    ) {
        let bfi = if matches!(frame_type, FrameType::NoData) {
            1
        } else {
            0
        };
        self.decode_with_type_and_bfi(bitstream, frame_type, bfi, output);
    }

    /// Decode a frame and return a new output array.
    ///
    /// This method is strict and rejects unsupported payload lengths with
    /// [`CodecError::InvalidBitstreamLength`].
    pub fn decode_frame(&mut self, data: &[u8]) -> Result<[i16; FRAME_SAMPLES], CodecError> {
        if !matches!(data.len(), 0 | SID_FRAME_BYTES | SPEECH_FRAME_BYTES) {
            return Err(CodecError::InvalidBitstreamLength {
                expected: &[0, SID_FRAME_BYTES, SPEECH_FRAME_BYTES],
                got: data.len(),
            });
        }

        let mut out = [0i16; FRAME_SAMPLES];
        self.decode(data, &mut out);
        Ok(out)
    }

    /// Decode an erasure/no-data frame.
    pub fn decode_erasure(&mut self, output: &mut [i16; FRAME_SAMPLES]) {
        self.decode_with_type(&[], FrameType::NoData, output);
    }

    /// Decode one ITU-serial style parameter frame.
    ///
    /// Expected layout:
    /// - `parm[0]`: BFI (0=good, 1=erasure)
    /// - `parm[1]`: frame type code (0=no-data, 1=speech, 2=SID)
    /// - `parm[2..]`: frame parameters
    #[cfg(feature = "itu_serial")]
    pub fn decode_parm(
        &mut self,
        parm: &mut [i16],
        output: &mut [i16; FRAME_SAMPLES],
    ) -> Result<(), CodecError> {
        if parm.len() < PRM_SIZE + 2 {
            return Err(CodecError::InvalidParameterLength {
                expected: PRM_SIZE + 2,
                got: parm.len(),
            });
        }

        let bfi = if parm[0] != 0 { 1 } else { 0 };

        match parm[1] {
            1 => {
                let mut speech = [Word16(0); PRM_SIZE];
                for i in 0..PRM_SIZE {
                    speech[i] = Word16(parm[i + 2]);
                }
                let bits = pack_speech_params(&speech);
                self.decode_with_type_and_bfi(&bits, FrameType::Speech, bfi, output);
                Ok(())
            }
            2 => {
                let mut sid = [Word16(0); 4];
                for i in 0..4 {
                    sid[i] = Word16(parm[i + 2]);
                }
                let bits = pack_sid_params(&sid);
                self.decode_with_type_and_bfi(&bits, FrameType::Sid, bfi, output);
                Ok(())
            }
            0 => {
                self.decode_with_type_and_bfi(&[], FrameType::NoData, bfi, output);
                Ok(())
            }
            code => Err(CodecError::InvalidFrameType { got: code }),
        }
    }

    /// Reset internal decoder state to ITU initial values.
    pub fn reset(&mut self) {
        self.state = DecoderState::default();
        self.consecutive_erasures = 0;
        #[cfg(feature = "annex_b")]
        {
            self.cng = CngState::default();
        }
    }

    /// Return the active decoder runtime configuration.
    pub fn config(&self) -> DecoderConfig {
        self.config
    }

    fn decode_with_type_and_bfi(
        &mut self,
        bitstream: &[u8],
        frame_type: FrameType,
        bfi: i16,
        output: &mut [i16; FRAME_SAMPLES],
    ) {
        let bfi = if bfi != 0 { 1 } else { 0 };
        self.state.post_filter_enabled = self.config.post_filter;
        let mute = self.update_erasure_streak_from_bfi(bfi);

        #[cfg(feature = "annex_b")]
        if self.config.annex_b {
            *output =
                decode_annex_b_frame(&mut self.state, &mut self.cng, frame_type, bitstream, bfi);
            if mute {
                output.fill(0);
            }
            return;
        }

        #[cfg(not(feature = "annex_b"))]
        let effective_frame_type = if bfi != 0 {
            FrameType::NoData
        } else {
            frame_type
        };
        #[cfg(feature = "annex_b")]
        let effective_frame_type = frame_type;

        match effective_frame_type {
            FrameType::Speech => {
                let mut frame = [0u8; SPEECH_FRAME_BYTES];
                let n = core::cmp::min(bitstream.len(), SPEECH_FRAME_BYTES);
                frame[..n].copy_from_slice(&bitstream[..n]);
                *output = decode_speech_frame(&mut self.state, &frame);
            }
            FrameType::Sid => {
                output.fill(0);
            }
            FrameType::NoData => {
                *output = decode_frame_typed(&mut self.state, FrameType::NoData, bitstream);
            }
        }

        if mute {
            output.fill(0);
        }
    }

    fn update_erasure_streak_from_bfi(&mut self, bfi: i16) -> bool {
        if bfi != 0 {
            self.consecutive_erasures = self.consecutive_erasures.saturating_add(1);
        } else {
            self.consecutive_erasures = 0;
        }
        self.config
            .max_consecutive_erasures
            .is_some_and(|max| self.consecutive_erasures > max)
    }
}

impl Default for G729Decoder {
    fn default() -> Self {
        Self::new(DecoderConfig::default())
    }
}
