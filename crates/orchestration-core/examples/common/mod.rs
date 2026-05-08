use async_trait::async_trait;
use rvoip_media_core::types::AudioFrame;
use rvoip_orchestration_core::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn support_call(from: &str) -> Call {
    Call::inbound(CallerIdentity::new(from), "sip:support@example.com")
}

pub fn available_human(id: &str, sip_uri: &str, skills: &[&str]) -> Agent {
    let mut agent = Agent::human(id, sip_uri);
    agent.state = AgentState::Available;
    agent.skills = skills.iter().copied().map(Skill::from).collect();
    agent
}

pub fn available_ai(id: &str, runtime_id: &str, skills: &[&str]) -> Agent {
    let mut agent = Agent::voice_ai(id, runtime_id);
    agent.state = AgentState::Available;
    agent.skills = skills.iter().copied().map(Skill::from).collect();
    agent
}

pub fn fake_runtime(turns: Vec<DialogTurn>) -> VoiceAiRuntime {
    VoiceAiRuntime {
        asr: Arc::new(EmptyAsrProvider),
        tts: Arc::new(EmptyTtsProvider),
        dialog: Arc::new(ScriptedDialog::new(turns)),
        recording: None,
        config: VoiceAiRuntimeConfig::default(),
    }
}

pub fn say(text: &str) -> DialogTurn {
    DialogTurn {
        say: vec![text.to_string()],
        action: VoiceAiAction::Say {
            text: text.to_string(),
        },
        metadata: HashMap::new(),
    }
}

pub fn transfer_to_queue(queue_id: &str) -> DialogTurn {
    DialogTurn {
        say: vec!["Transferring you now.".to_string()],
        action: VoiceAiAction::TransferToQueue {
            queue_id: QueueId::from(queue_id),
        },
        metadata: HashMap::new(),
    }
}

#[derive(Debug)]
struct EmptyAsrProvider;

#[async_trait]
impl AsrProvider for EmptyAsrProvider {
    async fn start_session(&self, _config: AsrConfig) -> Result<Box<dyn AsrSession>> {
        Ok(Box::new(EmptyAsrSession))
    }
}

struct EmptyAsrSession;

#[async_trait]
impl AsrSession for EmptyAsrSession {
    async fn push_audio(&mut self, _frame: AudioFrame) -> Result<()> {
        Ok(())
    }

    async fn next_transcript(&mut self) -> Result<Option<TranscriptEvent>> {
        Ok(None)
    }

    async fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    async fn cancel(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct EmptyTtsProvider;

#[async_trait]
impl TtsProvider for EmptyTtsProvider {
    async fn synthesize(&self, _request: TtsRequest) -> Result<Box<dyn TtsStream>> {
        Ok(Box::new(EmptyTtsStream))
    }
}

struct EmptyTtsStream;

#[async_trait]
impl TtsStream for EmptyTtsStream {
    async fn next_audio(&mut self) -> Result<Option<AudioFrame>> {
        Ok(None)
    }

    async fn cancel(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct ScriptedDialog {
    turns: Mutex<VecDeque<DialogTurn>>,
}

impl ScriptedDialog {
    fn new(turns: Vec<DialogTurn>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
        }
    }

    async fn next_turn(&self) -> DialogTurn {
        self.turns
            .lock()
            .await
            .pop_front()
            .unwrap_or_else(|| DialogTurn {
                say: Vec::new(),
                action: VoiceAiAction::Continue,
                metadata: HashMap::new(),
            })
    }
}

#[async_trait]
impl DialogManager for ScriptedDialog {
    async fn start_call(&self, _context: DialogCallContext) -> Result<DialogSessionId> {
        Ok(DialogSessionId::new())
    }

    async fn on_transcript(
        &self,
        _session_id: &DialogSessionId,
        _transcript: TranscriptEvent,
    ) -> Result<DialogTurn> {
        Ok(self.next_turn().await)
    }

    async fn on_dtmf(&self, _session_id: &DialogSessionId, _digit: char) -> Result<DialogTurn> {
        Ok(self.next_turn().await)
    }

    async fn end_call(&self, _session_id: &DialogSessionId) -> Result<()> {
        Ok(())
    }
}
