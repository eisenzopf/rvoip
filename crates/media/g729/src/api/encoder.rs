#[cfg(feature = "annex_b")]
use crate::annex_b::{dtx::DtxState, vad::VadState};
use crate::api::config::EncoderConfig;
use crate::api::frame::FrameType;
#[cfg(feature = "itu_serial")]
use crate::bitstream::itu_params::{unpack_sid_params, unpack_speech_params};
#[cfg(feature = "annex_b")]
use crate::codec::encode::encode_annex_b_frame;
use crate::codec::encode::encode_speech_frame;
use crate::codec::state::EncoderState;
#[cfg(feature = "itu_serial")]
use crate::constants::PRM_SIZE;
use crate::error::CodecError;
use crate::{FRAME_SAMPLES, SPEECH_FRAME_BYTES};

/// Stateful G.729 encoder instance.
pub struct G729Encoder {
    state: EncoderState,
    config: EncoderConfig,
    #[cfg(feature = "annex_b")]
    vad: VadState,
    #[cfg(feature = "annex_b")]
    dtx: DtxState,
}

impl G729Encoder {
    /// Create a new encoder with the supplied runtime configuration.
    pub fn new(config: impl Into<EncoderConfig>) -> Self {
        Self {
            state: EncoderState::default(),
            config: config.into(),
            #[cfg(feature = "annex_b")]
            vad: VadState::default(),
            #[cfg(feature = "annex_b")]
            dtx: DtxState::default(),
        }
    }

    /// Encode one 80-sample frame into packed output bytes.
    ///
    /// Returns the frame classification for Annex B mode.
    pub fn encode(
        &mut self,
        pcm: &[i16; FRAME_SAMPLES],
        output: &mut [u8; SPEECH_FRAME_BYTES],
    ) -> FrameType {
        #[cfg(feature = "annex_b")]
        {
            if self.config.annex_b {
                let (frame_type, bits) =
                    encode_annex_b_frame(&mut self.state, &mut self.vad, &mut self.dtx, pcm);
                *output = bits;
                return frame_type;
            }
        }

        *output = encode_speech_frame(&mut self.state, pcm);
        FrameType::Speech
    }

    /// Encode an arbitrary PCM slice and validate frame length.
    pub fn encode_frame(&mut self, pcm: &[i16]) -> Result<[u8; SPEECH_FRAME_BYTES], CodecError> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(CodecError::InvalidPcmLength {
                expected: FRAME_SAMPLES,
                got: pcm.len(),
            });
        }

        let mut frame = [0i16; FRAME_SAMPLES];
        frame.copy_from_slice(pcm);
        let mut out = [0u8; SPEECH_FRAME_BYTES];
        let _ = self.encode(&frame, &mut out);
        Ok(out)
    }

    /// Encode one frame and expose ITU-serial style parameters.
    ///
    /// `ana[0]` receives the frame type code (0=no-data, 1=speech, 2=SID).
    /// `ana[1..]` receives frame parameters according to that type.
    #[cfg(feature = "itu_serial")]
    pub fn encode_parm(
        &mut self,
        pcm: &[i16; FRAME_SAMPLES],
        ana: &mut [i16],
    ) -> Result<(FrameType, usize), CodecError> {
        if ana.len() < PRM_SIZE + 1 {
            return Err(CodecError::InvalidParameterLength {
                expected: PRM_SIZE + 1,
                got: ana.len(),
            });
        }

        ana.fill(0);

        let mut bits = [0u8; SPEECH_FRAME_BYTES];
        let frame_type = self.encode(pcm, &mut bits);

        match frame_type {
            FrameType::Speech => {
                let prm = unpack_speech_params(&bits);
                ana[0] = 1;
                for i in 0..PRM_SIZE {
                    ana[i + 1] = prm[i].0;
                }
                Ok((FrameType::Speech, PRM_SIZE))
            }
            FrameType::Sid => {
                let sid = [bits[0], bits[1]];
                let prm = unpack_sid_params(&sid);
                ana[0] = 2;
                for i in 0..4 {
                    ana[i + 1] = prm[i].0;
                }
                Ok((FrameType::Sid, 4))
            }
            FrameType::NoData => {
                ana[0] = 0;
                Ok((FrameType::NoData, 0))
            }
        }
    }

    /// Reset internal encoder state to ITU initial values.
    pub fn reset(&mut self) {
        self.state = EncoderState::default();
        #[cfg(feature = "annex_b")]
        {
            self.vad = VadState::default();
            self.dtx = DtxState::default();
        }
    }

    /// Return the active encoder runtime configuration.
    pub fn config(&self) -> EncoderConfig {
        self.config
    }
}

impl Default for G729Encoder {
    fn default() -> Self {
        Self::new(EncoderConfig::default())
    }
}
