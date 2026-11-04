use crate::{
    protocol::{
        self, AudioConfiguration, FinishType, InitSession, InterruptionType, Request, EVENT_TYPE,
    },
    trace::{TraceContext, TurnTrace},
    A2aError, Result,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

pub(crate) struct Turn {
    task: String,
    artifact: String,
    started: bool,
    pub interrupted: bool,
    audio: bool,
}
pub(crate) struct Session {
    pub context: String,
    pub init: InitSession,
    pub ready: bool,
    pub finished: bool,
    pub trace: TraceContext,
    pub turn: Option<Turn>,
}
impl Session {
    pub fn new(request: Request, trace: TraceContext, rate: u32) -> Result<(Self, Value)> {
        if request.event() != Some("INIT_SESSION") || request.params.message.context_id.is_empty() {
            return Err(A2aError::Protocol);
        }
        let mut init: InitSession = request.data("initSession")?;
        init.validate()?;
        let mut response = json!({"statusCode":"200"});
        if init.is_audio() {
            let format = AudioConfiguration::pcm(rate);
            response["audioInputConfiguration"] = json!(format);
            response["audioOutputConfiguration"] = json!(format);
            init.audio_input_configuration = Some(format.clone());
            init.audio_output_configuration = Some(format);
        }
        let context = request.params.message.context_id;
        let msg = protocol::message(
            &context,
            "INIT_SESSION_RESPONSE",
            json!({"initSessionResponse":response}),
        );
        let reply = json!({"jsonrpc":"2.0","id":request.id,"result":{"message":msg}});
        Ok((
            Self {
                context,
                init,
                ready: false,
                finished: false,
                trace,
                turn: None,
            },
            reply,
        ))
    }
    fn active(&self) -> Result<()> {
        if !self.ready || self.finished {
            Err(A2aError::State)
        } else {
            Ok(())
        }
    }
    pub fn audio(&mut self, bytes: &[u8]) -> Result<Value> {
        if !self.init.is_audio() || bytes.is_empty() || bytes.len() % 2 != 0 {
            return Err(A2aError::Protocol);
        }
        self.chunk(
            json!({"raw":STANDARD.encode(bytes),"mediaType":"audio/lpcm"}),
            true,
        )
    }
    pub fn text(&mut self, text: String) -> Result<Value> {
        if self.init.is_audio() {
            return Err(A2aError::Unsupported);
        }
        self.chunk(json!({"text":text,"mediaType":"text/plain"}), false)
    }
    fn chunk(&mut self, part: Value, audio: bool) -> Result<Value> {
        self.active()?;
        let turn = self.turn.get_or_insert_with(|| Turn {
            task: protocol::id(),
            artifact: protocol::id(),
            started: false,
            interrupted: false,
            audio,
        });
        if turn.interrupted || turn.audio != audio {
            return Err(A2aError::State);
        }
        let frame = artifact(&self.context, turn, part, false);
        turn.started = true;
        Ok(frame)
    }
    pub fn interrupt(&mut self, kind: InterruptionType) -> Result<Vec<Value>> {
        self.active()?;
        let mut frames = vec![protocol::envelope(
            json!({"message":protocol::message(&self.context,"INTERRUPTION",json!({"interruption":{"type":kind,"reason":"interrupted"}}))}),
        )];
        if let Some(turn) = self.turn.as_mut() {
            if turn.audio && !turn.interrupted {
                frames.push(artifact(
                    &self.context,
                    turn,
                    json!({"raw":"","mediaType":"audio/lpcm"}),
                    true,
                ));
            }
            turn.interrupted = true;
        }
        Ok(frames)
    }
    pub fn peer_interrupt(&mut self) {
        if let Some(turn) = self.turn.as_mut() {
            turn.interrupted = true;
        }
    }
    pub fn complete(&mut self, trace: TurnTrace) -> Result<Vec<Value>> {
        self.active()?;
        let turn = self.turn.take().ok_or(A2aError::State)?;
        let mut result = Vec::new();
        // Connect-originated interruption invalidates the artifact: never send
        // lastChunk for it. Local audio interruption closes the artifact before
        // marking it interrupted (see the adapter command handler).
        if !turn.interrupted {
            let part = if turn.audio {
                json!({"raw":"","mediaType":"audio/lpcm"})
            } else {
                json!({"text":"","mediaType":"text/plain"})
            };
            result.push(artifact(&self.context, &turn, part, true));
        }
        result.push(protocol::envelope(json!({"artifactUpdate":{"contextId":self.context,"taskId":turn.task,"artifact":{"artifactId":protocol::id(),"parts":[{"data":{"tracingSpan":{"otlp":trace.otlp(&self.trace)}},"mediaType":"application/json"}],"metadata":{EVENT_TYPE:"TRACING_SPAN"}},"append":false,"lastChunk":true}})));
        result.push(protocol::envelope(json!({"statusUpdate":{"contextId":self.context,"taskId":turn.task,"final":true,"status":{"state":"TASK_STATE_COMPLETED"}}})));
        Ok(result)
    }
    pub fn finish(&mut self, kind: FinishType, reason: String) -> Result<Value> {
        self.active()?;
        if self.turn.is_some() || !self.init.supported_finish_types.contains(&kind) {
            return Err(A2aError::State);
        }
        self.finished = true;
        Ok(protocol::envelope(
            json!({"message":protocol::message(&self.context,"FINISH",json!({"finish":{"type":kind,"reason":reason}}))}),
        ))
    }
    pub fn failure(&mut self) -> Value {
        self.finished = true;
        let task = self
            .turn
            .take()
            .map(|t| t.task)
            .unwrap_or_else(protocol::id);
        protocol::envelope(
            json!({"statusUpdate":{"contextId":self.context,"taskId":task,"final":true,"status":{"state":"TASK_STATE_FAILED","message":protocol::message(&self.context,"ERROR",json!({"error":{"errorCode":500,"errorCategory":"INTERNAL_ERROR","errorReason":"External agent session failed"}}))}}}),
        )
    }
}
fn artifact(context: &str, turn: &Turn, part: Value, last: bool) -> Value {
    protocol::envelope(
        json!({"artifactUpdate":{"contextId":context,"taskId":turn.task,"artifact":{"artifactId":turn.artifact,"parts":[part],"metadata":{EVENT_TYPE:if turn.audio {"AUDIO_RESPONSE_CHUNK"} else {"TEXT_RESPONSE_CHUNK"}}},"append":turn.started,"lastChunk":last}}),
    )
}
