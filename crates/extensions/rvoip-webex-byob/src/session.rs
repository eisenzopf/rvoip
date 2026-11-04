use crate::protocol::{self, Input, ProtocolError, MAX_AUDIO_BYTES};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use serde_json::{json, Value};

const MIN_CHUNK: usize = 320;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplyId(u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AudioOutput {
    #[default]
    RawPcmu,
    /// Each prompt is a complete PCM16 WAV. Sandbox-qualified separately from raw WS.
    #[cfg(feature = "wav-compat")]
    Pcm16Wav,
    /// Emit the first legal prompt immediately, then aggregate one-second prompts.
    #[cfg(feature = "wav-compat")]
    Pcm16WavFastStart,
    /// Cisco-approved deployment override: one complete WAV on FINAL per reply.
    /// Bounded to 60 seconds; exceeds the public raw-profile chunk limit.
    #[cfg(feature = "wav-compat")]
    Pcm16WavWhole,
}

/// Own on one task; every output is returned in required serialization order.
/// Transport code must finish writing the returned batch before processing another.
pub struct Session {
    org: String,
    agent: String,
    conversation: Option<String>,
    received_seq: u64,
    sent_seq: u64,
    next_reply: u64,
    reply: Option<ReplyId>,
    pending: Vec<u8>,
    ended: bool,
    audio_output: AudioOutput,
    answer_audio_output: Option<AudioOutput>,
    emitted_audio: bool,
    start_parameters: Value,
}

impl Session {
    pub fn new(org: String, agent: String) -> Self {
        Self {
            org,
            agent,
            conversation: None,
            received_seq: 0,
            sent_seq: 0,
            next_reply: 0,
            reply: None,
            pending: Vec::new(),
            ended: false,
            audio_output: AudioOutput::RawPcmu,
            answer_audio_output: None,
            emitted_audio: false,
            start_parameters: Value::Null,
        }
    }

    /// Configure output before admission. Incoming audio remains raw PCMU/8 kHz.
    pub fn with_audio_output(mut self, audio_output: AudioOutput) -> Self {
        self.audio_output = audio_output;
        self
    }

    /// Preserve the first reply's format/buffering and configure subsequent replies.
    pub fn with_answer_audio_output(mut self, output: AudioOutput) -> Self {
        self.answer_audio_output = Some(output);
        self
    }

    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation.as_deref()
    }
    /// Optional application data, captured only from the validated SESSION_START.
    pub fn start_parameters(&self) -> &Value {
        &self.start_parameters
    }
    pub fn is_ended(&self) -> bool {
        self.ended
    }
    pub fn active_reply(&self) -> Option<ReplyId> {
        self.reply
    }

    pub fn receive(&mut self, text: &str) -> Result<Input, ProtocolError> {
        if self.ended {
            return Err(ProtocolError::State);
        }
        let envelope = protocol::parse(text)?;
        if self
            .conversation
            .as_ref()
            .is_some_and(|id| id != &envelope.conversation_id)
        {
            return Err(ProtocolError::Identity);
        }
        if envelope.kind == "PONG" {
            if self.conversation.is_none() || envelope.payload.is_some() {
                return Err(ProtocolError::State);
            }
            return Ok(Input::Pong(envelope.seq));
        }
        if envelope.seq <= self.received_seq {
            return Err(ProtocolError::Sequence);
        }
        if envelope.kind == "PING" {
            if self.conversation.is_none() || envelope.payload.is_some() {
                return Err(ProtocolError::State);
            }
            self.received_seq = envelope.seq;
            return Ok(Input::Ping(envelope.seq));
        }
        if envelope.kind != "VOICE_VA_REQUEST" {
            return Err(ProtocolError::Envelope);
        }
        let payload = envelope.payload.ok_or(ProtocolError::Envelope)?;
        if payload.conversation_id != envelope.conversation_id
            || payload.customer_org_id != self.org
            || payload
                .virtual_agent_id
                .as_ref()
                .is_some_and(|id| id != &self.agent)
        {
            return Err(ProtocolError::Identity);
        }
        if payload.additional_info.len() > 32
            || payload
                .additional_info
                .iter()
                .any(|(k, v)| k.len() > 128 || v.len() > 4096)
            || payload
                .vendor_specific_config
                .as_ref()
                .is_some_and(|v| v.len() > 256)
        {
            return Err(ProtocolError::Limit);
        }
        // The protocol engine never emits PARTIAL, even when permitted.
        let _allow_partial = payload.allow_partial_responses;
        let parameters = payload
            .voice_va_input_type
            .event_input
            .as_ref()
            .filter(|e| e.event_type == "SESSION_START")
            .map(|e| e.parameters.clone())
            .unwrap_or(Value::Null);
        if !parameters.is_null()
            && (!parameters.is_object() || parameters.to_string().len() > 16_384)
        {
            return Err(ProtocolError::Limit);
        }
        let input = protocol::input(payload.voice_va_input_type)?;
        if self.conversation.is_none() {
            if !matches!(input, Input::Start) {
                return Err(ProtocolError::State);
            }
            self.conversation = Some(envelope.conversation_id);
            self.start_parameters = parameters;
        } else if matches!(input, Input::Start) {
            return Err(ProtocolError::State);
        }
        self.received_seq = envelope.seq;
        if matches!(input, Input::End) {
            self.ended = true;
            self.reply = None;
            self.pending.clear();
        }
        Ok(input)
    }

    fn envelope(&mut self, kind: &str) -> Result<Value, ProtocolError> {
        let id = self.conversation.as_ref().ok_or(ProtocolError::State)?;
        self.sent_seq = self
            .sent_seq
            .checked_add(1)
            .ok_or(ProtocolError::Sequence)?;
        Ok(
            json!({"type":kind,"seq":self.sent_seq,"ts":Utc::now().to_rfc3339(),
            "conversation_id":id,"metadata":{}}),
        )
    }

    fn response(&mut self, mut payload: Value) -> Result<Value, ProtocolError> {
        // Explicit schema defaults avoid null collections in downstream bridges.
        // FINAL may still contain no audio; it ends only this logical reply.
        let object = payload.as_object_mut().ok_or(ProtocolError::Envelope)?;
        object.entry("prompts").or_insert_with(|| json!([]));
        object.entry("output_events").or_insert_with(|| json!([]));
        object
            .entry("input_mode")
            .or_insert_with(|| json!("INPUT_VOICE_DTMF"));
        object
            .entry("input_sensitive")
            .or_insert_with(|| json!(false));
        object
            .entry("disable_prompt_cancellation")
            .or_insert_with(|| json!(false));
        let mut message = self.envelope("VOICE_VA_RESPONSE")?;
        message["payload"] = payload;
        Ok(message)
    }

    pub fn pong(&self, seq: u64) -> Result<Value, ProtocolError> {
        Ok(json!({"type":"PONG","seq":seq,"ts":Utc::now().to_rfc3339(),
            "conversation_id":self.conversation.as_ref().ok_or(ProtocolError::State)?}))
    }

    pub fn ping(&mut self) -> Result<Value, ProtocolError> {
        self.envelope("PING")
    }

    pub fn error(&mut self, code: &str, status: u16) -> Result<Value, ProtocolError> {
        let mut value = self.envelope("ERROR")?;
        value["code"] = json!(code);
        value["status"] = json!(status);
        value["detail"] = json!("Connector could not continue this session");
        self.ended = true;
        self.reply = None;
        self.pending.clear();
        Ok(value)
    }

    pub fn begin_reply(&mut self) -> Result<ReplyId, ProtocolError> {
        if self.ended || self.conversation.is_none() || self.reply.is_some() {
            return Err(ProtocolError::State);
        }
        if self.next_reply > 0 {
            if let Some(output) = self.answer_audio_output {
                self.audio_output = output;
            }
        }
        self.emitted_audio = false;
        self.next_reply = self.next_reply.checked_add(1).ok_or(ProtocolError::State)?;
        let id = ReplyId(self.next_reply);
        self.reply = Some(id);
        Ok(id)
    }

    fn check_reply(&self, id: ReplyId) -> Result<(), ProtocolError> {
        if self.ended || self.reply != Some(id) {
            return Err(ProtocolError::StaleReply);
        }
        Ok(())
    }

    fn audio_message(&mut self, bytes: &[u8]) -> Result<Value, ProtocolError> {
        let encoded = match self.audio_output {
            AudioOutput::RawPcmu => STANDARD.encode(bytes),
            #[cfg(feature = "wav-compat")]
            AudioOutput::Pcm16Wav | AudioOutput::Pcm16WavFastStart | AudioOutput::Pcm16WavWhole => {
                STANDARD.encode(crate::wav::pcmu_to_pcm16_wav(bytes)?)
            }
        };
        self.response(
            json!({"response_type":"CHUNK","input_mode":"INPUT_VOICE_DTMF",
            "disable_prompt_cancellation":self.audio_output != AudioOutput::RawPcmu,
            "prompts":[{"audio_content_b64":encoded,"is_barge_in_enabled":true}]}),
        )
    }

    pub fn audio(&mut self, id: ReplyId, bytes: &[u8]) -> Result<Vec<Value>, ProtocolError> {
        self.check_reply(id)?;
        if bytes.len() > 262_144 {
            return Err(ProtocolError::Limit);
        }
        #[cfg(feature = "wav-compat")]
        if self.audio_output == AudioOutput::Pcm16WavWhole {
            if self.pending.len().saturating_add(bytes.len()) > crate::wav::MAX_WAV_SAMPLES {
                return Err(ProtocolError::Limit);
            }
            self.pending.extend_from_slice(bytes);
            return Ok(vec![]);
        }
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        // A complete WAV per tiny provider frame creates hundreds of separate
        // playback prompts. Aggregate the compatibility path to one second of
        // samples, retaining the minimum tail for a valid audio-bearing FINAL.
        // Raw streaming retains its existing behavior. No playback clock/sleep.
        loop {
            let (target, limit) = match self.audio_output {
                AudioOutput::RawPcmu => (MIN_CHUNK, MAX_AUDIO_BYTES),
                #[cfg(feature = "wav-compat")]
                AudioOutput::Pcm16Wav => (8_000, 8_000),
                #[cfg(feature = "wav-compat")]
                AudioOutput::Pcm16WavFastStart if !self.emitted_audio => (MIN_CHUNK, 8_000),
                #[cfg(feature = "wav-compat")]
                AudioOutput::Pcm16WavFastStart => (8_000, 8_000),
                #[cfg(feature = "wav-compat")]
                AudioOutput::Pcm16WavWhole => unreachable!("whole replies buffer above"),
            };
            if self.pending.len() < target + MIN_CHUNK {
                break;
            }
            let count = (self.pending.len() - MIN_CHUNK).min(limit);
            let tail = self.pending.split_off(count);
            let ready = std::mem::replace(&mut self.pending, tail);
            output.push(self.audio_message(&ready)?);
            self.emitted_audio = true;
        }
        Ok(output)
    }

    /// The caller supplies an authoritative completion; this method never guesses it.
    pub fn finish(&mut self, id: ReplyId) -> Result<Vec<Value>, ProtocolError> {
        self.check_reply(id)?;
        if !self.pending.is_empty() && self.pending.len() < MIN_CHUNK {
            return Err(ProtocolError::ShortAudio);
        }
        let mut final_message = if !self.pending.is_empty() {
            let bytes = std::mem::take(&mut self.pending);
            self.audio_message(&bytes)?
        } else {
            self.response(json!({"response_type":"FINAL"}))?
        };
        // Carry the retained tail on FINAL itself. This preserves byte order
        // and avoids a separate empty completion after the last audio chunk.
        final_message["payload"]["response_type"] = json!("FINAL");
        self.reply = None;
        Ok(vec![final_message])
    }

    /// Prototype ordering: START_OF_INPUT then cancelled FINAL. Cisco qualification pending.
    pub fn interrupt(&mut self) -> Result<Vec<Value>, ProtocolError> {
        let mut output = vec![self.event("START_OF_INPUT", None)?];
        if self.reply.take().is_some() {
            self.pending.clear();
            output.push(self.response(json!({"response_type":"FINAL"}))?);
        }
        Ok(output)
    }

    pub fn event(&mut self, event: &str, transcript: Option<&str>) -> Result<Value, ProtocolError> {
        if self.ended
            || !matches!(
                event,
                "START_OF_INPUT" | "END_OF_INPUT" | "NO_INPUT" | "NO_MATCH"
            )
        {
            return Err(ProtocolError::State);
        }
        let mut payload = json!({"response_type":"CHUNK","output_events":[{"event_type":event}]});
        if let Some(text) = transcript {
            if text.len() > 8192 {
                return Err(ProtocolError::Limit);
            }
            payload["session_transcript"] = json!({"text":text});
        }
        self.response(payload)
    }

    pub fn terminal(&mut self, handoff: bool) -> Result<Vec<Value>, ProtocolError> {
        if self.ended {
            return Ok(Vec::new());
        }
        let mut messages = Vec::new();
        if let Some(id) = self.reply {
            messages.extend(self.finish(id)?);
        }
        let event = if handoff {
            "TRANSFER_TO_AGENT"
        } else {
            "SESSION_END"
        };
        messages.push(
            self.response(json!({"response_type":"FINAL","output_events":[{"event_type":event}]}))?,
        );
        self.ended = true;
        Ok(messages)
    }

    /// Commit one application result and end the virtual-agent leg. A final audio
    /// tail, if present, shares this FINAL; no second FINAL is emitted for it.
    pub fn complete_with_metadata(
        &mut self,
        name: &str,
        metadata: Value,
    ) -> Result<Vec<Value>, ProtocolError> {
        if self.ended || self.conversation.is_none() {
            return Err(ProtocolError::State);
        }
        if name.is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || !metadata.is_object()
            || metadata.to_string().len() > 16_384
        {
            return Err(ProtocolError::Limit);
        }
        let mut message = if let Some(id) = self.reply {
            self.finish(id)?.remove(0)
        } else {
            self.response(json!({"response_type":"FINAL"}))?
        };
        message["payload"]["output_events"] = json!([
            {"event_type":"CUSTOM_EVENT","name":name,"metadata":metadata},
            {"event_type":"SESSION_END","metadata":metadata}
        ]);
        self.ended = true;
        Ok(vec![message])
    }
}
