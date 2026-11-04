//! Cisco's protobuf contract and transport-specific response framing.
//!
//! The host must authenticate each RPC, bind identity, and preserve the provider
//! session across RPCs. An RPC ending is NOT a conversation ending.
#[allow(clippy::all)]
pub mod wire {
    tonic::include_proto!("com.cisco.wcc.ccai.media.v1");
}

use wire::{output_event::EventType, voice_va_response::ResponseType, *};
pub const SCHEMA_ID: &str = "5397013b-7920-4ffc-807c-e8a3e0a18f43";
pub const MAX_MESSAGE_BYTES: usize = 128 * 1024;
pub const MIN_CHUNK_BYTES: usize = 100;
pub const MAX_CHUNK_BYTES: usize = 64_000;

/// No inferred codec defaults: the prototype accepts only Vapi's PCMU/8 kHz.
pub fn validate_request(
    request: &VoiceVaRequest,
    org: &str,
    agent: &str,
) -> Result<(), &'static str> {
    if request.customer_org_id != org
        || (!request.virtual_agent_id.is_empty() && request.virtual_agent_id != agent)
    {
        return Err("identity mismatch");
    }
    if request.conversation_id.is_empty() || request.conversation_id.len() > 256 {
        return Err("invalid conversation ID");
    }
    match request
        .voice_va_input_type
        .as_ref()
        .ok_or("missing input")?
    {
        voice_va_request::VoiceVaInputType::AudioInput(audio) => {
            if audio.encoding != voice_input::VoiceEncoding::MulawFormat as i32
                || audio.sample_rate_hertz != 8000
                || audio.caller_audio.is_empty()
                || audio.caller_audio.len() > MAX_CHUNK_BYTES
                || audio.language_code.is_empty()
            {
                return Err("invalid audio format or size");
            }
        }
        voice_va_request::VoiceVaInputType::DtmfInput(digits) => {
            dtmf_text(digits)?;
        }
        voice_va_request::VoiceVaInputType::EventInput(event) => {
            if !(1..=5).contains(&event.event_type) {
                return Err("unknown input event");
            }
        }
    }
    Ok(())
}

/// Cisco's reference connectors ignore the initial three-byte audio sentinel.
/// It opens the listening RPC during prompt playback; it is not caller speech.
pub fn is_audio_placeholder(request: &VoiceVaRequest) -> bool {
    matches!(&request.voice_va_input_type,
        Some(voice_va_request::VoiceVaInputType::AudioInput(audio))
        if audio.caller_audio.len() <= 15)
}

pub fn dtmf_text(input: &DtmfInputs) -> Result<String, &'static str> {
    if input.dtmf_events.is_empty() || input.dtmf_events.len() > 32 {
        return Err("invalid digits");
    }
    input
        .dtmf_events
        .iter()
        .map(|digit| {
            "1234567890ABCD*#"
                .chars()
                .nth((*digit as usize).wrapping_sub(1))
                .ok_or("invalid digit")
        })
        .collect()
}

pub fn response(kind: ResponseType) -> VoiceVaResponse {
    VoiceVaResponse {
        response_type: kind as i32,
        input_mode: VoiceVaInputMode::InputVoiceDtmf as i32,
        input_handling_config: Some(InputHandlingConfig {
            dtmf_config: Some(DtmfInputConfig {
                inter_digit_timeout_msec: 3000,
                termchar: DtmfDigits::DtmfDigitPound as i32,
                dtmf_input_length: 32,
            }),
            speech_timers: Some(InputSpeechTimers {
                max_speech_timeout_msec: 30_000,
                complete_timeout_msec: 1000,
                incomplete_timeout_msec: 2000,
                no_input_timeout_msec: 10_000,
            }),
        }),
        ..Default::default()
    }
}

pub fn event(kind: EventType, terminal: bool) -> VoiceVaResponse {
    let mut result = response(if terminal {
        ResponseType::Final
    } else {
        ResponseType::Partial
    });
    result.output_events.push(OutputEvent {
        event_type: kind as i32,
        ..Default::default()
    });
    result
}

/// Hold one minimum-size tail to avoid emitting an illegal final short chunk.
/// No synthetic silence is added. A whole utterance shorter than 100 B fails.
#[derive(Default)]
pub struct ChunkReply {
    target: usize,
    pending: Vec<u8>,
    finished: bool,
    bytes: usize,
}
impl ChunkReply {
    /// Aggregate raw PCMU into one-second chunks, retaining the minimum tail.
    pub fn one_second() -> Self {
        Self {
            target: 8_000,
            ..Self::default()
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<VoiceVaResponse>, &'static str> {
        if self.finished {
            return Err("reply already finished");
        }
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err("provider frame too large");
        }
        self.bytes += bytes.len();
        self.pending.extend_from_slice(bytes);
        let mut result = Vec::new();
        let target = self.target.max(MIN_CHUNK_BYTES);
        let limit = if self.target == 0 {
            MAX_CHUNK_BYTES
        } else {
            self.target
        };
        while self.pending.len() >= target + MIN_CHUNK_BYTES {
            let count = (self.pending.len() - MIN_CHUNK_BYTES).min(limit);
            result.push(chunk(self.pending.drain(..count).collect()));
        }
        Ok(result)
    }
    pub fn finish(&mut self) -> Result<Vec<VoiceVaResponse>, &'static str> {
        if self.finished {
            return Err("duplicate final");
        }
        self.finished = true;
        if self.bytes == 0 || self.pending.len() < MIN_CHUNK_BYTES {
            return Err("utterance too short");
        }
        Ok(vec![
            chunk(std::mem::take(&mut self.pending)),
            response(ResponseType::Final),
        ])
    }
}
fn chunk(audio: Vec<u8>) -> VoiceVaResponse {
    let mut result = response(ResponseType::Chunk);
    result.prompts.push(Prompt {
        audio_content: audio,
        is_barge_in_enabled: true,
        ..Default::default()
    });
    result
}

/// Streaming PCM16/8 kHz WAV prompts, each containing at most one second.
/// A final short prompt is emitted without padding, followed by one empty FINAL.
#[derive(Default)]
pub struct WavChunkReply {
    pending: Vec<u8>,
    finished: bool,
}
impl WavChunkReply {
    pub fn push(&mut self, audio: &[u8]) -> Result<Vec<VoiceVaResponse>, &'static str> {
        if self.finished || audio.len() > MAX_MESSAGE_BYTES {
            return Err("finished reply or oversized provider frame");
        }
        self.pending.extend_from_slice(audio);
        let mut messages = Vec::new();
        while self.pending.len() >= 8_000 {
            messages.push(wav_chunk(&self.pending[..8_000])?);
            self.pending.drain(..8_000);
        }
        Ok(messages)
    }
    pub fn finish(&mut self) -> Result<Vec<VoiceVaResponse>, &'static str> {
        if self.finished {
            return Err("duplicate final");
        }
        self.finished = true;
        let mut messages = Vec::new();
        if !self.pending.is_empty() {
            messages.push(wav_chunk(&self.pending)?);
            self.pending.clear();
        }
        messages.push(response(ResponseType::Final));
        Ok(messages)
    }
}
fn wav_chunk(audio: &[u8]) -> Result<VoiceVaResponse, &'static str> {
    let wav = crate::wav::pcmu_to_pcm16_wav(audio).map_err(|_| "WAV encoding failed")?;
    Ok(chunk(wav))
}

/// Bounded whole-prompt alternative, at most 60 seconds at 8 kHz.
#[derive(Default)]
pub struct WavReply {
    audio: Vec<u8>,
    finished: bool,
}
impl WavReply {
    pub fn push(&mut self, audio: &[u8]) -> Result<Vec<VoiceVaResponse>, &'static str> {
        if self.finished
            || self.audio.len().saturating_add(audio.len()) > crate::wav::MAX_WAV_SAMPLES
        {
            return Err("WAV reply finished or exceeds 60 seconds");
        }
        self.audio.extend_from_slice(audio);
        Ok(vec![])
    }
    pub fn finish(&mut self) -> Result<Vec<VoiceVaResponse>, &'static str> {
        if self.finished || self.audio.is_empty() {
            return Err("empty or finished WAV reply");
        }
        self.finished = true;
        // Cisco's reference welcome prompt is mono PCM16 at 8 kHz.
        // Decode PCMU using rvoip's G.711 implementation; never relabel bytes.
        let wav = crate::wav::pcmu_to_pcm16_wav(&self.audio).map_err(|_| "WAV encoding failed")?;
        self.audio.clear();
        let mut result = response(ResponseType::Final);
        result.prompts.push(Prompt {
            audio_content: wav,
            is_barge_in_enabled: true,
            ..Default::default()
        });
        Ok(vec![result])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_wav_preserves_samples_and_emits_before_finish() {
        for size in [1, 7_999, 8_000, 8_001, 24_321] {
            let audio: Vec<u8> = (0..size).map(|n| (n % 256) as u8).collect();
            let mut reply = WavChunkReply::default();
            let mut messages = Vec::new();
            for frame in audio.chunks(341) {
                messages.extend(reply.push(frame).unwrap());
                assert!(reply.pending.len() < 8_000);
            }
            assert_eq!(messages.len(), size / 8_000);
            messages.extend(reply.finish().unwrap());
            let final_message = messages.pop().unwrap();
            assert_eq!(final_message.response_type, ResponseType::Final as i32);
            assert!(final_message.prompts.is_empty());
            let mut samples = Vec::new();
            for message in messages {
                assert_eq!(message.response_type, ResponseType::Chunk as i32);
                let wav = &message.prompts[0].audio_content;
                assert!(wav.len() <= 16_044);
                assert_eq!(&wav[..4], b"RIFF");
                assert_eq!(
                    u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
                    wav.len() - 44
                );
                samples.extend_from_slice(&wav[44..]);
            }
            let whole = crate::wav::pcmu_to_pcm16_wav(&audio).unwrap();
            assert_eq!(samples, whole[44..]);
            assert!(reply.finish().is_err());
            assert!(reply.push(&[42]).is_err());
        }
        assert!(WavChunkReply::default()
            .push(&vec![0; MAX_MESSAGE_BYTES + 1])
            .is_err());
    }
    #[test]
    fn wav_reply_decodes_mulaw_to_pcm16_with_bounded_buffer() {
        let mut reply = WavReply::default();
        assert!(reply.push(&[42; 101]).unwrap().is_empty());
        let messages = reply.finish().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].response_type, 0);
        let wav = &messages[0].prompts[0].audio_content;
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
            wav.len() - 8
        );
        assert_eq!(&wav[20..24], &[1, 0, 1, 0]);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 202);
        assert_eq!(wav.len(), 246);
        // ITU G.711 expansion of 0x2a is -5372 (little endian 04 eb).
        assert!(wav[44..]
            .chunks_exact(2)
            .all(|sample| sample == [0x04, 0xeb]));
        assert!(reply.finish().is_err());
        assert!(WavReply::default()
            .push(&vec![0; crate::wav::MAX_WAV_SAMPLES + 1])
            .is_err());
    }
    #[test]
    fn live_webex_audio_without_timestamp_and_initial_sentinel() {
        let mut request = VoiceVaRequest {
            customer_org_id: "org".into(),
            virtual_agent_id: "agent".into(),
            conversation_id: "call".into(),
            voice_va_input_type: Some(voice_va_request::VoiceVaInputType::AudioInput(VoiceInput {
                caller_audio: vec![0xff; 3],
                encoding: 2,
                sample_rate_hertz: 8000,
                language_code: "en-US".into(),
                audio_timestamp: None,
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(validate_request(&request, "org", "agent").is_ok());
        request.virtual_agent_id.clear(); // Optional in Cisco's protobuf.
        assert!(validate_request(&request, "org", "agent").is_ok());
        request.virtual_agent_id = "other".into();
        assert!(validate_request(&request, "org", "agent").is_err());
        request.virtual_agent_id = "agent".into();
        assert!(is_audio_placeholder(&request));
        if let Some(voice_va_request::VoiceVaInputType::AudioInput(audio)) =
            &mut request.voice_va_input_type
        {
            audio.caller_audio = vec![0xff; 160];
        }
        assert!(!is_audio_placeholder(&request));
        assert!(validate_request(&request, "org", "agent").is_ok());
        if let Some(voice_va_request::VoiceVaInputType::AudioInput(audio)) =
            &mut request.voice_va_input_type
        {
            audio.encoding = 0;
        }
        assert!(validate_request(&request, "org", "agent").is_err());
    }
    #[test]
    fn chunks_preserve_audio_and_empty_final() {
        for size in [100, 199, 200, 320, 64_000, 64_099, 128_000] {
            let audio = vec![42; size];
            let mut reply = ChunkReply::default();
            let mut messages = Vec::new();
            for frame in audio.chunks(137) {
                messages.extend(reply.push(frame).unwrap());
            }
            messages.extend(reply.finish().unwrap());
            let last = messages.pop().unwrap();
            assert_eq!(last.response_type, ResponseType::Final as i32);
            assert!(last.prompts.is_empty());
            let mut actual = Vec::new();
            for message in messages {
                assert_eq!(message.response_type, ResponseType::Chunk as i32);
                let bytes = &message.prompts[0].audio_content;
                assert!((MIN_CHUNK_BYTES..=MAX_CHUNK_BYTES).contains(&bytes.len()));
                actual.extend_from_slice(bytes);
            }
            assert_eq!(actual, audio);
            assert!(reply.finish().is_err());
            assert!(reply.push(&[1; 100]).is_err());
        }
    }
    #[test]
    fn one_second_pcmu_preserves_bytes_and_retains_legal_tail() {
        for size in [100, 8_000, 8_099, 8_100, 16_500, 72_000] {
            let input: Vec<u8> = (0..size).map(|n| (n % 256) as u8).collect();
            let mut reply = ChunkReply::one_second();
            let mut messages = Vec::new();
            for frame in input.chunks(341) {
                messages.extend(reply.push(frame).unwrap());
            }
            assert_eq!(messages.len(), (size - 100) / 8_000);
            assert!(messages
                .iter()
                .all(|m| m.prompts[0].audio_content.len() == 8_000));
            messages.extend(reply.finish().unwrap());
            assert!(messages.last().unwrap().prompts.is_empty());
            let actual: Vec<u8> = messages
                .iter()
                .flat_map(|m| &m.prompts)
                .flat_map(|p| p.audio_content.iter().copied())
                .collect();
            assert_eq!(actual, input);
        }
    }
    #[test]
    fn short_utterance_is_not_padded() {
        let mut reply = ChunkReply::default();
        reply.push(&[0; 99]).unwrap();
        assert!(reply.finish().is_err());
    }
    #[test]
    fn dtmf_order_and_invalid_values() {
        assert_eq!(
            dtmf_text(&DtmfInputs {
                dtmf_events: vec![10, 1, 15, 16]
            })
            .unwrap(),
            "01*#"
        );
        assert!(dtmf_text(&DtmfInputs {
            dtmf_events: vec![0]
        })
        .is_err());
    }
}
